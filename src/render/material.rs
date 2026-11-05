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
    let (width, height) = (size.0.max(1e-6), size.1.max(1e-6));
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

pub(super) struct Frame {
    screen: Option<glow::UniformLocation>,
    time: Option<glow::UniformLocation>,
    delta: Option<glow::UniformLocation>,
    texel: Option<glow::UniformLocation>,
    half_texel: Option<glow::UniformLocation>,
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
            }
        }
    }

    pub(super) fn animated(&self) -> bool {
        self.time.is_some() || self.delta.is_some()
    }

    /// # Safety
    /// The GL context and this frame's program must be current on this thread.
    pub(super) unsafe fn update(
        &self,
        gl: &glow::Context,
        target: (u32, u32),
        time: f32,
        delta: f32,
    ) {
        let width = target.0.max(1) as f32;
        let height = target.1.max(1) as f32;
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
            gl.uniform_3_f32(self.screen.as_ref(), width, height, width / height);
            gl.uniform_2_f32(self.texel.as_ref(), 1.0 / width, 1.0 / height);
            gl.uniform_2_f32(self.half_texel.as_ref(), 0.5 / width, 0.5 / height);
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
