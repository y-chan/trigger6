# t6indisp.dll Ghidra analysis notes

Date: 2026-06-28 JST

Target:

- `C:\Windows\System32\t6indisp.dll`
- File version: `1.9.25.423`
- Description: `T6 Indirect Display Driver`
- MD5 as imported by Ghidra: `2e89de34510330262de165a509ae3d95`

Ghidra project:

- `ghidra_projects/t6indisp_analysis`
- Program: `/t6indisp.dll`
- Auto-analysis completed successfully.
- PDB was not found.

Generated artifacts:

- `ghidra_t6indisp_import.log`
- `ghidra_t6indisp_survey.log`
- `ghidra_t6indisp_decompile.log`
- `ghidra_out/*.c`
- `ghidra_scripts/T6WindowsSurvey.java`
- `ghidra_t6indisp_decompile2.log`

## High-level finding

The Type7 path is present in `t6indisp.dll`.

The strongest current candidates are:

| Address | Role hypothesis |
| --- | --- |
| `180101b74` | Builds the Type7 command/header and submits header + payload |
| `1800fdf98` | Builds payload for a rect, including JPEG format `0x0d` |
| `1800fe814` | Alternate rect payload builder, likely for a different source-buffer path |
| `1800fce80` | Dirty rect processing / expansion / full-screen fallback decision |
| `1800fc7d0` | Full-screen update path, likely Type4/whole-screen equivalent |
| `1800f734c` | Display-mode/default setup, including 1920x1080 fallback |
| `180011e80` | IPP JPEG encoder comment construction / bundled JPEG encoder code |
| `180100e38` | Ring allocator for Type7 payload area |
| `1801016c0` | Ring allocation commit / cursor advance |
| `1800f7fec` | Buffer submission helper, with transfer splitting |
| `1800ff1f0` | Worker/consumer that drains queued update records and calls Type7 submit |

## Strings and JPEG encoder

`T6WindowsSurvey.java` found the expected IPP JPEG string:

```text
18010c5e0: Intel(R) IPP JPEG encoder [%d.%d.%d] - %s
```

The xref goes to `FUN_180011e80`.
This function is inside bundled Intel UIC/IPP JPEG encoder code, not the T6-specific Type7 policy layer.
It formats the JPEG encoder comment and contains many vectorized encoder internals.

Other relevant strings/classes:

```text
Intel(R) UIC JPEG Decoder
Intel(R) UIC JPEG Encoder
.?AVJPEGEncoderParamsBAS@UIC@@
.?AVBaseImageEncoder@UIC@@
.?AVJPEGEncoder@UIC@@
.?AVCJPEGEncoder@@
```

## Type7 header construction

`FUN_180101b74` is the clearest Type7 header constructor.

Key evidence:

- It writes `7` at `param_3 + 0x20`.
- It writes image format `0x0d`.
- It writes rect width/height from `right-left` and `bottom-top`.
- It writes canvas width/height from `param_1 + 0x120` and `param_1 + 0x124`.
- It allocates a payload/ring region via `FUN_180100e38`.
- It increments a sequence counter at `lVar8 + 0xdb4`.
- It submits either a combined buffer or a 32-byte command followed by payload via `FUN_1800f7fec`.

Relevant decompiler locations:

- [ghidra_out/180101b74_FUN_180101b74.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180101b74_FUN_180101b74.c:45)
- [ghidra_out/180101b74_FUN_180101b74.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180101b74_FUN_180101b74.c:53)
- [ghidra_out/180101b74_FUN_180101b74.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180101b74_FUN_180101b74.c:92)
- [ghidra_out/180101b74_FUN_180101b74.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180101b74_FUN_180101b74.c:107)
- [ghidra_out/180101b74_FUN_180101b74.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180101b74_FUN_180101b74.c:113)

Approximate structure mapping from decompiler stores:

```text
param_3 + 0x00: command phase fields
param_3 + 0x20: Type7 video header begins
param_3 + 0x20: type = 7
param_3 + 0x24: data_length-like field from param_1+0x110
param_3 + 0x28: sequence_counter
param_3 + 0x30: width
param_3 + 0x32: height
param_3 + 0x34: canvas_width
param_3 + 0x36: canvas_height
param_3 + 0x38/0x3c/0x40: source plane offsets / address fields
param_3 + 0x44: image_format = 0x0d
param_3 + 0x4f: trailing flag byte
```

This differs slightly from the earlier capture-only header naming: fields at offsets `0x38..0x40` look like computed source-plane offsets in this function, while capture `start_address/end_address` may be populated earlier in the command phase or by the ring allocator result at `param_3[2]`.

## Payload length and 1024 alignment

For JPEG format `0x0d`, both `FUN_1800fdf98` and `FUN_1800fe814` call `FUN_180001700`, then clear padding and align the payload size.

Evidence:

