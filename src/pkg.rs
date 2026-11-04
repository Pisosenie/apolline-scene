//! `scene.pkg`: a version string, a table of files with their offsets and lengths, then the
//! files one after another. Every number is a little-endian u32 and every string is
//! prefixed with its length.

use std::path::Path;

use crate::{Error, Reader};

pub struct Package {
    pub version: String,
    data: Vec<u8>,
    files: Vec<File>,
}

struct File {
    name: String,
    start: usize,
    len: usize,
}

impl Package {
    pub fn open(path: &Path) -> Result<Self, Error> {
        let data = std::fs::read(path).map_err(|err| Error::Io(path.display().to_string(), err))?;
        Package::parse(data)
    }

    pub fn parse(data: Vec<u8>) -> Result<Self, Error> {
        let mut reader = Reader::new(&data);
        let version = reader.string()?;
        if !version.starts_with("PKGV") {
            return Err(Error::Format(format!(
                "not a package: it starts with {version:?}"
            )));
        }
        let count = reader.u32()? as usize;
        let mut table = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            let name = reader.string()?;
            let offset = reader.u32()? as usize;
            let len = reader.u32()? as usize;
            table.push((name, offset, len));
        }
        // Offsets count from the end of the table.
        let base = reader.position();
        let files = table
            .into_iter()
            .map(|(name, offset, len)| {
                let start = base + offset;
                match start.checked_add(len) {
                    Some(end) if end <= data.len() => Ok(File { name, start, len }),
                    _ => Err(Error::Format(format!(
                        "{name} runs past the end of the package"
                    ))),
                }
            })
            .collect::<Result<_, _>>()?;
        Ok(Package {
            version,
            data,
            files,
        })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|file| file.name.as_str())
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.files
            .iter()
            .find(|file| file.name == name)
            .map(|file| &self.data[file.start..file.start + file.len])
    }
}

/// A package as the tests build one, the way the format describes it.
#[cfg(test)]
pub(crate) fn build(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let string = |out: &mut Vec<u8>, text: &str| {
        out.extend((text.len() as u32).to_le_bytes());
        out.extend(text.as_bytes());
    };
    string(&mut out, "PKGV0021");
    out.extend((files.len() as u32).to_le_bytes());
    let mut offset = 0u32;
    for (name, bytes) in files {
        string(&mut out, name);
        out.extend(offset.to_le_bytes());
        out.extend((bytes.len() as u32).to_le_bytes());
        offset += bytes.len() as u32;
    }
    for (_, bytes) in files {
        out.extend(*bytes);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_come_out_as_they_went_in() {
        let package = Package::parse(build(&[
            ("scene.json", b"{\"camera\":{}}"),
            ("materials/sky.tex", &[1, 2, 3]),
        ]))
        .unwrap();
        assert_eq!(package.version, "PKGV0021");
        assert_eq!(
            package.names().collect::<Vec<_>>(),
            ["scene.json", "materials/sky.tex"]
        );
        assert_eq!(package.get("materials/sky.tex"), Some(&[1u8, 2, 3][..]));
        assert_eq!(package.get("missing"), None);
    }

    #[test]
    fn broken_packages_are_errors_not_panics() {
        assert!(Package::parse(b"\x04\0\0\0ABCD".to_vec()).is_err());
        let mut short = build(&[("a", b"0123456789")]);
        short.truncate(short.len() - 3);
        assert!(Package::parse(short).is_err());
        let mut lying = build(&[("a", b"x")]);
        lying[0] = 0xff;
        assert!(Package::parse(lying).is_err());
        assert!(Package::parse(Vec::new()).is_err());
    }
}
