//! A scene's layers drawn with OpenGL ES 3, or desktop OpenGL 3.3, into whatever framebuffer
//! the host has bound. The scene covers the target the way a wallpaper does: scaled to fill,
//! centred and cropped.

mod compile;
mod material;

use std::collections::HashMap;
use std::sync::Arc;

use glow::HasContext;

use crate::Error;
use crate::project::{Blending, Layer};
use crate::scene::{Parallax, Scene};
use crate::tex::{self, Channels, Texture};

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

/// A 2D affine map from a layer's unit square to clip space, as two rows:
/// x' = r0·(x, y, 1), y' = r1·(x, y, 1).
pub type Affine = [[f32; 3]; 2];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fit {
    #[default]
    Cover,
    Contain,
}

/// Where a layer's unit square, x and y in -0.5..0.5, lands in clip space: `size` in scene
/// pixels centred on `origin`, turned by `angle`, in a `scene`-sized scene that covers a
/// `target`-sized framebuffer.
pub fn place(
    scene: (f32, f32),
    target: (u32, u32),
    origin: [f32; 2],
    size: (f32, f32),
    angle: f32,
) -> Affine {
    place_fitted(scene, target, origin, size, angle, Fit::Cover)
}

pub fn place_fitted(
    scene: (f32, f32),
    target: (u32, u32),
    origin: [f32; 2],
    size: (f32, f32),
    angle: f32,
    fit: Fit,
) -> Affine {
    let (sw, sh) = scene;
    let (tw, th) = (target.0.max(1) as f32, target.1.max(1) as f32);
    let fit = match fit {
        Fit::Cover => (tw / sw).max(th / sh),
        Fit::Contain => (tw / sw).min(th / sh),
    };
    let (kx, ky) = (2.0 * fit / tw, 2.0 * fit / th);
    let (sin, cos) = angle.sin_cos();
    let (w, h) = size;
    [
        [kx * cos * w, -kx * sin * h, kx * (origin[0] - sw / 2.0)],
        [ky * sin * w, ky * cos * h, ky * (origin[1] - sh / 2.0)],
    ]
}

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
                if let Some(fragment) = &layer.fragment {
                    let key = Arc::as_ptr(fragment);
                    program = match variants.get(&key) {
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
                            variants.insert(key, index);
                            index
                        }
                    };
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

/// `seconds` of the way from `eye` to the pointer, when the whole way takes `delay`.
fn toward(eye: [f32; 2], pointer: [f32; 2], seconds: f32, delay: f32) -> [f32; 2] {
    let reach = if delay > 0.0 {
        (seconds / delay).clamp(0.0, 1.0)
    } else {
        1.0
    };
    std::array::from_fn(|at| eye[at] + (pointer[at].clamp(-1.0, 1.0) - eye[at]) * reach)
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

/// The texture's first image with its mipmaps, and how much of it is picture rather than
/// padding.
unsafe fn upload(
    gl: &glow::Context,
    texture: &Texture,
) -> Result<(glow::Texture, [f32; 2]), Error> {
    let mips = texture
        .images
        .first()
        .filter(|mips| !mips.is_empty())
        .ok_or_else(|| Error::Format("a texture without images".into()))?;
    let first = &mips[0];
    let max = unsafe { gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) }.max(0) as u32;
    validate_mips(mips, max)?;
    let share = |picture: u32, stored: u32| {
        if picture == 0 || stored == 0 {
            1.0
        } else {
            (picture as f32 / stored as f32).min(1.0)
        }
    };
    let uv_max = [
        share(texture.image_width, first.width),
        share(texture.image_height, first.height),
    ];
    let nearest = texture.flags & tex::NO_INTERPOLATION != 0;
    unsafe {
        // The host shares this context with libmpv and its own drawing; an error flag they
        // left behind isn't this upload's.
        for _ in 0..16 {
            if gl.get_error() == glow::NO_ERROR {
                break;
            }
        }
        let id = gl
            .create_texture()
            .map_err(|err| Error::Format(format!("a texture: {err}")))?;
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(id));
        gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
        gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
        gl.pixel_store_i32(glow::UNPACK_SKIP_ROWS, 0);
        gl.pixel_store_i32(glow::UNPACK_SKIP_PIXELS, 0);
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        for (level, mip) in mips.iter().enumerate() {
            let (internal, format) = match mip.channels {
                Channels::Rgba => (glow::RGBA8, glow::RGBA),
                Channels::Rg => (glow::RG8, glow::RG),
                Channels::R => (glow::R8, glow::RED),
            };
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                level as i32,
                internal as i32,
                mip.width as i32,
                mip.height as i32,
                0,
                format,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&mip.pixels)),
            );
        }
        // Keep raw channels so the same upload can serve image layers and custom shaders.
        let (min, mag) = match (nearest, mips.len() > 1) {
            (true, _) => (glow::NEAREST, glow::NEAREST),
            (false, true) => (glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR),
            (false, false) => (glow::LINEAR, glow::LINEAR),
        };
        for (name, value) in [
            (glow::TEXTURE_MIN_FILTER, min),
            (glow::TEXTURE_MAG_FILTER, mag),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
        }
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAX_LEVEL,
            mips.len() as i32 - 1,
        );
        let error = gl.get_error();
        if error != glow::NO_ERROR {
            gl.delete_texture(id);
            return Err(Error::Format(format!(
                "uploading a texture failed (GL {error:#x})"
            )));
        }
        Ok((id, uv_max))
    }
}

