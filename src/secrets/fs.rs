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
