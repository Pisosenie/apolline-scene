//! Puppet warp: a picture cut into a mesh whose corners hang from a skeleton of bones, and the
//! animations that move those bones. Wallpaper Engine keeps it all in one `.mdl` file: the mesh
//! (`MDLV`), the skeleton (`MDLS`) and the animations (`MDLA`), each in versions of their own.
//! This reads the versions the scenes here use; the layout was worked out from those files.

mod pose;

use crate::Error;

pub use pose::{Pose, Posed};

/// The most corners and triangles a mesh may have, and bones a skeleton.
const CORNERS: usize = 1 << 20;
const TRIANGLES: usize = 1 << 21;
const BONES: usize = 256;
/// The most frames an animation may hold for each bone.
const FRAMES: usize = 1 << 16;

/// What a corner carries, by the bits of its mesh's flags.
const POSITION: u32 = 1;
const NORMAL: u32 = 2;
const TANGENT: u32 = 4;
const TEXCOORD: u32 = 8;
const SKIN: u32 = 0x0180_0000;

#[derive(Clone, Debug, PartialEq)]
pub struct Puppet {
    pub corners: Vec<Corner>,
    pub triangles: Vec<[u16; 3]>,
    /// In order, each after its parent.
    pub bones: Vec<Bone>,
    pub animations: Vec<Animation>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Corner {
    /// In the picture's pixels from its middle, y up.
    pub position: [f32; 2],
    /// Where in the picture it shows, 0 to 1 from its top left.
    pub uv: [f32; 2],
    pub bones: [u8; 4],
    pub weights: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bone {
    pub parent: Option<usize>,
    /// Where it sits from its parent when nothing moves it.
    pub rest: Pose,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Animation {
    pub id: u32,
    pub fps: f32,
    /// How many frames it runs; each bone holds one more, the first again for a loop.
    pub frames: usize,
    pub mode: Mode,
    /// Each bone's pose in each frame, in the skeleton's order.
    pub tracks: Vec<Vec<Pose>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Loop,
    /// Forwards, then backwards.
    Mirror,
    /// Once, then holding its last frame.
    Single,
}

impl Puppet {
    pub fn parse(bytes: &[u8]) -> Result<Puppet, Error> {
        let mut read = Reader { bytes, at: 0 };
        let version = read.version(b"MDLV")?;
        // Flags, then two counts nothing here needs.
        read.skip(12)?;
        read.text()?;
        let flags = match version {
            13 => {
                read.skip(4)?;
                POSITION | TEXCOORD | SKIN
            }
            14..=18 => {
                read.skip(4)?;
                read.u32()?
            }
            19..=23 => {
                read.skip(28)?;
                read.u32()?
            }
            _ => return Err(refuse("a puppet mesh version")),
        };
        if flags & (POSITION | TEXCOORD) != POSITION | TEXCOORD
            || flags & !(POSITION | NORMAL | TANGENT | TEXCOORD | SKIN) != 0
        {
            return Err(refuse("puppet corners laid out another way"));
        }
        let skinned = flags & SKIN == SKIN;
        let stride = 12
            + if flags & NORMAL != 0 { 12 } else { 0 }
            + if flags & TANGENT != 0 { 16 } else { 0 }
            + if skinned { 32 } else { 0 }
            + 8;
        let size = read.u32()? as usize;
        if !size.is_multiple_of(stride) || size / stride > CORNERS {
            return Err(refuse("a puppet mesh of the wrong size"));
        }
        let mut corners = Vec::with_capacity(size / stride);
        for _ in 0..size / stride {
            let [x, y, _] = read.floats()?;
            read.skip(stride - 12 - 8 - if skinned { 32 } else { 0 })?;
            let (bones, weights) = if skinned {
                let ids: [u32; 4] = [read.u32()?, read.u32()?, read.u32()?, read.u32()?];
                (ids.map(|id| id.min(255) as u8), read.floats()?)
            } else {
                ([0; 4], [0.0; 4])
            };
            corners.push(Corner {
                position: [x, y],
                uv: read.floats()?,
                bones,
                weights,
            });
        }
        let size = read.u32()? as usize;
        if !size.is_multiple_of(6) || size / 6 > TRIANGLES {
            return Err(refuse("a puppet mesh of the wrong size"));
        }
        let mut triangles = Vec::with_capacity(size / 6);
        for _ in 0..size / 6 {
            let triangle = [read.u16()?, read.u16()?, read.u16()?];
            if triangle.iter().any(|&at| usize::from(at) >= corners.len()) {
                return Err(refuse("a puppet triangle past its corners"));
            }
            triangles.push(triangle);
        }
        if !skinned {
            return Ok(Puppet {
                corners,
                triangles,
                bones: Vec::new(),
                animations: Vec::new(),
            });
        }
        let bones = skeleton(&mut read)?;
        if corners.iter().any(|corner| {
            corner
                .bones
                .iter()
                .any(|&bone| usize::from(bone) >= bones.len())
        }) {
            return Err(refuse("a puppet corner on a bone it doesn't have"));
        }
        let animations = if read.at < bytes.len() {
            animations(&mut read, bones.len())?
        } else {
            Vec::new()
        };
        Ok(Puppet {
            corners,
            triangles,
            bones,
            animations,
        })
    }
}

fn skeleton(read: &mut Reader) -> Result<Vec<Bone>, Error> {
    // Version 4 lays its bones out another way, not yet worked out.
    if !(1..=3).contains(&read.version(b"MDLS")?) {
        return Err(refuse("a puppet skeleton version"));
    }
    // Where the skeleton ends: past its bones, older ones keep what the editor draws them with.
    let end = read.u32()? as usize;
    let count = read.u32()? as usize;
    if count == 0 || count > BONES {
        return Err(refuse("a puppet without bones or with too many"));
    }
    let mut bones = Vec::with_capacity(count);
    for at in 0..count {
        read.text()?;
        read.skip(4)?;
        let parent = match read.u32()? {
            u32::MAX => None,
            parent if (parent as usize) < at => Some(parent as usize),
            _ => return Err(refuse("a puppet bone before its parent")),
        };
        if read.u32()? != 64 {
            return Err(refuse("a puppet bone without its matrix"));
        }
        let mut matrix = [0.0; 16];
        for value in &mut matrix {
            *value = read.f32()?;
        }
        // The editor's notes on the bone.
        read.text()?;
        bones.push(Bone {
            parent,
            rest: Pose::from_matrix(&matrix),
        });
    }
    if end < read.at || end > read.bytes.len() {
        return Err(refuse("a puppet skeleton of the wrong size"));
    }
    read.at = end;
    Ok(bones)
}

/// The first animation only: what follows each one's frames differs between versions and
/// isn't read, so where the next starts isn't known.
fn animations(read: &mut Reader, bones: usize) -> Result<Vec<Animation>, Error> {
    read.version(b"MDLA")?;
    read.skip(4)?;
    if read.u32()? == 0 {
        return Ok(Vec::new());
    }
    let id = read.u32()?;
    read.skip(4)?;
    read.text()?;
    let mode = match read.text()?.as_str() {
        "loop" => Mode::Loop,
        "mirror" => Mode::Mirror,
        "single" => Mode::Single,
        _ => return Err(refuse("a puppet animation that plays another way")),
    };
    let fps = read.f32()?;
    let frames = read.u32()? as usize;
    read.skip(4)?;
    if !(fps > 0.0 && fps <= 1000.0) || frames == 0 || frames >= FRAMES {
        return Err(refuse("a puppet animation without frames"));
    }
    if read.u32()? as usize != bones {
        return Err(refuse("a puppet animation for another skeleton"));
    }
    let mut tracks = Vec::with_capacity(bones);
    for _ in 0..bones {
        read.skip(4)?;
        let size = read.u32()? as usize;
        if !size.is_multiple_of(36) || size / 36 == 0 || size / 36 > frames + 1 {
            return Err(refuse("a puppet animation of the wrong size"));
        }
        let mut track = Vec::with_capacity(size / 36);
        for _ in 0..size / 36 {
            let [x, y, _] = read.floats()?;
            let [_, _, angle] = read.floats()?;
            let [sx, sy, _] = read.floats()?;
            track.push(Pose {
                position: [x, y],
                angle,
                scale: [sx, sy],
            });
        }
        tracks.push(track);
    }
    Ok(vec![Animation {
        id,
        fps,
        frames,
        mode,
        tracks,
    }])
}

fn refuse(what: &str) -> Error {
    Error::Format(format!("unsupported {what}"))
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], Error> {
        let end = self
            .at
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| Error::Format("a puppet model cut short".into()))?;
        let taken = &self.bytes[self.at..end];
        self.at = end;
        Ok(taken)
    }

    fn skip(&mut self, count: usize) -> Result<(), Error> {
        self.take(count).map(drop)
    }

    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().unwrap_or_default(),
        ))
    }

    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().unwrap_or_default(),
        ))
    }

    fn f32(&mut self) -> Result<f32, Error> {
        let value = f32::from_le_bytes(self.take(4)?.try_into().unwrap_or_default());
        if value.is_finite() {
            Ok(value)
        } else {
            Err(Error::Format("a puppet number that isn't one".into()))
        }
    }

    fn floats<const N: usize>(&mut self) -> Result<[f32; N], Error> {
        let mut out = [0.0; N];
        for value in &mut out {
            *value = self.f32()?;
        }
        Ok(out)
    }

    fn text(&mut self) -> Result<String, Error> {
        let rest = &self.bytes[self.at..];
        let end = rest
            .iter()
            .position(|&byte| byte == 0)
            .filter(|end| *end <= 4096)
            .ok_or_else(|| Error::Format("a puppet name cut short".into()))?;
        let text = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.at += end + 1;
        Ok(text)
    }

    /// A section's tag and four digit version, ended by a zero.
    fn version(&mut self, tag: &[u8; 4]) -> Result<u32, Error> {
        let head = self.take(9)?;
        if &head[..4] != tag || head[8] != 0 {
            return Err(refuse("a puppet model that isn't one"));
        }
        std::str::from_utf8(&head[4..8])
            .ok()
            .and_then(|digits| digits.parse().ok())
            .ok_or_else(|| refuse("a puppet model that isn't one"))
    }
}

#[cfg(test)]
mod tests;
