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
    /// placed, returning how many there are.
    ///
    /// # Safety
    /// The GL context must be current on this thread.
    pub unsafe fn update(
        &self,
        gl: &glow::Context,
        time: f32,
        origin: [f32; 2],
        scale: [f32; 2],
    ) -> i32 {
        let mut simulation = self.simulation.borrow_mut();
        simulation.seek(time);
        let particles = simulation.particles();
        let mut bytes = Vec::with_capacity(particles.len() * FLOATS * 4);
        for particle in particles {
            for value in [
                origin[0] + particle.position[0] * scale[0],
                origin[1] + particle.position[1] * scale[1],
                particle.size * scale[0].abs(),
                particle.rotation,
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
