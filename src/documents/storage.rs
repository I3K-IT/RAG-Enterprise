//! File storage: save uploaded files to disk, serve downloads.
//!
//! SICUREZZA (path traversal): `path_for` è l'UNICO costruttore di path usato da
//! upload (scrittura), download (lettura) e delete (rimozione). Sanifica ogni
//! componente a solo basename, così un filename ostile dal multipart
//! (`../../etc/passwd`, path assoluti, separatori Windows) non può mai far
//! uscire il path dalla cartella base. Un solo punto → una sola difesa.

use std::path::{Path, PathBuf};

pub struct FileStorage {
    base: PathBuf,
}

impl FileStorage {
    pub fn new(base: impl AsRef<Path>) -> Self {
        Self { base: base.as_ref().to_owned() }
    }

    pub fn path_for(&self, document_id: &str, filename: &str) -> PathBuf {
        self.base
            .join(sanitize_component(document_id))
            .join(sanitize_component(filename))
    }
}

/// Riduce una stringa a un singolo, innocuo componente di path.
///
/// Prende solo l'ultimo segmento dopo qualsiasi separatore (`/` o `\`, per
/// neutralizzare anche i path in stile Windows), quindi rifiuta i componenti
/// che risalirebbero l'albero (`.`, `..`) o vuoti. Per costruzione il risultato
/// non contiene separatori né sequenze `..`, quindi `base.join(...)` resta
/// sempre dentro `base`.
fn sanitize_component(s: &str) -> String {
    let last = s.rsplit(['/', '\\']).next().unwrap_or("");
    if last.is_empty() || last == "." || last == ".." {
        "unnamed".to_owned()
    } else {
        last.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_parent_traversal_from_filename() {
        let s = FileStorage::new("/data/uploads");
        let p = s.path_for("doc-uuid", "../../etc/passwd");
        assert_eq!(p, PathBuf::from("/data/uploads/doc-uuid/passwd"));
    }

    #[test]
    fn strips_absolute_path_from_filename() {
        let s = FileStorage::new("/data/uploads");
        let p = s.path_for("doc-uuid", "/etc/shadow");
        assert_eq!(p, PathBuf::from("/data/uploads/doc-uuid/shadow"));
    }

    #[test]
    fn strips_windows_separators() {
        let s = FileStorage::new("/data/uploads");
        let p = s.path_for("doc-uuid", "..\\..\\windows\\system32\\cmd.exe");
        assert_eq!(p, PathBuf::from("/data/uploads/doc-uuid/cmd.exe"));
    }

    #[test]
    fn dotdot_and_empty_become_unnamed() {
        let s = FileStorage::new("/data/uploads");
        assert_eq!(s.path_for("d", ".."), PathBuf::from("/data/uploads/d/unnamed"));
        assert_eq!(s.path_for("d", ""), PathBuf::from("/data/uploads/d/unnamed"));
        assert_eq!(s.path_for("d", "sub/"), PathBuf::from("/data/uploads/d/unnamed"));
    }

    #[test]
    fn plain_filename_is_preserved() {
        let s = FileStorage::new("/data/uploads");
        let p = s.path_for("doc-uuid", "Relazione 2026.pdf");
        assert_eq!(p, PathBuf::from("/data/uploads/doc-uuid/Relazione 2026.pdf"));
    }

    #[test]
    fn result_is_always_within_base() {
        let s = FileStorage::new("/data/uploads");
        for hostile in ["../../../etc/passwd", "/etc/shadow", "..", "", "a/../../b"] {
            let p = s.path_for("id", hostile);
            assert!(
                p.starts_with("/data/uploads/id"),
                "path {p:?} è uscito dalla base per input {hostile:?}"
            );
        }
    }
}
