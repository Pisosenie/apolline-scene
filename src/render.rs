//! A scene's layers drawn with OpenGL ES 3, or desktop OpenGL 3.3, into whatever framebuffer
//! the host has bound. The scene covers the target the way a wallpaper does: scaled to fill,
//! centred and cropped.

mod build;
mod chain;
mod compile;
mod geometry;
mod material;
mod state;
mod swarm;
mod target;
mod upload;

use geometry::toward;
pub use geometry::{Affine, Fit, place, place_fitted};
use upload::{reserve, upload};

use glow::HasContext;

use crate::Error;
use crate::project::{Blending, Layer};
use crate::scene::{Parallax, Scene};
use crate::shader::Constant;

/// How far a layer of depth 1 travels at the pointer's furthest, as a share of the scene,
/// per unit of the scene's parallax amount.
const PARALLAX_REACH: f32 = 0.05;

const VERTEX: &str = "
uniform vec3 row0;
uniform vec3 row1;
uniform vec2 uv_max;
out vec2 uv;
void main() {
    vec2 corner = vec2(float(gl_VertexID & 1), float((gl_VertexID >> 1) & 1));
    vec3 local = vec3(corner - 0.5, 1.0);
    gl_Position = vec4(dot(row0, local), dot(row1, local), 0.0, 1.0);
    // Textures keep their top row first.
    uv = vec2(corner.x, 1.0 - corner.y) * uv_max;
}";

const FRAGMENT: &str = "
uniform sampler2D picture;
uniform vec4 tint;
uniform int picture_channels;
in vec2 uv;
out vec4 color;
void main() {
    vec4 pixel = texture(picture, uv);
    if (picture_channels == 1) {
        pixel = vec4(pixel.rrr, 1.0);
    } else if (picture_channels == 2) {
        pixel = vec4(pixel.rrr, pixel.g);
    }
    color = pixel * tint;
}";

struct Input {
    slot: u8,
    texture: glow::Texture,
    resolution: [f32; 4],
}

/// Material values with where they go in their program.
type Values = Vec<(Option<glow::UniformLocation>, Constant)>;

/// One draw of a quad: which program, what it reads and the values it's given.
struct Step {
    program: usize,
    inputs: Vec<Input>,
    channels: i32,
    uv_max: [f32; 2],
    constants: Values,
}

struct Drawn {
    step: Step,
    /// The layer's effects, drawn first; `step` then places their result.
    chain: Option<chain::Chain>,
    /// Particles drawn with `step`'s texture instead of a picture; `origin` places the
    /// system and `size` scales it.
    swarm: Option<swarm::Swarm>,
    origin: [f32; 2],
    size: (f32, f32),
    angle: f32,
    tint: [f32; 4],
    blending: Blending,
    depth: [f32; 2],
}

struct Program {
    id: glow::Program,
    vao: glow::VertexArray,
    row0: Option<glow::UniformLocation>,
    row1: Option<glow::UniformLocation>,
    uv_max: Option<glow::UniformLocation>,
    mvp: Option<glow::UniformLocation>,
    layer_size: Option<glow::UniformLocation>,
    tint: Option<glow::UniformLocation>,
    channels: Option<glow::UniformLocation>,
    frame: Option<material::Frame>,
    resolution: [Option<glow::UniformLocation>; 8],
}

pub struct Renderer {
    programs: Vec<Program>,
    layers: Vec<Drawn>,
    textures: Vec<glow::Texture>,
    texture_bytes: usize,
    size: (f32, f32),
    clear: [f32; 3],
    parallax: Option<Parallax>,
    /// Where the parallax has got to on its way to the pointer, -1..1 across the target.
    eye: [f32; 2],
    inputs: Box<material::Inputs>,
}

/// What a renderer's shaders read that only the host can give.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Needs {
    pub pointer: bool,
    pub daytime: bool,
    pub audio: bool,
}