- [ghidra_out/1800fdf98_FUN_1800fdf98.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fdf98_FUN_1800fdf98.c:214)
- [ghidra_out/1800fdf98_FUN_1800fdf98.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fdf98_FUN_1800fdf98.c:234)
- [ghidra_out/1800fdf98_FUN_1800fdf98.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fdf98_FUN_1800fdf98.c:235)
- [ghidra_out/1800fe814_FUN_1800fe814.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fe814_FUN_1800fe814.c:328)
- [ghidra_out/1800fe814_FUN_1800fe814.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fe814_FUN_1800fe814.c:332)
- [ghidra_out/1800fe814_FUN_1800fe814.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fe814_FUN_1800fe814.c:333)

The key expression is:

```c
aligned_len = jpeg_len + 0x84f & 0xfffffc00;
```

This is equivalent to aligning `jpeg_len + 0x450` up to 1024.
That matches the pcap-side observation that the ring cursor advances by an aligned payload size with a command/header adjustment.

## Dirty rect expansion and fallback

`FUN_1800fce80` contains repeated logic that rounds dirty rect edges to 32-pixel boundaries and enforces a minimum 32-pixel width/height.

Evidence:

- Left/top are rounded down with `& 0xffffffe0`.
- Right/bottom are rounded up with `+ 0x1f ... & 0xffffffe0`.
- If width or height is below `0x20`, the rect is expanded.

Representative locations:

- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:331)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:347)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:357)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:425)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:447)

It also has at least two fallback-to-full-screen conditions:

```text
if format/mode == 6 and width-like field > 0x800 and area > threshold at lVar6+0x148:
    force full-screen region

else if full_screen_area * 0x46 < dirty_area * 100:
    force full-screen region
```

The second condition is effectively a 70% area threshold because `0x46 == 70`.

Representative locations:

- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:381)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:389)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:487)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:495)

This supports the handoff hypothesis that Type7 size is not raw dirty rect, but driver-side aligned/expanded output.

## Dirty rect ring and merge policy

The dirty rect source consumed by `FUN_1800fce80` is a small 16-entry ring at:

```text
param_1+0x1c8: rect ring, 16 entries
entry size: 0x14
entry+0x00: left
entry+0x04: top
entry+0x08: right
entry+0x0c: bottom
entry+0x10: valid flag
param_1+0x308: read index
param_1+0x30c: write index
```

`FUN_18010092c` appends a rect to this ring.
If the ring is full, it unions the new rect into the previous entry instead of allocating a new entry.

Relevant locations:

- [ghidra_out/18010092c_FUN_18010092c.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/18010092c_FUN_18010092c.c:19)
- [ghidra_out/18010092c_FUN_18010092c.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/18010092c_FUN_18010092c.c:24)
- [ghidra_out/18010092c_FUN_18010092c.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/18010092c_FUN_18010092c.c:28)
- [ghidra_out/18010092c_FUN_18010092c.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/18010092c_FUN_18010092c.c:32)

`FUN_180100aec` pops the next valid rect from this ring.
It skips invalid or degenerate entries.

Relevant locations:

- [ghidra_out/180100aec_FUN_180100aec.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180100aec_FUN_180100aec.c:18)
- [ghidra_out/180100aec_FUN_180100aec.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180100aec_FUN_180100aec.c:25)
- [ghidra_out/180100aec_FUN_180100aec.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180100aec_FUN_180100aec.c:30)

`FUN_1801008b4` is an overlap-merge helper.
It unions two rects only if they overlap or touch according to this condition:

```c
if (a.bottom < b.top || b.bottom < a.top || a.right < b.left || b.right < a.left)
    return false;

a.left   = min(a.left, b.left);
a.top    = min(a.top, b.top);
a.right  = max(a.right, b.right);
a.bottom = max(a.bottom, b.bottom);
return true;
```

Relevant location:

- [ghidra_out/1801008b4_FUN_1801008b4.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1801008b4_FUN_1801008b4.c:16)

`FUN_1801009d0` scans backward through the dirty rect ring.
If the incoming rect overlaps an existing entry, it unions into that entry.
Then it also tries to merge additional older overlapping entries into that same accumulated rect and invalidates the merged entries.

Relevant locations:

- [ghidra_out/1801009d0_FUN_1801009d0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1801009d0_FUN_1801009d0.c:15)
- [ghidra_out/1801009d0_FUN_1801009d0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1801009d0_FUN_1801009d0.c:25)
- [ghidra_out/1801009d0_FUN_1801009d0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1801009d0_FUN_1801009d0.c:36)
- [ghidra_out/1801009d0_FUN_1801009d0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1801009d0_FUN_1801009d0.c:39)

This explains why multiple nearby dirty updates may collapse before Type7 sizing, while separated rects can remain as multiple Type7 records until the 16-entry ring fills.

## Dirty rect external entry

`FUN_180100ba8` is the immediate dirty-rect entry into the ring.
It clamps an input rect to the visible screen bounds, drops empty rects, then tries:

1. merge into an overlapping existing ring entry via `FUN_1801009d0`
2. append to the 16-entry ring via `FUN_18010092c`
3. signal the dirty worker event at `param_1+0x310`

