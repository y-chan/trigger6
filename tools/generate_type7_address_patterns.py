#!/usr/bin/env python3
"""Generate browser-based type7 address capture test patterns.

The generated HTML is intentionally self-contained. Open it on the Windows
JUA365 display, press Space or click once to start, then start USB capture.
"""

from __future__ import annotations

import argparse
import csv
import html
import json
import pathlib
from dataclasses import dataclass


@dataclass(frozen=True)
class Step:
    label: str
    duration_ms: int
    x: int | None
    y: int | None
    w: int
    h: int
    color: str = "#ffffff"


def xscan(tile_w: int, tile_h: int, hold_ms: int, black_ms: int) -> list[Step]:
    xs = [0, 64, 128, 256, 512, 1024, 1856]
    steps = [Step("black_start", black_ms, None, None, tile_w, tile_h)]
    steps += [Step(f"x{x}_y0", hold_ms, x, 0, tile_w, tile_h) for x in xs]
    steps.append(Step("black_end", hold_ms, None, None, tile_w, tile_h))
    return steps


def yscan(tile_w: int, tile_h: int, hold_ms: int, black_ms: int) -> list[Step]:
    ys = [0, 64, 128, 256, 512, 1016]
    steps = [Step("black_start", black_ms, None, None, tile_w, tile_h)]
    steps += [Step(f"x0_y{y}", hold_ms, 0, y, tile_w, tile_h) for y in ys]
    steps.append(Step("black_end", hold_ms, None, None, tile_w, tile_h))
    return steps


def grid(tile_w: int, tile_h: int, hold_ms: int, black_ms: int) -> list[Step]:
    points = [
        (0, 0),
        (64, 64),
        (512, 256),
        (1024, 512),
        (1856, 1016),
    ]
    steps = [Step("black_start", black_ms, None, None, tile_w, tile_h)]
    steps += [Step(f"x{x}_y{y}", hold_ms, x, y, tile_w, tile_h) for x, y in points]
    steps.append(Step("black_end", hold_ms, None, None, tile_w, tile_h))
    return steps


def horizontal_bands(hold_ms: int, black_ms: int) -> list[Step]:
    ys = [0, 56, 112, 224, 448, 728, 1016]
    steps = [Step("black_start", black_ms, None, None, 1920, 56)]
    steps += [Step(f"band_y{y}_1920x56", hold_ms, 0, y, 1920, 56) for y in ys]
    steps.append(Step("black_end", hold_ms, None, None, 1920, 56))
    return steps


def vertical_bands(hold_ms: int, black_ms: int) -> list[Step]:
    xs = [0, 192, 384, 768, 1152, 1536, 1728]
    steps = [Step("black_start", black_ms, None, None, 192, 1080)]
    steps += [Step(f"band_x{x}_192x1080", hold_ms, x, 0, 192, 1080) for x in xs]
    steps.append(Step("black_end", hold_ms, None, None, 192, 1080))
    return steps


def large_rects(hold_ms: int, black_ms: int) -> list[Step]:
    rects = [
        ("rect_448x64_top", 0, 0, 448, 64),
        ("rect_448x64_mid", 736, 508, 448, 64),
        ("rect_448x64_bottom", 1472, 1016, 448, 64),
        ("rect_1376x800_left", 0, 0, 1376, 800),
        ("rect_1376x952_left", 0, 0, 1376, 952),
        ("rect_1376x952_center", 272, 64, 1376, 952),
    ]
    steps = [Step("black_start", black_ms, None, None, 64, 64)]
    steps += [Step(label, hold_ms, x, y, w, h) for label, x, y, w, h in rects]
    steps.append(Step("black_end", hold_ms, None, None, 64, 64))
    return steps


def fullscreen_colors(hold_ms: int, black_ms: int) -> list[Step]:
    steps = [Step("black_start", black_ms, None, None, 64, 64)]
    steps += [
        Step("full_white", hold_ms, 0, 0, 1920, 1080, "#ffffff"),
        Step("black_between_1", hold_ms, None, None, 64, 64),
        Step("full_red", hold_ms, 0, 0, 1920, 1080, "#ff0000"),
        Step("full_green", hold_ms, 0, 0, 1920, 1080, "#00ff00"),
        Step("full_blue", hold_ms, 0, 0, 1920, 1080, "#0000ff"),
        Step("black_end", hold_ms, None, None, 64, 64),
    ]
    return steps


