use clap::Args;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[derive(Clone, Args)]
pub struct FileWriter {
    /// File permission mode
    #[clap(long, env = "LOCKET_FILE_MODE", default_value = "600", value_parser = parse_permissions)]
    file_mode: u32,
    /// Directory permission mode
    #[clap(long, env = "LOCKET_DIR_MODE", default_value = "700", value_parser = parse_permissions)]
    dir_mode: u32,
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

    pub fn create_temp_for(&self, dst: &Path) -> io::Result<tempfile::NamedTempFile> {
        let parent = self.prepare(dst)?;
        let temp = tempfile::Builder::new()
            .prefix(".tmp.")
            .permissions(fs::Permissions::from_mode(self.file_mode))
            .tempfile_in(parent)?;
            
        Ok(temp)
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

impl Default for FileWriter {
    fn default() -> Self {
        Self {
            file_mode: 0o600,
            dir_mode: 0o700,
        }
    }
}

impl std::fmt::Debug for FileWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileWriter")
            .field("file_mode", &format_args!("0o{:o}", self.file_mode))
            .field("dir_mode", &format_args!("0o{:o}", self.dir_mode))
            .finish()
    }
}

fn parse_permissions(perms: &str) -> Result<u32, String> {
    let norm = perms.strip_prefix("0o").unwrap_or(perms);

    let mode = u32::from_str_radix(norm, 8)
        .map_err(|e| format!("Invalid octal permission format '{}': {}", perms, e))?;

    if mode > 0o7777 {
        return Err(format!("Permission mode '{:o}' is too large", mode));
    }

    Ok(mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Debug, Parser)]
    struct TestParser {
        #[arg(long, value_parser = parse_permissions)]
        mode: u32,
    }

    #[test]
    fn test_permission_parsing() {
        let opts = TestParser::try_parse_from(["test", "--mode", "600"]).unwrap();
        assert_eq!(opts.mode, 0o600); // 0o600 octal
        assert_ne!(opts.mode, 600); // NOT 600 decimal

        let opts = TestParser::try_parse_from(["test", "--mode", "0755"]).unwrap();
        assert_eq!(opts.mode, 0o755);

        let opts = TestParser::try_parse_from(["test", "--mode", "0o644"]).unwrap();
        assert_eq!(opts.mode, 0o644);
    }

    #[test]
    fn test_permission_parsing_errors() {
        assert!(TestParser::try_parse_from(["test", "--mode", "999"]).is_err());
        assert!(TestParser::try_parse_from(["test", "--mode", "abc"]).is_err());
        assert!(TestParser::try_parse_from(["test", "--mode", "0o70000"]).is_err());
    }

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
