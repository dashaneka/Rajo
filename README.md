<p align="center">
  <img src="data/io.github.rajnish.Rajo.svg" width="128" alt="Rajo icon">
</p>

<h1 align="center">Rajo</h1>

<p align="center">A small, fast image viewer for GNOME.</p>

Rajo does one thing: shows your images and gets out of the way. Built with Rust, GTK4 and libadwaita — same spirit as [Monet](https://github.com/dashaneka/Monet).

## What it does

- Controls fade away while you look
- Window takes the shape of the image
- Next / previous image in the folder
- Every image opens in Fit mode, including small images
- Zoom, pan, fit, and 100%
- Recent images on the start screen
- Open multiple images from Files, drag-and-drop, or `Ctrl+O`

## Image compatibility

On this system, the compatibility suite verifies JPEG, PNG, WebP, SVG, SVGZ,
GIF, TIFF, BMP, TGA, PPM, AVIF, and HEIC using real image fixtures. It checks
pixel colors, dimensions, transparency, and all eight JPEG EXIF orientations.
Available formats depend on the GdkPixbuf loaders installed by your distribution;
the file picker and folder navigation follow those installed loaders.

GIFs display a still frame; animation playback is not implemented. Tests for
optional formats report a skip when their system decoder is unavailable.

File-manager selections, multi-file drops, and the file picker open one window
per image. Filenames with spaces, Unicode, URI punctuation, and (on Unix)
non-UTF-8 bytes are covered by regression tests. Folder scanning is tested with
10,000 entries and cancellation.

## Performance

Image decoding and folder scanning run in the background. Opening another file
cancels the previous request, and unreadable folder entries are skipped.
A failed open keeps the current image available.

Large images initially use a preview capped at 2048 pixels on the longest edge.
Zooming loads more detail as needed; Fit releases the larger decoded texture.
Only one decoder runs at a time, and there is no full-resolution image cache.
Some system codecs still decode at full size before scaling, so temporary peak
memory depends on the image format. Full detail naturally needs more memory.

The release executable uses size optimization, LTO, and stripped symbols, with
GTK and libadwaita supplied by the system. No additional runtime dependencies
were added for background loading.

## Checks

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
desktop-file-validate data/io.github.rajnish.Rajo.desktop
```

The viewer integration test needs a graphical display (or `xvfb-run` in CI).
To compare memory and CPU time against a saved older executable:

```sh
python3 tools/benchmark.py --image /path/to/photo.jpg /tmp/rajo-before target/release/rajo
```

## Keys

| Key | Does |
|---|---|
| `←` `→` or `Space` | Previous / next |
| `+` `-` or `Ctrl`+scroll | Zoom |
| Drag | Pan when zoomed in |
| `0` | Fit to window |
| `1` | Actual size |
| `F` or double-click | Fullscreen |
| `Esc` | Leave fullscreen |

## Build it

You need [Rust](https://rustup.rs) and a few libraries.

**Fedora**

```sh
sudo dnf install gtk4-devel libadwaita-devel
```

**Ubuntu / Debian**

```sh
sudo apt install libgtk-4-dev libadwaita-1-dev
```

Then:

```sh
git clone https://github.com/dashaneka/Rajo.git
cd Rajo
./install.sh            # installs to ~/.local, no sudo
./install.sh --default  # same, and makes Rajo your default image viewer
```

Needs GTK 4.14+ and libadwaita 1.6+.

## Uninstall

```sh
rm ~/.local/bin/rajo ~/.local/share/applications/io.github.rajnish.Rajo.desktop \
   ~/.local/share/icons/hicolor/scalable/apps/io.github.rajnish.Rajo.svg
```

## License

[GPL-3.0-or-later](LICENSE). Use it, change it, share it.
