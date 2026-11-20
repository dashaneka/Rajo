//! Streaming, cancellable decoding on Gio's existing worker pool.
use std::cell::Cell;
use std::rc::Rc;

use gtk::{gdk, gdk_pixbuf, gio, glib, prelude::*};

/// A screen-sized decode avoids keeping a full-resolution photo in memory.
pub const PREVIEW_EDGE: i32 = 2048;

pub struct Image {
    pixels: glib::Bytes,
    width: i32,
    height: i32,
    stride: usize,
    alpha: bool,
    pub original: (i32, i32),
}

impl Image {
    pub fn texture(self) -> gdk::Texture {
        let format = if self.alpha {
            gdk::MemoryFormat::R8g8b8a8
        } else {
            gdk::MemoryFormat::R8g8b8
        };
        gdk::MemoryTexture::new(self.width, self.height, format, &self.pixels, self.stride).upcast()
    }
}

pub async fn load(file: gio::File, cancel: gio::Cancellable, edge: i32) -> Result<Image, String> {
    gio::spawn_blocking(move || {
        // Limit simultaneous decoder allocations across windows and superseded
        // requests. Cancelled jobs leave without opening or decoding a file.
        static DECODER: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = DECODER.lock().unwrap_or_else(|poison| poison.into_inner());
        if cancel.is_cancelled() {
            return Err("Loading cancelled".into());
        }
        decode(&file, &cancel, edge)
    })
    .await
    .map_err(|_| "Image loader stopped unexpectedly".to_owned())?
}

