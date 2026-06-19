# Linux driver gap 差分分析

現 Linux 実装と、Mac DEXT / Mac userspace driver / `captures/mctt6.pcapng` から見えた実 protocol の差分メモ。

プロジェクト全体の最終ゴールは Mac / Windows / Linux を含む全 platform 対応の T6 display stack。Mac は回転対応の必要性が高いため最初の実装ターゲットになりやすいが、Linux DRM/KMS driver も最終的には同じ protocol core の利用先として扱う。

## 影響の大きい差分

### 1. Bulk command phase の offset 20 は output index ではなさそう

現状:

```c
struct trigger6_session {
	__le32 session_number;
	__le32 payload_length;
	__le32 dest_addr;
	__le32 fragment_length;
	__le32 offset;
	__le32 output_index;
	__le32 unk7;
	__le32 unk8;
};
```

Mac DEXT と既存 pcap の両方から、offset `0x14` は少なくとも video bulk transfer では `more_fragments` flag と見るのが自然。

`captures/mctt6.pcapng` では、large payload が次のように分割されている。

```text
session=0 total=0x4f7e0 frag_len=0x19000 frag_off=0x0     more=1
session=0 total=0x4f7e0 frag_len=0x19000 frag_off=0x19000 more=1
session=0 total=0x4f7e0 frag_len=0x19000 frag_off=0x32000 more=1
session=0 total=0x4f7e0 frag_len=0x47e0  frag_off=0x4b000 more=0
```

現 Linux 実装は常に `output_index = 0` を書いているため、multi-fragment payload では最後以外も `more=0` になってしまう。

修正候補:

- `trigger6_session` を `trigger6_bulk_command` のような名前に変える。
- offset `0x14` を `u8 more_fragments` として扱う。
- 残り 3 byte + trailing 8 byte は reserved として 0 にする。

### 2. 現在の image upload は公式経路と違う

現状:

- `type = 0x3`
- `format = BGR24`
- `dest_addr = 0x30`
- `data_length = header + raw BGR payload`
- full frame を毎回送る

既存 pcap:

- `type = 0x4`, `format = 0x0d`, JPEG
- `data_length = payload length - 0x30`
- JPEG SOI は payload offset `0x30`
- command phase `dest` は `0x03000000` から始まり、VRAM/payload allocation のように増えていく
- `type = 0x7`, `format = 0` も出る。flip command または別種の compressed update に見える。

Mac userspace driver:

- `t6_submit_frame_surface_with_compressed_dirty_rects`
- `t6_submit_frame_surface_whole_screen_compressed`
- `t6_submit_frame_surface_compressed_flip`
- `t6_compress_and_upload`
- `reservePayloadVramAndFenceIdAsync`
- `MCTT6Device JPEG Encoder`
- `t6_upload_uncompressed_yuv420`
- `t6_upload_uncompressed_yuv444`

このため、raw BGR24 全画面送信だけでは実機で安定表示できない可能性が高い。少なくとも既存 capture と Mac 公式経路は JPEG/YUV/VRAM/fence を中心にしている。

### 3. VRAM allocator / fence / interrupt が未実装

既存 pcap では interrupt IN packet が次の形で出ている。

```text
flags=0x04 value=0x00000001 event=0x04
flags=0x04 value=0x00000002 event=0x04
...
```

Mac DEXT は 64 byte interrupt packet の:

- `packet[0]` bit 2 を video interrupt として見る
- `packet[0x0c..0x0f]` を 32-bit 値として user-space video client に渡す
- `packet[0x13]` を 1 byte 値として user-space video client に渡す

Mac user-space driver には以下がある。

- `FENCE_ID`
- `JPEG_ERROR`
- `MCTT6Device::handleVideoInterrupt: JPEG decoder error! (fence ID %08x)`
- `t6_fence_id_invalidate_until`
- `reservePayloadVramAndFenceIdAsync`

よって `packet[0x0c]` の連番は fence ID の可能性が高い。Linux 側で JPEG/VRAM 経路を実装するなら、interrupt IN を読み、fence ID を待つ仕組みが必要になる。

### 4. Software ready request が未実装

Mac DEXT は start 時に `0x31` を送る。

```text
display ready: bmRequestType=0x40 bRequest=0x31 wValue=0 wIndex=0 wLength=0
audio ready:   bmRequestType=0x40 bRequest=0x31 wValue=3 wIndex=0 wLength=0
```

既存 pcap にも `0x40,0x31` が出ている。Linux probe または enable sequence で display ready は送るべき候補。

### 5. `trigger6_transfer.c` は現状 build 対象外かつ古い経路に見える

