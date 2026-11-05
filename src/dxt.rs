//! DXT1, DXT3 and DXT5 (BC1–BC3) blocks decoded to RGBA, for drivers without S3TC and for
//! reading pixels on the CPU.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dxt1,
    Dxt3,
    Dxt5,
}

impl Kind {
    fn block_bytes(self) -> usize {
        match self {
            Kind::Dxt1 => 8,
            Kind::Dxt3 | Kind::Dxt5 => 16,
        }
    }
}

/// The bytes a `width` × `height` image takes in 4×4 blocks.
pub fn size(kind: Kind, width: usize, height: usize) -> Option<usize> {
    width
        .div_ceil(4)
        .checked_mul(height.div_ceil(4))?
        .checked_mul(kind.block_bytes())
}

/// Decodes a whole image; refuses empty, truncated or oversized images.
pub fn decode(kind: Kind, data: &[u8], width: usize, height: usize) -> Option<Vec<u8>> {
    let stored = size(kind, width, height)?;
    let bytes = width.checked_mul(height)?.checked_mul(4)?;
    if bytes == 0 || bytes > crate::MAX_TEXTURE_BYTES || data.len() < stored {
        return None;
    }
    let mut out = vec![0u8; bytes];
    let blocks = data.chunks_exact(kind.block_bytes());
    let across = width.div_ceil(4);
    for (index, block) in blocks.take(stored / kind.block_bytes()).enumerate() {
        let (bx, by) = (index % across * 4, index / across * 4);
        let pixels = match kind {
            Kind::Dxt1 => colours(block, true),
            Kind::Dxt3 => with_alpha(colours(&block[8..], false), explicit_alpha(&block[..8])),
            Kind::Dxt5 => with_alpha(colours(&block[8..], false), interpolated_alpha(&block[..8])),
        };
        for (at, pixel) in pixels.iter().enumerate() {
            let (x, y) = (bx + at % 4, by + at / 4);
            if x < width && y < height {
                let start = (y * width + x) * 4;
                out[start..start + 4].copy_from_slice(pixel);
            }
        }
    }
    Some(out)
}

fn rgb565(value: u16) -> [u8; 3] {
    let (r, g, b) = ((value >> 11) & 31, (value >> 5) & 63, value & 31);
    [
        ((r << 3) | (r >> 2)) as u8,
        ((g << 2) | (g >> 4)) as u8,
        ((b << 3) | (b >> 2)) as u8,
    ]
}

/// The 16 colours of a colour block. DXT1 alone has a mode with three colours and a
/// transparent one, chosen by the order of the two endpoints.
fn colours(block: &[u8], dxt1: bool) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let (a, b) = (rgb565(c0), rgb565(c1));
    let mix = |wa: u16, wb: u16, total: u16| -> [u8; 4] {
        let channel = |i: usize| ((a[i] as u16 * wa + b[i] as u16 * wb) / total) as u8;
        [channel(0), channel(1), channel(2), 255]
    };
    let palette = if c0 > c1 || !dxt1 {
        [
            [a[0], a[1], a[2], 255],
            [b[0], b[1], b[2], 255],
            mix(2, 1, 3),
            mix(1, 2, 3),
        ]
    } else {
        [
            [a[0], a[1], a[2], 255],
            [b[0], b[1], b[2], 255],
            mix(1, 1, 2),
            [0, 0, 0, 0],
        ]
    };
    let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    std::array::from_fn(|at| palette[(indices >> (at * 2) & 3) as usize])
}

fn explicit_alpha(block: &[u8]) -> [u8; 16] {
    let bits = u64::from_le_bytes(block[..8].try_into().unwrap_or_default());
    std::array::from_fn(|at| ((bits >> (at * 4)) & 15) as u8 * 17)
}

fn interpolated_alpha(block: &[u8]) -> [u8; 16] {
    let (a0, a1) = (block[0] as u16, block[1] as u16);
    let level = |i: u16| -> u8 {
        match i {
            0 => a0 as u8,
            1 => a1 as u8,
            i if a0 > a1 => (((8 - i) * a0 + (i - 1) * a1) / 7) as u8,
            6 => 0,
            7 => 255,
            i => (((6 - i) * a0 + (i - 1) * a1) / 5) as u8,
        }
    };
    let mut bits = [0u8; 8];
    bits[..6].copy_from_slice(&block[2..8]);
    let bits = u64::from_le_bytes(bits);
    std::array::from_fn(|at| level(((bits >> (at * 3)) & 7) as u16))
}

fn with_alpha(mut pixels: [[u8; 4]; 16], alpha: [u8; 16]) -> [[u8; 4]; 16] {
    for (pixel, alpha) in pixels.iter_mut().zip(alpha) {
        pixel[3] = alpha;
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dxt1_block_mixes_its_two_endpoints() {
        // White and black, then the indices 0, 1, 2, 3 across the first row.
        let block = [0xff, 0xff, 0x00, 0x00, 0b1110_0100, 0, 0, 0];
        let rgba = decode(Kind::Dxt1, &block, 4, 4).unwrap();
        let row: Vec<&[u8]> = rgba.chunks(4).take(4).collect();
        assert_eq!(
            row,
            [
                &[255, 255, 255, 255][..],
                &[0, 0, 0, 255],
                &[170, 170, 170, 255],
                &[85, 85, 85, 255]
            ]
        );
    }

    #[test]
    fn dxt1_with_its_endpoints_swapped_has_a_transparent_colour() {
        let block = [0x00, 0x00, 0xff, 0xff, 0b0000_0011, 0, 0, 0];
        let rgba = decode(Kind::Dxt1, &block, 4, 4).unwrap();
        assert_eq!(&rgba[..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn dxt5_alpha_runs_between_its_two_levels() {
        // Alpha 255 to 0 in eight steps; the first pixel takes level 2, 6/7 of the way to 255.
        let mut block = [0u8; 16];
        block[0] = 255;
        block[1] = 0;
        block[2] = 0b010;
        block[8..12].copy_from_slice(&[0xff, 0xff, 0xff, 0xff]);
        let rgba = decode(Kind::Dxt5, &block, 4, 4).unwrap();
        assert_eq!(rgba[3], 218);
        assert_eq!(rgba[7], 255);
    }

    #[test]
    fn images_that_are_not_whole_blocks_are_cropped_and_short_data_refused() {
        let block = [0xff, 0xff, 0x00, 0x00, 0, 0, 0, 0];
        assert_eq!(decode(Kind::Dxt1, &block, 3, 2).unwrap().len(), 3 * 2 * 4);
        assert_eq!(decode(Kind::Dxt1, &block[..7], 4, 4), None);
        assert_eq!(size(Kind::Dxt5, 5, 5), Some(4 * 16));
    }

    #[test]
    fn impossible_dimensions_do_not_allocate_or_overflow() {
        for (w, h) in [(0, 4), (4, 0), (usize::MAX, usize::MAX), (usize::MAX, 1)] {
            assert!(decode(Kind::Dxt5, &[0; 16], w, h).is_none());
        }
        assert_eq!(size(Kind::Dxt5, usize::MAX, usize::MAX), None);
    }
}
