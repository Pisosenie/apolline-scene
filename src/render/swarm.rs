//! Particle systems drawn as sprites, one instance each, from a buffer filled every frame. A
//! sprite sheet's particles each show their frame, blending into the next as they go.

use std::cell::RefCell;
use std::sync::Arc;

use glow::HasContext;

use crate::Error;
use crate::particles::{Renderer, Sheet, Simulation, System};
use crate::tex::Frame;

/// Floats per particle: centre x and y, size and turn; colour and alpha; the corners of its
/// frame and the next in the texture; their top and left sides; how far it is on to the next;
/// and how many times its width it is tall.
const FLOATS: usize = 18;

pub(super) const VERTEX: &str = "
uniform vec3 row0;
uniform vec3 row1;
uniform vec2 apolline_layer_size;
layout(location = 0) in vec4 particle;
layout(location = 1) in vec4 particle_color;
layout(location = 2) in vec4 frames;
layout(location = 3) in vec4 sides;
layout(location = 4) in vec2 onward;
out vec2 uv;
out vec2 next_uv;
out float blend;
out vec4 shade;
void main() {
    vec2 corner = vec2(float(gl_VertexID & 1), float((gl_VertexID >> 1) & 1));
    vec2 offset = (corner - 0.5) * particle.z * vec2(1.0, onward.y);
    float c = cos(particle.w);
    float s = sin(particle.w);
    vec2 at = particle.xy + vec2(c * offset.x - s * offset.y, s * offset.x + c * offset.y);
    vec3 local = vec3(at / apolline_layer_size - 0.5, 1.0);
    gl_Position = vec4(dot(row0, local), dot(row1, local), 0.0, 1.0);
    vec2 across = vec2(corner.x, 1.0 - corner.y);
    vec2 inside = across.x * sides.xy + across.y * sides.zw;
    uv = frames.xy + inside;
    next_uv = frames.zw + inside;
    blend = onward.x;
    shade = particle_color;
}";

pub(super) const FRAGMENT: &str = "
uniform sampler2D picture;
uniform vec4 tint;
uniform int picture_channels;
in vec2 uv;
in vec2 next_uv;
in float blend;
in vec4 shade;
out vec4 color;
void main() {
    vec4 pixel = texture(picture, uv);
    if (blend > 0.0) {
        pixel = mix(pixel, texture(picture, next_uv), blend);
    }
    if (picture_channels == 1) {
        pixel = vec4(pixel.rrr, 1.0);
    } else if (picture_channels == 2) {
        pixel = vec4(pixel.rrr, pixel.g);
    }
    color = pixel * shade * tint;
}";

pub(super) struct Swarm {
    simulation: RefCell<Simulation>,
    pub vao: glow::VertexArray,
    buffer: glow::Buffer,
    /// A sprite sheet's frames' corners, and all their sides; without one, the whole picture.
    corners: Vec<[f32; 2]>,
    sides: [f32; 4],
    /// How many times as tall as wide its picture is.
    ratio: f32,
}

