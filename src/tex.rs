//! `.tex`: Wallpaper Engine's texture container. Three tagged sections: `TEXV` names the
//! version, `TEXI` the pixel format, flags and sizes, and `TEXB` holds the images, each a list
//! of mipmaps that may be LZ4-compressed, DXT-compressed or a whole PNG or JPEG file.

use std::borrow::Cow;
use std::io::Cursor;

use image::{DynamicImage, ImageDecoder, ImageReader};

use crate::dxt;
use crate::{Error, MAX_TEXTURE_BYTES, Reader};

mod frames;

pub use frames::Frame;

const MAX_DIMENSION: u32 = 16384;
const MAX_IMAGES: usize = 1024;
const MAX_MIPS: usize = 15;

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
    /// An animation's frames, in order; none for a still picture.
    pub frames: Vec<Frame>,
}

impl Texture {
    /// One white pixel standing for a picture of this size, for layers of a plain colour.
    pub fn solid(width: u32, height: u32) -> Self {
        Texture {
            format: Format::Rgba8888,
            flags: CLAMP_UVS,
            width: 1,
            height: 1,
            image_width: width.clamp(1, MAX_DIMENSION),
            image_height: height.clamp(1, MAX_DIMENSION),
            images: vec![vec![Mip {
                width: 1,
                height: 1,
                channels: Channels::Rgba,
                pixels: vec![255; 4],
            }]],
            frames: Vec::new(),
        }
    }

    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        Self::parse_with_budget(data, MAX_TEXTURE_BYTES)
    }

    pub(crate) fn parse_with_budget(data: &[u8], budget: usize) -> Result<Self, Error> {
        Self::parse_inner(data, budget, false)
    }

    pub(crate) fn parse_still_with_budget(data: &[u8], budget: usize) -> Result<Self, Error> {
        Self::parse_inner(data, budget, true)
    }

    fn parse_inner(data: &[u8], budget: usize, still: bool) -> Result<Self, Error> {
        let mut reader = Reader::new(data);
        expect(&mut reader, "TEXV")?;
        expect(&mut reader, "TEXI")?;
        let format = Format::from_code(reader.i32()?)?;
        let flags = reader.u32()?;
        if still && flags & ANIMATED != 0 {
            return Err(Error::Format(
                "animated textures need another renderer".into(),
            ));
        }
        let (width, height) = (reader.u32()?, reader.u32()?);
        let (image_width, image_height) = (reader.u32()?, reader.u32()?);
        dimensions(width, height)?;
        dimensions(image_width, image_height)?;
        let _unknown = reader.u32()?;
        let container = expect(&mut reader, "TEXB")?;
        let version: u32 = container[4..].parse().unwrap_or(0);
        if !(1..=4).contains(&version) {
            return Err(Error::Format(format!(
                "unsupported texture container {container}"
            )));
        }
        let count = reader.u32()? as usize;
        if count == 0 || count > MAX_IMAGES {
            return Err(Error::Format(format!(
                "a texture must have 1–{MAX_IMAGES} images, found {count}"
            )));
        }
        if still && count != 1 {
            return Err(Error::Format(
                "multiple texture images need another renderer".into(),
            ));
        }
        // From the third version on, a FreeImage format says when mipmaps are whole image
        // files; the fourth adds whether the texture is a video.
        let file = if version >= 3 { reader.i32()? } else { -1 };
        if version >= 4 && reader.u32()? != 0 {
            return Err(Error::Format("video textures aren't read yet".into()));
        }
        let mut images = Vec::with_capacity(count);
        let mut remaining = budget.min(MAX_TEXTURE_BYTES);
        for _ in 0..count {
            let mips = reader.u32()? as usize;
            if mips == 0 || mips > MAX_MIPS {
                return Err(Error::Format(format!(
                    "an image must have 1–{MAX_MIPS} mipmaps, found {mips}"
                )));
            }
            let mut levels = Vec::with_capacity(mips);
            for _ in 0..mips {
                let mip = mip(&mut reader, version, format, file, remaining)?;
                remaining -= mip.pixels.len();
                levels.push(mip);
            }
            images.push(levels);
        }
        let frames = if flags & ANIMATED != 0 {
            let sizes: Vec<(u32, u32)> = images
                .iter()
                .map(|mips| (mips[0].width, mips[0].height))
                .collect();
            frames::read(&mut reader, &sizes)?
        } else {
            Vec::new()
        };
        Ok(Texture {
            format,
            flags,
            width,
            height,
            image_width,
            image_height,
            images,
            frames,
        })
    }

    /// The size of an animation's frames, in whole pixels.
    pub fn frame_size(&self) -> Option<(u32, u32)> {
        self.frames.first().map(|frame| {
            (
                (frame.size.0.round() as u32).clamp(1, MAX_DIMENSION),
                (frame.size.1.round() as u32).clamp(1, MAX_DIMENSION),
            )
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

fn dimensions(width: u32, height: u32) -> Result<(), Error> {
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(Error::Format(format!(
            "invalid texture size {width}×{height}; each side must be 1–{MAX_DIMENSION}"
        )));
    }
    Ok(())
}

fn decoded_bytes(
    width: u32,
    height: u32,
    channels: usize,
    remaining: usize,
) -> Result<usize, Error> {
    dimensions(width, height)?;
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(channels))
        .filter(|&bytes| bytes <= remaining)
        .ok_or_else(|| {
            Error::Format(format!(
                "texture exceeds the remaining decoded-pixel budget ({remaining} bytes)"
            ))
        })
}

fn mip(
    reader: &mut Reader,
    version: u32,
    format: Format,
    file: i32,
    remaining: usize,
) -> Result<Mip, Error> {
    let (width, height) = (reader.u32()?, reader.u32()?);
    let channels = match (file, format) {
        (-1, Format::Rg88) => Channels::Rg,
        (-1, Format::R8) => Channels::R,
        _ => Channels::Rgba,
    };
    let need = decoded_bytes(width, height, channels.count(), remaining)?;
    let (lz4, unpacked) = if version >= 2 {
        (reader.u32()? == 1, reader.u32()? as usize)
    } else {
        (false, 0)
    };
    let len = reader.u32()? as usize;
    if unpacked > MAX_TEXTURE_BYTES || len > MAX_TEXTURE_BYTES {
        return Err(Error::Format("a stored mipmap exceeds 256 MiB".into()));
    }
    // An embedded image's packed size says nothing of its pixels, so it only has to fit what
    // is left of the budget and what LZ4 can expand to.
    if lz4 && file != -1 && (unpacked > remaining || unpacked > len.saturating_mul(255)) {
        return Err(Error::Format(
            "an LZ4 image unpacks to more than it can hold".into(),
        ));
    }
    if lz4 && file == -1 {
        let expected = match format {
            Format::Dxt1 => dxt::size(dxt::Kind::Dxt1, width as usize, height as usize),
            Format::Dxt3 => dxt::size(dxt::Kind::Dxt3, width as usize, height as usize),
            Format::Dxt5 => dxt::size(dxt::Kind::Dxt5, width as usize, height as usize),
            _ => Some(need),
        };
        if expected != Some(unpacked) {
            return Err(Error::Format(
                "an LZ4 mipmap's size disagrees with its dimensions".into(),
            ));
        }
    }
    let stored = reader.bytes(len)?;
    let bytes = if lz4 {
        Cow::Owned(
            lz4_flex::block::decompress(stored, unpacked)
                .map_err(|err| Error::Format(format!("an LZ4 mipmap doesn't unpack: {err}")))?,
        )
    } else {
        Cow::Borrowed(stored)
    };
    if file != -1 {
        let fail = |err| Error::Format(format!("an embedded image doesn't decode: {err}"));
        let mut reader = ImageReader::new(Cursor::new(bytes.as_ref()))
            .with_guessed_format()
            .map_err(|err| Error::Format(err.to_string()))?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(MAX_DIMENSION);
        limits.max_image_height = Some(MAX_DIMENSION);
        limits.max_alloc = Some(remaining as u64);
        reader.limits(limits);
        let decoder = reader.into_decoder().map_err(fail)?;
        let (actual_w, actual_h) = decoder.dimensions();
        decoded_bytes(actual_w, actual_h, 4, remaining)?;
        let image = DynamicImage::from_decoder(decoder)
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
            if bytes.len() < need {
                return Err(short());
            }
            let pixels = match bytes {
                Cow::Borrowed(bytes) => bytes[..need].to_vec(),
                Cow::Owned(mut bytes) => {
                    bytes.truncate(need);
                    bytes
                }
            };
            (channels, pixels)
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
    fn still_preparation_refuses_animation_before_any_pixel_decode() {
        let animated = Built::new(0, ANIMATED, (16384, 16384), "TEXB0002", 1).out;
        assert!(
            Texture::parse_still_with_budget(&animated, 0)
                .err()
                .unwrap()
                .to_string()
                .contains("animated")
        );
        let frames = Built::new(0, 0, (1, 1), "TEXB0002", 2).out;
        assert!(
            Texture::parse_still_with_budget(&frames, 0)
                .err()
                .unwrap()
                .to_string()
                .contains("multiple texture images")
        );
        let still = Built::new(0, 0, (1, 1), "TEXB0002", 1)
            .u32(1)
            .mip(1, 1, None, &[1, 2, 3, 4])
            .out;
        assert_eq!(
            Texture::parse_still_with_budget(&still, 4).unwrap().images[0][0].pixels,
            [1, 2, 3, 4]
        );
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

    #[test]
    fn size_limits_apply_before_decompression() {
        for size in [(0, 1), (1, 0), (u32::MAX, u32::MAX), (16384, 16384)] {
            let data = Built::new(0, 0, size, "TEXB0002", 1)
                .u32(1)
                .mip(size.0, size.1, None, &[])
                .out;
            assert!(Texture::parse(&data).is_err());
        }
        let data = Built::new(0, 0, (1, 1), "TEXB0002", 1)
            .u32(1)
            .mip(1, 1, Some(u32::MAX as usize), &[])
            .out;
        assert!(
            Texture::parse(&data)
                .err()
                .unwrap()
                .to_string()
                .contains("256 MiB")
        );
        let data = Built::new(0, 0, (1, 1), "TEXB0002", 1)
            .u32(1)
            .mip(1, 1, Some(64), &[])
            .out;
        assert!(
            Texture::parse(&data)
                .err()
                .unwrap()
                .to_string()
                .contains("disagrees")
        );
    }

    #[test]
    fn the_decode_budget_includes_previous_frames() {
        let built = Built { out: Vec::new() }.mip(2, 2, None, &[1; 16]);
        assert!(mip(&mut Reader::new(&built.out), 2, Format::Rgba8888, -1, 15).is_err());
        assert_eq!(
            mip(&mut Reader::new(&built.out), 2, Format::Rgba8888, -1, 16)
                .unwrap()
                .pixels
                .len(),
            16
        );
    }

    #[test]
    fn embedded_images_cannot_bypass_the_decode_budget() {
        let mut png = Vec::new();
        image::RgbaImage::new(16, 16)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let built = Built { out: Vec::new() }.mip(1, 1, None, &png);
        assert!(mip(&mut Reader::new(&built.out), 3, Format::Rgba8888, 13, 64).is_err());
    }

    #[test]
    fn raw_mipmaps_do_not_retain_unused_stored_bytes() {
        let stored = vec![7; 1024 * 1024];
        for (format, channels) in [(0, 4), (8, 2), (9, 1)] {
            let data = Built::new(format, 0, (1, 1), "TEXB0002", 1)
                .u32(1)
                .mip(1, 1, None, &stored)
                .out;
            let texture = Texture::parse_with_budget(&data, channels).unwrap();
            let pixels = &texture.images[0][0].pixels;
            assert_eq!(pixels.as_slice(), vec![7; channels]);
            assert!(
                pixels.capacity() <= 16,
                "padding retained in the pixel allocation"
            );
        }
    }

    #[test]
    fn every_truncated_prefix_is_an_error() {
        let data = Built::new(0, 0, (2, 2), "TEXB0002", 1)
            .u32(1)
            .mip(2, 2, None, &[0; 16])
            .out;
        for end in 0..data.len() {
            assert!(Texture::parse(&data[..end]).is_err(), "prefix {end}");
        }
        assert!(Texture::parse(&data).is_ok());
    }
}
