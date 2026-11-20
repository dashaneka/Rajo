#!/usr/bin/env python3
"""Compare viewer RSS and CPU time with the same image and desktop renderer.

Usage: python3 tools/benchmark.py --image photo.jpg /tmp/rajo-before target/release/rajo
RSS includes shared libraries; GPU allocations and decoder subprocesses are excluded.
"""
import argparse
import json
import os
import signal
import subprocess
import time
from pathlib import Path


def measure(binary, image, seconds):
    command = [str(Path(binary).resolve()), str(Path(image).resolve())]
    samples = []
    ticks = 0
    with subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE) as proc:
        try:
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                if proc.poll() is not None:
                    raise RuntimeError(f"{binary} exited with status {proc.returncode}")
                status = Path(f"/proc/{proc.pid}/status").read_text()
                values = dict(line.split(":", 1) for line in status.splitlines() if ":" in line)
                samples.append(int(values["VmRSS"].split()[0]))
                stat = Path(f"/proc/{proc.pid}/stat").read_text().rsplit(")", 1)[1].split()
                ticks = int(stat[11]) + int(stat[12])
                time.sleep(0.05)
            return {
                "binary": binary,
                "binary_bytes": Path(binary).stat().st_size,
                "settled_rss_kib": samples[-1],
                "peak_sampled_rss_kib": max(samples),
                "cpu_seconds": round(ticks / os.sysconf("SC_CLK_TCK"), 3),
                "sample_seconds": seconds,
            }
        finally:
            if proc.poll() is None:
                proc.send_signal(signal.SIGTERM)
            try:
                _, err = proc.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
                _, err = proc.communicate()
            if err:
                import sys
                print(err.decode(errors="replace"), file=sys.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True)
    parser.add_argument("--seconds", type=float, default=5)
    parser.add_argument("binaries", nargs="+")
    args = parser.parse_args()
    if args.seconds <= 0:
        parser.error("--seconds must be positive")
    for executable in args.binaries:
        print(json.dumps(measure(executable, args.image, args.seconds)), flush=True)