impl Renderer {
    /// Uploads textures and builds the material programs. Hidden layers are left out.
    ///
    /// # Safety
    /// `gl` has to be current on this thread.
    pub unsafe fn new(gl: &glow::Context, scene: &Scene, layers: &[Layer]) -> Result<Self, Error> {
        let program = unsafe { program(gl, VERTEX, FRAGMENT)? };
        let mut renderer = Renderer {
            programs: vec![program],
            layers: Vec::new(),
            textures: Vec::new(),
            texture_bytes: 0,
            size: scene.size,
            clear: scene.clear,
            parallax: scene.parallax,
            eye: [0.0; 2],
            inputs: Box::default(),
        };
        let mut builder = build::Builder::new(gl);
        // Making effect targets binds framebuffers; the host gets its own back.
        let host = unsafe { state::Host::save(gl) };
        let prepared = (|| {
            if layers.len() > 4096 {
                return Err(Error::Format("renderer exceeds 4096 layers".into()));
            }
            for layer in layers.iter().filter(|layer| layer.image.visible) {
                let drawn = unsafe { builder.layer(&mut renderer, scene, layer)? };
                renderer.layers.push(drawn);
            }
            Ok(())
        })();
        unsafe { host.restore(gl) };
        if let Err(error) = prepared {
            unsafe { renderer.delete(gl) };
            return Err(error);
        }
        renderer.texture_bytes = builder.allocated;
        Ok(renderer)
    }

    /// Logical bytes in unique uploaded mipmaps and effect targets, excluding driver overhead.
    pub fn texture_bytes(&self) -> usize {
        self.texture_bytes
    }

    /// The number of distinct textures uploaded, regardless of how many layers use them.
    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    /// Whether an active GPU program or a particle system needs time or frame delta updates.
    pub fn animated(&self) -> bool {
        self.layers.iter().any(|layer| layer.swarm.is_some())
            || self.programs.iter().any(|program| {
                program
                    .frame
                    .as_ref()
                    .is_some_and(material::Frame::animated)
            })
    }

    /// Moves the parallax towards the pointer, -1..1 across the target with y up, taking the
    /// scene's delay to get there. Returns whether it moved.
    pub fn follow(&mut self, pointer: [f32; 2], seconds: f32) -> bool {
        let across = |at: [f32; 2]| [(at[0] + 1.0) / 2.0, (1.0 - at[1]) / 2.0];
        let pointed = across(pointer);
        let moved = pointed != self.inputs.pointer && self.needs().pointer;
        self.inputs.pointer = pointed;
        let Some(parallax) = self.parallax else {
            return moved;
        };
        let before = self.eye;
        self.eye = toward(self.eye, pointer, seconds, parallax.delay);
        self.inputs.parallax = across(self.eye);
        moved
            || before
                .iter()
                .zip(self.eye)
                .any(|(was, now)| (was - now).abs() > 1e-4)
    }

    /// How far through the local day it is, 0 to 1, for shaders that change with it.
    pub fn set_daytime(&mut self, daytime: f32) {
        if daytime.is_finite() {
            self.inputs.daytime = daytime.rem_euclid(1.0);
        }
    }

    /// The audio spectrum, left and right, in 64 bins from low to high and about 0 to 1.
    pub fn set_spectrum(&mut self, left: &[f32; 64], right: &[f32; 64]) {
        let clean =
            |bins: &[f32; 64]| bins.map(|bin| if bin.is_finite() { bin.max(0.0) } else { 0.0 });
        self.inputs.spectrum = [clean(left), clean(right)];
    }

    /// What the scene's shaders read that the host has to supply: the pointer through
    /// `follow`, the time of day and the audio spectrum.
    pub fn needs(&self) -> Needs {
        let frames = || {
            self.programs
                .iter()
                .filter_map(|program| program.frame.as_ref())
        };
        Needs {
            pointer: self.parallax.is_some() || frames().any(material::Frame::points),
            daytime: frames().any(|frame| frame.daytime.is_some()),
            audio: frames().any(material::Frame::listens),
        }
    }

    /// Draws at zero time/delta into the bound framebuffer, `target` pixels in size.
    ///
    /// # Safety
    /// `gl` has to be current on this thread, the one the renderer was made on.
    pub unsafe fn draw(&self, gl: &glow::Context, target: (u32, u32)) {
        unsafe { self.draw_fitted(gl, target, Fit::Cover) };
    }

