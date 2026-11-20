use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Directory(pub PathBuf);

impl Directory {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rajo-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    pub fn image(&self, name: &str, width: i32, height: i32) -> gtk::gio::File {
        let pixbuf =
            gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, width, height)
                .unwrap();
        pixbuf.fill(0x336699ff);
        let path = self.0.join(name);
        pixbuf.savev(&path, "png", &[]).unwrap();
        gtk::gio::File::for_path(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
