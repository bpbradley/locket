//! Secret file registry
//!
//! This module defines the `SecretFileRegistry`, which maintains
//! a mapping of secret source files to their intended output destinations
//! based on configured path mappings and `SecretFile` definitions.
use crate::path::{PathExt, PathMapping};
use crate::secrets::{MemSize, SecretError, SecretSource, file::SecretFile};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
enum RegistryKind {
    /// File belongs to a directory mapping (can be rebased)
    Mapped { mapping_idx: usize },
    /// File was explicitly pinned via configuration (cannot be rebased)
    Pinned,
}

#[derive(Debug, Clone)]
struct RegistryEntry {
    file: SecretFile,
    kind: RegistryKind,
}

/// Registry of secret files, tracking their source paths and intended destinations.
///
/// It supports operations to upsert files based on mappings,
/// remove files or directories, resolve output paths,
/// and optimistically rebase directories on move events.
///
/// The registry ensures that pinned files are respected
/// and that mapping precedence is correctly handled to avoid collisions.
#[derive(Debug, Default)]
pub struct SecretFileRegistry {
    mappings: Vec<PathMapping>,
    pinned: HashMap<PathBuf, SecretFile>,
    files: BTreeMap<PathBuf, RegistryEntry>,
    max_file_size: MemSize,
}

impl SecretFileRegistry {
    pub fn new(
        mappings: Vec<PathMapping>,
        secrets: Vec<SecretFile>,
        max_file_size: MemSize,
    ) -> Self {
        let mut pinned = HashMap::new();

        for s in secrets {
            if let SecretSource::File(p) = s.source() {
                pinned.insert(p.clone(), s);
            }
        }
        let mut registry = Self {
            mappings,
            pinned,
            files: BTreeMap::new(),
            max_file_size,
        };

        registry.scan();

        registry
    }

    fn scan(&mut self) {
        let roots: Vec<PathBuf> = self
            .mappings
            .iter()
            .map(|m| m.src().to_path_buf())
            .collect();

        for src in roots {
            for entry in WalkDir::new(&src)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                if let Err(e) = self.upsert(entry.path()) {
                    warn!("Failed to scan mapped file {:?}: {}", entry.path(), e);
                }
            }
        }