fn reserve(allocated: &mut usize, bytes: usize) -> Result<(), Error> {
    *allocated = allocated
        .checked_add(bytes)
        .filter(|total| *total <= 512 * 1024 * 1024)
        .ok_or_else(|| Error::Format("scene GPU texture allocations exceed 512 MiB".into()))?;
    Ok(())
}

fn validate_mips(mips: &[tex::Mip], max: u32) -> Result<(), Error> {
    let first = mips
        .first()
        .ok_or_else(|| Error::Format("a texture without mipmaps".into()))?;
    let mut expected = (first.width, first.height);
    for (level, mip) in mips.iter().enumerate() {
        let size = (mip.width, mip.height);
        let bytes = (mip.width as usize)
            .checked_mul(mip.height as usize)
            .and_then(|n| n.checked_mul(mip.channels.count()));
        if size != expected
            || mip.width == 0
            || mip.height == 0
            || mip.width > max
            || mip.height > max
            || mip.channels != first.channels
            || bytes != Some(mip.pixels.len())
        {
            return Err(Error::Format(format!(
                "invalid mipmap {level}: {}×{} (GPU limit {max})",
                mip.width, mip.height
            )));
        }
        if size == (1, 1) && level + 1 != mips.len() {
            return Err(Error::Format("mipmaps continue past 1×1".into()));
        }
        expected = ((mip.width / 2).max(1), (mip.height / 2).max(1));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_budget_bounds_unique_texture_allocations() {
        let mut allocated = 0;
        assert!(reserve(&mut allocated, 256 * 1024 * 1024).is_ok());
        assert!(reserve(&mut allocated, 256 * 1024 * 1024).is_ok());
        assert!(reserve(&mut allocated, 4).is_err());
        assert_eq!(allocated, 512 * 1024 * 1024);
        assert!(reserve(&mut allocated, usize::MAX).is_err());
    }

    fn mip(w: u32, h: u32) -> tex::Mip {
        tex::Mip {
            width: w,
            height: h,
            channels: Channels::Rgba,
            pixels: vec![0; (w * h * 4) as usize],
        }
    }

    #[test]
    fn uploads_require_a_complete_and_consistent_mipmap_chain() {
        assert!(validate_mips(&[mip(8, 4), mip(4, 2), mip(2, 1), mip(1, 1)], 4096).is_ok());
        assert!(validate_mips(&[mip(8, 4)], 4).is_err());
        assert!(validate_mips(&[mip(8, 4), mip(3, 2)], 4096).is_err());
        assert!(validate_mips(&[mip(1, 1), mip(1, 1)], 4096).is_err());
        assert!(validate_mips(&[mip(0, 1)], 4096).is_err());
        let mut short = mip(4, 4);
        short.pixels.pop();
        assert!(validate_mips(&[short], 4096).is_err());
        let mut different = mip(2, 2);
        different.channels = Channels::R;
        different.pixels.truncate(4);
        assert!(validate_mips(&[mip(4, 4), different], 4096).is_err());
    }

    fn apply(affine: Affine, x: f32, y: f32) -> (f32, f32) {
        let row = |r: [f32; 3]| r[0] * x + r[1] * y + r[2];
        (row(affine[0]), row(affine[1]))
    }

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5
    }

    #[test]
    fn the_parallax_eases_towards_the_pointer_and_stays_in_range() {
        assert_eq!(toward([0.0, 0.0], [1.0, -1.0], 0.05, 0.1), [0.5, -0.5]);
        assert_eq!(toward([0.0, 0.0], [3.0, 0.0], 1.0, 0.1), [1.0, 0.0]);
        assert_eq!(toward([0.2, 0.0], [1.0, 0.0], 0.01, 0.0), [1.0, 0.0]);
    }

    #[test]
    fn a_layer_the_size_of_the_scene_fills_a_target_of_its_shape() {
        let affine = place(
            (1920.0, 1080.0),
            (3840, 2160),
            [960.0, 540.0],
            (1920.0, 1080.0),
            0.0,
        );
        assert!(close(apply(affine, -0.5, -0.5), (-1.0, -1.0)));
        assert!(close(apply(affine, 0.5, 0.5), (1.0, 1.0)));
    }

    #[test]
    fn a_narrower_target_crops_the_sides_and_keeps_the_middle() {
        // A 16:9 scene on a square target fills its height and spills past both sides.
        let affine = place(
            (1600.0, 900.0),
            (900, 900),
            [800.0, 450.0],
            (1600.0, 900.0),
            0.0,
        );
        let (right, top) = apply(affine, 0.5, 0.5);
        assert!((top - 1.0).abs() < 1e-5);
        assert!((right - 16.0 / 9.0).abs() < 1e-5);
    }

    #[test]
    fn origins_put_layers_in_place_and_angles_turn_them() {
        // A 100×100 layer in the top right corner of a 1000×1000 scene.
        let affine = place(
            (1000.0, 1000.0),
            (1000, 1000),
            [950.0, 950.0],
            (100.0, 100.0),
            0.0,
        );
        assert!(close(apply(affine, 0.5, 0.5), (1.0, 1.0)));
        assert!(close(apply(affine, -0.5, -0.5), (0.8, 0.8)));
        // A quarter turn takes the right edge's middle to the top's.
        let turned = place(
            (1000.0, 1000.0),
            (1000, 1000),
            [500.0, 500.0],
            (100.0, 100.0),
            std::f32::consts::FRAC_PI_2,
        );
        assert!(close(apply(turned, 0.5, 0.0), (0.0, 0.1)));
    }
}
