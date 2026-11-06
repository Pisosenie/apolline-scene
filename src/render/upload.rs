//! Textures onto the GPU: every mipmap checked against what it claims and against the
//! driver's limits, and all of them together kept within the scene's budget.

use glow::HasContext;

use crate::Error;
use crate::tex::{self, Channels, Texture};

/// The texture's first image with its mipmaps, and how much of it is picture rather than
/// padding.
pub(super) unsafe fn upload(
    gl: &glow::Context,
    texture: &Texture,
) -> Result<(glow::Texture, [f32; 2]), Error> {
    unsafe {
        let id = gl
            .create_texture()
            .map_err(|err| Error::Format(format!("a texture: {err}")))?;
        match fill(gl, id, texture) {
            Ok(uv_max) => Ok((id, uv_max)),
            Err(error) => {
                gl.delete_texture(id);
                Err(error)
            }
        }
    }
}

/// Puts the texture's first image and its mipmaps in `id`, whatever was there before.
pub(super) unsafe fn fill(
    gl: &glow::Context,
    id: glow::Texture,
    texture: &Texture,
) -> Result<[f32; 2], Error> {
    let mips = texture
        .images
        .first()
        .filter(|mips| !mips.is_empty())
        .ok_or_else(|| Error::Format("a texture without images".into()))?;
    let first = &mips[0];
    let max = unsafe { gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) }.max(0) as u32;
    validate_mips(mips, max)?;
    let share = |picture: u32, stored: u32| {
        if picture == 0 || stored == 0 {
            1.0
        } else {
            (picture as f32 / stored as f32).min(1.0)
        }
    };
    let uv_max = [
        share(texture.image_width, first.width),
        share(texture.image_height, first.height),
    ];
    let nearest = texture.flags & tex::NO_INTERPOLATION != 0;
    // A texture repeats unless it says to clamp; one padded up to a larger size clamps anyway,
    // since what it would repeat is the padding.
    let wrap = if texture.flags & tex::CLAMP_UVS != 0 || uv_max != [1.0; 2] {
        glow::CLAMP_TO_EDGE
    } else {
        glow::REPEAT
    };
    unsafe {
        // The host shares this context with libmpv and its own drawing; an error flag they
        // left behind isn't this upload's.
        for _ in 0..16 {
            if gl.get_error() == glow::NO_ERROR {
                break;
            }
        }
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(id));
        gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
        gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
        gl.pixel_store_i32(glow::UNPACK_SKIP_ROWS, 0);
        gl.pixel_store_i32(glow::UNPACK_SKIP_PIXELS, 0);
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        for (level, mip) in mips.iter().enumerate() {
            let (internal, format) = match mip.channels {
                Channels::Rgba => (glow::RGBA8, glow::RGBA),
                Channels::Rg => (glow::RG8, glow::RG),
                Channels::R => (glow::R8, glow::RED),
            };
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                level as i32,
                internal as i32,
                mip.width as i32,
                mip.height as i32,
                0,
                format,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&mip.pixels)),
            );
        }
        // Keep raw channels so the same upload can serve image layers and custom shaders.
        let (min, mag) = match (nearest, mips.len() > 1) {
            (true, _) => (glow::NEAREST, glow::NEAREST),
            (false, true) => (glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR),
            (false, false) => (glow::LINEAR, glow::LINEAR),
        };
        for (name, value) in [
            (glow::TEXTURE_MIN_FILTER, min),
            (glow::TEXTURE_MAG_FILTER, mag),
            (glow::TEXTURE_WRAP_S, wrap),
            (glow::TEXTURE_WRAP_T, wrap),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
        }
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAX_LEVEL,
            mips.len() as i32 - 1,
        );
        let error = gl.get_error();
        if error != glow::NO_ERROR {
            return Err(Error::Format(format!(
                "uploading a texture failed (GL {error:#x})"
            )));
        }
        Ok(uv_max)
    }
}

pub(super) fn reserve(allocated: &mut usize, bytes: usize) -> Result<(), Error> {
    *allocated = allocated
        .checked_add(bytes)
        .filter(|total| *total <= 512 * 1024 * 1024)
        .ok_or_else(|| Error::Format("scene GPU texture allocations exceed 512 MiB".into()))?;
    Ok(())
}

fn validate_mips(mips: &[tex::Mip], max: u32) -> Result<(), Error> {
    let first = mips
        .first()
        .ok_or_else(|| Error::Format("a texture without mipmaps".into()))?;
    let mut expected = (first.width, first.height);
    for (level, mip) in mips.iter().enumerate() {
        let size = (mip.width, mip.height);
        let bytes = (mip.width as usize)
            .checked_mul(mip.height as usize)
            .and_then(|n| n.checked_mul(mip.channels.count()));
        if size != expected
            || mip.width == 0
            || mip.height == 0
            || mip.width > max
            || mip.height > max
            || mip.channels != first.channels
            || bytes != Some(mip.pixels.len())
        {
            return Err(Error::Format(format!(
                "invalid mipmap {level}: {}×{} (GPU limit {max})",
                mip.width, mip.height
            )));
        }
        if size == (1, 1) && level + 1 != mips.len() {
            return Err(Error::Format("mipmaps continue past 1×1".into()));
        }
        expected = ((mip.width / 2).max(1), (mip.height / 2).max(1));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_budget_bounds_unique_texture_allocations() {
        let mut allocated = 0;
        assert!(reserve(&mut allocated, 256 * 1024 * 1024).is_ok());
        assert!(reserve(&mut allocated, 256 * 1024 * 1024).is_ok());
        assert!(reserve(&mut allocated, 4).is_err());
        assert_eq!(allocated, 512 * 1024 * 1024);
        assert!(reserve(&mut allocated, usize::MAX).is_err());
    }

    fn mip(w: u32, h: u32) -> tex::Mip {
        tex::Mip {
            width: w,
            height: h,
            channels: Channels::Rgba,
            pixels: vec![0; (w * h * 4) as usize],
        }
    }

    #[test]
    fn uploads_require_a_complete_and_consistent_mipmap_chain() {
        assert!(validate_mips(&[mip(8, 4), mip(4, 2), mip(2, 1), mip(1, 1)], 4096).is_ok());
        assert!(validate_mips(&[mip(8, 4)], 4).is_err());
        assert!(validate_mips(&[mip(8, 4), mip(3, 2)], 4096).is_err());
        assert!(validate_mips(&[mip(1, 1), mip(1, 1)], 4096).is_err());
        assert!(validate_mips(&[mip(0, 1)], 4096).is_err());
        let mut short = mip(4, 4);
        short.pixels.pop();
        assert!(validate_mips(&[short], 4096).is_err());
        let mut different = mip(2, 2);
        different.channels = Channels::R;
        different.pixels.truncate(4);
        assert!(validate_mips(&[mip(4, 4), different], 4096).is_err());
    }
}
