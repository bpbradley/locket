//! Atomic file write utilities
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let parent = path.parent().ok_or_else(|| io::Error::other("no parent"))?;
    let tmp = tempfile::Builder::new()
        .prefix(".tmp.")
        .tempfile_in(parent)?;
    let mut f = tmp.reopen()?;
    f.write_all(bytes)?;
    f.sync_all()?;
    let tmp_path = tmp.into_temp_path();
    // Rename is atomic on the same filesystem
    std::fs::rename(&tmp_path, path)?;
    // fsync the parent dir for durability where supported
    if let Ok(dir) = File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

pub fn atomic_move(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
        fs::rename(from, to)?;
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
        Ok(())
    } else {
        Err(io::Error::other("destination has no parent"))
    }
}

pub fn atomic_copy(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
        let bytes = fs::read(from)?;
        atomic_write(to, &bytes)
    } else {
        Err(io::Error::other("destination has no parent"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_files() {
        let tmp = assert_fs::TempDir::new().unwrap();
        let path = tmp.join("out.txt");
        atomic_write(&path, b"hello").unwrap();
        let got = std::fs::read(&path).unwrap();
        assert_eq!(got, b"hello");
    }
}
