//! A puppet's mesh on screen: its corners where its bones put them now, each showing its
//! part of the layer's picture. The corners are worked out here and sent each frame; where in
//! the picture each shows never changes.

use std::cell::Cell;
use std::sync::Arc;

use glow::HasContext;

use crate::Error;
use crate::puppet::{Posed, Puppet};

/// The layer's own vertex shader, for a mesh: each corner placed in the layer's units, -0.5
/// to 0.5 across it with y up, and where it shows in the picture.
pub(super) const VERTEX: &str = "
uniform vec3 row0;
uniform vec3 row1;
uniform vec2 uv_at;
uniform vec4 uv_sides;
layout(location = 0) in vec2 place;
layout(location = 1) in vec2 picture;
out vec2 uv;
void main() {
    vec3 local = vec3(place, 1.0);
    gl_Position = vec4(dot(row0, local), dot(row1, local), 0.0, 1.0);
    uv = uv_at + picture.x * uv_sides.xy + picture.y * uv_sides.zw;
}";

pub(super) struct Rig {
    puppet: Arc<Puppet>,
    playing: Vec<Posed>,
    /// The picture's size in pixels, which the mesh's corners are measured in.
    size: [f32; 2],
    pub vao: glow::VertexArray,
    places: glow::Buffer,
    pictures: glow::Buffer,
    triangles: glow::Buffer,
    /// When the corners were last worked out.
    posed: Cell<Option<f32>>,
}

impl Rig {
    /// # Safety
    /// The GL context must be current on this thread.
    pub unsafe fn new(
        gl: &glow::Context,
        puppet: &Arc<Puppet>,
        playing: &[Posed],
        size: [f32; 2],
    ) -> Result<Rig, Error> {
        let fail = |what: String| Error::Format(format!("a puppet's mesh: {what}"));
        unsafe {
            let vao = gl.create_vertex_array().map_err(fail)?;
            let mut made = Vec::new();
            for _ in 0..3 {
                match gl.create_buffer() {
                    Ok(buffer) => made.push(buffer),
                    Err(error) => {
                        for buffer in made {
                            gl.delete_buffer(buffer);
                        }
                        gl.delete_vertex_array(vao);
                        return Err(fail(error));
                    }
                }
            }
            let [places, pictures, triangles] = [made[0], made[1], made[2]];
            gl.bind_vertex_array(Some(vao));
            let uvs: Vec<f32> = puppet.corners.iter().flat_map(|corner| corner.uv).collect();
            for (location, buffer) in [(0, places), (1, pictures)] {
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
                if location == 1 {
                    gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes(&uvs), glow::STATIC_DRAW);
                } else {
                    gl.buffer_data_size(
                        glow::ARRAY_BUFFER,
                        (uvs.len() * 4) as i32,
                        glow::STREAM_DRAW,
                    );
                }
                gl.enable_vertex_attrib_array(location);
                gl.vertex_attrib_pointer_f32(location, 2, glow::FLOAT, false, 8, 0);
            }
            let indices: Vec<u16> = puppet.triangles.iter().flatten().copied().collect();
            let index_bytes: Vec<u8> = indices
                .iter()
                .flat_map(|index| index.to_ne_bytes())
                .collect();
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(triangles));
            gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, &index_bytes, glow::STATIC_DRAW);
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);
            Ok(Rig {
                puppet: Arc::clone(puppet),
                playing: playing.to_vec(),
                size,
                vao,
                places,
                pictures,
                triangles,
                posed: Cell::new(None),
            })
        }
    }

    /// Bytes its buffers hold.
    pub fn bytes(puppet: &Puppet) -> usize {
        puppet.corners.len() * 16 + puppet.triangles.len() * 6
    }

    /// Whether its bones move as time goes on.
    pub fn moves(&self) -> bool {
        !self.playing.is_empty() && !self.puppet.animations.is_empty()
    }

    /// Puts the corners where the bones have them `seconds` in, and draws the mesh with the
    /// program in use.
    ///
    /// # Safety
    /// The GL context must be current on this thread, with the layer's program in use.
    pub unsafe fn draw(&self, gl: &glow::Context, seconds: f32) {
        unsafe {
            if self.posed.get() != Some(seconds) {
                let [width, height] = self.size.map(|side| side.max(1e-6));
                let places: Vec<f32> = self
                    .puppet
                    .corners_at(&self.playing, seconds)
                    .into_iter()
                    .flat_map(|[x, y]| [x / width, y / height])
                    .collect();
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.places));
                gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, &bytes(&places));
                gl.bind_buffer(glow::ARRAY_BUFFER, None);
                self.posed.set(Some(seconds));
            }
            gl.bind_vertex_array(Some(self.vao));
            gl.draw_elements(
                glow::TRIANGLES,
                (self.puppet.triangles.len() * 3) as i32,
                glow::UNSIGNED_SHORT,
                0,
            );
        }
    }

    /// # Safety
    /// The GL context must be current on this thread.
    pub unsafe fn delete(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_vertex_array(self.vao);
            for buffer in [self.places, self.pictures, self.triangles] {
                gl.delete_buffer(buffer);
            }
        }
    }
}

fn bytes(floats: &[f32]) -> Vec<u8> {
    floats
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}