fn decode(file: &gio::File, cancel: &gio::Cancellable, edge: i32) -> Result<Image, String> {
    let raw = file.read(Some(cancel)).map_err(|e| e.to_string())?;
    let buffered = gio::BufferedInputStream::new(&raw);
    buffered.fill(2, Some(cancel)).map_err(|e| e.to_string())?;
    // PixbufLoader does not unwrap compressed SVGs when fed a byte stream.
    // Gio supplies streaming gzip decompression, retaining cancellation support.
    let stream: gio::InputStream = if buffered.peek_buffer().starts_with(&[0x1f, 0x8b]) {
        gio::ConverterInputStream::new(
            &buffered,
            &gio::ZlibDecompressor::new(gio::ZlibCompressorFormat::Gzip),
        )
        .upcast()
    } else {
        buffered.upcast()
    };
    let loader = gdk_pixbuf::PixbufLoader::new();
    let original = Rc::new(Cell::new((0, 0)));
    let size = original.clone();
    loader.connect_size_prepared(move |loader, width, height| {
        if width <= 0 || height <= 0 {
            return;
        }
        size.set((width, height));
        if edge > 0 {
            let scale = (edge as f64 / width.max(height) as f64).min(1.0);
            loader.set_size(
                (width as f64 * scale).round().max(1.0) as i32,
                (height as f64 * scale).round().max(1.0) as i32,
            );
        }
    });
    // Never retain a second copy of the entire compressed file.
    let result = (|| {
        loop {
            let bytes = stream.read_bytes(64 * 1024, Some(cancel))?;
            if bytes.is_empty() {
                break;
            }
            loader.write(bytes.as_ref())?;
        }
        Ok::<_, glib::Error>(())
    })();
    let closed = loader.close();
    result.and(closed).map_err(|e| e.to_string())?;
    if cancel.is_cancelled() {
        return Err("Loading cancelled".into());
    }
    let pixbuf = loader.pixbuf().ok_or("No image data found")?;
    let mut original = original.get();
    if original.0 <= 0 || original.1 <= 0 {
        original = (pixbuf.width(), pixbuf.height());
    }
    if original.0 <= 0 || original.1 <= 0 {
        return Err("Image has empty dimensions".into());
    }
    if pixbuf
        .option("orientation")
        .is_some_and(|v| matches!(v.as_str(), "5" | "6" | "7" | "8"))
    {
        original = (original.1, original.0);
    }
    let mut pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
    // Some loaders scale only after decoding. Bound the retained pixels even
    // when a format does not honor the requested decode size precisely.
    let longest = pixbuf.width().max(pixbuf.height());
    if edge > 0 && longest > edge {
        let scale = edge as f64 / longest as f64;
        pixbuf = pixbuf
            .scale_simple(
                (pixbuf.width() as f64 * scale).round().max(1.0) as i32,
                (pixbuf.height() as f64 * scale).round().max(1.0) as i32,
                gdk_pixbuf::InterpType::Bilinear,
            )
            .ok_or("Unable to scale image")?;
    }
    Ok(Image {
        pixels: pixbuf.read_pixel_bytes(),
        width: pixbuf.width(),
        height: pixbuf.height(),
        stride: pixbuf.rowstride() as usize,
        alpha: pixbuf.has_alpha(),
        original,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Directory;

    #[test]
    fn preview_preserves_dimensions_and_reduces_pixel_storage() {
        let dir = Directory::new();
        let file = dir.image("large.png", 4096, 3072);
        let cancel = gio::Cancellable::new();
        let preview = decode(&file, &cancel, 1024).unwrap();
        assert_eq!(preview.original, (4096, 3072));
        assert_eq!((preview.width, preview.height), (1024, 768));
        assert!(preview.pixels.len() <= 1024 * 768 * 4);
        let full = decode(&file, &cancel, 4096).unwrap();
        assert_eq!((full.width, full.height), (4096, 3072));
        assert!(preview.pixels.len() * 15 < full.pixels.len());
    }

    #[test]
    fn rejects_corrupt_missing_and_cancelled_files() {
        let dir = Directory::new();
        let path = dir.0.join("broken.png");
        std::fs::write(&path, b"not an image").unwrap();
        let file = gio::File::for_path(path);
        assert!(decode(&file, &gio::Cancellable::new(), PREVIEW_EDGE).is_err());
        let missing = gio::File::for_path(dir.0.join("missing.png"));
        assert!(decode(&missing, &gio::Cancellable::new(), PREVIEW_EDGE).is_err());
        let empty_svg = dir.0.join("empty.svg");
        std::fs::write(
            &empty_svg,
            b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"0\" height=\"0\"/>",
        )
        .unwrap();
        assert!(
            decode(
                &gio::File::for_path(empty_svg),
                &gio::Cancellable::new(),
                PREVIEW_EDGE
            )
            .is_err()
        );
        let file = dir.image("valid.png", 32, 24);
        let cancel = gio::Cancellable::new();
        cancel.cancel();
        assert!(decode(&file, &cancel, PREVIEW_EDGE).is_err());
    }

    #[test]
    fn background_loading_yields_to_main_context() {
        let dir = Directory::new();
        let file = dir.image("test.png", 1024, 768);
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let heartbeat = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let tick = heartbeat.clone();
                glib::idle_source_new(None, glib::Priority::HIGH, move || {
                    tick.store(true, std::sync::atomic::Ordering::Relaxed);
                    glib::ControlFlow::Break
                })
                .attach(Some(&context));
                context.block_on(async {
                    let image = load(file, gio::Cancellable::new(), 256).await.unwrap();
                    assert!(heartbeat.load(std::sync::atomic::Ordering::Relaxed));
                    assert_eq!(image.original, (1024, 768));
                    assert_eq!((image.width, image.height), (256, 192));
                });
            })
            .unwrap();
    }
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;

    fn fixture(name: &str) -> gio::File {
        gio::File::for_path(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
    }

    fn pixel(image: &Image, x: usize, y: usize) -> &[u8] {
        let channels = if image.alpha { 4 } else { 3 };
        let offset = y * image.stride + x * channels;
        &image.pixels[offset..offset + channels]
    }

    #[test]
    fn common_formats_decode_with_correct_size_and_colors() {
        let cancel = gio::Cancellable::new();
        let formats = gdk_pixbuf::Pixbuf::formats();
        for ext in [
            "jpg", "png", "webp", "svg", "svgz", "gif", "tiff", "bmp", "tga", "ppm", "avif", "heic",
        ] {
            if !formats
                .iter()
                .any(|f| !f.is_disabled() && f.extensions().iter().any(|e| e == ext))
            {
                eprintln!("SKIP {ext}: system decoder not installed");
                continue;
            }
            let image = decode(&fixture(&format!("sample.{ext}")), &cancel, PREVIEW_EDGE)
                .unwrap_or_else(|err| panic!("{ext}: {err}"));
            assert_eq!(image.original, (48, 32), "{ext}: original dimensions");
            assert_eq!(
                (image.width, image.height),
                (48, 32),
                "{ext}: decoded dimensions"
            );
            let red = pixel(&image, 5, 5);
            let blue = pixel(&image, 42, 5);
            assert!(red[0] > 180 && red[2] < 70, "{ext}: red pixel {red:?}");
            assert!(blue[2] > 180 && blue[0] < 70, "{ext}: blue pixel {blue:?}");
            eprintln!("PASS {ext}: dimensions and pixel colors");
        }
    }

    #[test]
    fn exif_orientations_are_applied_once() {
        let cancel = gio::Cancellable::new();
        for orientation in 1..=8 {
            let image = decode(
                &fixture(&format!("orientation-{orientation}.jpg")),
                &cancel,
                PREVIEW_EDGE,
            )
            .unwrap();
            let size = if orientation >= 5 { (32, 48) } else { (48, 32) };
            assert_eq!(image.original, size, "EXIF {orientation}: original");
            assert_eq!(
                (image.width, image.height),
                size,
                "EXIF {orientation}: pixels"
            );
            let top_left = pixel(&image, 5, 5);
            let red = matches!(orientation, 1 | 4 | 5 | 6);
            assert!(
                if red {
                    top_left[0] > 180 && top_left[2] < 70
                } else {
                    top_left[2] > 180 && top_left[0] < 70
                },
                "EXIF {orientation}: {top_left:?}"
            );
        }
    }

    #[test]
    fn preserves_transparency_and_uri_escaping() {
        let dir = crate::test_support::Directory::new();
        let path = dir.0.join("-photo café #1 % & <test>.png");
        std::fs::copy(fixture("alpha.png").path().unwrap(), &path).unwrap();
        let file = gio::File::for_path(&path);
        let uri_file = gio::File::for_uri(&file.uri());
        let image = decode(&uri_file, &gio::Cancellable::new(), PREVIEW_EDGE).unwrap();
        assert!(image.alpha);
        assert_eq!(pixel(&image, 0, 0)[3], 0);
        assert_eq!(pixel(&image, 5, 5)[3], 255);
        assert_eq!(uri_file.path().unwrap(), path);
    }

    #[cfg(unix)]
    #[test]
    fn opens_non_utf8_filenames() {
        use std::os::unix::ffi::OsStringExt;
        let dir = crate::test_support::Directory::new();
        let path = dir
            .0
            .join(std::ffi::OsString::from_vec(b"photo-\xff.png".to_vec()));
        std::fs::copy(fixture("sample.png").path().unwrap(), &path).unwrap();
        let image = decode(
            &gio::File::for_path(path),
            &gio::Cancellable::new(),
            PREVIEW_EDGE,
        )
        .unwrap();
        assert_eq!(image.original, (48, 32));
    }
}