`Makefile` は `trigger6_transfer.o` を含めていない。

さらに `trigger6_transfer.c` は以下の未定義/不整合がある。

- `struct trigger6_urb` が header に無い。
- `struct trigger6_frame_update_header` が header に無い。
- `trigger6_xrgb_to_yuv422_line()` が見当たらない。
- `usb_sndbulkpipe(usb_dev, 4)` を使っているが、実 capture の bulk OUT endpoint は `0x02`。
- T5/T2 風の YUV422 rect update っぽい構造で、今見えている T6 JPEG/VRAM 経路とは別物に見える。

現時点では `trigger6_transfer.c` を復活させるより、T6 capture に合わせた bulk command / video payload / fence 実装を別途整理したほうがよい。

## 中程度の影響がある問題

### EDID read error handling

`trigger6_read_edid()` は `usb_control_msg()` の戻り値を見ずに常に 0 を返している。

```c
ret = usb_control_msg(...);
return 0;
```

修正候補:

- `ret < 0` なら `ret` を返す。
- short read なら `-EIO` などにする。
- debug log は削るか `drm_dbg_kms()` に落とす。

### Mode table read validation

`trigger6_read_modes()` は戻り値をそのまま返しているが、probe 側は戻り値を見ていない。

修正候補:

- `0x89` read の戻り length を検証する。
- 960 bytes / 30 entries 固定でよいか、`0x84` / `0x85` / `0x89` の使い分けを再確認する。

### Endpoint constants

`usb_sndbulkpipe()` には endpoint address ではなく endpoint number を渡すので、bulk OUT `2` はよい。

ただし `TRIGGER6_ENDPOINT_INTERRUPT_IN` は `0x3` と定義されている。実 capture では interrupt IN address は `0x83` だが、Linux の interrupt pipe 生成時は endpoint number `3` を使うはずなので、命名を `*_EP_NUM` にしたほうが誤解が少ない。

### Connector output index

現コードは output index 0 前提。JUA365 が2出力なら、output index 1 の connector / EDID / status / mode table を扱う設計が必要。

## 実装順序の候補

最短で「表示が出る可能性」を上げるなら、いきなり高性能 partial update ではなく、次の順が現実的。

1. Probe/init sequence を capture に寄せる。
   - `0x31` display ready
   - `0x88` VRAM size
   - connector status / EDID / timing table
2. interrupt IN read loop を入れる。
   - 64 byte packet を読む。
   - video event `flags & 0x04` を log する。
   - `value/event` を fence 候補として記録する。
3. `trigger6_bulk_command` を正しく送る。
   - session
   - total length
   - VRAM destination
   - fragment length
   - fragment offset
   - more fragments
4. まず capture と同じ JPEG payload 形を再現する。
   - kernel 内 JPEG encoder は現実的ではないので、最初は userspace helper か debugfs/test path が必要。
   - または uncompressed YUV420/YUV444 経路を探す。
5. fence interrupt を待ってから flip command/type `0x7` 相当を送る。
6. DRM fb update と dirty rect を後でつなぐ。

## capture または実機検証がまだ必要な点

ローカル静的解析だけでは、以下は確定しきれない。

- JUA365 実機が Windows で使う payload format。
- type `0x7` の完全な構造。
- JPEG payload header の各 field の正式名称。
- VRAM allocation range と wrap 条件。
- output index が bulk command phase に入るのか、control/VRAM state で暗黙に決まるのか。
- `0x31` が必須か、互換性用か。
- raw BGR24/type `0x3` が JUA365 firmware で受理されるか。

## macOS 移植に使える Ubuntu vendor driver の知見

Ubuntu 公式ドライバの `evdi_t6` userspace 実装から、Mac 向け Rust
prototype に移植できる実装詳細がかなり増えた。

### VRAM layout / address 割り当て

`extracted/ubuntu_T6_260223/evdi_t6/evdi_t6_1/main.c` の `usb_process()`
には、VRAM layout のコメントと実装がある。

基本方針:

- `0x88` で video RAM size を MB 単位で読む。
- `cmdAddr` は JPEG command / payload 領域。
- `fbAddr1..3` は triple buffer 用 framebuffer 領域。
- 1 port / 2 port、4K / 1080p、JPEG / YV12 で領域を変える。
- 実際の送信では `fbAddr1 -> fbAddr2 -> fbAddr3 -> fbAddr1` と回す。

1 port 1080p の代表例:

```text
0x0000000  cmdAddr 1080p/JPEG
0x2e00000  fbAddr1 4MB
0x3200000  fbAddr2 4MB
0x3600000  fbAddr3 4MB
```

