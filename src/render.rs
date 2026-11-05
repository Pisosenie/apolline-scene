//! A scene's layers drawn with OpenGL ES 3, or desktop OpenGL 3.3, into whatever framebuffer
//! the host has bound. The scene covers the target the way a wallpaper does: scaled to fill,
//! centred and cropped.

mod compile;
mod geometry;
mod material;
mod state;
mod upload;

use geometry::toward;
pub use geometry::{Affine, Fit, place, place_fitted};
use upload::{reserve, upload};

use std::collections::HashMap;
use std::sync::Arc;

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

struct Drawn {
    program: usize,
    inputs: Vec<Input>,
    channels: i32,
    uv_max: [f32; 2],
    origin: [f32; 2],
    size: (f32, f32),
    angle: f32,
    tint: [f32; 4],
    blending: Blending,
    depth: [f32; 2],
    constants: Vec<(Option<glow::UniformLocation>, Constant)>,
}

struct Program {
    id: glow::Program,
    vao: glow::VertexArray,
    row0: Option<glow::UniformLocation>,
    row1: Option<glow::UniformLocation>,
    uv_max: Option<glow::UniformLocation>,
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
}

impl Renderer {
    /// Uploads textures and builds the material programs. Hidden layers are left out.
    ///
    /// # Safety
    /// `gl` has to be current on this thread.
    pub unsafe fn new(gl: &glow::Context, scene: &Scene, layers: &[Layer]) -> Result<Self, Error> {
        let program = unsafe { program(gl)? };
        let mut renderer = Renderer {
            programs: vec![program],
            layers: Vec::new(),
            textures: Vec::new(),
            texture_bytes: 0,
            size: scene.size,
            clear: scene.clear,
            parallax: scene.parallax,
            eye: [0.0; 2],
        };
        let mut textures = HashMap::new();
        let mut variants = HashMap::new();
        let mut source_bytes = 0usize;
        let mut allocated = 0;
        let prepared = (|| {
            if layers.len() > 4096 {
                return Err(Error::Format("renderer exceeds 4096 layers".into()));
            }
            for layer in layers.iter().filter(|layer| layer.image.visible) {
                let count = 1 + layer.secondary.len();
                if count > 8 || (layer.fragment.is_none() && count != 1) {
                    return Err(Error::Format("unsupported number of layer textures".into()));
                }
                let mut bindings = 1u8;
                for input in &layer.secondary {
                    if input.slot == 0 || input.slot >= 8 || bindings & (1 << input.slot) != 0 {
                        return Err(Error::Format(
                            "duplicate or invalid secondary sampler slot".into(),
                        ));
                    }
                    bindings |= 1 << input.slot;
                }
                if let Some(fragment) = &layer.fragment {
                    fragment.validate_inputs(bindings)?;
                }
                let mut inputs = Vec::with_capacity(count);
                let mut uv_max = [1.0; 2];
                let mut channels = 4;
                for (slot, input) in std::iter::once((0, &layer.texture)).chain(
                    layer
                        .secondary
                        .iter()
                        .map(|input| (input.slot, &input.texture)),
                ) {
                    let key = Arc::as_ptr(input);
                    let (texture, uv) = match textures.get(&key) {
                        Some(uploaded) => *uploaded,
                        None => {
                            let bytes = input
                                .images
                                .first()
                                .into_iter()
                                .flatten()
                                .try_fold(0usize, |total, mip| total.checked_add(mip.pixels.len()))
                                .ok_or_else(|| {
                                    Error::Format("texture byte count overflow".into())
                                })?;
                            reserve(&mut allocated, bytes)?;
                            let uploaded = unsafe { upload(gl, input)? };
                            renderer.textures.push(uploaded.0);
                            textures.insert(key, uploaded);
                            uploaded
                        }
                    };
                    let first = input
                        .images
                        .first()
                        .and_then(|image| image.first())
                        .ok_or_else(|| Error::Format("layer has no texture image".into()))?;
                    if slot == 0 {
                        uv_max = uv;
                        channels = first.channels.count() as i32;
                    }
                    inputs.push(Input {
                        slot,
                        texture,
                        resolution: [
                            first.width as f32,
                            first.height as f32,
                            input.image_width.min(first.width) as f32,
                            input.image_height.min(first.height) as f32,
                        ],
                    });
                }
                let mut program = 0;
                let mut constants = Vec::new();
                if let Some(fragment) = &layer.fragment {
                    program = match variants.get(fragment.source.as_str()) {
                        Some(index) => *index,
                        None => {
                            if variants.len() >= 64
                                || fragment.source.len()
                                    > (16 * 1024 * 1024usize).saturating_sub(source_bytes)
                            {
                                return Err(Error::Format(
                                    "renderer exceeds fragment program budget".into(),
                                ));
                            }
                            let custom = unsafe { material::program(gl, fragment)? };
                            source_bytes += fragment.source.len();
                            let index = renderer.programs.len();
                            renderer.programs.push(custom);
                            variants.insert(fragment.source.as_str(), index);
                            index
                        }
                    };
                    let id = renderer.programs[program].id;
                    constants = fragment
                        .constants
                        .iter()
                        .map(|(name, value)| unsafe {
                            (gl.get_uniform_location(id, name), value.clone())
                        })
                        .collect();
                }
                let image = &layer.image;
                let (origin, size) = if layer.fullscreen {
                    ([scene.size.0 / 2.0, scene.size.1 / 2.0], scene.size)
                } else {
                    let natural = (
                        layer.texture.image_width as f32,
                        layer.texture.image_height as f32,
                    );
                    let (w, h) = image.size.unwrap_or(natural);
                    (
                        [image.origin[0], image.origin[1]],
                        (w * image.scale[0], h * image.scale[1]),
                    )
                };
                renderer.layers.push(Drawn {
                    program,
                    channels,
                    inputs,
                    uv_max,
                    origin,
                    size,
                    angle: image.angles[2],
                    tint: [image.color[0], image.color[1], image.color[2], image.alpha],
                    blending: layer.blending,
                    depth: image.depth,
                    constants,
                });
            }
            Ok(())
        })();
        if let Err(error) = prepared {
            unsafe { renderer.delete(gl) };
            return Err(error);
        }
        renderer.texture_bytes = allocated;
        Ok(renderer)
    }

