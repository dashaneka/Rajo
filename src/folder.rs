//! Sibling images in the same folder, for next / previous.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use gtk::gio;
use gtk::prelude::*;

pub fn siblings(file: &gio::File, cancel: &gio::Cancellable) -> (Vec<gio::File>, usize) {
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
        .take_while(|_| !cancel.is_cancelled())
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !is_image(&path) {
                return None;
            }
            // read_dir usually already has the type; avoid a stat per image.
            let kind = entry.file_type().ok()?;
            (kind.is_file() || (kind.is_symlink() && path.is_file())).then_some(path)
        })
        .collect();
    // A successfully opened file always belongs in its own navigation list,
    // including extensionless files and formats outside the extension hints.
    if !paths.contains(&path) {
        paths.push(path.clone());
    }
    paths.sort_unstable();
    let index = paths.binary_search(&path).expect("opened file is included");
    (paths.into_iter().map(gio::File::for_path).collect(), index)
}

fn is_image(path: &Path) -> bool {
    static EXTENSIONS: OnceLock<HashSet<String>> = OnceLock::new();
    let extensions = EXTENSIONS.get_or_init(|| {
        gtk::gdk_pixbuf::Pixbuf::formats()
            .into_iter()
            .filter(|format| !format.is_disabled())
            .flat_map(|format| format.extensions())
            .map(|extension| extension.to_ascii_lowercase())
            .collect()
    });
    let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    let extension = extension.to_ascii_lowercase();
    extensions.contains(&extension)
        || (extension == "gz"
            && extensions.contains("svg.gz")
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.to_ascii_lowercase().ends_with(".svg.gz")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Directory;

    #[test]
    fn keeps_opened_file_and_filters_directories() {
        let dir = Directory::new();
        for name in ["a.png", "b.PPM", "c.pnm", "opened", "ignore.txt"] {
            std::fs::write(dir.0.join(name), []).unwrap();
        }
        std::fs::create_dir(dir.0.join("folder.jpg")).unwrap();
        let file = gio::File::for_path(dir.0.join("opened"));
        let (files, index) = siblings(&file, &gio::Cancellable::new());
        assert_eq!(files.len(), 4);
        assert!(files[index].equal(&file));
        assert_eq!(index, 3);
    }

    #[test]
    fn handles_single_file_and_remote_uri() {
        let dir = Directory::new();
        let file = gio::File::for_path(dir.0.join("unlisted.format"));
        let (files, index) = siblings(&file, &gio::Cancellable::new());
        assert_eq!(files.len(), 1);
        assert!(files[0].equal(&file));
        assert_eq!(index, 0);
        let file = gio::File::for_uri("https://example.invalid/photo.png");
        assert!(siblings(&file, &gio::Cancellable::new()).0[0].equal(&file));
    }
}

#[cfg(test)]
mod stress_tests {
    use super::*;

    #[test]
    fn large_folder_and_cancelled_scan() {
        let dir = crate::test_support::Directory::new();
        for index in 0..10000 {
            std::fs::write(dir.0.join(format!("photo-{index:05}.PNG")), []).unwrap();
        }
        let file = gio::File::for_path(dir.0.join("photo-05000.PNG"));
        let start = std::time::Instant::now();
        let (files, index) = siblings(&file, &gio::Cancellable::new());
        assert_eq!(files.len(), 10000);
        assert!(files[index].equal(&file));
        assert_eq!(index, 5000);
        eprintln!("10,000-image folder scan: {:?}", start.elapsed());
        let cancel = gio::Cancellable::new();
        cancel.cancel();
        let (files, index) = siblings(&file, &cancel);
        assert_eq!(files.len(), 1);
        assert_eq!(index, 0);
    }

    #[test]
    fn recognizes_compressed_svg_and_installed_codec_extensions() {
        assert!(is_image(Path::new("photo.SVGZ")));
        assert!(is_image(Path::new("photo.svg.gz")));
        assert!(!is_image(Path::new("archive.tar.gz")));
        for format in gtk::gdk_pixbuf::Pixbuf::formats() {
            if format.is_disabled() {
                continue;
            }
            for ext in format.extensions() {
                assert!(is_image(Path::new(&format!("image.{ext}"))), "{ext}");
            }
        }
    }
}
