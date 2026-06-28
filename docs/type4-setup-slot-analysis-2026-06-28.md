# Type4 setup / Type7 slot timing analysis

対象 capture:

```text
captures/type7_motion_youtube_2s_fullsnap.pcap
```

目的:

`type=4, format=0x0d` setup が visible slot を切り替える packet なのか、単に active slot の内容を初期化する packet なのかを判定する。

生成物:

```text
captures/type7_motion_youtube_2s_fullsnap_setup_type7_timeline.csv
captures/type7_motion_youtube_2s_fullsnap_setup_type7_groups.csv
captures/type7_motion_youtube_2s_fullsnap_slot_summary.csv
captures/type7_motion_youtube_2s_fullsnap_min_group/
```

生成 script:

```text
tools/t6_setup_type7_groups.py
```

## Summary

`type=4, format=0x0d` setup は setup 専用の小さい metadata packet ではない。

この capture では、全 setup packet が `1920x1080` JPEG payload を持つ full-frame update である。
Header 上の canvas は `1920x1920` だが、JPEG SOF は `1920x1080` である。

したがって、type4 setup は少なくとも「選択 slot の内容を full JPEG で初期化/更新する packet」である。
また、その直後の Type7 は必ず同じ slot に書くため、type4 setup は後続 Type7 の active slot state も確立している。

pcap だけでは実画面の発光タイミングまでは確定できない。
ただし、type4 setup 自体が full-frame JPEG で ack も返るため、visible update が type4 setup 側で起きる可能性は高い。
Type7 はその同じ slot への差分追記と見るのが自然である。

## Counts

```text
video rows: 93
type4 setup rows: 52
Type7 rows: 41
interrupt rows: 93
setup groups: 52
```

Group distribution:

```text
setup only:      11 groups
setup + 1 Type7: 41 groups
setup + N Type7: 0 groups for N >= 2
```

Group direction:

```text
setup -> Type7
```

No `Type7... -> setup` group pattern was observed in this capture.
No other `type3/type4` video row appears inside setup-to-next-setup groups.

## Packet Fields

All type4 setup rows in this capture:

```text
video_type   = 4
image_format = 0x0d
mode         = 6
header size  = 0x30
canvas       = 1920x1920
JPEG SOF     = 1920x1080
slot order   = 2 -> 0 -> 1 -> 2 -> 0 -> 1 ...
```

All Type7 rows in this capture:

```text
video_type   = 7
image_format = 0x0d
mode         = 6
canvas       = 1920x1920
JPEG SOF     = 1376x800
```

The Type7 rows are not full-screen in this capture.
They are smaller JPEG updates against the same slot as the preceding setup.

## cmd_dest / length

For type4 setup:

```text
cmd_total_len varies by JPEG size.
observed setup cmd_total_len examples:
  666592
  648160
  647136
  630752
  586720

data_len = cmd_total_len - 48
```

For Type7:

```text
cmd_total_len also varies by JPEG size.
observed Type7 cmd_total_len examples:
  341984
  340960
  308192
  281568
  264160

data_len = cmd_total_len - 48
```

The `-48` relationship is consistent with a 32-byte bulk command plus a 0x30-byte video/JPEG header distinction:

```text
cmd_total_len = video_payload_len
video data_len = video_payload_len - 0x30
```

`cmd_dest` advances by the previous command's `cmd_total_len` until the ring wraps back to `0x03200000`.

Example:

```text
setup frame 209:
  cmd_dest      = 0x03524b60
  cmd_total_len = 648160

Type7 frame 229:
  cmd_dest      = 0x035c2f40
  delta         = 648160
```

## Slot Relationship

Every Type7 row belongs to the same slot as the immediately preceding setup group.

```text
groups with Type7: 41
same-slot Type7:   41
mismatches:        0
```

The setup slot order is strict rotation:

```text
2, 0, 1, 2, 0, 1, ...
```

This maps to the known base families:

```text
slot0: base0=0x02500430 base1=0x026fe430
slot1: base0=0x029556f0 base1=0x02b536f0
slot2: base0=0x02daa9b0 base1=0x02fa89b0
```

## Ack / Fence

Every setup and every Type7 row has an interrupt ack with `flags=0x04`, `event=0x04`, and `value == sequence`.

```text
setup ack missing: 0 / 52
Type7 ack missing: 0 / 41
```

Ack latency:

```text
setup:
  min 11.954 ms
  p50 13.270 ms
  max 16.979 ms

Type7:
  min 6.122 ms
  p50 8.461 ms
  max 20.779 ms
```

Same-slot setup reuse waits for the previous same-slot setup ack:

```text
setup same-slot reuse count: 49
reuse before previous setup ack: 0
```

However, Type7 after setup does not always wait for the setup ack.
In 18 of 41 immediate `setup -> Type7` groups, the Type7 packet was sent before the setup ack frame appeared.

Example:

```text
setup frame 209 seq=0x648 slot=1 ack_frame=233
Type7 frame 229 seq=0x649 slot=1
```

This suggests:

- The driver may pipeline Type7 after setup without waiting for setup ack.
- Slot reuse across later setup packets is more conservative and does wait for same-slot setup ack.

## Sequence Relationship

For all `setup -> Type7` groups, the Type7 sequence is exactly setup sequence + 1.

```text
setup seq 0x648 -> Type7 seq 0x649
setup seq 0x64a -> Type7 seq 0x64b
setup seq 0x64c -> Type7 seq 0x64d
...
```

There were no immediate setup-to-Type7 sequence gaps in this capture.

## Minimum Replay Group

The smallest observed group with Type7 contains exactly two video payloads:

```text
setup frame 209 seq=0x648 slot=1
Type7 frame 229 seq=0x649 slot=1
```

Extracted files:

```text
captures/type7_motion_youtube_2s_fullsnap_min_group/
  00_setup_frame209_seq00000648.bin
  01_type7_frame229_seq00000649.bin
  manifest.json
```

The manifest contains each row's command fields and payload bytes.

## Interpretation

The evidence does not support treating type4 setup as a metadata-only packet.
In this capture, type4 setup carries a full `1920x1080` JPEG and is acked independently.

The better model is:

```text
type4 setup:
  choose/prepare the rotating surface slot
  upload a full-frame JPEG into that slot
  establish the base0/base1/base2 state used by subsequent Type7

Type7:
  apply one smaller JPEG update to the same slot
```

Whether type4 setup itself is the exact visible flip point cannot be proven from pcap alone.
But since the payload is a full-frame JPEG and the following Type7 is smaller and same-slot, replay should start with `type4 setup -> Type7` groups, not Type7-only packets.