impl Swarm {
    /// # Safety
    /// The GL context must be current on this thread.
    /// `frames` are its texture's, a sprite sheet's, and `uv_max` how much of the texture is
    /// picture.
    pub unsafe fn new(
        gl: &glow::Context,
        system: &Arc<System>,
        frames: &[Frame],
        uv_max: [f32; 2],
        ratio: f32,
        seed: u64,
    ) -> Result<Self, Error> {
        let fail = |what: String| Error::Format(format!("a particle buffer: {what}"));
        unsafe {
            let buffer = gl.create_buffer().map_err(fail)?;
            let vao = match gl.create_vertex_array() {
                Ok(vao) => vao,
                Err(error) => {
                    gl.delete_buffer(buffer);
                    return Err(fail(error));
                }
            };
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
            gl.buffer_data_size(
                glow::ARRAY_BUFFER,
                (system.max.max(1) * FLOATS * 4) as i32,
                glow::STREAM_DRAW,
            );
            let stride = (FLOATS * 4) as i32;
            for (location, size, offset) in
                [(0, 4, 0), (1, 4, 16), (2, 4, 32), (3, 4, 48), (4, 2, 64)]
            {
                gl.enable_vertex_attrib_array(location);
                gl.vertex_attrib_pointer_f32(location, size, glow::FLOAT, false, stride, offset);
                gl.vertex_attrib_divisor(location, 1);
            }
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            let (corners, sides) = match frames.first() {
                Some(first) => (
                    frames.iter().map(|frame| frame.corner).collect(),
                    [
                        first.across[0],
                        first.across[1],
                        first.down[0],
                        first.down[1],
                    ],
                ),
                None => (vec![[0.0; 2]], [uv_max[0], 0.0, 0.0, uv_max[1]]),
            };
            Ok(Swarm {
                simulation: RefCell::new(Simulation::new(Arc::clone(system), seed)),
                vao,
                buffer,
                corners,
                sides,
                ratio: if ratio.is_finite() && ratio > 0.0 {
                    ratio
                } else {
                    1.0
                },
            })
        }
    }

    /// Bytes its buffer holds.
    pub fn bytes(system: &System) -> usize {
        system.max.max(1) * FLOATS * 4
    }

    /// Brings the particles to `time` and fills the buffer with them where the system is
    /// placed, scaled and turned, returning how many there are.
    ///
    /// # Safety
    /// The GL context must be current on this thread.
    /// `camera` is the middle of the scene and how far in front of it the camera is, which
    /// a system drawn in perspective is seen from.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn update(
        &self,
        gl: &glow::Context,
        time: f32,
        origin: [f32; 2],
        scale: [f32; 3],
        angle: f32,
        camera: ([f32; 2], f32),
    ) -> i32 {
        let mut simulation = self.simulation.borrow_mut();
        simulation.seek(time);
        let perspective = simulation.perspective();
        let sheet = simulation.sheet();
        let renderer = simulation.renderer();
        let particles = simulation.particles();
        let mut bytes = Vec::with_capacity(particles.len() * FLOATS * 4);
        let (sin, cos) = angle.sin_cos();
        let (middle, distance) = camera;
        for particle in particles {
            let x = particle.position[0] * scale[0];
            let y = particle.position[1] * scale[1];
            let at = [origin[0] + cos * x - sin * y, origin[1] + sin * x + cos * y];
            let depth = if perspective {
                particle.position[2] * scale[2]
            } else {
                0.0
            };
            let (at, near) = seen(at, depth, middle, distance);
            let ([now, next], onward) = frames(self.corners.len(), particle.sheet(sheet), sheet);
            let (turn, stretch) = match renderer {
                Renderer::Sprite => (particle.rotation + angle, self.ratio),
                Renderer::Trail { length, min, max } => {
                    let [vx, vy, _] = particle.velocity();
                    let (heading, long) =
                        trail([vx * scale[0], vy * scale[1]], angle, [length, min, max]);
                    (heading, long * self.ratio)
                }
            };
            let (now, next) = (self.corners[now], self.corners[next]);
            for value in [
                at[0],
                at[1],
                particle.size * scale[0].abs() * near,
                turn,
                particle.color[0],
                particle.color[1],
                particle.color[2],
                particle.alpha,
                now[0],
                now[1],
                next[0],
                next[1],
                self.sides[0],
                self.sides[1],
                self.sides[2],
                self.sides[3],
                onward,
                stretch,
            ] {
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
        }
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.buffer));
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, &bytes);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }
        particles.len() as i32
    }

    /// # Safety
    /// The GL context must be current on this thread.
    pub unsafe fn delete(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_vertex_array(self.vao);
            gl.delete_buffer(self.buffer);
        }
    }
}

