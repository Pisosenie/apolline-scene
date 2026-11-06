//! The scene's bloom, drawn over the finished frame as Wallpaper Engine's own is: the frame is
//! copied, what's brighter than the threshold kept at a quarter of its size and made more
//! saturated, blurred down and then across at an eighth, and added back over the frame. The
//! blur reaches a fixed number of the screen's pixels, so it's smaller on bigger screens.

use std::cell::RefCell;

use glow::HasContext;

use super::target::Target;
use crate::Error;
use crate::scene::Bloom as Settings;

const VERTEX: &str = "
out vec2 uv;
void main() {
    vec2 corner = vec2(float(gl_VertexID & 1), float((gl_VertexID >> 1) & 1));
    gl_Position = vec4(corner * 2.0 - 1.0, 0.0, 1.0);
    uv = corner;
}";

/// The average of four of the frame's pixels around each of its own, from its brightest
/// channel above the threshold up.
const BRIGHT: &str = "
uniform sampler2D source;
uniform vec2 texel;
uniform float strength;
uniform float threshold;
uniform vec3 tint;
in vec2 uv;
out vec4 color;
void main() {
    vec3 light = 0.25 * (texture(source, uv - texel).rgb + texture(source, uv + texel).rgb
        + texture(source, uv + vec2(-texel.x, texel.y)).rgb
        + texture(source, uv + vec2(texel.x, -texel.y)).rgb);
    light *= clamp(max(max(light.r, light.g), light.b) - threshold, 0.0, 1.0);
    float grey = dot(vec3(0.2989, 0.5870, 0.1140), light);
    color = vec4(max((2.0 * light - grey) * strength * tint, 0.0), 1.0);
}";

/// Six steps of `along` each way, weighted as a bell.
const BLUR: &str = "
uniform sampler2D source;
uniform vec2 along;
in vec2 uv;
out vec4 color;
void main() {
    vec3 sum = texture(source, uv).rgb * WEIGHTS[0];
    for (int step = 1; step < 7; step++) {
        vec2 offset = along * float(step);
        sum += (texture(source, uv + offset).rgb + texture(source, uv - offset).rgb) * WEIGHTS[step];
    }
    color = vec4(sum, 1.0);
}";

const ADD: &str = "
uniform sampler2D source;
in vec2 uv;
out vec4 color;
void main() {
    color = vec4(texture(source, uv).rgb, 1.0);
}";

/// How wide the bell the blur weights its steps by is, in steps.
const SPREAD: f32 = 2.33;
/// How many of the screen's pixels each blur step is.
const STEP: f32 = 8.0;

pub(super) struct Bloom {
    settings: Settings,
    bright: Program,
    blur: Program,
    add: Program,
    vao: glow::VertexArray,
    targets: RefCell<Option<Sized>>,
}

/// The quarter size picture and the two eighth size ones, and the screen they're sized for.
type Sized = ((u32, u32), [Target; 3]);

struct Program {
    id: glow::Program,
    texel: Option<glow::UniformLocation>,
    along: Option<glow::UniformLocation>,
}

impl Bloom {
    /// # Safety
    /// The GL context must be current on this thread.
    pub unsafe fn new(gl: &glow::Context, settings: Settings) -> Result<Self, Error> {
        let blur = BLUR.replace("WEIGHTS", "weights");
        let blur = format!("const float weights[7] = float[7]({});\n{blur}", weights());
        unsafe {
            let mut made: Vec<glow::Program> = Vec::new();
            let result = (|| {
                for fragment in [BRIGHT, blur.as_str(), ADD] {
                    made.push(super::compile::link(gl, VERTEX, fragment)?);
                }
                gl.create_vertex_array()
                    .map_err(|error| Error::Format(format!("the bloom: {error}")))
            })();
            let vao = match result {
                Ok(vao) => vao,
                Err(error) => {
                    for id in made {
                        gl.delete_program(id);
                    }
                    return Err(error);
                }
            };
            let [bright, blur, add] = [made[0], made[1], made[2]].map(|id| {
                gl.use_program(Some(id));
                gl.uniform_1_i32(gl.get_uniform_location(id, "source").as_ref(), 0);
                Program {
                    id,
                    texel: gl.get_uniform_location(id, "texel"),
                    along: gl.get_uniform_location(id, "along"),
                }
            });
            gl.use_program(Some(bright.id));
            let uniform = |name| gl.get_uniform_location(bright.id, name);
            gl.uniform_1_f32(uniform("strength").as_ref(), settings.strength);
            gl.uniform_1_f32(uniform("threshold").as_ref(), settings.threshold);
            let [r, g, b] = settings.tint;
            gl.uniform_3_f32(uniform("tint").as_ref(), r, g, b);
            Ok(Bloom {
                settings,
                bright,
                blur,
                add,
                vao,
                targets: RefCell::new(None),
            })
        }
    }