Relevant locations:

- [ghidra_out/180100ba8_FUN_180100ba8.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180100ba8_FUN_180100ba8.c:29)
- [ghidra_out/180100ba8_FUN_180100ba8.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180100ba8_FUN_180100ba8.c:47)
- [ghidra_out/180100ba8_FUN_180100ba8.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180100ba8_FUN_180100ba8.c:68)
- [ghidra_out/180100ba8_FUN_180100ba8.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180100ba8_FUN_180100ba8.c:70)
- [ghidra_out/180100ba8_FUN_180100ba8.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180100ba8_FUN_180100ba8.c:79)

The two direct callers are:

- `FUN_1800f7840`: iterates an input dirty rect list as-is.
- `FUN_1800f78c0`: rounds each input rect coordinate down to an even value before insertion.

Both callers first copy the dirty region from the incoming source frame into the driver's internal surface, then enqueue the dirty rect.

Relevant locations:

- [ghidra_out/1800f7840_FUN_1800f7840.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800f7840_FUN_1800f7840.c:16)
- [ghidra_out/1800f7840_FUN_1800f7840.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800f7840_FUN_1800f7840.c:21)
- [ghidra_out/1800f7840_FUN_1800f7840.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800f7840_FUN_1800f7840.c:22)
- [ghidra_out/1800f78c0_FUN_1800f78c0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800f78c0_FUN_1800f78c0.c:24)
- [ghidra_out/1800f78c0_FUN_1800f78c0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800f78c0_FUN_1800f78c0.c:28)
- [ghidra_out/1800f78c0_FUN_1800f78c0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800f78c0_FUN_1800f78c0.c:34)
- [ghidra_out/1800f78c0_FUN_1800f78c0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800f78c0_FUN_1800f78c0.c:35)

The copy helpers:

- `FUN_1801018e8`: copies rect bytes from source frame to internal surface.
- `FUN_180101774`: copies into two planes/regions, consistent with a planar or subsampled mode.

Relevant locations:

- [ghidra_out/1801018e8_FUN_1801018e8.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1801018e8_FUN_1801018e8.c:20)
- [ghidra_out/1801018e8_FUN_1801018e8.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1801018e8_FUN_1801018e8.c:30)
- [ghidra_out/180101774_FUN_180101774.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180101774_FUN_180101774.c:24)
- [ghidra_out/180101774_FUN_180101774.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180101774_FUN_180101774.c:32)
- [ghidra_out/180101774_FUN_180101774.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180101774_FUN_180101774.c:38)

The current reconstructed pipeline is:

```text
OS / indirect-display dirty rect list
  -> FUN_1800f7840 or FUN_1800f78c0
  -> copy dirty pixels into internal surface
  -> FUN_180100ba8
  -> clamp to screen bounds
  -> overlap-merge into 16-entry dirty rect ring
  -> signal worker
  -> FUN_1800fce80 worker pops dirty rect
  -> choose source format / JPEG mode
  -> align rect to 32-pixel boundaries and enforce minimum size
  -> maybe force full-screen fallback
  -> FUN_1800fdf98 or FUN_1800fe814 builds payload
  -> write queued update record
  -> FUN_1800ff1f0 consumes record
  -> FUN_180101b74 builds Type7 command + video header
  -> FUN_1800f7fec submits buffer
```

## Dirty rect collapse modes

`FUN_1800fce80` uses two important internal selectors:

```text
iVar22: dirty rect handling / collapse mode, initially from lVar6+0xda8
iVar33: source/payload format selector, later written to record+0x38c
local_118: full-screen fallback flag
```

Observed `iVar33` meanings from consumers:

```text
0x0d: JPEG path, Type7 JPEG when record type remains 7
0x06: YUV-like planar/subsampled path
0x09: RGB-like or packed converted path
0x14: alternate converted path
0x04: uncompressed/packed path used by full-screen or non-JPEG cases
```

`iVar22` controls whether the rect remains local or collapses to a larger band:

```text
iVar22 == 0: normal partial update with area-threshold fallback
iVar22 == 1: force full-screen
iVar22 == 2: partial update, but skip the 70% area fallback
iVar22 == 3: collapse rect to a full-width or full-height band
```

For `iVar22 == 3`, the driver rewrites the aligned rect before payload generation.
Using the decompiler variable mapping:

```text
local_70  = { top, left }
uStack_68 = { bottom, right }
```

The horizontal-band form is:

```c
left = 0;
right = screen_width;
top/bottom preserved from the dirty rect;
```

Relevant locations:

- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:367)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:368)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:369)

The vertical-band form is:

```c
top = 0;
bottom = screen_height;
left/right preserved from the dirty rect;
```

Relevant locations:

- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:472)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:473)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:478)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:479)

This maps well to the pcap observations:

```text
1920x56  -> horizontal-band collapse
192x1080 -> vertical-band collapse
64x64    -> normal local partial update after 32px alignment/min-size handling
```