/// Which way a trail lies and how many times its width it is long, for a particle moving at
/// `velocity` in a system turned by `angle`: along the way it goes, `length` times its speed
/// between `min` and `max`.
fn trail(velocity: [f32; 2], angle: f32, [length, min, max]: [f32; 3]) -> (f32, f32) {
    let (sin, cos) = angle.sin_cos();
    let [vx, vy] = velocity;
    let (vx, vy) = (cos * vx - sin * vy, sin * vx + cos * vy);
    let heading = vy.atan2(vx) - std::f32::consts::FRAC_PI_2;
    (heading, (vx.hypot(vy) * length).clamp(min, max))
}

/// Which of `count` frames a particle `through` its sheet shows and the one after, and how far
/// it is on to that one: only a sequence blends, and its last frame stays.
fn frames(count: usize, through: f32, sheet: Sheet) -> ([usize; 2], f32) {
    let last = count.saturating_sub(1);
    let at = through.clamp(0.0, 1.0) * count as f32;
    let now = (at.floor() as usize).min(last);
    match sheet {
        Sheet::Random => ([now; 2], 0.0),
        Sheet::Sequence(_) => (
            [now, (now + 1).min(last)],
            if now < last { at.fract() } else { 0.0 },
        ),
    }
}

/// Where a point `depth` in front of the scene is seen from a camera `distance` in front of its
/// `middle`, and how much bigger it looks: nearer is bigger and further out, and at or behind
/// the camera it's nothing.
fn seen(at: [f32; 2], depth: f32, middle: [f32; 2], distance: f32) -> ([f32; 2], f32) {
    let ahead = distance - depth;
    let near = if ahead > distance * 0.01 {
        distance / ahead
    } else {
        0.0
    };
    (
        [
            middle[0] + (at[0] - middle[0]) * near,
            middle[1] + (at[1] - middle[1]) * near,
        ],
        near,
    )
}

#[cfg(test)]
mod tests {
    use super::{frames, seen, trail};

    #[test]
    fn trails_lie_along_their_way_as_long_as_their_speed_says() {
        // Rising: the sprite's top already points up.
        let (heading, long) = trail([0.0, 100.0], 0.0, [0.05, 1.0, 3.0]);
        assert!(heading.abs() < 1e-6);
        assert!((long - 5.0f32.min(3.0)).abs() < 1e-6);
        // Going right turns it a quarter clockwise; slow ones keep their least length.
        let (heading, long) = trail([10.0, 0.0], 0.0, [0.05, 1.0, 3.0]);
        assert!((heading + std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(long, 1.0);
        // The system's turn turns the way it goes.
        let (heading, _) = trail([10.0, 0.0], std::f32::consts::FRAC_PI_2, [0.05, 1.0, 3.0]);
        assert!(heading.abs() < 1e-5);
    }
    use crate::particles::Sheet;

    #[test]
    fn a_sequence_blends_frame_into_frame_and_a_random_pick_holds_one() {
        let sequence = Sheet::Sequence(1.0);
        assert_eq!(frames(4, 0.0, sequence), ([0, 1], 0.0));
        assert_eq!(frames(4, 0.375, sequence), ([1, 2], 0.5));
        assert_eq!(frames(4, 0.99, sequence).0, [3, 3]);
        assert_eq!(frames(4, 0.99, sequence).1, 0.0);
        assert_eq!(frames(4, 0.6, Sheet::Random), ([2, 2], 0.0));
        assert_eq!(frames(1, 0.5, sequence), ([0, 0], 0.0));
    }

    #[test]
    fn nearer_particles_look_bigger_and_further_out() {
        let middle = [100.0, 50.0];
        assert_eq!(
            seen([150.0, 50.0], 0.0, middle, 1000.0),
            ([150.0, 50.0], 1.0)
        );
        let (at, near) = seen([150.0, 50.0], 500.0, middle, 1000.0);
        assert_eq!((at, near), ([200.0, 50.0], 2.0));
        let (_, away) = seen([150.0, 50.0], -1000.0, middle, 1000.0);
        assert_eq!(away, 0.5);
        assert_eq!(seen([150.0, 50.0], 1000.0, middle, 1000.0).1, 0.0);
    }
}
