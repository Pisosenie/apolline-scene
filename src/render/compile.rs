use crate::Error;
use glow::HasContext;

/// # Safety
/// The GL context must be current on this thread.
pub(super) unsafe fn link(
    gl: &glow::Context,
    vertex: &str,
    fragment: &str,
) -> Result<glow::Program, Error> {
    let fail = |message: String| {
        Error::Format(format!(
            "material shader: {}",
            message.chars().take(16384).collect::<String>()
        ))
    };
    unsafe {
        let program = gl.create_program().map_err(fail)?;
        let mut shaders = Vec::new();
        let result = (|| {
            for (kind, body) in [
                (glow::VERTEX_SHADER, vertex),
                (glow::FRAGMENT_SHADER, fragment),
            ] {
                let shader = gl.create_shader(kind).map_err(fail)?;
                shaders.push(shader);
                gl.shader_source(shader, &format!("{}{body}", super::header(gl)));
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    return Err(fail(gl.get_shader_info_log(shader)));
                }
                gl.attach_shader(program, shader);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                return Err(fail(gl.get_program_info_log(program)));
            }
            Ok(program)
        })();
        for shader in shaders {
            if gl.get_shader_compile_status(shader) {
                gl.detach_shader(program, shader);
            }
            gl.delete_shader(shader);
        }
        if result.is_err() {
            gl.delete_program(program);
        }
        result
    }
}