    /// Draws at zero time/delta with cropping or black letterboxing; layers are clipped.
    ///
    /// # Safety
    /// `gl` has to be current on this thread, the one the renderer was made on.
    pub unsafe fn draw_fitted(&self, gl: &glow::Context, target: (u32, u32), fit: Fit) {
        unsafe { self.draw_timed(gl, target, fit, 0.0, 0.0) };
    }

    /// Draws at host-owned scene time; paused hosts keep time fixed and use zero delta.
    ///
    /// # Safety
    /// The renderer's GL context must be current on this thread.
    pub unsafe fn draw_timed(
        &self,
        gl: &glow::Context,
        target: (u32, u32),
        fit: Fit,
        time: f32,
        delta: f32,
    ) {
        let shift = self.parallax.map_or([0.0; 2], |parallax| {
            let reach = PARALLAX_REACH * parallax.amount * parallax.mouse;
            [
                -self.eye[0] * reach * self.size.0,
                -self.eye[1] * reach * self.size.1,
            ]
        });
        unsafe {
            let host = state::Host::save(gl);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::CULL_FACE);
            gl.disable(glow::STENCIL_TEST);
            gl.color_mask(true, true, true, true);
            gl.blend_equation(glow::FUNC_ADD);
            for chain in self.layers.iter().filter_map(|layer| layer.chain.as_ref()) {
                chain.run(gl, &self.programs, target, time, delta, &self.inputs);
            }
            host.bind(gl);
            gl.viewport(0, 0, target.0 as i32, target.1 as i32);
            if fit == Fit::Contain {
                gl.clear_color(0.0, 0.0, 0.0, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
                let scale = (target.0 as f32 / self.size.0).min(target.1 as f32 / self.size.1);
                let w = (self.size.0 * scale).round() as i32;
                let h = (self.size.1 * scale).round() as i32;
                gl.enable(glow::SCISSOR_TEST);
                gl.scissor((target.0 as i32 - w) / 2, (target.1 as i32 - h) / 2, w, h);
            }
            gl.clear_color(self.clear[0], self.clear[1], self.clear[2], 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            // Layer alpha blends RGB over the scene background; the final scene is opaque.
            gl.color_mask(true, true, true, false);
            let mut current_program = None;
            for layer in &self.layers {
                let program = &self.programs[layer.step.program];
                if current_program != Some(layer.step.program) {
                    gl.use_program(Some(program.id));
                    gl.bind_vertex_array(Some(program.vao));
                    if let Some(frame) = &program.frame {
                        frame.update(gl, target, target, time, delta, &self.inputs);
                    }
                    current_program = Some(layer.step.program);
                }
                let moved = [shift[0] * layer.depth[0], shift[1] * layer.depth[1]];
                let origin = [layer.origin[0] + moved[0], layer.origin[1] + moved[1]];
                let (affine, size) = match &layer.swarm {
                    // Particles are placed in scene units over the whole scene.
                    Some(_) => {
                        let centre = [self.size.0 / 2.0 + moved[0], self.size.1 / 2.0 + moved[1]];
                        let whole = place_fitted(self.size, target, centre, self.size, 0.0, fit);
                        (whole, self.size)
                    }
                    None => (
                        place_fitted(self.size, target, origin, layer.size, layer.angle, fit),
                        layer.size,
                    ),
                };
                match layer.blending {
                    Blending::Opaque => gl.disable(glow::BLEND),
                    Blending::Translucent => {
                        gl.enable(glow::BLEND);
                        gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                    }
                    Blending::Additive => {
                        gl.enable(glow::BLEND);
                        gl.blend_func(glow::SRC_ALPHA, glow::ONE);
                    }
                }
                bind(gl, program, &layer.step, &affine, size, layer.tint, None);
                match &layer.swarm {
                    Some(swarm) => {
                        let scale = [layer.size.0, layer.size.1];
                        let count = swarm.update(gl, time, layer.origin, scale);
                        gl.bind_vertex_array(Some(swarm.vao));
                        gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, count);
                        // The next layer may share the program but not these instances.
                        current_program = None;
                    }
                    None => gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4),
                }
            }
            gl.disable(glow::BLEND);
            gl.disable(glow::SCISSOR_TEST);
            gl.color_mask(true, true, true, true);
            gl.bind_vertex_array(None);
            host.restore(gl);
        }
    }

    /// # Safety
    /// `gl` has to be current on this thread, the one the renderer was made on.
    pub unsafe fn delete(self, gl: &glow::Context) {
        unsafe {
            for texture in self.textures {
                gl.delete_texture(texture);
            }
            for chain in self.layers.iter().filter_map(|layer| layer.chain.as_ref()) {
                chain.delete(gl);
            }
            for swarm in self.layers.iter().filter_map(|layer| layer.swarm.as_ref()) {
                swarm.delete(gl);
            }
            for program in self.programs {
                gl.delete_vertex_array(program.vao);
                gl.delete_program(program.id);
            }
        }
    }
}

