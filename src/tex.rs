//! `.tex`: Wallpaper Engine's texture container. Three tagged sections: `TEXV` names the
//! version, `TEXI` the pixel format, flags and sizes, and `TEXB` holds the images, each a list
//! of mipmaps that may be LZ4-compressed, DXT-compressed or a whole PNG or JPEG file.

use crate::dxt;
use crate::{Error, Reader};

pub const NO_INTERPOLATION: u32 = 1;
pub const CLAMP_UVS: u32 = 2;
/// The images are an animation's frames.
pub const ANIMATED: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Rgba8888,
    Dxt5,
    Dxt3,
    Dxt1,
    Rg88,
    R8,
}

impl Format {
    fn from_code(code: i32) -> Result<Self, Error> {
        Ok(match code {
            0 => Format::Rgba8888,
            4 => Format::Dxt5,
            6 => Format::Dxt3,
            7 => Format::Dxt1,
            8 => Format::Rg88,
            9 => Format::R8,
            other => return Err(Error::Format(format!("unknown texture format {other}"))),
        })
    }
}

/// How many channels a mipmap's pixels have, which is how it goes to the GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channels {
    Rgba,
    Rg,
    R,
}

impl Channels {
    pub fn count(self) -> usize {
        match self {
            Channels::Rgba => 4,
            Channels::Rg => 2,
            Channels::R => 1,
        }
    }
}

pub struct Mip {
    pub width: u32,
    pub height: u32,
    pub channels: Channels,
    pub pixels: Vec<u8>,
}

pub struct Texture {
    pub format: Format,
    pub flags: u32,
    /// The size it is stored at, often padded up to a power of two.
    pub width: u32,
    pub height: u32,
    /// The picture inside, from the top left corner; the rest is padding.
    pub image_width: u32,
    pub image_height: u32,
    /// Each image's mipmaps, largest first; more than one image only when animated.
    pub images: Vec<Vec<Mip>>,
}

impl Texture {
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        let mut reader = Reader::new(data);
        expect(&mut reader, "TEXV")?;
        expect(&mut reader, "TEXI")?;
        let format = Format::from_code(reader.i32()?)?;
        let flags = reader.u32()?;
        let (width, height) = (reader.u32()?, reader.u32()?);
        let (image_width, image_height) = (reader.u32()?, reader.u32()?);
        let _unknown = reader.u32()?;
        let container = expect(&mut reader, "TEXB")?;
        let version: u32 = container[4..].parse().unwrap_or(0);
        let count = reader.u32()? as usize;
        // From the third version on, a FreeImage format says when mipmaps are whole image
        // files; the fourth adds whether the texture is a video.
        let file = if version >= 3 { reader.i32()? } else { -1 };
        if version >= 4 && reader.u32()? != 0 {
            return Err(Error::Format("video textures aren't read yet".into()));
        }
        let mut images = Vec::with_capacity(count.min(256));
        for _ in 0..count {
            let mips = reader.u32()? as usize;
            let mut levels = Vec::with_capacity(mips.min(16));
            for _ in 0..mips {
                levels.push(mip(&mut reader, version, format, file)?);
            }
            images.push(levels);
        }
        Ok(Texture {
            format,
            flags,
            width,
            height,
            image_width,
            image_height,
            images,
        })
    }

    pub fn animated(&self) -> bool {
        self.flags & ANIMATED != 0
    }
}

fn expect(reader: &mut Reader, prefix: &str) -> Result<String, Error> {
    let tag = reader.tag()?;
    if tag.starts_with(prefix) && tag.len() == 8 {
        Ok(tag)
    } else {
        Err(Error::Format(format!("expected {prefix}, found {tag:?}")))
    }
}

