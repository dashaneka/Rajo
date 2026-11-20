#!/usr/bin/env python3
"""Launch the installed desktop entry with multiple unusual filenames.
Requires Python's Gio bindings; closes only the process tagged by this test.
"""
import os
from pathlib import Path
import shutil
import signal
import tempfile
import time
import uuid
import gi

gi.require_version('Gio', '2.0')
from gi.repository import Gio

binary = (Path.home() / '.local/bin/rajo').resolve()
desktop = Path.home() / '.local/share/applications/io.github.rajnish.Rajo.desktop'
fixture = Path(__file__).resolve().parents[1] / 'tests/fixtures/sample.png'
tag = f'RAJO_DESKTOP_SMOKE={uuid.uuid4().hex}'.encode()


def test_processes():
    result = []
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if (entry / 'exe').resolve() == binary and tag in (entry / 'environ').read_bytes().split(b'\0'):
                result.append(int(entry.name))
        except (OSError, RuntimeError):
            pass
    return result


with tempfile.TemporaryDirectory(prefix='rajo-desktop-smoke-') as folder:
    root = Path(folder)
    files = []
    for name in ('photo #1 % & <test>.png', 'ფოტო café 图片.png', '-photo.png'):
        path = root / name
        shutil.copyfile(fixture, path)
        files.append(Gio.File.new_for_path(str(path)))
    context = Gio.AppLaunchContext()
    context.setenv('RAJO_DESKTOP_SMOKE', tag.decode().split('=', 1)[1])
    context.setenv('XDG_DATA_HOME', str(root / 'data'))
    app = Gio.DesktopAppInfo.new_from_filename(str(desktop))
    if app is None:
        raise RuntimeError('Installed Rajo desktop entry was not found')
    try:
        assert app.launch(files, context), 'Desktop launch failed'
        deadline = time.monotonic() + 5
        pids = test_processes()
        while not pids and time.monotonic() < deadline:
            time.sleep(0.05)
            pids = test_processes()
        assert len(pids) == 1, f'Expected one Rajo process, found {pids}'
        arguments = (Path('/proc') / str(pids[0]) / 'cmdline').read_bytes().split(b'\0')[1:]
        actual = [Gio.File.new_for_commandline_arg(os.fsdecode(arg)) for arg in arguments if arg and arg != b'--']
        assert len(actual) == len(files), 'Some file-manager selections were lost'
        assert all(a.equal(b) for a, b in zip(actual, files)), 'Filename or URI escaping changed a selection'
        time.sleep(1)
        assert pids[0] in test_processes(), 'Viewer exited unexpectedly'
        print('PASS: installed desktop entry launches all three selections with exact filenames')
    finally:
        for pid in test_processes():
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        deadline = time.monotonic() + 5
        while test_processes() and time.monotonic() < deadline:
            time.sleep(0.05)
        assert not test_processes(), 'Test viewer did not shut down cleanly'