/// Sets what a step reads for one quad placed by `affine`, `size` in the units its vertex
/// shader's positions are in. `before` is the picture an effect pass draws over.
///
/// # Safety
/// The GL context and the step's program must be current on this thread.
unsafe fn bind(
    gl: &glow::Context,
    program: &Program,
    step: &Step,
    affine: &Affine,
    size: (f32, f32),
    tint: [f32; 4],
    before: Option<&Input>,
) {
    unsafe {
        gl.uniform_3_f32_slice(program.row0.as_ref(), &affine[0]);
        gl.uniform_3_f32_slice(program.row1.as_ref(), &affine[1]);
        gl.uniform_2_f32_slice(program.uv_max.as_ref(), &step.uv_max);
        if program.mvp.is_some() {
            gl.uniform_matrix_4_f32_slice(
                program.mvp.as_ref(),
                false,
                &material::model_view_projection(affine, size),
            );
        }
        gl.uniform_2_f32(program.layer_size.as_ref(), size.0, size.1);
        gl.uniform_4_f32_slice(program.tint.as_ref(), &tint);
        gl.uniform_1_i32(program.channels.as_ref(), step.channels);
        for (location, value) in &step.constants {
            material::set(gl, location.as_ref(), value);
        }
        for input in before.into_iter().chain(&step.inputs) {
            gl.active_texture(glow::TEXTURE0 + u32::from(input.slot));
            gl.bind_sampler(u32::from(input.slot), None);
            gl.bind_texture(glow::TEXTURE_2D, Some(input.texture));
            gl.uniform_4_f32_slice(
                program.resolution[input.slot as usize].as_ref(),
                &input.resolution,
            );
        }
        gl.active_texture(glow::TEXTURE0);
    }
}

/// The shaders' first lines: GLSL ES 3 on GLES, GLSL 3.3 on desktop OpenGL.
fn header(gl: &glow::Context) -> &'static str {
    if gl.version().is_embedded {
        "#version 300 es\nprecision highp float;\nprecision highp int;\n"
    } else {
        "#version 330 core\n"
    }
}

/// One of our own programs: the plain layer's, or the particle sprites'.
unsafe fn program(gl: &glow::Context, vertex: &str, fragment: &str) -> Result<Program, Error> {
    let fail = |what: String| Error::Format(format!("the layer shader: {what}"));
    unsafe {
        let id = compile::link(gl, vertex, fragment)?;
        let result = (|| {
            let uniform = |name| gl.get_uniform_location(id, name);
            let row0 = uniform("row0");
            let row1 = uniform("row1");
            let uv_max = uniform("uv_max");
            let tint = uniform("tint");
            let sampler = uniform("picture");
            let vao = gl.create_vertex_array().map_err(fail)?;
            gl.use_program(Some(id));
            gl.uniform_1_i32(sampler.as_ref(), 0);
            Ok(Program {
                id,
                vao,
                row0,
                row1,
                uv_max,
                mvp: None,
                layer_size: uniform("apolline_layer_size"),
                tint,
                channels: uniform("picture_channels"),
                frame: None,
                resolution: std::array::from_fn(|_| None),
            })
        })();
        if result.is_err() {
            gl.delete_program(id);
        }
        result
    }
}
