//! Particle systems drawn as sprites, one instance each, from a buffer filled every frame.

use std::cell::RefCell;
use std::sync::Arc;

use glow::HasContext;

use crate::Error;
use crate::particles::{Simulation, System};

/// Floats per particle: centre x and y, size and turn, then colour and alpha.
const FLOATS: usize = 8;

pub(super) const VERTEX: &str = "
uniform vec3 row0;
uniform vec3 row1;
uniform vec2 uv_max;
uniform vec2 apolline_layer_size;
layout(location = 0) in vec4 particle;
layout(location = 1) in vec4 particle_color;
out vec2 uv;
out vec4 shade;
void main() {
    vec2 corner = vec2(float(gl_VertexID & 1), float((gl_VertexID >> 1) & 1));
    vec2 offset = (corner - 0.5) * particle.z;
    float c = cos(particle.w);
    float s = sin(particle.w);
    vec2 at = particle.xy + vec2(c * offset.x - s * offset.y, s * offset.x + c * offset.y);
    vec3 local = vec3(at / apolline_layer_size - 0.5, 1.0);
    gl_Position = vec4(dot(row0, local), dot(row1, local), 0.0, 1.0);
    uv = vec2(corner.x, 1.0 - corner.y) * uv_max;
    shade = particle_color;
}";

pub(super) const FRAGMENT: &str = "
uniform sampler2D picture;
uniform vec4 tint;
uniform int picture_channels;
in vec2 uv;
in vec4 shade;
out vec4 color;
void main() {
    vec4 pixel = texture(picture, uv);
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
}

impl Swarm {
    /// # Safety
    /// The GL context must be current on this thread.
    pub unsafe fn new(gl: &glow::Context, system: &Arc<System>, seed: u64) -> Result<Self, Error> {
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
            for (location, offset) in [(0, 0), (1, 16)] {
                gl.enable_vertex_attrib_array(location);
                gl.vertex_attrib_pointer_f32(location, 4, glow::FLOAT, false, stride, offset);
                gl.vertex_attrib_divisor(location, 1);
            }
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            Ok(Swarm {
                simulation: RefCell::new(Simulation::new(Arc::clone(system), seed)),
                vao,
                buffer,
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
            for value in [
                at[0],
                at[1],
                particle.size * scale[0].abs() * near,
                particle.rotation + angle,
                particle.color[0],
                particle.color[1],
                particle.color[2],
                particle.alpha,
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
    use super::seen;

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
