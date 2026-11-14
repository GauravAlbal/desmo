//! Path containment, digest, and atomic write helpers for adapter runs.
//!
//! Symlinks are rejected rather than followed. A path that leaves its root is a
//! traversal, even when the lexical `..` would still land inside a parent the
//! host created.

use crate::canonical::sha256_hex;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub const PATH_TRAVERSAL: &str = "path_traversal";
pub const SYMLINK_REJECTED: &str = "symlink_rejected";

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    Ok(sha256_hex(&bytes))
}

pub fn write_atomic(directory: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let tmp_name = format!(".{name}.{}.tmp", std::process::id());
    let tmp = directory.join(tmp_name);
    write_new(&tmp, bytes)?;
    let final_path = directory.join(name);
    fs::rename(&tmp, &final_path).map_err(|error| error.to_string())
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = File::create(path).map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())
}

/// Resolve `relative` inside `root`. Rejects absolute paths, `..`, and symlinks
/// created under `root`. Ancestor symlinks (a platform temp directory) are not
/// adapter-controlled and are not treated as artifact symlinks.
pub fn contained_file(root: &Path, relative: &str) -> Result<PathBuf, &'static str> {
    if !relative_is_local(relative) {
        return Err(PATH_TRAVERSAL);
    }
    if relative_has_symlink(root, Path::new(relative)) {
        return Err(SYMLINK_REJECTED);
    }
    Ok(root.join(relative))
}

fn relative_has_symlink(root: &Path, relative: &Path) -> bool {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        if metadata_is_symlink(&current) {
            return true;
        }
    }
    false
}

pub fn relative_is_local(relative: &str) -> bool {
    if relative.is_empty() || relative.contains('\0') || relative.starts_with('/') {
        return false;
    }
    let path = Path::new(relative);
    if path.is_absolute() {
        return false;
    }
    path.components().all(component_is_local)
}

fn component_is_local(component: Component) -> bool {
    matches!(component, Component::Normal(_) | Component::CurDir)
}

fn metadata_is_symlink(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(meta) => meta.file_type().is_symlink(),
        Err(_) => false,
    }
}

pub fn is_symlink(path: &Path) -> bool {
    metadata_is_symlink(path)
}

pub fn non_empty_regular_file(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => false,
        Ok(meta) if meta.is_file() => meta.len() > 0,
        _ => false,
    }
}

pub fn path_inside(child: &Path, root: &Path) -> bool {
    let Ok(child) = child.canonicalize() else {
        return false;
    };
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    child.starts_with(root)
}