The driver chooses between horizontal and vertical collapse using `param_1+0x13c` mode bits.
From the visible branch, modes `0` and `2` favor the horizontal-band rewrite, while other modes can use the vertical-band rewrite.

## Full-screen fallback flag

`local_118` is the local flag that later causes the queued record type to become the full-screen type instead of Type7.

When `local_118 == 0`, the queued record type is usually `7`:

```c
record_type = 7;
```

When `local_118 != 0`, the code clears auxiliary placement fields and writes:

```c
record_type = 4 - (display_mode != 1);
```

That yields type `4` or `3` depending on `param_1+0xf2`.

Relevant locations:

- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:539)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:544)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:548)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:550)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:552)

The two main fallback tests are:

```c
if (iVar33 == 6 && field_0x134 > 0x800 &&
    dirty_area > *(lVar6 + 0x148))
    local_118 = 1;

if (screen_width * screen_height * 70 < dirty_area * 100 &&
    iVar22 != 2)
    local_118 = 1;
```

Relevant locations:

- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:381)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:389)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:487)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:495)

## Callback registration

`FUN_1800f4e10` registers the dirty-rect handlers through function pointers.
The normal dirty-rect-list handler is `FUN_1800f7840`.
For one mode, it also installs `FUN_1800f78c0`, which rounds each rect coordinate down to an even value before enqueue.

Relevant locations:

- [ghidra_out/1800f4e10_FUN_1800f4e10.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800f4e10_FUN_1800f4e10.c:40)
- [ghidra_out/1800f4e10_FUN_1800f4e10.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800f4e10_FUN_1800f4e10.c:42)
- [ghidra_out/1800f4e10_FUN_1800f4e10.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800f4e10_FUN_1800f4e10.c:72)
- [ghidra_out/1800f4e10_FUN_1800f4e10.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800f4e10_FUN_1800f4e10.c:74)

## Full-screen path

`FUN_1800fc7d0` appears to be the whole-screen update path.
It uses `param_1+0x134` and `param_1+0x138` as width/height-like fields, prepares a full-frame buffer, then writes metadata:

```text
param_1+0x370: type-ish field, 4 or 3 depending on param_1+0xf2
param_1+0x37c: width
param_1+0x380: height
param_1+0x384: ring offset / command destination candidate
param_1+0x388: payload length
param_1+0x38c: image format / source format selector
```

For JPEG mode, it sets `param_1+0x38c = 0x0d`.

Relevant locations:

- [ghidra_out/1800fc7d0_FUN_1800fc7d0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fc7d0_FUN_1800fc7d0.c:72)
- [ghidra_out/1800fc7d0_FUN_1800fc7d0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fc7d0_FUN_1800fc7d0.c:77)
- [ghidra_out/1800fc7d0_FUN_1800fc7d0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fc7d0_FUN_1800fc7d0.c:111)
- [ghidra_out/1800fc7d0_FUN_1800fc7d0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fc7d0_FUN_1800fc7d0.c:120)

## Display setup

`FUN_1800f734c` has a default 1920x1080 setup:

```text
param_1+0x3d8 = 0x780
param_1+0x3dc = 0x438
param_1+0x3e0 = 0x1fa400
```

`0x1fa400` equals `1920 * 1080`.

Relevant location:

- [ghidra_out/1800f734c_FUN_1800f734c.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800f734c_FUN_1800f734c.c:32)

## Current interpretation

The DLL confirms several pieces that were previously inferred from pcap:

1. Type7 is a real explicit path, with `type=7` and `image_format=0x0d`.
2. The JPEG encoder is Intel UIC/IPP-based and embedded in the DLL.
3. JPEG output is padded and aligned on 1024-byte boundaries.
4. Dirty rects are rounded to 32-pixel boundaries and expanded to at least 32 pixels.
5. Large dirty regions fall back to full-screen handling around a 70% area threshold.

The main unresolved point is the exact mapping of capture `start_address/end_address`.
`FUN_180101b74` computes several source-plane offsets, and also obtains a ring allocation result from `FUN_180100e38`.
The capture address pair likely comes from the command/ring allocation side, so the next step is to decompile:

- callers of `FUN_180101b74`

## Ring allocator and submit helper

`FUN_180100e38` allocates a region from a ring-like state object.
It receives a requested size and returns an offset through `param_3`.

Relevant locations:

- [ghidra_out/180100e38_FUN_180100e38.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180100e38_FUN_180100e38.c:18)
- [ghidra_out/180100e38_FUN_180100e38.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180100e38_FUN_180100e38.c:39)
- [ghidra_out/180100e38_FUN_180100e38.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180100e38_FUN_180100e38.c:44)
- [ghidra_out/180100e38_FUN_180100e38.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180100e38_FUN_180100e38.c:53)

`FUN_1801016c0` commits the allocation and advances the cursor:

```c
next = param_3 + 0x1f + param_2 & 0xffffffe0;
```

This is 32-byte alignment at the allocator/cursor layer.

