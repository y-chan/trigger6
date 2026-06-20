# JUA365 / T6 Type7 Address Capture Handoff

Date: 2026-06-21 JST
Workspace: C:\Users\y-cha\Documents\trigger6
USBPcap interface used: \\.\USBPcap1
Display target: JUA365-side display intended at landscape 1920x1080, scaling 100%. In Windows screen query there were two 1920x1080 externals; DISPLAY18 was treated as likely JUA365, but manual visible-display confirmation is more reliable.

## Important capture-quality note
Initial automated browser captures were invalid/suspect because Edge was launched hidden and video was not visible on JUA365. Suspect files were renamed with `.bad_no_video_20260621_025353` etc. Use the non-suffixed recaptures for current analysis.

## Pattern captures

### grid recapture
Files:
- captures/win_type7_addr_grid_64x64.pcapng
- captures/win_type7_addr_grid_64x64.csv
- captures/win_type7_addr_grid_64x64_analysis.txt
- captures/win_type7_addr_grid_64x64_jpegs/

Summary:
- type7_rows=58, groups=58, interrupts=83
- Most type7 JPEGs are 448x64; one type7 is 1184x1080.
- Address pairs:
  - 0x250f710-0x2705f10 count=27 size=448x64
  - 0x2db9c90-0x2fb0490 count=23 size=448x64
  - 0x29649d0-0x2b5b1d0 count=7 size=448x64
  - 0x2daa9b0-0x2fa89b0 count=1 size=1184x1080
- JPEG extraction showed fullscreen-overlay content in the 1184x1080 image, so this pcap includes browser fullscreen UI contamination.
- Extracted type4 1920x1080 JPEGs are incomplete as standalone JPEG files: SOI exists, EOI 0xffd9 is absent. This may mean the packet/payload is incomplete for standalone decode or that reassembly across fragments/commands is needed.

### xscan recapture
Files:
- captures/win_type7_addr_xscan_64x64.pcapng
- captures/win_type7_addr_xscan_64x64.csv
- captures/win_type7_addr_xscan_64x64_analysis.txt
- captures/win_type7_addr_xscan_64x64_jpegs/

Summary:
- type7_rows=10, groups=10, interrupts=37
- All type7 rows are 192x1080, not 64x64.
- Single address pair: start=0x2500430 end=0x26fe430 span=0x1fe000
- cmd_dest min=0x34a5240 max=0x34b9520, monotonic; every delta is 0x23e0.
- cmd_dest delta matches align(payload)-32 for all consecutive type7 rows.
- This suggests black/white xscan updates dirty a vertical strip/full-height region rather than a 64x64 tile.

### yscan
Not recaptured. Based on grid/xscan behavior, it was considered low value for this pass.

## YouTube playback 2s capture
Files:
- captures/win_youtube_playback_2s.pcapng
- captures/win_youtube_playback_2s_summary.txt

Summary:
- Duration requested: 2s on \\.\USBPcap1 during YouTube playback.
- pcap size: about 21.7 MB, 2765 packets.
- t6_pcap_summary result: commands=337, video_payloads=0, interrupts=68.
- Traffic is dominated by large fragmented commands, usually totals around 0x8fbe0-0x917e0 with fragments at offsets 0x0, 0x19000, 0x32000, 0x4b000, 0x64000, 0x7d000.
- Existing parser did not classify these as video payloads, likely because this path needs fragmented command reassembly / different header handling before JPEG/video summary works.
- cmd_dest advances through payload ring and wraps to 0x03200000 multiple times.

## Current interpretation
- For simple black/white pattern updates, type7 does not necessarily correspond to the visible 64x64 rectangle. The observed dirty region can be much larger: grid produced 448x64 strips and xscan produced 192x1080 strips.
- Fullscreen/large black-white updates also trigger type4 1920x1080 commands, but extracted single-payload JPEGs lack EOI and are not standalone complete images with the current extraction method.
- YouTube playback uses much larger fragmented command transfers; next useful step is to improve parser reassembly for multi-fragment command totals before trying image extraction or address correlation on playback captures.

## Addendum: YouTube JPEG extraction caveat

For captures/win_youtube_playback_2s.pcapng, the existing tools did not detect extractable JPEG/video payloads:

```text
packets        2765
commands       337
video_payloads 0
interrupts     68
```

This does not prove there is no image/video data in the capture. The capture is dominated by large fragmented commands, so the current JPEG extractor (`tools/t6_extract_jpeg_payloads.py`) cannot extract images from it as-is. The likely next step is command fragment reassembly first, then header/JPEG detection on the reassembled payload.
