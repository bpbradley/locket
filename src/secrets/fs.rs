use crate::secrets::{manager::PathMapping, types::SecretFile};
use std::collections::{HashMap, hash_map::Values};
use std::iter::Once;
use std::path::{Path, PathBuf};
use tracing::debug;
use walkdir::WalkDir;

/// A directory store of file-backed secrets.
#[derive(Debug, Clone)]
pub struct SecretDir {
    pub mapping: PathMapping,
    // Map of relative paths to SecretFile entries
    pub files: HashMap<PathBuf, SecretFile>,
}

#[derive(Debug, Clone)]
pub enum SecretEntry {
    Dir(SecretDir),
    File(SecretFile),
}

/// Top-level FS structure of watched entries.
#[derive(Debug, Default)]
pub struct SecretFs {
    entries: HashMap<PathBuf, SecretEntry>,
}

impl SecretFs {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn entries(&self) -> &HashMap<PathBuf, SecretEntry> {
        &self.entries
    }

    pub fn entries_mut(&mut self) -> &mut HashMap<PathBuf, SecretEntry> {
        &mut self.entries
    }

    pub fn add_mapping(&mut self, mapping: &PathMapping) {
        let mut dir = SecretDir {
            mapping: mapping.clone(),
            files: HashMap::new(),
        };

        for entry in WalkDir::new(&dir.mapping.src)
            .into_iter()
            .filter_map(|r| r.ok())
            .filter(|e| e.file_type().is_file())
        {
            let src = entry.path();

            // Calculate relative path from the Mapping Source
            if let Ok(rel) = src.strip_prefix(&dir.mapping.src) {
                let rel = rel.to_path_buf();

                // Join relative path to the Mapping Destination
                let dst = dir.mapping.dst.join(&rel);

                debug!(src=?src, dst=?dst, "collected file secret");

                dir.files.insert(
                    rel,
                    SecretFile {
                        src: src.to_path_buf(),
                        dst,
                    },
                );
            }
        }

        self.entries
            .insert(mapping.src.to_path_buf(), SecretEntry::Dir(dir));
    }

    fn owning_dir(&mut self, src: &Path) -> Option<&mut SecretDir> {
        self.entries.values_mut().find_map(|entry| match entry {
            SecretEntry::Dir(dir) if src.starts_with(&dir.mapping.src) => Some(dir),
            _ => None,
        })
    }

    /// Structural upsert: ensure a SecretFile exists for this src.
    ///
    /// Returns a mutable reference to the stored SecretFile if it’s in a managed dir.
    pub fn upsert(&mut self, src: &Path) -> Option<&mut SecretFile> {
        let dir = self.owning_dir(src)?;

        let rel = src.strip_prefix(&dir.mapping.src).ok()?.to_path_buf();
        let dst = dir.mapping.dst.join(&rel);

        let file = dir.files.entry(rel.clone()).or_insert_with(|| SecretFile {
            src: src.to_path_buf(),
            dst: dst.clone(),
        });

        // Always update src/dst in case structure changed.
        file.src = src.to_path_buf();
        file.dst = dst;

        Some(file)
    }

    /// Remove struct entry for this src and return the SecretFile if there was one.
    pub fn remove(&mut self, src: &Path) -> Option<SecretFile> {
        // Future: explicit SecretEntry::File case would go here.

        for entry in self.entries.values_mut() {
            if let SecretEntry::Dir(dir) = entry
                && src.starts_with(&dir.mapping.src)
            {
                let rel = src.strip_prefix(&dir.mapping.src).ok()?.to_path_buf();
                if let Some(file) = dir.files.remove(&rel) {
                    return Some(file);
                }
            }
        }
        None
    }

    /// Iterate over all file-backed secrets.
    pub fn iter_files(&self) -> impl Iterator<Item = &SecretFile> {
        self.entries.values().flat_map(EntryFiles::new)
    }

    pub fn iter_entries(&self) -> impl Iterator<Item = &SecretEntry> {
        self.entries.values()
    }
}

#[derive(Debug)]
enum EntryFiles<'a> {
    File(Once<&'a SecretFile>),
    Dir(Values<'a, PathBuf, SecretFile>),
}

impl<'a> EntryFiles<'a> {
    fn new(entry: &'a SecretEntry) -> Self {
        match entry {
            SecretEntry::File(f) => EntryFiles::File(std::iter::once(f)),
            SecretEntry::Dir(d) => EntryFiles::Dir(d.files.values()),
        }
    }
}

impl<'a> Iterator for EntryFiles<'a> {
    type Item = &'a SecretFile;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            EntryFiles::File(it) => it.next(),
            EntryFiles::Dir(it) => it.next(),
        }
    }
}
