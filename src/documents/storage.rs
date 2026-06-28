//! File storage: save uploaded files to disk, serve downloads.
//! TODO Fase 1: implement save / get_path / delete_file

use std::path::{Path, PathBuf};

pub struct FileStorage {
    base: PathBuf,
}

impl FileStorage {
    pub fn new(base: impl AsRef<Path>) -> Self {
        Self { base: base.as_ref().to_owned() }
    }

    pub fn path_for(&self, document_id: &str, filename: &str) -> PathBuf {
        self.base.join(document_id).join(filename)
    }
}