HTML_TEMPLATE = """<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title}</title>
  <style>
    html, body {{
      margin: 0;
      width: 100%;
      height: 100%;
      overflow: hidden;
      background: #000;
      cursor: none;
    }}
    canvas {{
      display: block;
      width: 100vw;
      height: 100vh;
      background: #000;
    }}
  </style>
</head>
<body>
<canvas id="pattern" width="{width}" height="{height}"></canvas>
<script>
const WIDTH = {width};
const HEIGHT = {height};
const STEPS = {steps_json};
const canvas = document.getElementById("pattern");
const ctx = canvas.getContext("2d", {{ alpha: false }});
let running = false;

function draw(step) {{
  ctx.fillStyle = "rgb(0,0,0)";
  ctx.fillRect(0, 0, WIDTH, HEIGHT);
  if (step.x !== null && step.y !== null) {{
    ctx.fillStyle = step.color || "#ffffff";
    ctx.fillRect(step.x, step.y, step.w, step.h);
  }}
}}

async function run() {{
  if (running) return;
  running = true;
  if (document.documentElement.requestFullscreen) {{
    try {{ await document.documentElement.requestFullscreen(); }} catch (_) {{}}
  }}
  console.log("type7-address-pattern-start", new Date().toISOString());
  for (const [index, step] of STEPS.entries()) {{
    console.log("step", index + 1, step.label, step.x, step.y, step.w, step.h, step.duration_ms);
    draw(step);
    await new Promise(resolve => setTimeout(resolve, step.duration_ms));
  }}
  draw({{ label: "black_done", x: null, y: null, w: 0, h: 0, duration_ms: 0 }});
  console.log("type7-address-pattern-done", new Date().toISOString());
  running = false;
}}

draw(STEPS[0]);
window.addEventListener("keydown", event => {{
  if (event.code === "Space" || event.code === "Enter") run();
}});
window.addEventListener("pointerdown", run);
</script>
</body>
</html>
"""


def write_pattern(
    out_dir: pathlib.Path,
    name: str,
    steps: list[Step],
    width: int,
    height: int,
) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    rows = [
        {
            "label": step.label,
            "duration_ms": step.duration_ms,
            "x": step.x,
            "y": step.y,
            "w": step.w,
            "h": step.h,
            "color": step.color,
        }
        for step in steps
    ]
    html_path = out_dir / f"{name}.html"
    csv_path = out_dir / f"{name}.csv"
    html_path.write_text(
        HTML_TEMPLATE.format(
            title=html.escape(name),
            width=width,
            height=height,
            steps_json=json.dumps(rows, separators=(",", ":")),
        ),
        encoding="utf-8",
    )
    with csv_path.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(
            f,
            fieldnames=["index", "label", "duration_ms", "x", "y", "w", "h", "color"],
        )
        writer.writeheader()
        for index, row in enumerate(rows, start=1):
            writer.writerow({"index": index, **row})
    print(f"wrote {html_path}")
    print(f"wrote {csv_path}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out-dir", type=pathlib.Path, default=pathlib.Path("captures/type7_address_patterns"))
    parser.add_argument("--width", type=int, default=1920)
    parser.add_argument("--height", type=int, default=1080)
    parser.add_argument("--tile-width", type=int, default=64)
    parser.add_argument("--tile-height", type=int, default=64)
    parser.add_argument("--hold-ms", type=int, default=1000)
    parser.add_argument("--black-ms", type=int, default=2000)
    args = parser.parse_args()

    write_pattern(
        args.out_dir,
        "type7_addr_xscan_64x64",
        xscan(args.tile_width, args.tile_height, args.hold_ms, args.black_ms),
        args.width,
        args.height,
    )
    write_pattern(
        args.out_dir,
        "type7_addr_yscan_64x64",
        yscan(args.tile_width, args.tile_height, args.hold_ms, args.black_ms),
        args.width,
        args.height,
    )
    write_pattern(
        args.out_dir,
        "type7_addr_grid_64x64",
        grid(args.tile_width, args.tile_height, args.hold_ms, args.black_ms),
        args.width,
        args.height,
    )
    write_pattern(
        args.out_dir,
        "type7_motion_horizontal_bands",
        horizontal_bands(args.hold_ms, args.black_ms),
        args.width,
        args.height,
    )
    write_pattern(
        args.out_dir,
        "type7_motion_vertical_bands",
        vertical_bands(args.hold_ms, args.black_ms),
        args.width,
        args.height,
    )
    write_pattern(
        args.out_dir,
        "type7_motion_large_rects",
        large_rects(args.hold_ms, args.black_ms),
        args.width,
        args.height,
    )
    write_pattern(
        args.out_dir,
        "type7_motion_fullscreen_colors",
        fullscreen_colors(args.hold_ms, args.black_ms),
        args.width,
        args.height,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
