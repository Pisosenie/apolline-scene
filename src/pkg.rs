//! `scene.pkg`: a version string, a table of files with their offsets and lengths, then the
//! files one after another. Every number is a little-endian u32 and every string is
//! prefixed with its length.

use std::collections::HashMap;
use std::path::Path;

use crate::{Error, Reader};

pub struct Package {
    pub version: String,
    data: Vec<u8>,
    files: Vec<File>,
    index: HashMap<String, usize>,
}

struct File {
    name: String,
    start: usize,
    len: usize,
}

impl Package {
    pub fn open(path: &Path) -> Result<Self, Error> {
        let data = crate::files::read(path, crate::files::PACKAGE_BYTES)?;
        Package::parse(data)
    }

    pub fn parse(data: Vec<u8>) -> Result<Self, Error> {
        if data.len() > crate::files::PACKAGE_BYTES {
            return Err(Error::Format("package exceeds 512 MiB".into()));
        }
        let mut reader = Reader::new(&data);
        let version = reader.string(32)?;
        if !version.starts_with("PKGV") {
            return Err(Error::Format(format!(
                "not a package: it starts with {version:?}"
            )));
        }
        let count = reader.u32()? as usize;
        if count > 65536 {
            return Err(Error::Format("a package has more than 65536 files".into()));
        }
        let mut table = Vec::with_capacity(count.min(4096));
        let mut name_bytes = 0;
        for _ in 0..count {
            let name = reader.string(4096)?;
            name_bytes += name.len();
            if name_bytes > 8 * 1024 * 1024 {
                return Err(Error::Format(
                    "package filenames exceed 8 MiB in total".into(),
                ));
            }
            let offset = reader.u32()? as usize;
            let len = reader.u32()? as usize;
            table.push((name, offset, len));
        }
        // Offsets count from the end of the table.
        let base = reader.position();
        let files = table
            .into_iter()
            .map(|(name, offset, len)| {
                let start = base
                    .checked_add(offset)
                    .ok_or_else(|| Error::Format(format!("{name} has an invalid offset")))?;
                match start.checked_add(len) {
                    Some(end) if end <= data.len() => Ok(File { name, start, len }),
                    _ => Err(Error::Format(format!(
                        "{name} runs past the end of the package"
                    ))),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut index = HashMap::with_capacity(files.len());
        for (at, file) in files.iter().enumerate() {
            if index.insert(file.name.clone(), at).is_some() {
                return Err(Error::Format(format!(
                    "{} appears twice in the package",
                    file.name
                )));
            }
        }
        Ok(Package {
            version,
            data,
            files,
            index,
        })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|file| file.name.as_str())
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.index
            .get(name)
            .map(|&at| &self.files[at])
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
    fn package_metadata_strings_have_independent_limits() {
        let oversized_version = 33u32.to_le_bytes().to_vec();
        assert!(
            Package::parse(oversized_version)
                .err()
                .unwrap()
                .to_string()
                .contains("32 bytes")
        );
        let bytes = build(&[(&"x".repeat(4097), b"payload")]);
        assert!(
            Package::parse(bytes)
                .err()
                .unwrap()
                .to_string()
                .contains("4096 bytes")
        );
        let mut invalid = build(&[("a", b"payload")]);
        invalid[20] = 255;
        assert!(Package::parse(invalid).is_err());
        let names: Vec<_> = (0..2049)
            .map(|index| format!("{index:04}/{}", "x".repeat(4091)))
            .collect();
        let files: Vec<_> = names.iter().map(|name| (name.as_str(), &b""[..])).collect();
        assert!(
            Package::parse(build(&files))
                .err()
                .unwrap()
                .to_string()
                .contains("8 MiB")
        );
    }

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

    #[test]
    fn duplicate_names_and_truncated_tables_are_refused() {
        assert!(Package::parse(build(&[("a", b"1"), ("a", b"2")])).is_err());
        let data = build(&[("a", b"123"), ("b", b"456")]);
        for end in 0..data.len() {
            assert!(
                Package::parse(data[..end].to_vec()).is_err(),
                "prefix {end}"
            );
        }
        let mut data = build(&[]);
        let at = data.len() - 4;
        data[at..].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Package::parse(data).is_err());
    }
}