        let pinned: Vec<PathBuf> = self.pinned.keys().cloned().collect();
        for path in pinned {
            if path.exists()
                && let Err(e) = self.upsert(&path)
            {
                warn!("Failed to scan pinned file {:?}: {}", path, e);
            }
        }
    }

    pub fn resolve(&self, src: &Path) -> Option<PathBuf> {
        let mapping = self
            .mappings
            .iter()
            .filter(|m| src.starts_with(m.src()))
            .max_by_key(|m| m.src().as_os_str().len())?;
        let rel = src.strip_prefix(mapping.src()).ok()?;
        Some(mapping.dst().join(rel))
    }

    pub fn upsert(&mut self, src: &Path) -> Result<Option<SecretFile>, SecretError> {
        // Check Pinned Config first
        // If the file matches a pinned configuration, enforce that config.
        if let Some(pinned) = self.pinned.get(src) {
            let entry = RegistryEntry {
                file: pinned.clone(),
                kind: RegistryKind::Pinned,
            };
            self.files.insert(src.to_path_buf(), entry);
            debug!("Tracked pinned file: {:?}", src);
            return Ok(Some(pinned.clone()));
        }

        // Check existing
        if let Some(entry) = self.files.get(src) {
            return Ok(Some(entry.file.clone()));
        }

        // Find the best mapping. i.e. the longest matching prefix.
        let map = self
            .mappings
            .iter()
            .enumerate()
            .filter(|(_, m)| src.starts_with(m.src()))
            .max_by_key(|(_, m)| m.src().as_os_str().len());

        if let Some((idx, mapping)) = map {
            let rel = src
                .strip_prefix(mapping.src())
                .map_err(|_| SecretError::Parse("path strip failed".into()))?;
            let dest = mapping.dst().join(rel);

            match SecretFile::from_file(src, dest, self.max_file_size) {
                Ok(file) => {
                    let entry = RegistryEntry {
                        file: file.clone(),
                        kind: RegistryKind::Mapped { mapping_idx: idx },
                    };
                    self.files.insert(src.to_path_buf(), entry);
                    debug!("Tracked mapped file: {:?}", src);
                    return Ok(Some(file));
                }
                Err(SecretError::SourceMissing(_)) => {
                    debug!("File Missing: {:?}. Ignoring.", src);
                    return Ok(None);
                }
                Err(e) => return Err(e),
            }
        }

        Ok(None)
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
            if let Some(entry) = self.files.remove(&key) {
                debug!("Removed secret file: {:?}", key);
                results.push(entry.file);
            }
        }
        results
    }

    /// Optimistically attempts to reflect a directory move by renaming the output directory.
    /// Returns Some((old_output, new_output)) if the move is safe and valid.
    /// Returns None if the move involves pinned files, crosses mappings, or implies state drift.
    pub fn try_rebase(&mut self, from: &Path, to: &Path) -> Option<(PathBuf, PathBuf)> {
        // Identify all affected files in the registry
        let keys: Vec<PathBuf> = self
            .files
            .range(from.to_path_buf()..)
            .take_while(|(k, _)| k.starts_with(from))
            .map(|(k, _)| k.clone())
            .collect();

        if keys.is_empty() {
            return None;
        }

        // Establish an anchor
        // All moved files must belong to the same mapping for a directory rename to work.
        let first_entry = self.files.get(&keys[0])?;
        let reference_idx = match first_entry.kind {
            RegistryKind::Mapped { mapping_idx } => mapping_idx,
            RegistryKind::Pinned => return None, // Pinned files cannot be rebased via directory moves
        };

        let mapping = &self.mappings[reference_idx];

        // Calculate roots to pivot
        // Determine the relative movement within the mapping to project the output paths.
        let rel_from = from.strip_prefix(mapping.src()).ok()?;
        let old_root_dst = mapping.dst().join(rel_from).canon().ok()?;

        let rel_to = to.strip_prefix(mapping.src()).ok()?;
        let new_root_dst = mapping.dst().join(rel_to).absolute();

        // Verification pass
        // Ensure every file is eligible and consistent before mutating state.
        let mut updates = Vec::with_capacity(keys.len());

        for k in &keys {
            let entry = self.files.get(k)?;

            // Mixed mappings prevent atomic rebase
            match entry.kind {
                RegistryKind::Mapped { mapping_idx } if mapping_idx == reference_idx => {}
                _ => return None,
            }

            let rel = k.strip_prefix(from).ok()?;

            // Check for drift
            // i.e. the file's current destination doesn't match calculation
            if entry.file.dest() != old_root_dst.join(rel).clean() {
                return None;
            }

            // Calculate new state
            let new_k = to.join(rel).clean();
            let new_d = new_root_dst.join(rel).clean();

            updates.push((k.clone(), new_k, new_d));
        }

        // Commit updates
        // Update the registry state to reflect the move.
        for (old_k, new_k, new_d) in updates {
            if let Some(mut entry) = self.files.remove(&old_k) {
                // Re-create SecretFile to ensure internal consistency (validating new paths)
                match SecretFile::from_file(&new_k, new_d, self.max_file_size) {
                    Ok(new_file) => {
                        entry.file = new_file;
                        self.files.insert(new_k, entry);
                    }
                    Err(e) => {
                        warn!("Failed to rebase file entry {:?}: {}", new_k, e);
                        // continue even on error to attempt to reach a consistent state,
                        // rather than aborting halfway through a commit.
                    }
                }
            }
        }

        Some((old_root_dst, new_root_dst))
    }

    pub fn iter(&self) -> impl Iterator<Item = &SecretFile> {
        self.files.values().map(|e| &e.file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn test_mapping_priority() {
        // Setup FS
        let tmp = tempdir().unwrap();
        let root = tmp.path();

        let src_root = root.join("templates");
        let src_secure = src_root.join("secure");
        let src_nested = src_secure.join("nested");

        fs::create_dir_all(&src_nested).unwrap();

        // Create files on disk so canonicalization succeeds
        let f_common = src_root.join("common.yaml");
        let f_db = src_secure.join("db.yaml");
        let f_key = src_nested.join("key");

        fs::write(&f_common, "data").unwrap();
        fs::write(&f_db, "data").unwrap();
        fs::write(&f_key, "data").unwrap();

        // Setup Logic
        let mut fs = SecretFileRegistry {
            mappings: vec![
                PathMapping::new(&src_root, "/secrets/general"),
                PathMapping::new(&src_secure, "/secrets/specific"),
            ],
            ..Default::default()
        };

        // General file
        let general = fs
            .upsert(&f_common)
            .expect("io error")
            .expect("should be tracked");
        assert_eq!(
            general.dest(),
            PathBuf::from("/secrets/general/common.yaml")
        );

        // Specific file
        let specific = fs
            .upsert(&f_db)
            .expect("io error")
            .expect("should be tracked");
        assert_eq!(specific.dest(), PathBuf::from("/secrets/specific/db.yaml"));

        // Specific nested
        let specific_nested = fs
            .upsert(&f_key)
            .expect("io error")
            .expect("should be tracked");
        assert_eq!(
            specific_nested.dest(),
            PathBuf::from("/secrets/specific/nested/key")
        );
    }

    #[test]
    fn test_prefix_collision() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let src_root = root.join("app");

        let dir_a = src_root.join("DIRA");
        let dir_aa = src_root.join("DIRAA");

        fs::create_dir_all(&dir_a).unwrap();
        fs::create_dir_all(&dir_aa).unwrap();

        let f_a = dir_a.join("file.txt");
        let f_aa = dir_aa.join("file.txt");

        fs::write(&f_a, "").unwrap();
        fs::write(&f_aa, "").unwrap();

        let mut fs = SecretFileRegistry::default();
        fs.mappings.push(PathMapping::new(&src_root, "/out"));

        fs.upsert(&f_a).unwrap();
        fs.upsert(&f_aa).unwrap();

        assert_eq!(fs.files.len(), 2);

        // Remove DIRA. Should not remove DIRAA.
        let removed = fs.remove(&dir_a);

        assert_eq!(removed.len(), 1);

        // Check that the removed file is indeed f_a
        // We check the source because SecretFile stores canonical paths
        if let crate::secrets::SecretSource::File(p) = removed[0].source() {
            assert_eq!(p, &f_a.canonicalize().unwrap());
        }

        // Verify DIRAA is still there
        assert!(fs.files.contains_key(&f_aa));
    }

    #[test]
    fn test_recursive_removal() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let src = root.join("root");

        let sub = src.join("sub");
        let nested = sub.join("nested");
        fs::create_dir_all(&nested).unwrap();

        let f_a = src.join("a.txt");
        let f_b = sub.join("b.txt");
        let f_c = nested.join("c.txt");
        let f_z = src.join("z.txt");

        for p in [&f_a, &f_b, &f_c, &f_z] {
            fs::write(p, "").unwrap();
        }

        let mut fs = SecretFileRegistry::default();
        fs.mappings.push(PathMapping::new(&src, "/out"));

        fs.upsert(&f_a).unwrap();
        fs.upsert(&f_b).unwrap();
        fs.upsert(&f_c).unwrap();
        fs.upsert(&f_z).unwrap();

        assert_eq!(fs.files.len(), 4);

        let removed = fs.remove(&sub);

        assert_eq!(removed.len(), 2);

        // Verify state
        assert!(fs.files.contains_key(&f_a));
        assert!(fs.files.contains_key(&f_z));
        assert!(!fs.files.contains_key(&f_b));
        assert!(!fs.files.contains_key(&f_c));
    }

    #[test]
    fn test_ignore_unmapped() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();

        let src = root.join("templates");
        fs::create_dir_all(&src).unwrap();

        let mut fs = SecretFileRegistry::default();
        fs.mappings.push(PathMapping::new(&src, "/secrets"));

        // File totally outside
        let outside = root.join("passwd");
        fs::write(&outside, "").unwrap();

        let res = fs.upsert(&outside).unwrap();
        assert!(res.is_none());

        // Unmapped prefix
        let backup = root.join("templates_backup");
        fs::create_dir_all(&backup).unwrap();
        let backup_file = backup.join("file");
        fs::write(&backup_file, "").unwrap();

        let res = fs.upsert(&backup_file).unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn test_resolve_logic() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let src = root.join("t");
        fs::create_dir_all(&src).unwrap();

        let mut fs = SecretFileRegistry::default();
        fs.mappings.push(PathMapping::new(&src, "/s"));

        let input = src.join("subdir/file");
        // We don't need to create the file to test resolve() because resolve()
        // purely calculates the destination path string.
        let dst = fs.resolve(&input).unwrap();

        assert_eq!(dst, PathBuf::from("/s/subdir/file"));
    }

    #[test]
    fn test_rebase_dir_intra_mapping() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let data = root.join("data");
        let output = root.join("output");

        let old_sub = data.join("old_sub");
        let new_sub = data.join("new_sub");

        fs::create_dir_all(&old_sub).unwrap();
        fs::create_dir_all(&new_sub).unwrap();
        fs::create_dir_all(output.join("old_sub")).unwrap();

        let mut fs = SecretFileRegistry::default();
        fs.mappings.push(PathMapping::new(&data, &output));

        let p_old = old_sub.join("file.txt");
        fs::write(&p_old, "content").unwrap();
        fs.upsert(&p_old).unwrap();

        // try_rebase enforces existence on the NEW path.
        // So the file must exist at the new location for rebase to track it.
        let p_new = new_sub.join("file.txt");
        fs::write(&p_new, "content").unwrap();

        // Action: Move "old_sub" -> "new_sub"
        let res = fs.try_rebase(&old_sub, &new_sub);

        assert!(res.is_some());
        let (old_dst, new_dst) = res.unwrap();

        assert_eq!(old_dst, output.join("old_sub"));
        assert_eq!(new_dst, output.join("new_sub"));

        // Verify internal state
        assert!(!fs.files.contains_key(&p_old));

        let new_entry = fs.files.get(&p_new).expect("new file should be tracked");
        assert_eq!(new_entry.file.dest(), output.join("new_sub/file.txt"));
    }

    #[test]
    fn test_rebase_dir_inter_mapping() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();

        let src_a = root.join("src_a");
        let src_b = root.join("src_b");
        let out_a = root.join("out_a");
        let out_b = root.join("out_b");

        let folder_a = src_a.join("folder");
        let folder_b = src_b.join("moved_folder");

        fs::create_dir_all(&folder_a).unwrap();
        fs::create_dir_all(&folder_b).unwrap();

        let mut fs = SecretFileRegistry::default();
        fs.mappings.push(PathMapping::new(&src_a, &out_a));
        fs.mappings.push(PathMapping::new(&src_b, &out_b));

        let f_old = folder_a.join("config.yaml");
        fs::write(&f_old, "").unwrap();
        fs.upsert(&f_old).unwrap();

        // Simulate move
        let f_new = folder_b.join("config.yaml");
        fs::write(&f_new, "").unwrap();

        let res = fs.try_rebase(&folder_a, &folder_b);
        assert!(res.is_none());
        assert!(fs.files.contains_key(&f_old));
        assert!(!fs.files.contains_key(&f_new));
    }

    #[test]
    fn test_rebase_dir_nested_mapping() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();

        let tpl = root.join("templates");
        let tpl_secure = tpl.join("secure");
        let tpl_new = root.join("templates_new");

        fs::create_dir_all(&tpl_secure).unwrap();
        fs::create_dir_all(&tpl_new).unwrap();

        let mut fs = SecretFileRegistry::default();
        fs.mappings.push(PathMapping::new(&tpl, "/secrets"));
        fs.mappings.push(PathMapping::new(&tpl_secure, "/vault"));

        let f1 = tpl.join("common.yaml");
        let f2 = tpl_secure.join("db_pass");

        fs::write(&f1, "").unwrap();
        fs::write(&f2, "").unwrap();

        fs.upsert(&f1).unwrap();
        fs.upsert(&f2).unwrap();

        // Move "/templates" -> "/templates_new"
        // Should fail because f2 maps to /vault, which cannot be linearly rebased
        // to a new location relative to /secrets just by changing the parent dir.
        let res = fs.try_rebase(&tpl, &tpl_new);

        assert!(res.is_none());

        // State remains untouched
        assert!(fs.files.contains_key(&f1));
        assert!(fs.files.contains_key(&f2));
    }
}
