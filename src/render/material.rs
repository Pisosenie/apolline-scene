use super::Program;
use crate::Error;
use crate::shader::{Constant, Fragment};
use glow::HasContext;

/// # Safety
/// The GL context must be current on this thread.
pub(super) unsafe fn program(gl: &glow::Context, fragment: &Fragment) -> Result<Program, Error> {
    let vertex = match &fragment.vertex {
        Some(vertex) => vertex.clone(),
        None => stand_in(fragment.texcoord),
    };
    unsafe {
        let id = super::compile::link(gl, &vertex, &fragment.source).map_err(|error| {
            Error::Format(format!("{error} (source IDs: {:?})", fragment.files))
        })?;
        let vao = match gl.create_vertex_array() {
            Ok(vao) => vao,
            Err(error) => {
                gl.delete_program(id);
                return Err(Error::Format(error));
            }
        };
        gl.use_program(Some(id));
        for slot in 0..8 {
            gl.uniform_1_i32(
                gl.get_uniform_location(id, &format!("g_Texture{slot}"))
                    .as_ref(),
                slot,
            );
        }
        Ok(Program {
            id,
            vao,
            row0: gl.get_uniform_location(id, "row0"),
            row1: gl.get_uniform_location(id, "row1"),
            uv_max: gl
                .get_uniform_location(id, "uv_max")
                .or_else(|| gl.get_uniform_location(id, "apolline_uv_max")),
            uv_at: None,
            uv_sides: None,
            mvp: gl.get_uniform_location(id, "g_ModelViewProjectionMatrix"),
            layer_size: gl.get_uniform_location(id, "apolline_layer_size"),
            tint: gl.get_uniform_location(id, "apolline_layer_tint"),
            channels: None,
            frame: Some(Frame::new(gl, id)),
            resolution: std::array::from_fn(|slot| {
                gl.get_uniform_location(id, &format!("g_Texture{slot}Resolution"))
            }),
        })
    }
}

/// Our vertex shader, for a material that has none: the layer's quad, with the texture
/// coordinates in as many components as the fragment declares.
fn stand_in(texcoord: u8) -> String {
    let varying = match texcoord {
        2 => "uv",
        3 => "vec3(uv, 0.0)",
        _ => "vec4(uv, 0.0, 0.0)",
    };
    format!(
        "\
uniform vec3 row0;
uniform vec3 row1;
uniform vec2 uv_max;
out vec{texcoord} v_TexCoord;
void main() {{
    vec2 corner = vec2(float(gl_VertexID & 1), float((gl_VertexID >> 1) & 1));
    vec3 local = vec3(corner - 0.5, 1.0);
    gl_Position = vec4(dot(row0, local), dot(row1, local), 0.0, 1.0);
    vec2 uv = vec2(corner.x, 1.0 - corner.y) * uv_max;
    v_TexCoord = {varying};
}}"
    )
}

/// The layer's placement as a matrix for `mul(vec4(a_Position, 1.0), M)`, with `a_Position`
/// in the layer's own units, centred.
pub(super) fn model_view_projection(affine: &super::Affine, size: (f32, f32)) -> [f32; 16] {
    let [row0, row1] = affine;
    // A mirrored layer has a negative size, which a_Position carries too.
    let away = |side: f32| {
        if side.abs() < 1e-6 {
            1e-6f32.copysign(side)
        } else {
            side
        }
    };
    let (width, height) = (away(size.0), away(size.1));
    [
        row0[0] / width,
        row1[0] / width,
        0.0,
        0.0,
        row0[1] / height,
        row1[1] / height,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
        row0[2],
        row1[2],
        0.0,
        1.0,
    ]
}

/// What the engine gives shaders besides time, as the host last set it.
pub(super) struct Inputs {
    /// Where the pointer is across the screen, 0 to 1 from the top left.
    pub pointer: [f32; 2],
    /// Where the parallax has got to, in the same terms.
    pub parallax: [f32; 2],
    /// How far through the local day it is, 0 to 1.
    pub daytime: f32,
    /// The left and right audio spectrum in 64 bins, low to high; silence is all zeros.
    pub spectrum: [[f32; 64]; 2],
}

impl Default for Inputs {
    fn default() -> Self {
        Inputs {
            pointer: [0.5; 2],
            parallax: [0.5; 2],
            daytime: 0.5,
            spectrum: [[0.0; 64]; 2],
        }
    }
}

pub(super) struct Frame {
    screen: Option<glow::UniformLocation>,
    time: Option<glow::UniformLocation>,
    delta: Option<glow::UniformLocation>,
    texel: Option<glow::UniformLocation>,
    half_texel: Option<glow::UniformLocation>,
    pointer: Option<glow::UniformLocation>,
    parallax: Option<glow::UniformLocation>,
    pub daytime: Option<glow::UniformLocation>,
    /// 16, 32 and 64 bins, left then right.
    spectrum: [[Option<glow::UniformLocation>; 3]; 2],
}