    /// Logical bytes in unique uploaded mipmaps, excluding driver overhead.
    pub fn texture_bytes(&self) -> usize {
        self.texture_bytes
    }

    /// The number of distinct textures uploaded, regardless of how many layers use them.
    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    /// Whether an active GPU program needs time or frame delta updates.
    pub fn animated(&self) -> bool {
        self.programs.iter().any(|program| {
            program
                .frame
                .as_ref()
                .is_some_and(material::Frame::animated)
        })
    }

    /// Moves the parallax towards the pointer, -1..1 across the target with y up, taking the
    /// scene's delay to get there. Returns whether it moved.
    pub fn follow(&mut self, pointer: [f32; 2], seconds: f32) -> bool {
        let Some(parallax) = self.parallax else {
            return false;
        };
        let before = self.eye;
        self.eye = toward(self.eye, pointer, seconds, parallax.delay);
        before
            .iter()
            .zip(self.eye)
            .any(|(was, now)| (was - now).abs() > 1e-4)
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
            host.bind(gl);
            gl.viewport(0, 0, target.0 as i32, target.1 as i32);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::CULL_FACE);
            gl.disable(glow::STENCIL_TEST);
            gl.color_mask(true, true, true, true);
            gl.blend_equation(glow::FUNC_ADD);
            gl.bind_sampler(0, None);
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
            gl.active_texture(glow::TEXTURE0);
            let mut current_program = None;
            for layer in &self.layers {
                let program = &self.programs[layer.program];
                if current_program != Some(layer.program) {
                    gl.use_program(Some(program.id));
                    gl.bind_vertex_array(Some(program.vao));
                    if let Some(frame) = &program.frame {
                        frame.update(gl, target, time, delta);
                    }
                    current_program = Some(layer.program);
                }
                let origin = [
                    layer.origin[0] + shift[0] * layer.depth[0],
                    layer.origin[1] + shift[1] * layer.depth[1],
                ];
                let affine = place_fitted(self.size, target, origin, layer.size, layer.angle, fit);
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
                gl.uniform_3_f32_slice(program.row0.as_ref(), &affine[0]);
                gl.uniform_3_f32_slice(program.row1.as_ref(), &affine[1]);
                gl.uniform_2_f32_slice(program.uv_max.as_ref(), &layer.uv_max);
                gl.uniform_4_f32_slice(program.tint.as_ref(), &layer.tint);
                gl.uniform_1_i32(program.channels.as_ref(), layer.channels);
                for (location, value) in &layer.constants {
                    material::set(gl, location.as_ref(), value);
                }
                for input in &layer.inputs {
                    gl.active_texture(glow::TEXTURE0 + u32::from(input.slot));
                    gl.bind_sampler(u32::from(input.slot), None);
                    gl.bind_texture(glow::TEXTURE_2D, Some(input.texture));
                    gl.uniform_4_f32_slice(
                        program.resolution[input.slot as usize].as_ref(),
                        &input.resolution,
                    );
                }
                gl.active_texture(glow::TEXTURE0);
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
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
            for program in self.programs {
                gl.delete_vertex_array(program.vao);
                gl.delete_program(program.id);
            }
        }
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

unsafe fn program(gl: &glow::Context) -> Result<Program, Error> {
    let fail = |what: String| Error::Format(format!("the layer shader: {what}"));
    unsafe {
        let id = compile::link(gl, VERTEX, FRAGMENT)?;
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
