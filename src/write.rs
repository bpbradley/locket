//! Atomic file write utilities
use rand::{Rng, distributions::Alphanumeric};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = tmp_path(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    // Rename is atomic on the same filesystem
    std::fs::rename(&tmp, path)?;
    // fsync the parent dir for durability where supported
    if let Some(parent) = path.parent() {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

pub fn atomic_move(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    // Rename is atomic on the same filesystem
    std::fs::rename(from, to)?;
    // fsync the parent dir for durability where supported
    if let Some(parent) = to.parent() {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

fn tmp_path(path: &Path) -> PathBuf {
    let suffix: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(8)
        .map(char::from)
        .collect();
    let mut pb = path.as_os_str().to_owned();
    let s = format!(".tmp.{}", suffix);
    pb.push(&s);
    PathBuf::from(pb)
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