1 port 4K30 の代表例:

```text
0x0000000  cmdAddr 4K/NV12 or YV12
(ramsize - 36)MB  fbAddr1
(ramsize - 24)MB  fbAddr2
(ramsize - 12)MB  fbAddr3
```

2 port device では、4K 対応 port と 1080p port に別々の command / frame
領域を割り当てる。4K 対応 port 判定は display capability の DVO bit
`0x02` を見ている。

### `cmdAddr` の ring 動作

JPEG 系では frame ごとに `cmdAddr` を進める。

Ubuntu 実装:

- `len = jpeg_size + 1024 + 48`
- `len < 1MB` なら `cmdoffset = 1MB`
- `len < 2MB` なら `cmdoffset = 2MB`
- それ以上は通常 `3MB`
- `USE4KJPEG` の 4K JPEG path では最大 `4MB`
- `cmdAddr + cmdoffset > fbAddr1` なら `cmdAddr = initial_cmdAddr`

`VIDEO_FLIP_HEADER.Flag` の reset flag:

- 起動直後の数 frame は `0x80`。
- `cmdAddr` wrap 時にも path によって `0x80` または `0x00`。
- `0x80` は `FLAG_RESET_JPEG` と定義されている。

### frame upload path

公式 Ubuntu 実装には少なくとも次の upload path がある。

- `t6_libusb_FilpJpegFrame()`
  - bulk header `PayloadAddress = cmdAddr`
  - `VIDEO_FLIP_HEADER.SourceFormat = VIDEO_COLOR_JPEG`
  - `VIDEO_FLIP_HEADER.TargetFormat = VIDEO_COLOR_NV12`
  - JPEG data は 48 byte video header 直後
  - payload length は `jpeg_size + 48 + 1024`
- `t6_libusb_FilpYV12Frame()`
  - bulk header `PayloadAddress = fbAddr`
  - `SourceFormat = TargetFormat = VIDEO_COLOR_YV12`
  - payload buffer 先頭に 48 byte header 用 reserve が必要
- `t6_libusb_FilpNV12Frame()`
  - bulk header `PayloadAddress = fbAddr`
  - `SourceFormat = TargetFormat = VIDEO_COLOR_NV12`
- `t6_libusb_Rgb24_full_block()`
  - RGB24 full frame path。実運用主経路ではなさそうだが test path として存在。

通常 build では、おおむね:

- non-4K: JPEG
- 4K30: YV12

`USE4KJPEG` build では、おおむね:

- 4K30 or USB2: JPEG
- USB3 1080p: YV12

つまり送信方式は firmware capability / USB speed / build-time strategy に依存する。

### Ubuntu driver 由来の初期化順序

Ubuntu userspace path から見える基本 sequence:

1. USB speed を確認。
2. `0x88` `VENDOR_REQ_QUERY_VIDEO_RAM_SIZE` で RAM size を読む。
3. display section / capability を読む。
4. connector status を読む。
5. `0x80` `VENDOR_REQ_GET_EDID` で EDID を読む。
6. EDID と timing table から 4K30 可否を判定。
7. `0x84`/`0x89` で resolution timing table を読む。
8. requested mode の timing を選ぶ。
9. `0x12` `VENDOR_REQ_SET_RESOLUTION_DETAIL_TIMING` で detailed timing を送る。
10. `0x31` `VENDOR_REQ_SET_SOFTWARE_READY` を送る。
11. 必要に応じて `0x03` monitor power on。
12. frame queue から JPEG/YV12/NV12 を bulk OUT する。

4K30 判定条件:

- Monitor EDID が 4K を含む。
- T6 timing table に `3840x2160@30` がある。
- USB speed が SuperSpeed 以上。

条件を満たさない場合、Ubuntu driver は EDID を 1080p generic EDID に差し替える
path を持つ。

### macOS 計画への反映

Rust prototype では次に以下を移植するのがよい。

- `VramLayout`
  - `ramsize_mb`
  - `interface_count`
  - `display_index`
  - `display_caps`
  - `is_4k30`
  - `usb_speed`
  - から `cmd_addr` / `fb_addr[3]` を決める。
- `FrameScheduler`
  - triple buffer rotation
  - `cmdAddr` ring update
  - JPEG reset flag
  - frame payload size から `cmdoffset` 決定。
- `send_jpeg_frame()`
  - `BulkDmaHeader`
  - `VideoFlipHeader::jpeg`
  - JPEG bytes
  - 1024 byte padding
  - fragmenting bulk OUT

これにより「静止 JPEG を表示する」段階は、かなり公式 Ubuntu 実装に寄せて再現できる。