impl Frame {
    unsafe fn new(gl: &glow::Context, program: glow::Program) -> Self {
        unsafe {
            Self {
                screen: gl.get_uniform_location(program, "g_Screen"),
                time: gl.get_uniform_location(program, "g_Time"),
                delta: gl.get_uniform_location(program, "g_Frametime"),
                texel: gl.get_uniform_location(program, "g_TexelSize"),
                half_texel: gl.get_uniform_location(program, "g_TexelSizeHalf"),
                pointer: gl.get_uniform_location(program, "g_PointerPosition"),
                parallax: gl.get_uniform_location(program, "g_ParallaxPosition"),
                daytime: gl.get_uniform_location(program, "g_Daytime"),
                spectrum: ["Left", "Right"].map(|side| {
                    [16, 32, 64].map(|bins| {
                        gl.get_uniform_location(program, &format!("g_AudioSpectrum{bins}{side}"))
                    })
                }),
            }
        }
    }

    /// Whether it reads something that changes from frame to frame.
    pub(super) fn animated(&self) -> bool {
        self.time.is_some()
            || self.delta.is_some()
            || self.pointer.is_some()
            || self.parallax.is_some()
            || self.listens()
    }

    pub(super) fn points(&self) -> bool {
        self.pointer.is_some() || self.parallax.is_some()
    }

    pub(super) fn listens(&self) -> bool {
        self.spectrum.iter().flatten().any(Option::is_some)
    }

    /// # Safety
    /// The GL context and this frame's program must be current on this thread.
    pub(super) unsafe fn update(
        &self,
        gl: &glow::Context,
        screen: (u32, u32),
        target: (u32, u32),
        time: f32,
        delta: f32,
        inputs: &Inputs,
    ) {
        let width = target.0.max(1) as f32;
        let height = target.1.max(1) as f32;
        let screen = (screen.0.max(1) as f32, screen.1.max(1) as f32);
        let finite = |value: f32| {
            if value.is_finite() {
                value.max(0.0)
            } else {
                0.0
            }
        };
        unsafe {
            gl.uniform_1_f32(self.time.as_ref(), finite(time));
            gl.uniform_1_f32(self.delta.as_ref(), finite(delta));
            gl.uniform_3_f32(
                self.screen.as_ref(),
                screen.0,
                screen.1,
                screen.0 / screen.1,
            );
            gl.uniform_2_f32(self.texel.as_ref(), 1.0 / width, 1.0 / height);
            gl.uniform_2_f32(self.half_texel.as_ref(), 0.5 / width, 0.5 / height);
            gl.uniform_2_f32_slice(self.pointer.as_ref(), &inputs.pointer);
            gl.uniform_2_f32_slice(self.parallax.as_ref(), &inputs.parallax);
            gl.uniform_1_f32(self.daytime.as_ref(), inputs.daytime);
            for (locations, bins) in self.spectrum.iter().zip(inputs.spectrum.iter()) {
                let mut bins = bins.to_vec();
                for location in locations.iter().rev() {
                    gl.uniform_1_f32_slice(location.as_ref(), &bins);
                    bins = bins
                        .chunks(2)
                        .map(|pair| (pair[0] + pair[1]) / 2.0)
                        .collect();
                }
            }
        }
    }
}

/// # Safety
/// The GL context and the program the location belongs to must be current on this thread.
pub(super) unsafe fn set(
    gl: &glow::Context,
    location: Option<&glow::UniformLocation>,
    value: &Constant,
) {
    unsafe {
        match *value {
            Constant::Bool(flag) => gl.uniform_1_i32(location, i32::from(flag)),
            Constant::Int(number) => gl.uniform_1_i32(location, number),
            Constant::Float { count, .. } => {
                let [x, y, z, w] = value.floats();
                match count {
                    1 => gl.uniform_1_f32(location, x),
                    2 => gl.uniform_2_f32(location, x, y),
                    3 => gl.uniform_3_f32(location, x, y, z),
                    _ => gl.uniform_4_f32(location, x, y, z, w),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_matrix_places_a_position_where_the_affine_places_its_corner() {
        let affine = [[0.4, -0.3, 0.1], [0.2, 0.5, -0.2]];
        for size in [(300.0, 200.0), (-300.0, 200.0), (300.0, -200.0)] {
            let m = model_view_projection(&affine, size);
            for local in [[0.5f32, 0.5], [-0.5, 0.5], [-0.5, -0.5]] {
                let p = [local[0] * size.0, local[1] * size.1];
                let x = m[0] * p[0] + m[4] * p[1] + m[12];
                let y = m[1] * p[0] + m[5] * p[1] + m[13];
                let want = |row: [f32; 3]| row[0] * local[0] + row[1] * local[1] + row[2];
                assert!((x - want(affine[0])).abs() < 1e-5 && (y - want(affine[1])).abs() < 1e-5);
            }
        }
    }
}
