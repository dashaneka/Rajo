#!/usr/bin/env python3
"""Generate tiny, synthetic compatibility fixtures (maintainer tool only).
Requires Pillow, pillow-heif and ffmpeg with libaom-av1; not needed to run Rajo/tests.
"""
from pathlib import Path
import gzip
import subprocess
from PIL import Image
import pillow_heif

root = Path(__file__).resolve().parents[1] / 'tests' / 'fixtures'
root.mkdir(parents=True, exist_ok=True)
image = Image.new('RGB', (48, 32), 'red')
image.paste('blue', (24, 0, 48, 32))
for fmt, ext in [('PNG', 'png'), ('JPEG', 'jpg'), ('WEBP', 'webp'), ('TIFF', 'tiff'), ('BMP', 'bmp'), ('TGA', 'tga'), ('PPM', 'ppm')]:
    image.save(root / f'sample.{ext}', format=fmt)
image.save(root / 'sample.gif', save_all=True, append_images=[Image.new('RGB', image.size, 'green')], duration=100, loop=0)
rgba = image.convert('RGBA')
rgba.putpixel((0, 0), (255, 0, 0, 0))
rgba.save(root / 'alpha.png')
for orientation in range(1, 9):
    exif = Image.Exif()
    exif[274] = orientation
    image.save(root / f'orientation-{orientation}.jpg', exif=exif, quality=95)
svg = b'<svg xmlns="http://www.w3.org/2000/svg" width="48" height="32"><path fill="red" d="M0 0h24v32H0z"/><path fill="blue" d="M24 0h24v32H24z"/></svg>'
(root / 'sample.svg').write_bytes(svg)
(root / 'sample.svgz').write_bytes(gzip.compress(svg, mtime=0))
pillow_heif.register_heif_opener()
image.save(root / 'sample.heic', quality=90)
subprocess.run(['ffmpeg', '-y', '-loglevel', 'error', '-i', str(root / 'sample.png'), '-c:v', 'libaom-av1', '-still-picture', '1', str(root / 'sample.avif')], check=True)