fn mip(reader: &mut Reader, version: u32, format: Format, file: i32) -> Result<Mip, Error> {
    let (width, height) = (reader.u32()?, reader.u32()?);
    let (lz4, unpacked) = if version >= 2 {
        (reader.u32()? == 1, reader.u32()? as usize)
    } else {
        (false, 0)
    };
    let len = reader.u32()? as usize;
    let stored = reader.bytes(len)?;
    let bytes = if lz4 {
        lz4_flex::block::decompress(stored, unpacked)
            .map_err(|err| Error::Format(format!("an LZ4 mipmap doesn't unpack: {err}")))?
    } else {
        stored.to_vec()
    };
    if file != -1 {
        let image = image::load_from_memory(&bytes)
            .map_err(|err| Error::Format(format!("an embedded image doesn't decode: {err}")))?
            .into_rgba8();
        return Ok(Mip {
            width: image.width(),
            height: image.height(),
            channels: Channels::Rgba,
            pixels: image.into_raw(),
        });
    }
    let (w, h) = (width as usize, height as usize);
    let short = || Error::Format(format!("a {width}×{height} mipmap is cut short"));
    let (channels, pixels) = match format {
        Format::Dxt1 | Format::Dxt3 | Format::Dxt5 => {
            let kind = match format {
                Format::Dxt1 => dxt::Kind::Dxt1,
                Format::Dxt3 => dxt::Kind::Dxt3,
                _ => dxt::Kind::Dxt5,
            };
            (
                Channels::Rgba,
                dxt::decode(kind, &bytes, w, h).ok_or_else(short)?,
            )
        }
        Format::Rgba8888 | Format::Rg88 | Format::R8 => {
            let channels = match format {
                Format::Rgba8888 => Channels::Rgba,
                Format::Rg88 => Channels::Rg,
                _ => Channels::R,
            };
            let need = w * h * channels.count();
            if bytes.len() < need {
                return Err(short());
            }
            (channels, bytes[..need].to_vec())
        }
    };
    Ok(Mip {
        width,
        height,
        channels,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Built {
        out: Vec<u8>,
    }

    impl Built {
        fn new(format: i32, flags: u32, size: (u32, u32), container: &str, count: u32) -> Self {
            let mut out = b"TEXV0005\0TEXI0001\0".to_vec();
            for n in [format as u32, flags, size.0, size.1, size.0, size.1, 0] {
                out.extend(n.to_le_bytes());
            }
            out.extend(container.as_bytes());
            out.push(0);
            out.extend(count.to_le_bytes());
            Built { out }
        }

        fn u32(mut self, n: u32) -> Self {
            self.out.extend(n.to_le_bytes());
            self
        }

        /// One mipmap in the second container version or later.
        fn mip(self, width: u32, height: u32, lz4: Option<usize>, bytes: &[u8]) -> Self {
            let mut built = self
                .u32(width)
                .u32(height)
                .u32(lz4.is_some() as u32)
                .u32(lz4.unwrap_or(0) as u32)
                .u32(bytes.len() as u32);
            built.out.extend(bytes);
            built
        }
    }

    #[test]
    fn an_lz4_rgba_texture_unpacks() {
        let pixels: Vec<u8> = (0..2 * 2 * 4).map(|n| n as u8 * 10).collect();
        let packed = lz4_flex::block::compress(&pixels);
        let data = Built::new(0, CLAMP_UVS, (2, 2), "TEXB0003", 1)
            .u32(u32::MAX) // no FreeImage format
            .u32(1)
            .mip(2, 2, Some(pixels.len()), &packed)
            .out;
        let texture = Texture::parse(&data).unwrap();
        assert_eq!(texture.format, Format::Rgba8888);
        assert_eq!(texture.flags & CLAMP_UVS, CLAMP_UVS);
        assert_eq!(texture.images[0][0].pixels, pixels);
        assert!(!texture.animated());
    }

    #[test]
    fn dxt_and_single_channel_mipmaps_keep_their_shape() {
        let block = [0xff, 0xff, 0x00, 0x00, 0, 0, 0, 0];
        let data = Built::new(7, 0, (4, 4), "TEXB0002", 1)
            .u32(1)
            .mip(4, 4, None, &block)
            .out;
        let mip = &Texture::parse(&data).unwrap().images[0][0];
        assert_eq!((mip.channels, mip.pixels.len()), (Channels::Rgba, 64));
        assert_eq!(&mip.pixels[..4], &[255, 255, 255, 255]);

        let data = Built::new(9, 0, (2, 1), "TEXB0002", 1)
            .u32(1)
            .mip(2, 1, None, &[7, 9])
            .out;
        let mip = &Texture::parse(&data).unwrap().images[0][0];
        assert_eq!(
            (mip.channels, mip.pixels.as_slice()),
            (Channels::R, &[7u8, 9][..])
        );
    }

    #[test]
    fn an_embedded_png_is_decoded() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 4]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let data = Built::new(0, 0, (4, 2), "TEXB0004", 1)
            .u32(13) // FreeImage's PNG
            .u32(0) // not a video
            .u32(1)
            .mip(3, 2, None, &png)
            .out;
        let mip = &Texture::parse(&data).unwrap().images[0][0];
        assert_eq!((mip.width, mip.height), (3, 2));
        assert_eq!(&mip.pixels[..4], &[1, 2, 3, 4]);
    }

    #[test]
    fn broken_textures_are_errors_not_panics() {
        assert!(Texture::parse(b"TEXV0005\0TEXX0001\0").is_err());
        assert!(Texture::parse(b"").is_err());
        let data = Built::new(0, 0, (4, 4), "TEXB0002", 1)
            .u32(1)
            .mip(4, 4, None, &[1, 2, 3])
            .out;
        assert!(Texture::parse(&data).is_err());
        let data = Built::new(42, 0, (1, 1), "TEXB0002", 0).out;
        assert!(Texture::parse(&data).is_err());
        let data = Built::new(0, 0, (1, 1), "TEXB0004", 1)
            .u32(u32::MAX)
            .u32(1)
            .out;
        let refused = Texture::parse(&data).err().map(|err| err.to_string());
        assert!(refused.unwrap_or_default().contains("video"));
    }
}