Relevant location:

- [ghidra_out/1801016c0_FUN_1801016c0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1801016c0_FUN_1801016c0.c:13)

`FUN_1800f7fec` is a lower-level submit helper.
For one mode it splits large transfers into chunks:

```c
chunk_count = (param_4 + 0xffbff) / 0xffc00;
max_chunk = 0xffc00;
```

If `param_1+0x110 != 0`, it submits as a single chunk.

Relevant locations:

- [ghidra_out/1800f7fec_FUN_1800f7fec.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800f7fec_FUN_1800f7fec.c:38)
- [ghidra_out/1800f7fec_FUN_1800f7fec.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800f7fec_FUN_1800f7fec.c:48)
- [ghidra_out/1800f7fec_FUN_1800f7fec.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800f7fec_FUN_1800f7fec.c:64)
- [ghidra_out/1800f7fec_FUN_1800f7fec.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800f7fec_FUN_1800f7fec.c:110)

Current interpretation:

- The 1024-byte alignment seen in pcap comes from JPEG payload sizing in `FUN_1800fdf98` / `FUN_1800fe814`.
- The ring allocator itself commits offsets with 32-byte alignment.
- The capture `cmd_dest` behavior is probably a composition of the Type7 command phase, the ring allocation result, and the padded payload size.
- The next missing link is the caller of `FUN_180101b74`, where the command phase fields before `param_3+0x20` are filled.

## Type7 queued-record consumer

`FUN_1800ff1f0` is the direct caller of `FUN_180101b74`.
It drains queued update records from two slots:

```text
slot A base = param_1 + 0x150
slot B base = param_1 + 0x6c0
```

Because the record fields are relative to those slot bases, the same layout appears at:

```text
slot A: param_1 + 0x4c0 .. 0x500
slot B: param_1 + 0xa30 .. 0xa70
```

For Type7 JPEG:

```text
record+0x370: packet/update type, 7 for Type7
record+0x374: left
record+0x378: top
record+0x37c: right
record+0x380: bottom
record+0x384: payload buffer offset
record+0x388: payload buffer length
record+0x38c: image format, 0x0d for JPEG
```

The consumer copies these fields into locals and calls:

```c
FUN_180101b74(slot_base, &rect, payload_buffer_base + payload_offset, payload_len, flags);
```

Relevant locations:

- [ghidra_out/1800ff1f0_FUN_1800ff1f0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800ff1f0_FUN_1800ff1f0.c:68)
- [ghidra_out/1800ff1f0_FUN_1800ff1f0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800ff1f0_FUN_1800ff1f0.c:96)
- [ghidra_out/1800ff1f0_FUN_1800ff1f0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800ff1f0_FUN_1800ff1f0.c:110)
- [ghidra_out/1800ff1f0_FUN_1800ff1f0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800ff1f0_FUN_1800ff1f0.c:151)
- [ghidra_out/1800ff1f0_FUN_1800ff1f0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800ff1f0_FUN_1800ff1f0.c:157)
- [ghidra_out/1800ff1f0_FUN_1800ff1f0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800ff1f0_FUN_1800ff1f0.c:247)
- [ghidra_out/1800ff1f0_FUN_1800ff1f0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800ff1f0_FUN_1800ff1f0.c:309)

The whole-screen path producers `FUN_1800fc7d0` and `FUN_1800fcb54` write the same record layout with type `3` or `4`, full-screen rect, payload offset, payload length, and format selector.

Relevant locations:

- [ghidra_out/1800fc7d0_FUN_1800fc7d0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fc7d0_FUN_1800fc7d0.c:111)
- [ghidra_out/1800fcb54_FUN_1800fcb54.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fcb54_FUN_1800fcb54.c:111)

For dirty-rect Type7, `FUN_1800fce80` is the producer.
After payload generation succeeds, it writes the queued record:

```text
record+0x370: type, usually 7 unless full-screen fallback forces 3/4
record+0x374: left/top packed as two u32 values
record+0x37c: right/bottom packed as two u32 values
record+0x384: payload ring offset/current cursor
record+0x388: payload length
record+0x38c: image format/source format
record+0x390..0x3b0: auxiliary rect/source metadata
```

Then it releases the lock and signals the worker event.

Relevant locations:

- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:531)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:538)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:552)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:554)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:556)
- [ghidra_out/1800fce80_FUN_1800fce80.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800fce80_FUN_1800fce80.c:658)

## Type7 command phase fields

Inside `FUN_180101b74`, the first 32 bytes before the Type7 video header are initialized after ring allocation:

```text
param_3 + 0x00: 0
param_3 + 0x04: param_4 - 0x20
param_3 + 0x08: ring allocation offset from FUN_180100e38
param_3 + 0x0c: param_4 - 0x20
param_3 + 0x10..0x1f: zero
param_3 + 0x20: Type7 video header starts
```

This gives a concrete explanation for the pcap-side `cmd_dest` interpretation:

