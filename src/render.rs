//! A scene's layers drawn with OpenGL ES 3, or desktop OpenGL 3.3, into whatever framebuffer
//! the host has bound. The scene covers the target the way a wallpaper does: scaled to fill,
//! centred and cropped.

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
in vec2 uv;
out vec4 color;
void main() {
    color = texture(picture, uv) * tint;
}";

/// A 2D affine map from a layer's unit square to clip space, as two rows:
/// x' = r0·(x, y, 1), y' = r1·(x, y, 1).
pub type Affine = [[f32; 3]; 2];

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
    let (sw, sh) = scene;
    let (tw, th) = (target.0.max(1) as f32, target.1.max(1) as f32);
    let fit = (tw / sw).max(th / sh);
    let (kx, ky) = (2.0 * fit / tw, 2.0 * fit / th);
    let (sin, cos) = angle.sin_cos();
    let (w, h) = size;
    [
        [kx * cos * w, -kx * sin * h, kx * (origin[0] - sw / 2.0)],
        [ky * sin * w, ky * cos * h, ky * (origin[1] - sh / 2.0)],
    ]
}

struct Drawn {
    texture: glow::Texture,
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
    row0: glow::UniformLocation,
    row1: glow::UniformLocation,
    uv_max: glow::UniformLocation,
    tint: glow::UniformLocation,
}

pub struct Renderer {
    program: Program,
    layers: Vec<Drawn>,
    size: (f32, f32),
    clear: [f32; 3],
    parallax: Option<Parallax>,
    /// Where the parallax has got to on its way to the pointer, -1..1 across the target.
    eye: [f32; 2],
}

impl Renderer {
    /// Uploads the layers' textures and builds the program. Hidden layers are left out.
    ///
    /// # Safety
    /// `gl` has to be current on this thread.
    pub unsafe fn new(gl: &glow::Context, scene: &Scene, layers: &[Layer]) -> Result<Self, Error> {
        let program = unsafe { program(gl)? };
        let mut drawn = Vec::new();
        for layer in layers.iter().filter(|layer| layer.image.visible) {
            let (texture, uv_max) = unsafe { upload(gl, &layer.texture)? };
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
            drawn.push(Drawn {
                texture,
                uv_max,
                origin,
                size,
                angle: image.angles[2],
                tint: [image.color[0], image.color[1], image.color[2], image.alpha],
                blending: layer.blending,
                depth: image.depth,
            });
        }
        Ok(Renderer {
            program,
            layers: drawn,
            size: scene.size,
            clear: scene.clear,
            parallax: scene.parallax,
            eye: [0.0; 2],
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

    /// Draws the scene into the bound framebuffer, `target` pixels in size.
    ///
    /// # Safety
    /// `gl` has to be current on this thread, the one the renderer was made on.
    pub unsafe fn draw(&self, gl: &glow::Context, target: (u32, u32)) {
        let program = &self.program;
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
            gl.clear_color(self.clear[0], self.clear[1], self.clear[2], 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.use_program(Some(program.id));
            gl.bind_vertex_array(Some(program.vao));
            gl.active_texture(glow::TEXTURE0);
            for layer in &self.layers {
                let origin = [
                    layer.origin[0] + shift[0] * layer.depth[0],
                    layer.origin[1] + shift[1] * layer.depth[1],
                ];
                let affine = place(self.size, target, origin, layer.size, layer.angle);
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
                gl.uniform_3_f32_slice(Some(&program.row0), &affine[0]);
                gl.uniform_3_f32_slice(Some(&program.row1), &affine[1]);
                gl.uniform_2_f32_slice(Some(&program.uv_max), &layer.uv_max);
                gl.uniform_4_f32_slice(Some(&program.tint), &layer.tint);
                gl.bind_texture(glow::TEXTURE_2D, Some(layer.texture));
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            }
            gl.disable(glow::BLEND);
            gl.bind_vertex_array(None);
        }
    }

    /// # Safety
    /// `gl` has to be current on this thread, the one the renderer was made on.
    pub unsafe fn delete(self, gl: &glow::Context) {
        unsafe {
            for layer in self.layers {
                gl.delete_texture(layer.texture);
            }
            gl.delete_vertex_array(self.program.vao);
            gl.delete_program(self.program.id);
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
        "#version 300 es\nprecision highp float;\n"
    } else {
        "#version 330 core\n"
    }
}

unsafe fn program(gl: &glow::Context) -> Result<Program, Error> {
    let fail = |what: String| Error::Format(format!("the layer shader: {what}"));
    unsafe {
        let id = gl.create_program().map_err(fail)?;
        let mut shaders = Vec::new();
        for (kind, body) in [
            (glow::VERTEX_SHADER, VERTEX),
            (glow::FRAGMENT_SHADER, FRAGMENT),
        ] {
            let shader = gl.create_shader(kind).map_err(fail)?;
            gl.shader_source(shader, &format!("{}{body}", header(gl)));
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                return Err(fail(gl.get_shader_info_log(shader)));
            }
            gl.attach_shader(id, shader);
            shaders.push(shader);
        }
        gl.link_program(id);
        let linked = gl.get_program_link_status(id);
        for shader in shaders {
            gl.detach_shader(id, shader);
            gl.delete_shader(shader);
        }
        if !linked {
            return Err(fail(gl.get_program_info_log(id)));
        }
        let uniform = |name: &str| {
            gl.get_uniform_location(id, name)
                .ok_or_else(|| fail(format!("no {name} uniform")))
        };
        let program = Program {
            id,
            vao: gl.create_vertex_array().map_err(fail)?,
            row0: uniform("row0")?,
            row1: uniform("row1")?,
            uv_max: uniform("uv_max")?,
            tint: uniform("tint")?,
        };
        gl.use_program(Some(id));
        gl.uniform_1_i32(gl.get_uniform_location(id, "picture").as_ref(), 0);
        Ok(program)
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
        let id = gl
            .create_texture()
            .map_err(|err| Error::Format(format!("a texture: {err}")))?;
        gl.bind_texture(glow::TEXTURE_2D, Some(id));
        gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
        gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
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
        // One channel is grey, two are grey and alpha.
        let swizzle = match first.channels {
            Channels::Rgba => None,
            Channels::Rg => Some([glow::RED, glow::RED, glow::RED, glow::GREEN]),
            Channels::R => Some([glow::RED, glow::RED, glow::RED, glow::ONE]),
        };
        if let Some(swizzle) = swizzle {
            let names = [
                glow::TEXTURE_SWIZZLE_R,
                glow::TEXTURE_SWIZZLE_G,
                glow::TEXTURE_SWIZZLE_B,
                glow::TEXTURE_SWIZZLE_A,
            ];
            for (name, value) in names.into_iter().zip(swizzle) {
                gl.tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
            }
        }
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
        Ok((id, uv_max))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
