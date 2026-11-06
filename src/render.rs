//! A scene's layers drawn with OpenGL ES 3, or desktop OpenGL 3.3, into whatever framebuffer
//! the host has bound. The scene covers the target the way a wallpaper does: scaled to fill,
//! centred and cropped.

mod build;
mod chain;
mod compile;
mod compose;
mod geometry;
mod material;
mod scripted;
mod sprite;
mod state;
mod swarm;
mod target;
mod upload;
mod words;

use geometry::toward;
pub use geometry::{Affine, Fit, place, place_fitted};
use upload::{reserve, upload};

use glow::HasContext;

use crate::Error;
use crate::project::{Blending, Layer};
use crate::scene::{Parallax, Pose, Scene};
use crate::script::{Moment, Stage};
use crate::shader::Constant;

/// How far a layer of depth 1 travels at the pointer's furthest, as a share of the scene,
/// per unit of the scene's parallax amount.
const PARALLAX_REACH: f32 = 0.05;

const VERTEX: &str = "
uniform vec3 row0;
uniform vec3 row1;
// Where the picture's top left corner is in the texture, and its top and left sides.
uniform vec2 uv_at;
uniform vec4 uv_sides;
out vec2 uv;
void main() {
    vec2 corner = vec2(float(gl_VertexID & 1), float((gl_VertexID >> 1) & 1));
    vec3 local = vec3(corner - 0.5, 1.0);
    gl_Position = vec4(dot(row0, local), dot(row1, local), 0.0, 1.0);
    // Textures keep their top row first.
    vec2 across = vec2(corner.x, 1.0 - corner.y);
    uv = uv_at + across.x * uv_sides.xy + across.y * uv_sides.zw;
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

#[derive(Clone)]
struct Input {
    slot: u8,
    texture: glow::Texture,
    resolution: [f32; 4],
}

/// Material values with where they go in their program.
type Values = Vec<(Option<glow::UniformLocation>, Constant)>;

/// One draw of a quad: which program, what it reads and the values it's given.
#[derive(Clone)]
struct Step {
    program: usize,
    inputs: Vec<Input>,
    channels: i32,
    uv_max: [f32; 2],
    /// The frame of a sprite sheet it draws, as `[corner, across, down]` in the texture, in
    /// place of the picture inside `uv_max`.
    sheet: Option<[f32; 6]>,
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
    /// Which of the scene's objects it is, which scripts may move, show and hide.
    object: usize,
    base: Base,
    visible: bool,
    /// Where its object was last put, which a change of size places it from again.
    posed: Pose,
    /// A text layer's words, which scripts may change.
    words: Option<words::Words>,
    /// The frames of a sprite sheet, which play as time goes on.
    sprite: Option<sprite::Sprite>,
    /// A composition layer, whose chain reads the scene drawn so far under it.
    compose: bool,
}

/// How a layer is placed from where its object is.
#[derive(Clone, Copy)]
enum Base {
    /// `size` before scaling, `anchor` where its origin is on it, and what its material
    /// multiplies its brightness and alpha by.
    Picture {
        size: (f32, f32),
        anchor: [f32; 2],
        shade: [f32; 2],
    },
    /// Covering the scene, whatever its object says.
    Whole { scene: (f32, f32), shade: [f32; 2] },
    /// A particle system: the object's origin and scale place and size it, and its material
    /// brightens it.
    Particles { shade: [f32; 2] },
}

impl Drawn {
    fn pose(&mut self, pose: &Pose) {
        self.posed = *pose;
        self.visible = pose.visible;
        self.angle = pose.angle;
        let shaded = |shade: [f32; 2]| {
            let light = pose.brightness * shade[0];
            [
                pose.color[0] * light,
                pose.color[1] * light,
                pose.color[2] * light,
                pose.alpha * shade[1],
            ]
        };
        match self.base {
            Base::Picture {
                size,
                anchor,
                shade,
            } => {
                let size = (size.0 * pose.scale[0], size.1 * pose.scale[1]);
                // The origin is the anchor's place; the layer's centre is that far back from it.
                let (sin, cos) = pose.angle.sin_cos();
                let [ax, ay] = [anchor[0] * size.0, anchor[1] * size.1];
                self.origin = [
                    pose.origin[0] - (cos * ax - sin * ay),
                    pose.origin[1] - (sin * ax + cos * ay),
                ];
                self.size = size;
                self.tint = shaded(shade);
            }
            Base::Whole { scene, shade } => {
                self.origin = [scene.0 / 2.0, scene.1 / 2.0];
                self.size = scene;
                self.tint = shaded(shade);
            }
            Base::Particles { shade } => {
                self.origin = [pose.origin[0], pose.origin[1]];
                self.size = (pose.scale[0], pose.scale[1]);
                self.tint = [shade[0], shade[0], shade[0], 1.0];
            }
        }
    }
}

struct Program {
    id: glow::Program,
    vao: glow::VertexArray,
    row0: Option<glow::UniformLocation>,
    row1: Option<glow::UniformLocation>,
    uv_max: Option<glow::UniformLocation>,
    uv_at: Option<glow::UniformLocation>,
    uv_sides: Option<glow::UniformLocation>,
    mvp: Option<glow::UniformLocation>,
    /// The inverses of where the layer is on screen, in the pass's own units and in its
    /// picture's.
    unproject: Option<glow::UniformLocation>,
    unproject_picture: Option<glow::UniformLocation>,
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
    shake: Option<crate::scene::Shake>,
    /// How far in front of the scene its camera is, in scene pixels, for what's drawn in
    /// perspective.
    camera: f32,
    /// Where the parallax has got to on its way to the pointer, -1..1 across the target.
    eye: [f32; 2],
    inputs: Box<material::Inputs>,
    stage: Option<Box<Stage>>,
    /// Where the scripts' say over effects lands.
    wiring: scripted::Wiring,
    /// The objects whose finished pictures other layers' effects read.
    composited: std::collections::BTreeSet<usize>,
    /// The frame drawn so far, copied for composition layers.
    frame: compose::Frame,
    /// The size of the last target drawn to, which scripts see as the screen.
    screen: std::cell::Cell<(u32, u32)>,
}

/// What a renderer's shaders read that only the host can give.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Needs {
    pub pointer: bool,
    pub daytime: bool,
    pub audio: bool,
}

