//! Which seat owns a path: the longest configured folder that contains it.

use crate::config::{Config, Owner};
use crate::paths;
use std::path::{Component, Path, PathBuf};

/// Absolute path with `.`/`..` removed and symlinks resolved on the deepest part that exists
/// (the rest may not exist yet, e.g. a file about to be written).
pub fn normalize(p: &Path, cwd: &Path) -> PathBuf {
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    };
    let mut clean = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            other => clean.push(other.as_os_str()),
        }
    }
    let mut existing = clean;
    let mut missing = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name().map(|n| n.to_os_string()) else {
            break;
        };
        missing.push(name);
        existing.pop();
    }
    let mut out = dunce::canonicalize(&existing).unwrap_or(existing);
    out.extend(missing.into_iter().rev());
    out
}

/// Comparable form: components, case-folded where the usual file systems ignore case.
pub(crate) fn key(p: &Path) -> Vec<String> {
    p.components()
        .map(|c| {
            let s = c.as_os_str().to_string_lossy();
            if cfg!(any(target_os = "macos", windows)) {
                s.to_lowercase()
            } else {
                s.into_owned()
            }
        })
        .collect()
}

/// A string that is equal for two paths exactly when they name the same folder (after normalizing).
pub fn key_string(p: &Path) -> String {
    key(&normalize(p, &paths::home())).join("/")
}

/// Owner of an already-normalized path, or `None` if no configured folder contains it.
pub fn owner_of<'a>(cfg: &'a Config, path: &Path) -> Option<Owner<'a>> {
    let target = key(path);
    let home = paths::home();
    cfg.folders
        .iter()
        .map(|f| (key(&normalize(&paths::expand(&f.path), &home)), f))
        .filter(|(folder, _)| target.starts_with(folder))
        .max_by_key(|(folder, _)| folder.len())
        .map(|(_, f)| f.owner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Folder;

    fn cfg(entries: &[(&Path, &str)]) -> Config {
        Config {
            folders: entries
                .iter()
                .map(|(p, o)| Folder {
                    path: p.to_string_lossy().into(),
                    owner: o.to_string(),
                })
                .collect(),
            ..Config::default()
        }
    }

    #[test]
    fn longest_folder_wins() {
        let t = tempfile::tempdir().unwrap();
        let root = normalize(t.path(), Path::new("/"));
        let (a, nested) = (root.join("a"), root.join("a/b/c"));
        std::fs::create_dir_all(&nested).unwrap();
        let c = cfg(&[(&root, "none"), (&a, "work"), (&nested, "other")]);
        let own = |p: &Path| owner_of(&c, &normalize(p, &root));
        assert_eq!(own(&root.join("x")), Some(Owner::Nobody));
        assert_eq!(own(&a.join("b")), Some(Owner::Seat("work")));
        assert_eq!(
            own(&nested.join("new-file.txt")),
            Some(Owner::Seat("other"))
        );
        assert_eq!(
            own(Path::new("a/b/c/../x")),
            Some(Owner::Seat("work")),
            "relative + .."
        );
        assert_eq!(
            own(&root.join("ab")),
            Some(Owner::Nobody),
            "prefix match is per component"
        );
        assert_eq!(owner_of(&c, Path::new("/elsewhere")), None);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_resolve_to_the_real_folder() {
        let t = tempfile::tempdir().unwrap();
        let root = normalize(t.path(), Path::new("/"));
        std::fs::create_dir_all(root.join("real/proj")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
        let c = cfg(&[(&root.join("link/proj"), "work")]);
        assert_eq!(
            owner_of(&c, &normalize(&root.join("real/proj/f"), &root)),
            Some(Owner::Seat("work"))
        );
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn case_is_ignored_where_the_file_system_ignores_it() {
        let t = tempfile::tempdir().unwrap();
        let root = normalize(t.path(), Path::new("/"));
        let c = cfg(&[(&root.join("Proj"), "work")]);
        assert_eq!(
            owner_of(&c, &normalize(&root.join("PROJ/x"), &root)),
            Some(Owner::Seat("work"))
        );
    }
}
