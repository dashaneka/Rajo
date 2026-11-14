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
- Zoom, pan, fit, and 100%
- Recent images on the start screen
- Drag an image in, or press `Ctrl+O`

## Keys

| Key | Does |
|---|---|
| `←` `→` or `Space` | Previous / next |
| `+` `-` or scroll | Zoom |
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
