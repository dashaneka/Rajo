//! Sibling images in the same folder, for next / previous.

use std::path::{Path, PathBuf};

use gtk::gio;
use gtk::prelude::*;

const EXTENSIONS: [&str; 14] = [
    "jpg", "jpeg", "jpe", "png", "gif", "bmp", "webp", "svg", "tif", "tiff", "tga",
    "avif", "heic", "heif",
];

pub fn siblings(file: &gio::File) -> (Vec<gio::File>, usize) {
    let Some(path) = file.path() else {
        return (vec![file.clone()], 0);
    };
    let Some(dir) = path.parent() else {
        return (vec![file.clone()], 0);
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (vec![file.clone()], 0);
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| is_image(path))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return (vec![file.clone()], 0);
    }
    let index = paths.iter().position(|p| p == &path).unwrap_or(0);
    (paths.into_iter().map(gio::File::for_path).collect(), index)
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}