    /// Adds the bloom of `frame`, the frame drawn so far `size` big, back over it in the host's
    /// framebuffer, which `host` binds again, within the scissor box when `boxed`. Leaves
    /// blending, scissoring and the program to the caller.
    ///
    /// # Safety
    /// The GL context must be current on this thread, with scissoring off and alpha writes
    /// masked.
    pub unsafe fn draw(
        &self,
        gl: &glow::Context,
        frame: glow::Texture,
        size: (u32, u32),
        host: &super::state::Host,
        boxed: bool,
    ) {
        if self.settings.strength <= 0.0 {
            return;
        }
        let mut targets = self.targets.borrow_mut();
        let sizes = [4, 8, 8].map(|scale| super::chain::shrunk(size, scale));
        unsafe {
            match targets.as_mut() {
                Some((made, held)) if *made != size => {
                    for (target, wanted) in held.iter().zip(sizes) {
                        target.resize(gl, wanted);
                    }
                    *made = size;
                }
                Some(_) => {}
                None => {
                    let Some(made) = made(gl, sizes) else {
                        return;
                    };
                    *targets = Some((size, made));
                }
            }
            let Some((_, [quarter, down, across])) = targets.as_ref() else {
                return;
            };
            let texel = [1.0 / size.0.max(1) as f32, 1.0 / size.1.max(1) as f32];
            gl.disable(glow::BLEND);
            gl.bind_vertex_array(Some(self.vao));
            gl.active_texture(glow::TEXTURE0);
            let passes = [
                (&self.bright, frame, quarter, [0.0, 0.0]),
                (&self.blur, quarter.texture, down, [0.0, STEP * texel[1]]),
                (&self.blur, down.texture, across, [STEP * texel[0], 0.0]),
            ];
            for ((program, source, into, along), wanted) in passes.into_iter().zip(sizes) {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(into.framebuffer));
                gl.viewport(0, 0, wanted.0 as i32, wanted.1 as i32);
                gl.use_program(Some(program.id));
                gl.uniform_2_f32(program.texel.as_ref(), texel[0], texel[1]);
                gl.uniform_2_f32(program.along.as_ref(), along[0], along[1]);
                gl.bind_texture(glow::TEXTURE_2D, Some(source));
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            }
            host.bind(gl);
            gl.viewport(0, 0, size.0 as i32, size.1 as i32);
            gl.use_program(Some(self.add.id));
            gl.bind_texture(glow::TEXTURE_2D, Some(across.texture));
            if boxed {
                gl.enable(glow::SCISSOR_TEST);
            }
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }

    /// # Safety
    /// The GL context must be current on this thread.
    pub unsafe fn delete(&self, gl: &glow::Context) {
        unsafe {
            for program in [&self.bright, &self.blur, &self.add] {
                gl.delete_program(program.id);
            }
            gl.delete_vertex_array(self.vao);
            if let Some((_, targets)) = self.targets.take() {
                for target in &targets {
                    target.delete(gl);
                }
            }
        }
    }
}

/// The quarter and eighth size targets, or none if one can't be made.
unsafe fn made(gl: &glow::Context, sizes: [(u32, u32); 3]) -> Option<[Target; 3]> {
    let mut made = Vec::new();
    for size in sizes {
        match unsafe { Target::new(gl, size) } {
            Ok(target) => made.push(target),
            Err(_) => {
                for target in &made {
                    unsafe { target.delete(gl) };
                }
                return None;
            }
        }
    }
    made.try_into().ok()
}

/// The blur's seven weights from the middle out, adding up to one over both sides.
fn weights() -> String {
    let bell: Vec<f32> = (0..7)
        .map(|step| (-((step * step) as f32) / (2.0 * SPREAD * SPREAD)).exp())
        .collect();
    let total = bell[0] + 2.0 * bell[1..].iter().sum::<f32>();
    bell.iter()
        .map(|weight| format!("{:.6}", weight / total))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::weights;

    #[test]
    fn the_blur_weights_add_up_to_one_and_fall_away() {
        let weights: Vec<f32> = weights()
            .split(", ")
            .map(|weight| weight.parse().unwrap())
            .collect();
        let total = weights[0] + 2.0 * weights[1..].iter().sum::<f32>();
        assert!((total - 1.0).abs() < 1e-4);
        assert!(weights.windows(2).all(|pair| pair[0] > pair[1]));
        assert!((weights[0] - 0.172).abs() < 0.002);
    }
}
