//! `TEXS`: an animated texture's frames, after its images. Each frame names its image, how long
//! it shows, and where it is in that image: the corner of the box it fills, and its top and left
//! sides, which may run backwards when a frame lies turned in the sheet. The first version stores
//! the places as whole pixels, the later ones as floats, and the third puts the frames' size in
//! front of them.

use crate::{Error, Reader};

/// The most frames an animation may have.
const MOST: usize = 65536;
const RECORD: usize = 32;

/// One frame of an animated texture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub image: usize,
    pub seconds: f32,
    /// Where the frame's top left corner is in its image, and how far its top and left sides
    /// run, as shares of the image's size: the frame's own (0, 0), (1, 0) and (0, 1).
    pub corner: [f32; 2],
    pub across: [f32; 2],
    pub down: [f32; 2],
    /// Its size in pixels.
    pub size: (f32, f32),
}

/// The frames after an animated texture's images, if it has any. `sizes` are each image's
/// largest mipmap's.
pub(super) fn read(reader: &mut Reader, sizes: &[(u32, u32)]) -> Result<Vec<Frame>, Error> {
    if reader.remaining() == 0 {
        return Ok(Vec::new());
    }
    let tag = reader.tag()?;
    let version = match tag.as_str() {
        "TEXS0001" => 1,
        "TEXS0002" => 2,
        "TEXS0003" => 3,
        _ => return Err(Error::Format(format!("unsupported texture frames {tag}"))),
    };
    let count = reader.u32()? as usize;
    if count == 0 || count > MOST {
        return Err(Error::Format(format!(
            "an animation must have 1–{MOST} frames, found {count}"
        )));
    }
    if version == 3 {
        // The frames' size, which their sides say too.
        reader.bytes(8)?;
    }
    if reader.remaining() < count * RECORD {
        return Err(Error::Format("texture frames cut short".into()));
    }
    let mut frames = Vec::with_capacity(count);
    for _ in 0..count {
        let image = usize::try_from(reader.i32()?)
            .ok()
            .filter(|image| *image < sizes.len())
            .ok_or_else(|| Error::Format("a frame in an image the texture hasn't".into()))?;
        let seconds = f32::from_bits(reader.u32()?);
        let mut place = [0.0f32; 6];
        for value in &mut place {
            let raw = reader.u32()?;
            *value = if version == 1 {
                raw as i32 as f32
            } else {
                f32::from_bits(raw)
            };
        }
        if !seconds.is_finite() || seconds < 0.0 || place.iter().any(|value| !value.is_finite()) {
            return Err(Error::Format("a frame with a broken time or place".into()));
        }
        let (width, height) = sizes[image];
        let share = |x: f32, y: f32| [x / width as f32, y / height as f32];
        // A side running backwards starts at the far edge of the box.
        let start =
            |box_corner: f32, across: f32, down: f32| box_corner - across.min(0.0) - down.min(0.0);
        frames.push(Frame {
            image,
            seconds,
            corner: share(
                start(place[0], place[2], place[4]),
                start(place[1], place[3], place[5]),
            ),
            across: share(place[2], place[3]),
            down: share(place[4], place[5]),
            size: (place[2].hypot(place[3]), place[4].hypot(place[5])),
        });
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use crate::tex::{ANIMATED, Texture};

    /// A 4×2 animated texture in container version 2, then `frames`.
    fn sheet(frames: &[u8]) -> Vec<u8> {
        let mut out = b"TEXV0005\0TEXI0001\0".to_vec();
        for n in [0, ANIMATED, 4, 2, 4, 2, 0] {
            out.extend(u32::to_le_bytes(n));
        }
        out.extend(b"TEXB0002\0");
        for n in [1, 1, 4, 2, 0, 0, 32] {
            out.extend(u32::to_le_bytes(n));
        }
        out.extend([0; 32]);
        out.extend(frames);
        out
    }

    fn frames(
        tag: &str,
        size: Option<(u32, u32)>,
        records: &[(i32, f32, [f32; 6])],
        whole: bool,
    ) -> Vec<u8> {
        let mut out = tag.as_bytes().to_vec();
        out.push(0);
        out.extend((records.len() as u32).to_le_bytes());
        if let Some((width, height)) = size {
            out.extend(width.to_le_bytes());
            out.extend(height.to_le_bytes());
        }
        for (image, seconds, place) in records {
            out.extend(image.to_le_bytes());
            out.extend(seconds.to_le_bytes());
            for value in place {
                if whole {
                    out.extend((*value as i32).to_le_bytes());
                } else {
                    out.extend(value.to_le_bytes());
                }
            }
        }
        out
    }

    #[test]
    fn frames_are_read_in_every_version_as_shares_of_their_image() {
        let records = [
            (0, 0.25, [0.0, 0.0, 2.0, 0.0, 0.0, 2.0]),
            // Turned in the sheet: its top runs up from the box's bottom left corner.
            (0, 0.5, [2.0, 0.0, 0.0, -2.0, 2.0, 0.0]),
        ];
        for (tag, size, whole) in [
            ("TEXS0001", None, true),
            ("TEXS0002", None, false),
            ("TEXS0003", Some((2, 2)), false),
        ] {
            let texture = Texture::parse(&sheet(&frames(tag, size, &records, whole))).unwrap();
            let [first, turned] = texture.frames[..] else {
                panic!("{tag}: {:?}", texture.frames);
            };
            assert_eq!(
                (first.corner, first.across, first.down),
                ([0.0; 2], [0.5, 0.0], [0.0, 1.0])
            );
            assert_eq!(turned.seconds, 0.5);
            assert_eq!(
                (turned.corner, turned.across, turned.down),
                ([0.5, 1.0], [0.0, -1.0], [0.5, 0.0])
            );
            assert_eq!(texture.frame_size(), Some((2, 2)));
        }
        // A still texture has no frames, and an animated one may have none written.
        assert!(Texture::parse(&sheet(&[])).unwrap().frames.is_empty());
    }

    #[test]
    fn broken_frames_are_errors() {
        let one = |image: i32, seconds: f32| {
            frames("TEXS0002", None, &[(image, seconds, [0.0; 6])], false)
        };
        for (bytes, why) in [
            (one(1, 0.1), "an image the texture hasn't"),
            (one(0, f32::NAN), "broken time"),
            (one(0, -1.0), "broken time"),
            (
                frames("TEXS0009", None, &[], false),
                "unsupported texture frames",
            ),
            (frames("TEXS0002", None, &[], false), "1–65536 frames"),
            (one(0, 0.1)[..20].to_vec(), "cut short"),
        ] {
            let error = Texture::parse(&sheet(&bytes)).err().unwrap().to_string();
            assert!(error.contains(why), "{why}: {error}");
        }
    }
}