- `cmd_dest` corresponds to the ring allocation offset stored at command offset `0x08`.
- The value is allocated by `FUN_180100e38`.
- After successful submit, `FUN_1801016c0` commits the allocation and advances the allocator cursor.

Relevant locations:

- [ghidra_out/180101b74_FUN_180101b74.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180101b74_FUN_180101b74.c:92)
- [ghidra_out/180101b74_FUN_180101b74.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180101b74_FUN_180101b74.c:107)
- [ghidra_out/180101b74_FUN_180101b74.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180101b74_FUN_180101b74.c:114)
- [ghidra_out/180101b74_FUN_180101b74.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180101b74_FUN_180101b74.c:138)

## Implementation blockers only

The remaining unknowns that can block a robust Type7 implementation are:

- How to generate the Type7 address fields at header offsets `0x38`, `0x3c`, and `0x40` for a self-generated update.
- Whether the device accepts a single synthetic Type7 tile, or whether it expects the Windows-like grouped sequence produced by the dirty-rect worker.
- The minimum ack/fence policy needed before reusing or advancing payload-ring space aggressively.
- The payload-ring initial cursor, wrap limit, and out-of-space behavior for long-running synthetic streams.

The dirty-rect sizing question is now less blocking than before.
`FUN_1800fce80` explains the important transforms: 32-byte alignment, minimum `32x32`, mode `3` collapse to full-width/full-height bands, and fallback to type `3/4` for large updates.

## Type7 address fields

The previous pcap-side names `start_address` and `end_address` are probably too narrow.
In `FUN_180101b74`, Type7 header offset `0x38`, `0x3c`, and `0x40` are generated as up to three source-plane addresses or offsets.

Header mapping from the decompiled stores:

```text
0x20: type = 7
+0x24: data_length = payload length minus 0x50
+0x28: sequence counter
+0x2c: source mode from context+0x110
+0x30: update width
+0x32: update height
+0x34: canvas width from context+0x120
+0x36: canvas height from context+0x124
+0x38: plane 0 address/offset
+0x3c: plane 1 address/offset, or 0
+0x40: plane 2 address/offset, or 0
+0x44: image format = 0x0d
+0x4f: caller-supplied flags byte
```

For mode `0`, `8`, and `9`, only the first address field is used:

```text
mode 0: plane0 = base0 + canvas_width * top + left * 2
mode 8: plane0 = base0 + canvas_width * top + left * 4
mode 9: plane0 = base0 + canvas_width * top + left * 3
```

For planar or multi-plane modes, the second and third fields are active:

```text
mode 4:
  plane0 = base0 + canvas_width * top + left
  plane1 = base1 + (top / 2) * canvas_height + (left / 2) * 2
  plane2 = base2 + (top / 2) * canvas_height + (left / 2) * 2

mode 6:
  plane0 = base0 + canvas_width * top + left
  plane1 = base1 + (top / 2) * canvas_height + (left / 2) * 2
  plane2 = 0

mode 0x0b:
  plane0 = base0 + canvas_width * top + left
  plane1 = base1 + canvas_height * top + left
  plane2 = base2 + canvas_height * top + left
```

The base fields are read from context offsets `0x114`, `0x118`, and `0x11c`.
The width and height stride fields are read from `0x120` and `0x124`.

The address-range initializer found so far is `FUN_180100fbc`.
It seeds two address windows using constants `0x2500000` and `0x2500400`, and computes internal third-split cursors inside each window.
This matches the pcap observation that representative address pairs are near `0x2500000`.

Relevant locations:

- [ghidra_out/180101b74_FUN_180101b74.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180101b74_FUN_180101b74.c:45)
- [ghidra_out/180101b74_FUN_180101b74.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180101b74_FUN_180101b74.c:46)
- [ghidra_out/180101b74_FUN_180101b74.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180101b74_FUN_180101b74.c:49)
- [ghidra_out/180101b74_FUN_180101b74.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180101b74_FUN_180101b74.c:62)
- [ghidra_out/180101b74_FUN_180101b74.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180101b74_FUN_180101b74.c:68)
- [ghidra_out/180101b74_FUN_180101b74.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180101b74_FUN_180101b74.c:79)
- [ghidra_out/180100fbc_FUN_180100fbc.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180100fbc_FUN_180100fbc.c:15)
- [ghidra_out/180100fbc_FUN_180100fbc.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180100fbc_FUN_180100fbc.c:21)
- [ghidra_out/1801033a4_FUN_1801033a4.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1801033a4_FUN_1801033a4.c:34)
- [ghidra_out/1801033a4_FUN_1801033a4.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1801033a4_FUN_1801033a4.c:84)

Current implementation implication:

- For replay, keep using captured address fields first.
- For synthetic generation, treat `0x38/0x3c/0x40` as mode-dependent plane addresses, not as a simple `start/end` pair.
- The safest first synthetic model is `mode 0` or the mode observed in the target capture, with `plane0 = captured_base + canvas_width * top + bytes_per_pixel * left` and the unused plane fields zeroed only if the captured mode also does that.

