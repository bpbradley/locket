use crate::secrets::{manager::PathMapping, types::SecretFile};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tracing::debug;
use walkdir::WalkDir;

#[derive(Debug, Default)]
pub struct SecretFs {
    mappings: Vec<PathMapping>,
    files: BTreeMap<PathBuf, SecretFile>,
}

impl SecretFs {
    pub fn new(mappings: Vec<PathMapping>) -> Self {
        let mut fs = Self {
            mappings,
            files: BTreeMap::new(),
        };

        fs.scan();

        fs
    }

    fn scan(&mut self) {
        let roots: Vec<PathBuf> = self.mappings.iter().map(|m| m.src.clone()).collect();

        for src in roots {
            for entry in WalkDir::new(&src)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                self.upsert(entry.path());
            }
        }
    }

    pub fn resolve(&self, src: &Path) -> Option<PathBuf> {
        let mapping = self
            .mappings
            .iter()
            .filter(|m| src.starts_with(&m.src))
            .max_by_key(|m| m.src.as_os_str().len())?;
        let rel = src.strip_prefix(&mapping.src).ok()?;
        Some(mapping.dst.join(rel))
    }

    /// Structural upsert: ensure a SecretFile exists for this src.
    ///
    /// Returns an immutable reference to the stored SecretFile if it’s in a managed dir.
    pub fn upsert(&mut self, src: &Path) -> Option<&SecretFile> {
        if self.files.contains_key(src) {
            return self.files.get(src);
        }
        if let Some(dst) = self.resolve(src) {
            let file = SecretFile {
                src: src.to_path_buf(),
                dst,
            };
            self.files.insert(src.to_path_buf(), file);
            debug!("Added secret file: {:?}", src);
            return self.files.get(src);
        }
        None
    }

    /// Remove struct entry for this src and return the SecretFile if there was one.
    pub fn remove(&mut self, src: &Path) -> Vec<SecretFile> {
        let removed_keys: Vec<PathBuf> = self
            .files
            .range(src.to_path_buf()..)
            .take_while(|(k, _)| k.starts_with(src))
            .map(|(k, _)| k.clone())
            .collect();
        let mut results = Vec::with_capacity(removed_keys.len());
        for key in removed_keys {
            if let Some(file) = self.files.remove(&key) {
                debug!("Removed secret file: {:?}", key);
                results.push(file);
            }
        }
        results
    }

    pub fn iter_files(&self) -> impl Iterator<Item = &SecretFile> {
        self.files.values()
    }
    pub fn len(&self) -> usize {
        self.files.len()
    }
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // Helper to make paths readable
    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn test_mapping_priority() {
        let mut fs = SecretFs::default();

        fs.mappings.push(PathMapping {
            src: p("/templates"),
            dst: p("/secrets/general"),
        });

        fs.mappings.push(PathMapping {
            src: p("/templates/secure"),
            dst: p("/secrets/specific"),
        });

        let general = fs.upsert(&p("/templates/common.yaml")).expect("should map");
        assert_eq!(general.dst, p("/secrets/general/common.yaml"));

        let specific = fs.upsert(&p("/templates/secure/db.yaml")).expect("should map");
        assert_eq!(specific.dst, p("/secrets/specific/db.yaml"));

        let specific_nested = fs.upsert(&p("/templates/secure/nested/key")).expect("should map");
        assert_eq!(specific_nested.dst, p("/secrets/specific/nested/key"));
    }

    #[test]
    fn test_prefix_collision() {
        let mut fs = SecretFs::default();
        fs.mappings.push(PathMapping {
            src: p("/app"),
            dst: p("/out"),
        });

        // Setup state manually
        let path_dira = p("/app/DIRA/file.txt");
        let path_diraa = p("/app/DIRAA/file.txt");

        fs.upsert(&path_dira);
        fs.upsert(&path_diraa);

        assert_eq!(fs.len(), 2);

        let removed = fs.remove(&p("/app/DIRA"));

        // ASSERT: Only DIRA is removed
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].src, path_dira);
        
        // Verify DIRAA is still there
        assert!(fs.files.contains_key(&path_diraa));
    }

    #[test]
    fn test_recursive_removal() {
        let mut fs = SecretFs::default();
        fs.mappings.push(PathMapping {
            src: p("/root"),
            dst: p("/out"),
        });

        fs.upsert(&p("/root/a.txt"));
        fs.upsert(&p("/root/sub/b.txt"));
        fs.upsert(&p("/root/sub/nested/c.txt"));
        fs.upsert(&p("/root/z.txt"));

        assert_eq!(fs.len(), 4);

        // ACTION: Remove directory "/root/sub"
        let removed = fs.remove(&p("/root/sub"));

        assert_eq!(removed.len(), 2);
        
        // Verify exact matches
        let src_paths: Vec<_> = removed.iter().map(|f| f.src.clone()).collect();
        assert!(src_paths.contains(&p("/root/sub/b.txt")));
        assert!(src_paths.contains(&p("/root/sub/nested/c.txt")));

        // Verify remaining
        assert_eq!(fs.len(), 2);
        assert!(fs.files.contains_key(&p("/root/a.txt")));
        assert!(fs.files.contains_key(&p("/root/z.txt")));
    }

    #[test]
    fn test_ignore_unmapped() {
        let mut fs = SecretFs::default();
        fs.mappings.push(PathMapping {
            src: p("/templates"),
            dst: p("/secrets"),
        });

        // Upsert file totally outside
        let res = fs.upsert(&p("/etc/passwd"));
        assert!(res.is_none());
        assert_eq!(fs.len(), 0);

        // Upsert file that matches prefix string but not path component
        let res = fs.upsert(&p("/templates_backup/file"));
        assert!(res.is_none());
        assert_eq!(fs.len(), 0);
    }
    
    #[test]
    fn test_resolve_logic() {
         let mut fs = SecretFs::default();
         fs.mappings.push(PathMapping { src: p("/t"), dst: p("/s") });
         
         // We can test logic without upserting into state
         let dst = fs.resolve(&p("/t/subdir/file")).unwrap();
         assert_eq!(dst, p("/s/subdir/file"));
    }
}
