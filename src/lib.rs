//! Wallpaper Engine scenes, read from the user's own install and drawn with GLES. It stands
//! on its own: any program with a GL context can draw a scene with it.

pub mod dxt;
mod files;
pub mod particles;
pub mod pkg;
pub mod project;
pub mod properties;
pub mod render;
pub mod scene;
pub mod script;
pub mod shader;
pub mod tex;
mod value;

pub use project::{Blending, Layer, Prepared, Project, TextureInput};
pub use render::Renderer;
pub use scene::{Missing, Pose, Scene};
pub use script::{Moment, Stage};

use std::fmt;

/// A decoded texture, including all its frames and mipmaps, may use at most 256 MiB.
pub const MAX_TEXTURE_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug)]
pub enum Error {
    Io(String, std::io::Error),
    /// The bytes aren't what the format says they should be.
    Format(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(path, err) => write!(f, "{path}: {err}"),
            Error::Format(what) => f.write_str(what),
        }
    }
}

impl std::error::Error for Error {}

/// Little-endian numbers and strings from a byte slice, failing instead of panicking at its
/// end.
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, at: 0 }
    }

    pub fn position(&self) -> usize {
        self.at
    }

    pub fn bytes(&mut self, len: usize) -> Result<&'a [u8], Error> {
        let end = self
            .at
            .checked_add(len)
            .filter(|&end| end <= self.data.len())
            .ok_or_else(|| Error::Format(format!("cut short at byte {}", self.at)))?;
        let bytes = &self.data[self.at..end];
        self.at = end;
        Ok(bytes)
    }

    pub fn u32(&mut self) -> Result<u32, Error> {
        let bytes = self.bytes(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn i32(&mut self) -> Result<i32, Error> {
        self.u32().map(|n| n as i32)
    }

    /// A bounded UTF-8 string prefixed with its byte length.
    pub fn string(&mut self, limit: usize) -> Result<String, Error> {
        let len = self.u32()? as usize;
        if len > limit {
            return Err(Error::Format(format!("string exceeds {limit} bytes")));
        }
        let bytes = self.bytes(len)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| Error::Format("a package string is not UTF-8".into()))
    }

    /// An eight-byte texture tag followed by a zero byte, like `TEXV0005`.
    pub fn tag(&mut self) -> Result<String, Error> {
        let bytes = self.bytes(9)?;
        if bytes[8] != 0 || !bytes[..8].is_ascii() || bytes[..8].contains(&0) {
            return Err(Error::Format(
                "a texture tag must contain eight ASCII bytes and a terminator".into(),
            ));
        }
        String::from_utf8(bytes[..8].to_vec())
            .map_err(|_| Error::Format("invalid texture tag".into()))
    }
}

#[cfg(test)]
mod reader_tests {
    use super::*;
    #[test]
    fn oversized_tags_and_strings_are_rejected_before_copying_their_contents() {
        let mut data = vec![b'x'; 1024 * 1024];
        data.push(0);
        let mut reader = Reader::new(&data);
        assert!(reader.tag().is_err());
        assert_eq!(reader.position(), 9);
        let mut reader = Reader::new(&[255, 255, 255, 255]);
        assert!(
            reader
                .string(4096)
                .unwrap_err()
                .to_string()
                .contains("4096")
        );
        assert_eq!(reader.position(), 4);
        assert_eq!(Reader::new(b"TEXV0005\0").tag().unwrap(), "TEXV0005");
        assert!(Reader::new(b"TEXV\x00005\0").tag().is_err());
        assert!(Reader::new(&[1, 0, 0, 0, 255]).string(4096).is_err());
    }
}