impl Renderer {
    /// Uploads textures and builds the material programs. Hidden layers are kept, for scripts
    /// to show, but not drawn.
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
            shake: scene.shake,
            camera: scene.size.1 / 2.0 / (scene.fov.to_radians() / 2.0).tan(),
            eye: [0.0; 2],
            inputs: Box::default(),
            stage: None,
            wiring: scripted::Wiring::default(),
            composited: std::collections::BTreeSet::new(),
            frame: compose::Frame::default(),
            screen: std::cell::Cell::new((scene.size.0 as u32, scene.size.1 as u32)),
        };
        let mut builder = build::Builder::new(gl);
        builder.composited = scene.composited();
        renderer.composited = builder.composited.clone();
        // Making effect targets binds framebuffers; the host gets its own back.
        let host = unsafe { state::Host::save(gl) };
        let prepared = (|| {
            if layers.len() > 4096 {
                return Err(Error::Format("renderer exceeds 4096 layers".into()));
            }
            for layer in layers {
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
        renderer.wiring = scripted::Wiring::new(scene, &renderer.layers);
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

    /// Runs the scene's scripts from here on, each `animate`.
    pub fn script(&mut self, stage: Stage) {
        self.stage = Some(Box::new(stage));
    }

    /// Runs a frame of the scene's scripts and timelines, at the host's scene time, and
    /// moves, shows and hides layers and their effects, gives effects new values and changes
    /// words the way they say; sprite sheets turn to the frame for that time. Hosts call it
    /// before each `draw_timed`.
    ///
    /// # Safety
    /// `gl` has to be current on this thread, the one the renderer was made on.
    pub unsafe fn animate(&mut self, gl: &glow::Context, time: f32, delta: f32) {
        for layer in &mut self.layers {
            layer.play(time);
        }
        let Some(stage) = self.stage.as_mut() else {
            return;
        };
        let poses: &[Pose] = stage.tick(Moment {
            time: f64::from(time),
            delta,
            daytime: self.inputs.daytime,
            screen: self.screen.get(),
            spectrum: Some(&self.inputs.spectrum),
        });
        for layer in &mut self.layers {
            if let Some(pose) = poses.get(layer.object) {
                layer.pose(pose);
            }
        }
        let (shown, values) = stage.effects();
        self.wiring.apply(&mut self.layers, shown, values);
        for (object, text) in stage.written() {
            for layer in self
                .layers
                .iter_mut()
                .filter(|layer| layer.object == object)
            {
                unsafe { layer.retext(gl, &text, &mut self.texture_bytes) };
            }
        }
    }

    /// Whether the scene changes as time goes on: a program, particle system, sprite sheet or
    /// script that needs time or frame delta updates. A scene that only follows the pointer
    /// says so in `needs`, and changes when `follow` says it moved.
    pub fn animated(&self) -> bool {
        self.stage.is_some()
            || self.shake.is_some()
            || self
                .layers
                .iter()
                .any(|layer| layer.swarm.is_some() || layer.sprite.is_some())
            || self
                .programs
                .iter()
                .any(|program| program.frame.as_ref().is_some_and(material::Frame::timed))
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
        let (daytime, audio) = self
            .stage
            .as_ref()
            .map_or((false, false), |stage| stage.needs());
        Needs {
            pointer: self.parallax.is_some() || frames().any(material::Frame::points),
            daytime: daytime || frames().any(|frame| frame.daytime.is_some()),
            audio: audio || frames().any(material::Frame::listens),
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
        let mut shift = self.parallax.map_or([0.0; 2], |parallax| {
            let reach = PARALLAX_REACH * parallax.amount * parallax.mouse;
            [
                -self.eye[0] * reach * self.size.0,
                -self.eye[1] * reach * self.size.1,
            ]
        });
        // A shaking camera moves everything the way parallax at full depth does.
        if let Some(shake) = self.shake {
            let [x, y] = geometry::shaken(shake, time);
            shift = [shift[0] + x * self.size.0, shift[1] + y * self.size.1];
        }
        self.screen.set(target);
        unsafe {
            let host = state::Host::save(gl);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::CULL_FACE);
            gl.disable(glow::STENCIL_TEST);
            gl.color_mask(true, true, true, true);
            gl.blend_equation(glow::FUNC_ADD);
            // Layers whose pictures others read go first, shown or not, so theirs are ready.
            let mut pictures: Vec<(usize, glow::Texture, (u32, u32))> = Vec::new();
            for read in [true, false] {
                for layer in &self.layers {
                    let wanted = self.composited.contains(&layer.object);
                    // A composition layer's chain waits for what's drawn under it.
                    let Some(chain) = layer
                        .chain
                        .as_ref()
                        .filter(|_| wanted == read && !layer.compose)
                    else {
                        continue;
                    };
                    if !layer.visible && !wanted {
                        continue;
                    }
                    let (placed, _) = self.placed(layer, shift, target, fit);
                    let at = chain::Moment {
                        screen: target,
                        time,
                        delta,
                        placed,
                        own: None,
                    };
                    chain.run(gl, &self.programs, at, &self.inputs, &pictures);
                    if wanted {
                        pictures.push((layer.object, chain.result(), chain.size()));
                    }
                }
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
            for layer in self.layers.iter().filter(|layer| layer.visible) {
                if layer.compose
                    && let Some(chain) = &layer.chain
                {
                    // What's been drawn so far, through the layer's effects, then back here.
                    let (placed, _) = self.placed(layer, shift, target, fit);
                    let Some(frame) = self.frame.copy(gl, target) else {
                        continue;
                    };
                    gl.disable(glow::SCISSOR_TEST);
                    gl.color_mask(true, true, true, true);
                    let at = chain::Moment {
                        screen: target,
                        time,
                        delta,
                        placed,
                        own: Some((frame, compose::reading(&placed))),
                    };
                    chain.run(gl, &self.programs, at, &self.inputs, &pictures);
                    host.bind(gl);
                    gl.viewport(0, 0, target.0 as i32, target.1 as i32);
                    if fit == Fit::Contain {
                        gl.enable(glow::SCISSOR_TEST);
                    }
                    gl.color_mask(true, true, true, false);
                    current_program = None;
                }
                let program = &self.programs[layer.step.program];
                if current_program != Some(layer.step.program) {
                    gl.use_program(Some(program.id));
                    gl.bind_vertex_array(Some(program.vao));
                    if let Some(frame) = &program.frame {
                        frame.update(gl, target, target, time, delta, &self.inputs);
                    }
                    current_program = Some(layer.step.program);
                }
                let (affine, size) = self.placed(layer, shift, target, fit);
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
                bind(
                    gl,
                    program,
                    &layer.step,
                    &affine,
                    &affine,
                    size,
                    layer.tint,
                    &[],
                );
                match &layer.swarm {
                    Some(swarm) => {
                        let scale = [layer.size.0, layer.size.1, layer.size.0];
                        let middle = [self.size.0 / 2.0, self.size.1 / 2.0];
                        let camera = (middle, self.camera);
                        let count =
                            swarm.update(gl, time, layer.origin, scale, layer.angle, camera);
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

    /// Where a layer is on a target this size, with the parallax `shift`, and its size.
    fn placed(
        &self,
        layer: &Drawn,
        shift: [f32; 2],
        target: (u32, u32),
        fit: Fit,
    ) -> (Affine, (f32, f32)) {
        let moved = [shift[0] * layer.depth[0], shift[1] * layer.depth[1]];
        match &layer.swarm {
            // Particles are placed in scene units over the whole scene.
            Some(_) => {
                let centre = [self.size.0 / 2.0 + moved[0], self.size.1 / 2.0 + moved[1]];
                let whole = place_fitted(self.size, target, centre, self.size, 0.0, fit);
                (whole, self.size)
            }
            None => {
                let origin = [layer.origin[0] + moved[0], layer.origin[1] + moved[1]];
                (
                    place_fitted(self.size, target, origin, layer.size, layer.angle, fit),
                    layer.size,
                )
            }
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
            self.frame.delete(gl);
        }
    }
}

/// Sets what a step reads for one quad placed by `affine`, `size` in the units its vertex
/// shader's positions are in. `before` is the picture an effect pass draws over.
///
/// # Safety
/// The GL context and the step's program must be current on this thread.
/// `affine` draws the step and `placed` is where its layer is on screen, which is the same
/// unless the step draws into one of the layer's effect targets.
#[allow(clippy::too_many_arguments)]
unsafe fn bind(
    gl: &glow::Context,
    program: &Program,
    step: &Step,
    affine: &Affine,
    placed: &Affine,
    size: (f32, f32),
    tint: [f32; 4],
    before: &[Input],
) {
    unsafe {
        if program.unproject.is_some() {
            let back = material::inverse(material::model_view_projection(placed, size));
            gl.uniform_matrix_4_f32_slice(program.unproject.as_ref(), false, &back);
        }
        if program.unproject_picture.is_some() {
            let back = material::inverse(material::picture_projection(placed));
            gl.uniform_matrix_4_f32_slice(program.unproject_picture.as_ref(), false, &back);
        }
        gl.uniform_3_f32_slice(program.row0.as_ref(), &affine[0]);
        gl.uniform_3_f32_slice(program.row1.as_ref(), &affine[1]);
        gl.uniform_2_f32_slice(program.uv_max.as_ref(), &step.uv_max);
        let [x, y, across_x, across_y, down_x, down_y] =
            step.sheet
                .unwrap_or([0.0, 0.0, step.uv_max[0], 0.0, 0.0, step.uv_max[1]]);
        gl.uniform_2_f32(program.uv_at.as_ref(), x, y);
        gl.uniform_4_f32(
            program.uv_sides.as_ref(),
            across_x,
            across_y,
            down_x,
            down_y,
        );
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
        for input in before.iter().chain(&step.inputs) {
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
            let uv_at = uniform("uv_at");
            let uv_sides = uniform("uv_sides");
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
                uv_at,
                uv_sides,
                mvp: None,
                unproject: None,
                unproject_picture: None,
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
