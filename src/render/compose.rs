//! Composition layers: a layer whose picture is the scene drawn so far under it, which its
//! effects change before it's drawn back in place. The frame is copied when the layer's turn
//! comes, and the layer's chain reads the part it covers.

use std::cell::Cell;

use glow::HasContext;

use super::Affine;

/// The frame drawn so far, copied for composition layers, and its size.
#[derive(Default)]
pub(super) struct Frame(Cell<Option<(glow::Texture, (u32, u32))>>);

impl Frame {
    /// Copies the frame drawn so far, `size` big, from the framebuffer being read.
    ///
    /// # Safety
    /// The GL context must be current on this thread, with the host's framebuffer bound.
    pub unsafe fn copy(&self, gl: &glow::Context, size: (u32, u32)) -> Option<glow::Texture> {
        unsafe {
            let texture = match self.0.get() {
                Some((texture, made)) if made == size => texture,
                held => {
                    if let Some((texture, _)) = held {
                        gl.delete_texture(texture);
                    }
                    let texture = gl.create_texture().ok()?;
                    gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                    gl.tex_image_2d(
                        glow::TEXTURE_2D,
                        0,
                        glow::RGBA8 as i32,
                        size.0 as i32,
                        size.1 as i32,
                        0,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelUnpackData::Slice(None),
                    );
                    for (name, value) in [
                        (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                        (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                        (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                        (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
                    ] {
                        gl.tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
                    }
                    self.0.set(Some((texture, size)));
                    texture
                }
            };
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.copy_tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                0,
                0,
                0,
                0,
                size.0 as i32,
                size.1 as i32,
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            Some(texture)
        }
    }

    /// # Safety
    /// The GL context must be current on this thread.
    pub unsafe fn delete(&self, gl: &glow::Context) {
        if let Some((texture, _)) = self.0.take() {
            unsafe { gl.delete_texture(texture) };
        }
    }
}

/// Where a layer placed by `affine` reads the copied frame: its picture's top left corner and
/// top and left sides, from the frame's bottom left with y up.
pub(super) fn reading(affine: &Affine) -> [f32; 6] {
    let [row0, row1] = affine;
    let corner = [
        -0.5 * row0[0] + 0.5 * row0[1] + row0[2],
        -0.5 * row1[0] + 0.5 * row1[1] + row1[2],
    ];
    [
        0.5 * corner[0] + 0.5,
        0.5 * corner[1] + 0.5,
        0.5 * row0[0],
        0.5 * row1[0],
        -0.5 * row0[1],
        -0.5 * row1[1],
    ]
}

#[cfg(test)]
mod tests {
    use super::reading;

    #[test]
    fn a_layer_reads_the_frame_where_it_lies() {
        // The left half of the screen, its top left at the screen's.
        let affine = [[1.0, 0.0, -0.5], [0.0, 2.0, 0.0]];
        let [x, y, ax, ay, dx, dy] = reading(&affine);
        assert_eq!([x, y], [0.0, 1.0]);
        assert_eq!([ax, ay, dx, dy], [0.5, 0.0, 0.0, -1.0]);
    }
}