Additional pcap/parser check:

- `tools/t6_pcap_summary.py` reads Type7 `start_addr` and `end_addr` from video-header offsets `0x18` and `0x1c`, which correspond to command-buffer offsets `0x38` and `0x3c` in `FUN_180101b74`.
- The observed address fields are therefore the same bytes as Ghidra's `param_3[0xe]` and `param_3[0xf]`.
- The parser names are historical; byte alignment is correct, but the semantic names should become `plane0_addr` and `plane1_addr` for Type7 mode `6`.

The checked captures all use source mode `6`:

```text
win_type7_addr_xscan_64x64.pcapng:          mode 6, 10 Type7 rows
win_type7_addr_yscan_64x64.pcapng:          mode 6, 248 Type7 rows
type7_motion_horizontal_bands.pcapng:       mode 6, 4 Type7 rows
type7_motion_vertical_bands.pcapng:         mode 6, 2 Type7 rows
type7_motion_large_rects.pcapng:            mode 6, 11 Type7 rows
```

For those captures, the active formula is:

```text
plane0 = base0 + canvas_width * top + left
plane1 = base1 + (top / 2) * canvas_height + (left / 2) * 2
plane2 = 0
```

With the observed `1920x1920` canvas and even-aligned `left`, this implies:

```text
plane1 - plane0 = (base1 - base0) - 960 * top
```

Therefore the pcap-side `span = end_addr - start_addr` is not the byte length of the updated tile.
It is the distance between two plane addresses after the luma/chroma-style addressing formula is applied.
This explains why the same `0x1fe000` span can appear with different tile sizes, and why `0x10e000` / `0x1ef000` spans do not match JPEG payload size or visible rectangle area.

Representative observed mode-6 pairs:

```text
0x02500430-0x026fe430  span=0x1fe000
0x026e0430-0x027ee430  span=0x10e000
0x029556f0-0x02b536f0  span=0x1fe000
0x02b356f0-0x02c436f0  span=0x10e000
0x02f8a9b0-0x030989b0  span=0x10e000
```

Working conclusion:

- Type7 `start_addr/end_addr` are better understood as `plane0_addr/plane1_addr`.
- The `0x2500000` neighborhood is a surface/plane window base, not a tile-local payload address.
- The field at Type7 `+0x40` is `plane2_addr`, but it is zero for the observed mode-6 captures.
- A synthetic Type7 implementation should first reproduce mode `6` exactly, including `plane2=0`, `canvas=1920x1920`, and captured or correctly initialized `base0/base1`.

Next base-address trace:

The remaining binary-side target is to find where these context fields are written:

```text
context+0x114: base0 used for plane0_addr
context+0x118: base1 used for plane1_addr
context+0x11c: base2 used for plane2_addr
context+0x120: canvas width / plane0 stride input
context+0x124: canvas height / plane1 stride input
```

Because `FUN_180101b74` receives `slot_base`, these same fields can appear as parent-object offsets:

```text
slot A base = parent+0x150:
  parent+0x264, +0x268, +0x26c, +0x270, +0x274

slot B base = parent+0x6c0:
  parent+0x7d4, +0x7d8, +0x7dc, +0x7e0, +0x7e4
```

Added helper:

- [ghidra_scripts/T6AddressFieldRefs.java](/C:/Users/y-cha/Documents/trigger6/ghidra_scripts/T6AddressFieldRefs.java)

Run it with:

```powershell
& 'C:\Users\y-cha\Downloads\ghidra_12.1.2_PUBLIC\support\analyzeHeadless.bat' `
  'C:\Users\y-cha\Documents\trigger6\ghidra_projects' `
  t6indisp_analysis `
  -process t6indisp.dll `
  -scriptPath 'C:\Users\y-cha\Documents\trigger6\ghidra_scripts' `
  -postScript T6AddressFieldRefs.java `
  -noanalysis
```

Expected useful hits:

- stores into parent offsets `0x264/0x268/0x26c` or `0x7d4/0x7d8/0x7dc`;
- stores or copies involving `0x2500000`, `0x2500400`, and the split cursors made by `FUN_180100fbc`;
- any call path that copies the temporary address-window table from `FUN_1801033a4` into each slot.

## Type7 base-address setup

`T6AddressFieldRefs.java` found the concrete writers for `context+0x110..0x124`.
The main functions are:

- `FUN_180102638`: JPEG/surface setup for image format `0x0d`.
- `FUN_180102d98`: setup for formats `4`, `6`, and `0x0b`.
- `FUN_180102398`: setup for format `0x14`.
- `FUN_180102a88`: setup for format `9`.

`FUN_1800ff1f0` calls these setup functions when queued record type is `3` or `4`.
It later calls the Type7 emitters when queued record type is `7`.
Therefore `context+0x114/+0x118/+0x11c/+0x120/+0x124` are not arbitrary globals.
They are the active surface/plane layout saved by the most recent setup command for that slot.

