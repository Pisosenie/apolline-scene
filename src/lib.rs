//! Wallpaper Engine scenes, read from the user's own install and drawn with GLES. It stands
//! on its own: any program with a GL context can draw a scene with it.

pub mod dxt;
pub mod pkg;
pub mod project;
pub mod render;
pub mod scene;
pub mod tex;
mod value;

pub use project::{Blending, Layer, Project};
pub use render::Renderer;
pub use scene::Scene;

use std::fmt;

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

    /// A string prefixed with its length.
    pub fn string(&mut self) -> Result<String, Error> {
        let len = self.u32()? as usize;
        Ok(String::from_utf8_lossy(self.bytes(len)?).into_owned())
    }

    /// A tag ending in a zero byte, like `TEXV0005`.
    pub fn tag(&mut self) -> Result<String, Error> {
        let rest = &self.data[self.at.min(self.data.len())..];
        let len = rest
            .iter()
            .position(|&byte| byte == 0)
            .ok_or_else(|| Error::Format(format!("no tag at byte {}", self.at)))?;
        let tag = String::from_utf8_lossy(&rest[..len]).into_owned();
        self.at += len + 1;
        Ok(tag)
    }
}
