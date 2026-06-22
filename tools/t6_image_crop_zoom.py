#!/usr/bin/env python3
"""Crop and zoom an image for visual artifact inspection using macOS sips."""

from __future__ import annotations

import argparse
import pathlib
import subprocess
import tempfile


def parse_rect(value: str) -> tuple[int, int, int, int]:
    try:
        size, pos = value.split("+", 1)
        width, height = size.lower().split("x", 1)
        x, y = pos.split("+", 1)
        return int(x), int(y), int(width), int(height)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("rect must be WxH+X+Y") from exc


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("input", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    parser.add_argument("--rect", type=parse_rect, required=True, help="Crop rectangle WxH+X+Y")
    parser.add_argument("--zoom", type=int, default=8)
    args = parser.parse_args()

    if args.zoom <= 0:
        raise SystemExit("--zoom must be greater than zero")

    x, y, width, height = args.rect
    with tempfile.TemporaryDirectory(prefix="t6-crop-") as tmp:
        cropped = pathlib.Path(tmp) / "crop.png"
        subprocess.run(
            [
                "sips",
                "--cropOffset",
                str(y),
                str(x),
                "--cropToHeightWidth",
                str(height),
                str(width),
                str(args.input),
                "--out",
                str(cropped),
            ],
            check=True,
        )
        subprocess.run(
            [
                "sips",
                "--resampleHeightWidth",
                str(height * args.zoom),
                str(width * args.zoom),
                str(cropped),
                "--out",
                str(args.output),
            ],
            check=True,
        )
    print(f"wrote {args.output} crop={width}x{height}+{x}+{y} zoom={args.zoom}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
