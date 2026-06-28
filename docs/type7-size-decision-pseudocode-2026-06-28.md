# Type7 size decision pseudocode

Date: 2026-06-28 JST

Source: Ghidra analysis of `C:\Windows\System32\t6indisp.dll` version `1.9.25.423`.

This is a simplified reconstruction of the path that turns Windows dirty rects into Type7 update sizes.

## Rect flow

```c
void on_dirty_rect_list(surface, source_frame, stride, rects, rect_count) {
    for (rect in rects) {
        copy_rect_pixels_to_internal_surface(surface, source_frame, stride, rect);
        enqueue_dirty_rect(surface, rect);
    }
}
```

`enqueue_dirty_rect` clamps to the visible screen, merges overlapping existing dirty rects, then appends to a 16-entry ring.
If the ring is full, it unions the new rect into the previous ring entry.

```c
void enqueue_dirty_rect(surface, rect) {
    rect.left   = clamp(rect.left,   0, screen_width);
    rect.right  = clamp(rect.right,  0, screen_width);
    rect.top    = clamp(rect.top,    0, screen_height);
    rect.bottom = clamp(rect.bottom, 0, screen_height);

    if (rect.left >= rect.right || rect.top >= rect.bottom)
        return;

    if (merge_with_overlapping_ring_entry(surface->dirty_ring, rect))
        signal_dirty_worker();
    else
        append_or_union_last(surface->dirty_ring, rect);

    signal_dirty_worker();
}
```

## Worker sizing

```c
void dirty_worker(surface) {
    rect = pop_dirty_rect(surface->dirty_ring);

    if (force_fullscreen_pending)
        rect = full_screen_rect();

    format = choose_payload_format(surface);

    if (mode_forces_fullscreen)
        rect = full_screen_rect();

    rect = align_rect_32_and_min_32(rect, screen_width, screen_height);

    if (collapse_mode == 3) {
        if (horizontal_band_mode(surface)) {
            rect.left = 0;
            rect.right = screen_width;
        } else {
            rect.top = 0;
            rect.bottom = screen_height;
        }
    }

    force_fullscreen = false;

    if (format == 6 && surface->field_0x134 > 0x800 &&
        area(rect) > surface->field_0x148) {
        force_fullscreen = true;
    }

    if (area(rect) * 100 > screen_width * screen_height * 70 &&
        collapse_mode != 2) {
        force_fullscreen = true;
    }

    if (force_fullscreen)
        emit_type3_or_type4_fullscreen();
    else
        emit_type7(rect, format);
}
```

## Alignment helper

```c
rect align_rect_32_and_min_32(rect r, int screen_width, int screen_height) {
    r.left = floor32(max(r.left - 1, 0));
    r.right = min(ceil32(r.right + 31), screen_width);

    r.top = floor32(max(r.top - 1, 0));
    r.bottom = min(ceil32(r.bottom + 31), screen_height);

    if (r.right - r.left < 32) {
        if (r.left < 33)
            r.right += 32;
        else
            r.left -= 32;
    }

    if (r.bottom - r.top < 32) {
        if (r.top < 33)
            r.bottom += 32;
        else
            r.top -= 32;
    }

    return r;
}
```

## How this explains observed captures

```text
Small local update:
  dirty rect -> 32px alignment/min-size -> Type7 local tile
  Example: 64x64 remains plausible as 64x64.

Horizontal-band collapse:
  dirty rect top/bottom preserved, left=0, right=screen_width
  Example: 1920x56.

Vertical-band collapse:
  dirty rect left/right preserved, top=0, bottom=screen_height
  Example: 192x1080.

Large update:
  if area > 70% screen area, Type7 is abandoned for type 3/4 full-screen path,
  unless collapse_mode == 2.
```

## Key functions

```text
FUN_1800f7840  dirty rect list handler
FUN_1800f78c0  dirty rect list handler with even-coordinate rounding
FUN_180100ba8  clamp and enqueue dirty rect
FUN_1801009d0  merge into overlapping ring entries
FUN_18010092c  append rect to dirty ring
FUN_1800fce80  worker; align/collapse/fallback decision
FUN_1800fdf98  JPEG payload builder
FUN_1800fe814  alternate JPEG payload builder
FUN_1800ff1f0  queued update consumer
FUN_180101b74  Type7 command/header builder
```
