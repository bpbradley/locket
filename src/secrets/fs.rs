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

    pub fn upsert(&mut self, src: &Path) -> Option<&SecretFile> {
        if self.files.contains_key(src) {
            return self.files.get(src);
        }
        if let Some(dst) = self.resolve(src) {
            let file = SecretFile::new(src, dst);
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

    pub fn try_rebase(&mut self, from: &Path, to: &Path) -> Option<(PathBuf, PathBuf)> {
        let from_root = self.resolve(from)?;
        let to_root = self.resolve(to)?;

        // Find rebase candidates
        let keys: Vec<PathBuf> = self
            .files
            .range(from.to_path_buf()..)
            .take_while(|(k, _)| k.starts_with(from))
            .map(|(k, _)| k.clone())
            .collect();

        if keys.is_empty() {
            return None;
        }

        // Check homogeneity.
        // Verify that EVERY file currently inside `from` would map to the expected new location.
        // This catches cases where a subdirectory might have a different mapping rule.
        let mut updates = Vec::with_capacity(keys.len());

        for k in &keys {
            let file = self.files.get(k)?;
            let rel = k.strip_prefix(from).ok()?;
            if file.dst != from_root.join(rel) {
                // We must fall back to individual file processing.
                return None;
            }

            // Calculate new state
            let new_k = to.join(rel);
            let new_d = to_root.join(rel);
            updates.push((k.clone(), new_k, new_d));
        }

        // Commit updates
        for (old_k, new_k, new_d) in updates {
            if let Some(mut file) = self.files.remove(&old_k) {
                file = SecretFile::new(&new_k, &new_d);
                self.files.insert(file.src.clone(), file);
            }
        }

        Some((from_root, to_root))
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

        let specific = fs
            .upsert(&p("/templates/secure/db.yaml"))
            .expect("should map");
        assert_eq!(specific.dst, p("/secrets/specific/db.yaml"));

        let specific_nested = fs
            .upsert(&p("/templates/secure/nested/key"))
            .expect("should map");
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
        fs.mappings.push(PathMapping {
            src: p("/t"),
            dst: p("/s"),
        });

        // We can test logic without upserting into state
        let dst = fs.resolve(&p("/t/subdir/file")).unwrap();
        assert_eq!(dst, p("/s/subdir/file"));
    }
    #[test]
    fn test_rebase_dir_intra_mapping() {
        let mut fs = SecretFs::default();
        // Setup: /data -> /output
        fs.mappings.push(PathMapping {
            src: p("/data"),
            dst: p("/output"),
        });

        // Initial State: /data/old_sub/file.txt
        let p_old = p("/data/old_sub/file.txt");
        fs.upsert(&p_old);

        // Action: Move "/data/old_sub" -> "/data/new_sub"
        let res = fs.try_rebase(&p("/data/old_sub"), &p("/data/new_sub"));

        // Assert: Rebase permitted
        assert!(res.is_some());
        let (old_dst, new_dst) = res.unwrap();

        // Assert: Calculated renaming of the OUTPUT directory
        assert_eq!(old_dst, p("/output/old_sub"));
        assert_eq!(new_dst, p("/output/new_sub"));

        // Assert: Internal state updated
        // Old key gone
        assert!(!fs.files.contains_key(&p_old));
        // New key present
        let p_new = p("/data/new_sub/file.txt");
        let new_entry = fs.files.get(&p_new).expect("new file should exist");
        assert_eq!(new_entry.dst, p("/output/new_sub/file.txt"));
    }

    #[test]
    fn test_rebase_dir_inter_mapping() {
        let mut fs = SecretFs::default();
        // Mapping 1: /src_a -> /out_a
        fs.mappings.push(PathMapping {
            src: p("/src_a"),
            dst: p("/out_a"),
        });
        // Mapping 2: /src_b -> /out_b
        fs.mappings.push(PathMapping {
            src: p("/src_b"),
            dst: p("/out_b"),
        });

        fs.upsert(&p("/src_a/folder/config.yaml"));

        // Action: Move "/src_a/folder" -> "/src_b/moved_folder"
        // This is a move between two totally different mappings.
        let res = fs.try_rebase(&p("/src_a/folder"), &p("/src_b/moved_folder"));

        assert!(res.is_some());
        let (old_dst, new_dst) = res.unwrap();

        // Verify output paths jump correctly
        assert_eq!(old_dst, p("/out_a/folder"));
        assert_eq!(new_dst, p("/out_b/moved_folder"));

        // Verify state
        assert!(fs.files.contains_key(&p("/src_b/moved_folder/config.yaml")));
    }

    #[test]
    fn test_rebase_dir_nested_mapping() {
        let mut fs = SecretFs::default();
        fs.mappings.push(PathMapping {
            src: p("/templates"),
            dst: p("/secrets"),
        });
        fs.mappings.push(PathMapping {
            src: p("/templates/secure"),
            dst: p("/vault_mount"),
        });

        // Add files
        fs.upsert(&p("/templates/common.yaml"));
        fs.upsert(&p("/templates/secure/db_pass"));

        // Action: Move "/templates" -> "/templates_new"
        // This SHOULD FAIL because we cannot linearly rename the output.
        // "/secrets" cannot be renamed to "/secrets_new" because "/secrets" does NOT contain "db_pass".
        // "db_pass" lives in "/vault_mount", which is somewhere else entirely.
        let res = fs.try_rebase(&p("/templates"), &p("/templates_new"));

        assert!(
            res.is_none(),
            "Should reject rebase because of heterogeneous children"
        );

        // Verify state is untouched
        assert!(fs.files.contains_key(&p("/templates/common.yaml")));
        assert!(fs.files.contains_key(&p("/templates/secure/db_pass")));
    }
}
