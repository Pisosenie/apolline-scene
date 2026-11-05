//! What a draw changes on the host's context, saved before and put back after: the host binds
//! its own framebuffer and viewport, and render targets inside a draw must not leave theirs.

use glow::HasContext;

pub(super) struct Host {
    framebuffer: Option<glow::Framebuffer>,
    viewport: [i32; 4],
}

impl Host {
    /// # Safety
    /// `gl` has to be current on this thread.
    pub unsafe fn save(gl: &glow::Context) -> Self {
        let mut viewport = [0; 4];
        unsafe {
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            Host {
                framebuffer: gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING),
                viewport,
            }
        }
    }

    /// The host's framebuffer, bound again for drawing into it.
    ///
    /// # Safety
    /// `gl` has to be current on this thread.
    pub unsafe fn bind(&self, gl: &glow::Context) {
        unsafe { gl.bind_framebuffer(glow::FRAMEBUFFER, self.framebuffer) };
    }

    /// # Safety
    /// `gl` has to be current on this thread.
    pub unsafe fn restore(self, gl: &glow::Context) {
        let [x, y, w, h] = self.viewport;
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, self.framebuffer);
            gl.viewport(x, y, w, h);
        }
    }
}
