use crate::secrets::{
    path::{PathMapping, PathExt},
    types::{MemSize, SecretError, SecretFile},
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};
use walkdir::WalkDir;

#[derive(Debug, Default)]
pub struct SecretFs {
    mappings: Vec<PathMapping>,
    files: BTreeMap<PathBuf, SecretFile>,
    max_file_size: MemSize,
}

impl SecretFs {
    pub fn new(mappings: Vec<PathMapping>, max_file_size: MemSize) -> Self {
        let mut fs = Self {
            mappings,
            files: BTreeMap::new(),
            max_file_size,
        };

        fs.scan();

        fs
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
                    warn!("Failed to scan file {:?}: {}", entry.path(), e);
                }
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
        if let Some(file) = self.files.get(src) {
            return Ok(Some(file.clone()));
        }
        if let Some(dest) = self.resolve(src) {
            match SecretFile::from_file(src, dest, self.max_file_size) {
                Ok(file) => {
                    self.files.insert(src.to_path_buf(), file.clone());
                    debug!("Added secret file: {:?}", src);
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

        // Check homogeneity
        let mut updates = Vec::with_capacity(keys.len());

        for k in &keys {
            let file = self.files.get(k)?;
            let rel = k.strip_prefix(from).ok()?;
            if file.dest() != from_root.join(rel) {
                return None;
            }

            // Calculate new state
            let new_k = to.join(rel).clean();
            let new_d = to_root.join(rel).clean();
            updates.push((k.clone(), new_k, new_d));
        }

        // Commit updates
        for (old_k, new_k, new_d) in updates {
            if self.files.remove(&old_k).is_some() {
                match SecretFile::from_file(&new_k, new_d, self.max_file_size) {
                    Ok(file) => {
                        self.files.insert(new_k, file);
                    }
                    Err(e) => {
                        warn!("Failed to rebase file to {:?}: {}", new_k, e);
                    }
                }
            }
        }
        Some((from_root, to_root))
    }

    pub fn iter_files(&self) -> impl Iterator<Item = &SecretFile> {
        self.files.values()
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
        let mut fs = SecretFs {
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

        let mut fs = SecretFs::default();
        fs.mappings.push(PathMapping::new(&src_root, "/out"));

        fs.upsert(&f_a).unwrap();
        fs.upsert(&f_aa).unwrap();

        assert_eq!(fs.files.len(), 2);

        // Remove DIRA. Should not remove DIRAA.
        let removed = fs.remove(&dir_a);

        assert_eq!(removed.len(), 1);

        // Check that the removed file is indeed f_a
        // We check the source because SecretFile stores canonical paths
        if let crate::secrets::types::SecretSource::File(p) = removed[0].source() {
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

        let mut fs = SecretFs::default();
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

        let mut fs = SecretFs::default();
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

        let mut fs = SecretFs::default();
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
        fs::create_dir_all(&new_sub).unwrap(); // New dir must exist

        let mut fs = SecretFs::default();
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
        assert_eq!(new_entry.dest(), output.join("new_sub/file.txt"));
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

        let mut fs = SecretFs::default();
        fs.mappings.push(PathMapping::new(&src_a, &out_a));
        fs.mappings.push(PathMapping::new(&src_b, &out_b));

        let f_old = folder_a.join("config.yaml");
        fs::write(&f_old, "").unwrap();
        fs.upsert(&f_old).unwrap();

        // Simulate move
        let f_new = folder_b.join("config.yaml");
        fs::write(&f_new, "").unwrap();

        let res = fs.try_rebase(&folder_a, &folder_b);

        assert!(res.is_some());
        let (old_dst, new_dst) = res.unwrap();

        assert_eq!(old_dst, out_a.join("folder"));
        assert_eq!(new_dst, out_b.join("moved_folder"));

        assert!(fs.files.contains_key(&f_new));
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

        let mut fs = SecretFs::default();
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
