use std::fs::File;
#[cfg(unix)]
use std::fs::OpenOptions;
use std::path::{Component, Path, PathBuf};

use crate::Error;

pub(crate) struct Root {
    path: PathBuf,
    #[cfg(unix)]
    directory: File,
}

impl Root {
    pub fn open(path: &Path) -> Result<Self, Error> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // The root itself may be a user-selected symlink; pin its directory once.
            let directory = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NONBLOCK)
                .open(path)
                .map_err(|error| Error::Io(path.display().to_string(), error))?;
            Ok(Self {
                path: path.to_owned(),
                directory,
            })
        }
        #[cfg(not(unix))]
        {
            let path = path
                .canonicalize()
                .map_err(|error| Error::Io(path.display().to_string(), error))?;
            if !path.is_dir() {
                return Err(Error::Format("resource root must be a directory".into()));
            }
            Ok(Self { path })
        }
    }

    pub fn read(&self, name: &str, limit: usize) -> Result<Option<Vec<u8>>, Error> {
        if name.len() > 4096 || name.is_empty() || name.contains(['\\', ':', '\0']) {
            return Err(Error::Format("invalid rooted resource path".into()));
        }
        let mut parts = Vec::new();
        for part in Path::new(name).components() {
            match part {
                Component::Normal(part) => parts.push(part),
                Component::CurDir => {}
                _ => {
                    return Err(Error::Format(
                        "resource path must remain relative to its root".into(),
                    ));
                }
            }
        }
        if parts.is_empty() {
            return Err(Error::Format("resource path has no filename".into()));
        }
        let label = self.path.join(name);
        #[cfg(unix)]
        let opened = self.open_relative(&parts);
        #[cfg(not(unix))]
        let opened = (|| {
            let path = label.canonicalize()?;
            if !path.starts_with(&self.path) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "resource escapes its root",
                ));
            }
            File::open(path)
        })();
        match opened {
            Ok(file) => super::read_file(file, &label, limit).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(Error::Io(label.display().to_string(), error)),
        }
    }

    #[cfg(unix)]
    fn open_relative(&self, parts: &[&std::ffi::OsStr]) -> std::io::Result<File> {
        use std::ffi::CString;
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::ffi::OsStrExt;

        let mut parent = None;
        for (index, part) in parts.iter().enumerate() {
            let name = CString::new(part.as_bytes()).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in resource path")
            })?;
            let directory = parent.as_ref().unwrap_or(&self.directory);
            let final_component = index + 1 == parts.len();
            let flags = libc::O_RDONLY
                | libc::O_CLOEXEC
                | libc::O_NONBLOCK
                | libc::O_NOFOLLOW
                | if final_component {
                    0
                } else {
                    libc::O_DIRECTORY
                };
            // Every ancestor is opened without following symlinks. Renames after an open
            // cannot redirect subsequent lookups through a different directory pathname.
            let descriptor = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
            if descriptor < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let file = unsafe { File::from_raw_fd(descriptor) };
            if final_component {
                return Ok(file);
            }
            parent = Some(file);
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "resource path has no filename",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_pin_directories_and_reject_symlink_ancestors() {
        let base = std::env::temp_dir().join(format!("scene-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let folder = base.join("project");
        let outside = base.join("outside");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(folder.join("value"), b"inside").unwrap();
        std::fs::write(outside.join("value"), b"outside").unwrap();
        let root = Root::open(&folder).unwrap();
        assert_eq!(root.read("./value", 6).unwrap().unwrap(), b"inside");
        assert!(root.read("missing", 10).unwrap().is_none());
        assert!(root.read("value", 5).is_err());
        for name in ["../outside/value", "/etc/passwd", "", ".", "a\\b", "a\0b"] {
            assert!(root.read(name, 10).is_err(), "{name}");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, folder.join("linked")).unwrap();
            std::os::unix::fs::symlink(folder.join("value"), folder.join("alias")).unwrap();
            assert!(root.read("linked/value", 10).is_err());
            assert!(root.read("alias", 10).is_err());
            std::fs::rename(&folder, base.join("moved")).unwrap();
            std::fs::create_dir(&folder).unwrap();
            std::fs::write(folder.join("value"), b"replaced").unwrap();
            assert_eq!(root.read("value", 10).unwrap().unwrap(), b"inside");
        }
        drop(root);
        std::fs::remove_dir_all(base).unwrap();
    }
}
