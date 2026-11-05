mod root;
pub(crate) use root::Root;

use std::fs::OpenOptions;
use std::io::Read;
use std::path::Path;

use crate::Error;

pub(crate) const JSON_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const RESOURCE_BYTES: usize = 256 * 1024 * 1024;
pub(crate) const PACKAGE_BYTES: usize = 512 * 1024 * 1024;

pub(crate) fn read(path: &Path, limit: usize) -> Result<Vec<u8>, Error> {
    let fail = |err| Error::Io(path.display().to_string(), err);
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(fail)?;
    read_file(file, path, limit)
}

fn read_file(file: std::fs::File, path: &Path, limit: usize) -> Result<Vec<u8>, Error> {
    let fail = |err| Error::Io(path.display().to_string(), err);
    let metadata = file.metadata().map_err(fail)?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(Error::Format(format!(
            "{} must be a regular file no larger than {} MiB",
            path.display(),
            limit / (1024 * 1024)
        )));
    }
    bounded(file, metadata.len() as usize, limit).map_err(fail)
}

fn bounded(mut reader: impl Read, hint: usize, limit: usize) -> std::io::Result<Vec<u8>> {
    let allocation = |error: std::collections::TryReserveError| {
        std::io::Error::other(format!("cannot allocate bounded resource: {error}"))
    };
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(hint.min(limit).min(64 * 1024))
        .map_err(allocation)?;
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let remaining = limit - bytes.len();
        let wanted = remaining.min(chunk.len()).max(1);
        let read = match reader.read(&mut chunk[..wanted]) {
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if read == 0 {
            return Ok(bytes);
        }
        if read > remaining {
            return Err(std::io::Error::other("resource grew past its size limit"));
        }
        let length = bytes.len() + read;
        if length > bytes.capacity() {
            let capacity = bytes.capacity().saturating_mul(2).max(length).min(limit);
            bytes
                .try_reserve_exact(capacity - bytes.len())
                .map_err(allocation)?;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_reads_limit_capacity_and_detect_growth_without_a_size_hint() {
        for (length, limit) in [(0, 0), (10, 11), (100000, 100003), (131073, 131073)] {
            let bytes = bounded(std::io::Cursor::new(vec![7; length]), 0, limit).unwrap();
            assert_eq!(bytes.len(), length);
            assert!(bytes.capacity() <= limit);
        }
        assert!(bounded(std::io::Cursor::new(vec![7; 100004]), 0, 100003).is_err());
        assert!(bounded(std::io::Cursor::new([7]), 0, 0).is_err());
    }

    #[test]
    fn file_limits_and_special_files_are_refused() {
        let path = std::env::temp_dir().join(format!("scene-limit-{}", std::process::id()));
        std::fs::write(&path, [1; 32]).unwrap();
        assert!(read(&path, 31).is_err());
        assert_eq!(read(&path, 32).unwrap().len(), 32);
        assert!(read(&std::env::temp_dir(), 100).is_err());
        #[cfg(unix)]
        {
            assert!(read(Path::new("/dev/zero"), 100).is_err());
            let link = path.with_extension("link");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(read(&link, 100).is_err());
            std::fs::remove_file(link).unwrap();
        }
        std::fs::remove_file(path).unwrap();
    }
}
