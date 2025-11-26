use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct FileWriter {
    file_mode: u32,
    dir_mode: u32,
}

impl Default for FileWriter {
    fn default() -> Self {
        Self {
            file_mode: 0o600,
            dir_mode: 0o700,
        }
    }
}

impl FileWriter {
    pub fn new(file_mode: u32, dir_mode: u32) -> Self {
        Self {
            file_mode,
            dir_mode,
        }
    }

    /// Creates a temp file with specific permissions, writes data,
    /// then atomically swaps it into place.
    pub fn atomic_write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let parent = self.prepare(path)?;

        let mut tmp = tempfile::Builder::new()
            .prefix(".tmp.")
            .permissions(fs::Permissions::from_mode(self.file_mode))
            .tempfile_in(parent)?;

        tmp.write_all(bytes)?;
        tmp.as_file().sync_all()?;

        // Atomic Swap
        // If it fails, the temp file is automatically cleaned up by the destructor.
        tmp.persist(path).map_err(|e| e.error)?;

        self.sync_dir(parent)?;

        Ok(())
    }

    /// Streams data from source to destination using a temp file.
    pub fn atomic_copy(&self, from: &Path, to: &Path) -> io::Result<()> {
        let parent = self.prepare(to)?;
        let mut source = File::open(from)?;

        let mut tmp = tempfile::Builder::new()
            .prefix(".tmp.")
            .permissions(fs::Permissions::from_mode(self.file_mode))
            .tempfile_in(parent)?;

        io::copy(&mut source, &mut tmp)?;
        tmp.as_file().sync_all()?;

        tmp.persist(to).map_err(|e| e.error)?;
        self.sync_dir(parent)?;

        Ok(())
    }

    /// Renames a file within the filesystem.
    /// Note: This cannot change file permissions easily without a race condition,
    /// so we assume the source file already has the desired permissions
    /// or we rely on the directory permissions to restrict access.
    pub fn atomic_move(&self, from: &Path, to: &Path) -> io::Result<()> {
        let parent = self.prepare(to)?;

        fs::rename(from, to)?;

        self.sync_dir(parent)?;
        Ok(())
    }

    /// Ensures parent directory exists and applies configured directory permissions.
    fn prepare<'a>(&self, path: &'a Path) -> io::Result<&'a Path> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("path has no parent"))?;

        if !parent.exists() {
            fs::create_dir_all(parent)?;

            let perm = fs::Permissions::from_mode(self.dir_mode);
            fs::set_permissions(parent, perm)?;
        }
        Ok(parent)
    }

    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        let file = File::open(dir)?;
        file.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permissions_are_applied() {
        let tmp = tempfile::tempdir().unwrap();
        let output = tmp.path().join("secure_file");

        let writer = FileWriter::new(0o600, 0o700);

        writer.atomic_write(&output, b"data").unwrap();

        let meta = fs::metadata(&output).unwrap();
        let mode = meta.permissions().mode();

        // Mask 0o777 to ignore file type bits
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn test_atomic_copy_streaming() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");

        fs::write(&src, b"content").unwrap();

        let writer = FileWriter::default();
        writer.atomic_copy(&src, &dst).unwrap();

        let content = fs::read(dst).unwrap();
        assert_eq!(content, b"content");
    }
}