For observed Type7 JPEG captures, the relevant setup is `FUN_180102638`.
It first allocates a base surface address with `FUN_180100da0`, then stores:

```text
context+0x114 = param_2[0x0d]  // base0
context+0x118 = param_2[0x0e]  // base1
context+0x11c = param_2[0x0f]  // base2 or 0
context+0x110 = param_2[0x0b]  // source mode
context+0x120 = header canvas/stride width
context+0x124 = header canvas/stride height
```

For the observed mode-6 path inside `FUN_180102638`:

```text
tile_blocks = ceil(width / 16) * ceil(height / 16)
plane_span = tile_blocks * 0x100

base0 = allocated_base + 0x30
base1 = base0 + plane_span
base2 = 0
mode  = 6
canvas_width  = align16(width)
canvas_height = align16(width)   // as decompiled; observed Type7 later has 1920x1920
```

This directly explains the common capture pair:

```text
width=1920 height=1080
ceil(1920 / 16) = 120
ceil(1080 / 16) = 68
plane_span = 120 * 68 * 0x100 = 0x1fe000

allocated_base = 0x2500400
base0 = 0x2500400 + 0x30 = 0x2500430
base1 = 0x2500430 + 0x1fe000 = 0x26fe430
```

This matches:

```text
0x02500430-0x026fe430
```

So the old pcap-side `span=end_addr-start_addr` is usually `plane_span` only when `top=0` and `left=0`.
For later Type7 partial updates, `FUN_180101b74` adds the dirty-rect offset to both planes:

```text
plane0_addr = base0 + canvas_width * top + left
plane1_addr = base1 + (top / 2) * canvas_height + (left / 2) * 2
```

With `canvas_width=canvas_height=1920`, the pair span becomes:

```text
plane1_addr - plane0_addr = plane_span - 960 * top
```

This explains why observed spans include `0x1fe000`, `0x1ef000`, and `0x10e000` without matching JPEG payload length or visible tile area.

Allocator detail:

`FUN_180100da0` chooses one of three surface slots per display/session.
It returns the free slot's base address through `param_3`.
`FUN_18010166c` marks that slot active and updates the rotating index.
The initial address windows are seeded by `FUN_180100fbc`, including `0x2500000` and `0x2500400`.

Relevant locations:

- [ghidra_out/1800ff1f0_FUN_1800ff1f0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800ff1f0_FUN_1800ff1f0.c:137)
- [ghidra_out/1800ff1f0_FUN_1800ff1f0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/1800ff1f0_FUN_1800ff1f0.c:151)
- [ghidra_out/180102638_FUN_180102638.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180102638_FUN_180102638.c:35)
- [ghidra_out/180102638_FUN_180102638.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180102638_FUN_180102638.c:73)
- [ghidra_out/180102638_FUN_180102638.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180102638_FUN_180102638.c:91)
- [ghidra_out/180102638_FUN_180102638.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180102638_FUN_180102638.c:130)
- [ghidra_out/180102638_FUN_180102638.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180102638_FUN_180102638.c:149)
- [ghidra_out/180100da0_FUN_180100da0.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/180100da0_FUN_180100da0.c:6)
- [ghidra_out/18010166c_FUN_18010166c.c](/C:/Users/y-cha/Documents/trigger6/ghidra_out/18010166c_FUN_18010166c.c:6)

Current implementation rule for mode-6 Type7:

1. Establish or replay the setup command that creates `base0/base1/base2/mode/canvas`.
2. For Type7 JPEG updates, compute `plane0_addr` and `plane1_addr` from the dirty rect using the mode-6 formula.
3. Keep `plane2_addr=0`.
4. Do not treat `plane1_addr` as an end pointer for the JPEG payload.

## Low-priority unknowns

The Type7 address fields are now understood well enough to implement the mode-6 JPEG path.
The remaining unknowns below should be treated as lower-priority validation items, not as blockers for writing code.

- Whether the device accepts a single synthetic Type7 tile after ordinary setup, or whether Windows-like grouped replay is required for reliable display.
- Whether the setup command must be replayed exactly, or whether reproducing the resulting `base0/base1/base2/mode/canvas` state is enough.
- Whether the three-surface rotation from `FUN_180100da0` and `FUN_18010166c` must match Windows exactly for long-running streams.
- How aggressively the implementation may advance or reuse payload-ring space without waiting for interrupt ack/fence.
- Whether non-JPEG Type7 formats `6`, `9`, `0x14`, and `0x0b` are useful for the Mac/Linux implementation.
- Whether mode-6 Type7 works for resolutions other than the observed `1920x1080` with `1920x1920` canvas without additional setup quirks.

Implementation should proceed in this order:

1. Replay capture-derived Type7 groups unchanged.
2. Replay or reproduce the setup command that establishes mode-6 surface state.
3. Generate mode-6 Type7 JPEG using `plane0_addr/plane1_addr` formulas.
4. Only then optimize dirty-rect grouping, surface rotation, and ack pacing.
