# Trigger6 reverse engineering notes

JUA365 / MCT Trigger 6 系 USB display adapter の解析メモ。現時点では Linux DRM ドライバ実装途中のリポジトリを、既存の Mac 用 DriverKit ドライバと公開解析リポジトリの情報で照合している。

## 対象デバイス

- 手元のデバイス: JUA365
- USB 表示名: `T6 USB Station`
- Vendor: MCT Corp.
- VID/PID: `0711:5601`
- macOS 上では MCT の DriverKit extension が排他的にデバイスを掴んでいる。

macOS のインストール済みドライバ:

- App: `/Applications/USB Display.app`
- Main app bundle id: `tw.com.mct.mcttriggerdriver.USB-Display`
- Main app version: `4.2.0`
- Driver app bundle id: `tw.com.mct.mcttriggerdriver.USBDisplayDriverApp`
- DEXT bundle id: `tw.com.mct.mcttriggerdriver.Trigger6Dext`
- DEXT version: `1.2.4`
- DEXT binary: `tw.com.mct.mcttriggerdriver.Trigger6Dext`
- DEXT build log string: `Aug 21 2025 13:23:48`

`Info.plist` では `idVendor = 0x0711`, `idProduct = 0x5600`, `idProductMask = 0xffe0` なので、`0x5600..0x561f` が同じ personality にマッチする。JUA365 の `0x5601` はこの範囲に入る。

## 参考にした公開リポジトリ

`https://github.com/cyrozap/mct-usb-display-adapter-re/tree/master`

このリポジトリはかなり参考になる。特に以下が有用。

- `doc/Protocol-T6.md`
- `wireshark/proto_t6.c`
- T6 系サンプル capture と descriptor
- `t6-dissector-experiments` ブランチの実験的 video payload dissector

公開メモと Mac DEXT の静的解析は、control request と bulk command phase の大枠でよく一致している。

## 既存 Linux 実装の状態

このリポジトリは Linux DRM/KMS USB ドライバの作りかけ。

- `trigger6_drv.c`
  - USB DRM driver 本体。
  - 対象 VID/PID は `0711:5601`。
  - framebuffer 全体を RGB565 っぽく変換し、bulk OUT に同期転送している。
  - 各 chunk の前に 32 byte の `trigger6_session` を送る。
  - 最大転送長は `TRIGGER6_MAX_TRANSFER_LENGTH = 0x19000` で固定。
- `trigger6_commands.c`
  - vendor control request 群。
  - mode table / connector status / monitor enable / detailed timing 設定。
- `trigger6_connector.c`
  - EDID read。
  - USB read 失敗時にも 0 を返す箇所があり、後で直す必要がありそう。
- `trigger6_transfer.c`
  - 現在の `Makefile` では build 対象外。

現状は「最小限 DRM device と framebuffer 転送を組もうとしている」段階で、protocol の細部、割り込み処理、multi-output、partial update、圧縮形式、audio はまだ詰め切れていない。

## Control request 対応表

公開リポジトリと Mac DEXT の静的解析から、少なくとも以下はほぼ確度が高い。

| Direction | Request | 意味 | wValue | wIndex | Length |
| --- | ---: | --- | --- | --- | ---: |
| OUT | `0x03` | video output enable/disable | output index | enabled bool | 0 |
| OUT | `0x08` | set resolution by mode index | output index | mode index | 0 |
| OUT | `0x12` | set detailed timing | output index | 0 | 32 |
| IN | `0x80` | read EDID | byte offset | output index | variable |
| IN | `0x84` | get resolution table count | output index | 0 | 4 |
| IN | `0x85` | get resolution table data | output index | 0 | variable |
| IN | `0x87` | query monitor connection | output index | 0 | 1 |
| IN | `0x88` | query video RAM size | 0 | 0 | 1 |
| IN | `0x89` | get timing table | output index | byte offset or index | variable |
| OUT | `0x31` | software ready 系 | 不明 | 不明 | 0 |

既存 Linux 実装との照合:

- `trigger6_read_modes()` の `0x89` は Mac DEXT の `GetResTimingTable2` と合っている。
- `trigger6_get_connector_status()` の `0x87` は Mac DEXT の `QueryMonitorConnection2` と合っている。
- `trigger6_get_edid()` の `0x80` は Mac DEXT の `ReadEDID2` と合っている。
- `trigger6_set_resolution()` の `0x12` は Mac DEXT の `SetResDetailTiming2` と合っている。
- `trigger6_enable_monitor()` は request 自体は合っているが、`wValue` と `wIndex` の意味を再確認したほうがよい。Mac DEXT では `wValue = output index`, `wIndex = enabled bool` に見える。

## Bulk video command phase

T6 bulk OUT は、おそらく 32 byte の command phase のあとに video payload phase が続く。

公開リポジトリでは次のように説明されている。

- offset 0: session number
- offset 4: total payload length
- offset 8: destination address
- offset 12: fragment length
- offset 16: fragment offset
- offset 20 以降: output index などと推測されていた領域

Mac DEXT の `MCTTrigger6_IVars::QueueVideoBulkCommand` では以下に見える。

- offset 0: `0`, video session
- offset 4: total payload length
- offset 8: 呼び出し側から渡される第1引数。destination address または video command 種別らしい。
- offset 12: 今回 fragment の長さ
- offset 16: payload 内 offset
- offset 20: more-fragments flag らしい 1 byte
- 残り: 0

重要な差分:

現在の `trigger6.h` では offset 20 付近が `output_index` のように扱われているが、Mac DEXT の更新処理ではここを「続き fragment があるか」の flag として書き換えているように見える。Linux 実装が常に 0 を送っているなら、単一 fragment では動いても multi-fragment 転送で問題になる可能性がある。

Mac DEXT は device speed に応じて video packet size を変えている。Linux 側の `0x19000` 固定値は動く可能性はあるが、正しい上限かは未確定。

## Video payload format

まだ最大の不明点。

公開リポジトリの実験 branch では以下のような image format / video type が仮置きされている。

- video type: `3`, `4`, `7`
- image format:
  - `0x06`: NV12
  - `0x09`: BGR24
  - `0x0d`: JPEG

既存 Linux 実装は framebuffer 全体を単純に RGB565 へ変換して bulk OUT しているように見えるが、T6 実機が本当にその形式を受けるかは未確認。Mac/Windows の実 traffic で確認する必要がある。

## Windows motion capture: 2026-06-21

Windows 公式 driver で、制御した motion pattern を横向き `1920x1080` で capture した。

対象 pcap:

- `captures/type7_motion_horizontal_bands.pcapng`
- `captures/type7_motion_vertical_bands.pcapng`
- `captures/type7_motion_large_rects.pcapng`
- `captures/type7_motion_fullscreen_colors.pcapng`

`tools/t6_reassemble_video.py --salvage-eoi` と `tools/t6_type7_timeline.py --jpeg-summary` で確認した結果:

| Capture | type4 | type7 | type7 JPEG size | 備考 |
| --- | ---: | ---: | --- | --- |
| `fullscreen_colors` | 23 | 2 | `192x728` | 大きい色変化はほぼ type4 |
| `horizontal_bands` | 37 | 4 | `192x1080`, `32x1080`, `32x344` | 帯更新でも type4 が多い |
| `large_rects` | 28 | 11 | `192x1080` | type7 は同一 zone 内で連続 |
| `vertical_bands` | 28 | 2 | `192x1080` | type4 が主体 |

JPEG は type4/type7 ともに baseline で、SOF は `0xc0`。component sampling は全件で以下だった。

```text
id0:2x2:q0,id1:1x1:q1,id2:1x1:q1
```

つまり Windows 公式 driver も JPEG path では 4:2:0 を使っている。抽出 JPEG の comment には以下が入っていた。

```text
Intel(R) IPP JPEG encoder [7.1.37466] - Sep 25 2012;
```

このため、Windows 公式 driver は Intel IPP 系 JPEG encoder を使っている可能性が高い。

2026-06-21 に fullsnap 由来の Windows JPEG を `tools/t6_jpeg_inspect.py` で詳しく見たところ、YouTube type4 と niconico type7 のどちらも同じ傾向だった。

```text
comment = Intel(R) IPP JPEG encoder [7.1.37466] - Sep 25 2012;
SOF0 baseline
components = id0:2x2:q0,id1:1x1:q1,id2:1x1:q1
DQT luma sum = 369
DQT chroma sum = 558
```

macOS の `sips` / ImageIO で同じ JPEG を quality 95 再エンコードすると、同じ baseline 4:2:0 だが component id は `1,2,3` になり、量子化テーブルは Windows IPP より軽い。

```text
components = id1:2x2:q0,id2:1x1:q1,id3:1x1:q1
DQT luma sum = 330
DQT chroma sum = 497
```

したがって、Windows が JPEG 4:2:0 でも綺麗に見える理由は、単純に「量子化が弱いから」だけでは説明しにくい。候補は以下。

- IPP encoder の RGB->YCbCr 変換と chroma downsample filter が TurboJPEG FASTDCT より UI に有利。
- Windows 側は動画/表示内容に応じて type4/type7 の領域を切り替え、UI の細い赤/緑境界を劣化しにくいスケールで送っている。
- Mac 側 capture/rotate/resize のどこかで、JPEG 4:2:0 前に既に色境界がぼけている。
- T6 側 JPEG decoder は component id `0,1,2` と `1,2,3` の両方を受けるが、内部 YUV target や header color 指定との組み合わせで見え方が変わる可能性がある。

VideoToolbox JPEG encoder は `t6-vt-jpeg-probe` で試したが、この環境では `VTCompressionSessionCreate(JPEG) failed: -12903` で作成できなかった。少なくとも現状の probe 実装では VideoToolbox JPEG を本命にできない。macOS 標準 encoder を試すなら、まず ImageIO / CGImageDestination 系での静止画 encode を比較する。

公式 Mac driver 4.2.0 のインストール物も確認した。

```text
/Library/Application Support/USBDisplayDriver/Driver/USBDisplayDriverAppLauncher
/Library/LaunchAgents/tw.com.mct.USBDisplayDriverAppLauncher.plist
/Applications/USB Display.app/Contents/Resources/USB Display Device Driver.app
/Applications/USB Display.app/Contents/Resources/USB Display Device Driver.app/Contents/Library/SystemExtensions/tw.com.mct.mcttriggerdriver.Trigger6Dext.dext
```

`USBDisplayDriverAppLauncher` は device match 時に `USB Display Device Driver.app` を起動する launcher。実際の USB bulk は DriverKit dext `Trigger6Dext`、画面 capture/encode は user-space app 側に寄っている。

`USB Display Device Driver` 本体は以下を link していた。

```text
VideoToolbox.framework
OpenCL.framework
Accelerate.framework
IOSurface.framework
CoreVideo.framework
Metal.framework
CoreGraphics.framework
```

strings には以下が出ている。

```text
mct_jpeg_videotoolbox_encode
jpeg_videotoolbox.cpp
t6_metaljpeg_compress_texture_async
t6_compress_yuv420
t6_compress_yuv420_gpurle
t6_compress_yuv420_gpu_dctquantrlehuff_whole_image
jpegencode_dctquant_420
jpegencode_dctquant_420_2mb_parallel_sbb
jpegencode_dctquant_444_4b_parallel_sbb
jpegencode_dctquant_rle_420
jpegencode_rlehuffman_420_bytewise
t6_upload_uncompressed_yuv420
t6_upload_uncompressed_yuv444
t6_submit_frame_surface_whole_screen_compressed
t6_submit_frame_surface_with_compressed_dirty_rects
MCTT6Device JPEG Encoder
```

したがって公式 Mac driver は ImageIO ではなく、VideoToolbox path と独自 Metal JPEG encoder / YUV420 compressor を持つと見るのが自然。信号機が公式 driver で滲みにくい理由は、単純な TurboJPEG 420 ではなく、Metal 上の YUV420 downsample / DCT / Huffman pipeline、または raw YUV upload path にある可能性が高い。

address は主に次の 3 zone を回る。

```text
0x02500430 - 0x026fe430 span=0x1fe000
0x029556f0 - 0x02b536f0 span=0x1fe000
0x02daa9b0 - 0x02fa89b0 span=0x1fe000
```

`large_rects` の type7 では、`start_addr/end_addr = 0x02500430-0x026fe430` のまま `cmd_dest` だけが `0x23e0` ずつ進んでいた。`0x23e0 = align1024(payload_len) - 32` なので、`cmd_dest` は画面位置ではなく payload ring cursor と見るのが自然。

`type4` は `1920x1080` JPEG を送る full-frame/large-update path に見える。今回の pcap では type4 JPEG の多くが capture 上 EOI 欠落だったが、trailing zero を落として EOI を付けると復元できた。`fullscreen_colors` では青の全画面 JPEG を復元できた。

結論:

- 大きい画面変化は type7 dirty tile だけではなく、type4 `1920x1080` JPEG が主体。
- type7 は小さい補助 update または dirty strip に見える。
- JPEG path は Windows 公式でも 4:2:0。Mac 実装で JPEG 4:4:4 に寄せるより、公式互換性を重視するなら 4:2:0 を前提にするべき。
- type7 の `start_addr/end_addr` は直接の画面座標ではなく、3 zone / surface とその部分範囲を指している可能性が高い。
- 正しい再現には type7 だけでなく type4 large-update path の実装/再生も必要。

### fullsnap 追加確認

通常 snaplen の capture では type4 JPEG の EOI が欠け、`--salvage-eoi` で preview は作れても灰色欠けが混ざることがあった。`USBPcapCMD -s 2000000 -b 134217728` で取り直した fullsnap では、同じ fullscreen colors で全 JPEG が complete になった。

`captures/type7_motion_fullscreen_colors_fullsnap.pcap`:

- `type4 format=0x0d`: 24
- `type7 format=0x0d`: 15
- incomplete JPEG: 0
- type4 は `1920x1080`
- type7 は `512x1080` または `544x1080`

この fullsnap 由来の type4 `30..32` の 3 zone を replay すると、灰色欠けなしで表示できた。したがって、以前の灰色欠けは T6 側の decode/flip 失敗ではなく、通常 capture の snaplen 不足で payload が欠けたことが主因と見てよい。

YouTube 2秒も fullsnap で取り直した。

`captures/type7_motion_youtube_2s_fullsnap.pcap`:

- `type4 format=0x0d`: 52
- `type7 format=0x0d`: 41
- incomplete JPEG: 0
- type4 JPEG: `1920x1080`
- type7 JPEG: `1376x800`

address zone:

```text
type4:
0x02500430 - 0x026fe430 span=0x1fe000
0x029556f0 - 0x02b536f0 span=0x1fe000
0x02daa9b0 - 0x02fa89b0 span=0x1fe000

type7:
0x0253c430 - 0x0271c430 span=0x1e0000
0x029916f0 - 0x02b716f0 span=0x1e0000
0x02de69b0 - 0x02fc69b0 span=0x1e0000
```

type7 JPEG は fullsnap では画像として素直に復元できた。以前の「ブロック配置がランダムに見える」「YouTube から絵が取れない」問題は、少なくとも大部分が snaplen 不足による incomplete JPEG の副作用だった。

YouTube の `1376x800` type7 は、直前 type4 zone から見ると常に以下の差分になった。

```text
delta_start = 0x3c000
delta_end   = 0x1e000
span        = 0x1e0000
```

pitch 2048 と仮定すると `delta_start` は `(x=0,y=120)` に相当する。`end_addr` は `type4_end + 0x1e000` なので、単純な `start + JPEG height` ではなく、type4 の framebuffer zone と同じ終端基準も混ざっている。

YouTube fullsnap の replay は、`tools/t6_reassemble_video.py --export-payloads` で生成した manifest を `t6-send-type7 --replay-manifest-json` に渡せば type4/type7 混在シーケンスをそのまま送れる。

生成例:

```sh
python3 tools/t6_reassemble_video.py captures/type7_motion_youtube_2s_fullsnap.pcap \
  --summary-only \
  --export-payloads /tmp/t6-youtube-fullsnap-replay \
  --salvage-eoi
```

replay 例:

```sh
cargo run --features usb --bin t6-replay-video -- \
  --manifest /tmp/t6-youtube-fullsnap-replay/type7_motion_youtube_2s_fullsnap_manifest.json \
  --record-start 1 \
  --record-end 6 \
  --sequence-start 1000 \
  --ready \
  --power-on \
  --wait-interrupt-ms 200 \
  --sleep-ms 20
```

まずは短い範囲で試す。`t6-replay-video` は指定しなければ capture 元の `cmd_dest` をそのまま使う。`--payload-addr` で詰め直すこともできるが、YouTube frame は payload が大きく、`0x02800000` から 12 record 送るだけでも `cmd_dest` は `0x02df...` 付近まで進む。全 93 record を一気に送ると payload ring を大きく進めるため、VRAM command 領域との関係を見ながら `1..6`, `7..12` のように分けて試す。

type4 の直後に対応する type7 を送る最小確認:

```sh
cargo run --features usb --bin t6-replay-video -- \
  --manifest /tmp/t6-youtube-fullsnap-replay/type7_motion_youtube_2s_fullsnap_manifest.json \
  --record-start 6 \
  --record-end 7 \
  --sequence-start 7000 \
  --max-packet 0x8000 \
  --ready \
  --power-on \
  --wait-interrupt-ms 100 \
  --sleep-ms 50
```

### niconico fullsnap での type7

YouTube と同じく、niconico 再生でも fullsnap では type7 JPEG を完全に復元できた。

`captures/type7_niconico_comment_off_2s_fullsnap.pcap`:

- complete transfers: 267
- type4: 16
- type7: 251
- type7 JPEG:
  - `832x480`: 152
  - `832x800`: 35
  - `832x544`: 2
  - `1440x736`: 1
  - `1920x56`: 61

`captures/type7_niconico_comment_on_2s_fullsnap.pcap`:

- complete transfers: 163
- type4: 3
- type7: 159
- type7 JPEG:
  - `832x480`: 138
  - `832x544`: 2
  - `1440x736`: 1
  - `1920x56`: 18

JPEG の中身を確認すると、`832x480` はコメント込みの動画領域そのもの、`1920x56` は Windows タスクバー領域、`1440x736` はブラウザ領域の初期大領域だった。

直前 type4 zone からの差分はかなり安定している。

```text
832x480 / 832x544 / 832x800:
  delta_start = 0x78260
  delta_end   = 0x3c260
  span        = 0x1c2000

1920x56:
  delta_start = 0x1e0000
  delta_end   = 0xf0000
  span        = 0x10e000

1440x736:
  delta_start = 0x0
  delta_end   = 0x0
  span        = 0x1fe000
```

pitch 2048 と仮定すると、`0x78260` は `(x=608,y=240)`、`0x1e0000` は `(x=0,y=960)` に相当する。niconico の `832x480` は画面上の動画プレイヤー位置と整合するため、type7 は任意 dirty rect というより「公式 driver が認識した動画/帯領域」を固定 address range へ送る経路と見るのが自然。

一方、制御した小矩形・modal fade テストでは type7 が少ない、または出ない。したがって、macOS 側で汎用 dirty rect から type7 を生成するにはまだ情報不足。まずは「動画領域として扱う固定 tile」を生成/再生する実験に寄せる。

## Existing capture: `captures/mctt6.pcapng`

同梱 capture を `tshark` と `tools/t6_pcap_summary.py` で確認した。

- File: `captures/mctt6.pcapng`
- Encapsulation: Linux usbmon
- Packets: 1654
- Duration: 29.504097 seconds
- Device: `0711:5601`
- Capture date in file: 2023-10-17
- Endpoint examples:
  - `0x02`: bulk OUT
  - `0x83`: interrupt IN

この capture は JUA365 実機で取ったものとは限らないが、VID/PID は同じで、T6 protocol の実例としてかなり有用。

Control request には既知の T6 request が出ている。

- `0xc0, 0x80`: EDID
- `0xc0, 0x87`: connector status
- `0xc0, 0x88`: video RAM size
- `0xc0, 0x89`: timing table
- `0x40, 0x03`: monitor enable
- `0x40, 0x12`: detailed timing
- `0x40, 0x31`: ready 系
- `0xc0, 0xb0..0xb4`, `0xa1..0xa5`, `0xcc` など、公開メモにある adapter/session/info 系 request も出ている。

bulk OUT の最初の video command phase:

```text
00000000 e0870000 00000003 e0870000 00000000 00000000 00000000 00000000
```

little-endian で読むと:

| Offset | Value | Meaning |
| ---: | ---: | --- |
| `0x00` | `0x00000000` | session 0, video |
| `0x04` | `0x000087e0` | total payload length = 34784 |
| `0x08` | `0x03000000` | destination / command class, JPEG系らしい |
| `0x0c` | `0x000087e0` | fragment length = 34784 |
| `0x10` | `0x00000000` | fragment offset = 0 |
| `0x14` | `0x00000000` | more-fragments flag 0 |

続く data phase の先頭:

```text
04000000 b0870000 01000000 06000000
50055005 530fe7f0 130ee8f0 10000000
0d000000 00000000 00000000 00000000
ffd8ffe0 ...
```

観察:

- video payload type は `0x04` に見える。
- payload 内 length は `0x87b0`。command phase total `0x87e0` との差は `0x30` bytes で、payload header size と一致する。
- `0x20` の `0x0d` は image format JPEG と見てよい。
- `0x30` byte header の直後に JPEG SOI `ff d8 ff e0` が来る。
- JPEG comment には `Intel(R) IPP JPEG encoder ... Sep 25 2012` が含まれていた。
- JPEG SOF から画像サイズは `1360 x 768` と取れる。
- T6 header の `width_field` は `0x550` で JPEG width と一致するが、`height_field` も `0x550` なので、現在の `width/height` 命名はまだ疑わしい。

つまり、この capture では raw RGB565 ではなく、明確に JPEG payload が送られている。

large payload では multi-fragment command phase が出ている。

```text
session=0 total=0x4f7e0 dest=0x030b7dc0 frag_len=0x19000 frag_off=0x0     more=1
session=0 total=0x4f7e0 dest=0x030b7dc0 frag_len=0x19000 frag_off=0x19000 more=1
session=0 total=0x4f7e0 dest=0x030b7dc0 frag_len=0x19000 frag_off=0x32000 more=1
session=0 total=0x4f7e0 dest=0x030b7dc0 frag_len=0x47e0  frag_off=0x4b000 more=0
```

これにより、bulk command phase offset `0x14` は `more_fragments` flag である可能性がかなり高くなった。

別系統で、session 3 の 32 byte command phase が大量にある。

```text
03000000 80070000 00000000 80070000 00000000 00000000 00000000 00000000
```

これは little-endian で:

- session 3
- total/fragment length `0x780` = 1920
- destination 0

public protocol では session 3 は audio とされており、data phase も 16-bit sample 風に見える。video 解析時は session 0 と session 3 を分けて扱う必要がある。

bulk OUT の data length 分布:

- 32 bytes: command phase が多数
- 1920 bytes: session 3 らしき audio data が多数
- 34784 / 38880 / 42976 / 102400 bytes など: JPEG/画像 payload らしき video data

capture 内の interrupt IN 64 byte packet は次の形が繰り返される。

```text
04000000 00000000 00000000 01000000 00000004 ...
04000000 00000000 00000000 02000000 00000004 ...
...
04000000 00000000 00000000 3b000000 00000004 ...
```

DExt の解析と照合すると:

- `packet[0] = 0x04` なので bit 2 が立ち、video interrupt。
- `*(uint32_t *)&packet[0x0c]` は `1..0x3b` の連番。
- `packet[0x13]` は `0x04`。
- user-space video client へはこの2つが渡される。

user-space 側の文字列に `FENCE_ID` があるので、`packet[0x0c]` の連番は fence ID である可能性が高い。`packet[0x13] = 0x04` は event kind / func mask の候補。

## macOS と Windows の挙動差

手元情報:

- Mac では専用アプリ/DriverKit extension が動き、仮想ディスプレイを作って、その画面を USB 側へ転送しているように見える。
- Windows では追加ドライバ無しでも動くが、デバイスマネージャ上は `Trigger 6 External Graphics` として見えており、MCT 製 driver file が使われている。

Windows で確認済みの driver / device 情報:

- Device Manager name: `Trigger 6 External Graphics`
- USB Device Tree Viewer: `Magic Control T6 USB Station` device の下に `Trigger 6 External Graphics` がいる形。
- USB device name: `T6 USB Station`
- Hardware IDs: `USB\VID_0711&PID_5601&REV_1010`, `USB\VID_0711&PID_5601`
- USB class: vendor specific, `bDeviceClass = 0xff`, interface class `0xff`
- USB version / speed: USB 3.1 Gen 1, SuperSpeed operating mode
- Device driver:
  - `C:/Windows/System32/Drivers/t6sta.sys`
  - Version: `1.0.24.711`
  - Date: `2025-10-29`
  - Company: `Magic Control Technology Corporation`
  - INF: `C:/Windows/inf/oem77.inf`
- Driver files:
  - `C:/Windows/System32/DRIVERS/UMDF/Trgldd.dll`
  - `C:/Windows/System32/t6indisp.dll`
- Endpoints:
  - `0x81`: bulk IN, endpoint 1, max packet `0x400`
  - `0x02`: bulk OUT, endpoint 2, max packet `0x400`
  - `0x83`: interrupt IN, endpoint 3, max packet `0x40`, interval 2 ms
- Product behavior note: public/product-level behavior is reportedly 4K at up to 30 fps, and 1080p at up to 60 fps. This matches the working hypothesis that 4K may use a distinct high-resolution/lower-fps transfer path from the 1080p JPEG-tile path.

`t6indisp.dll` という名前から、Windows 側は indirect display driver 系の構成である可能性が高い。手動インストール不要に見えても、Windows Update などで MCT の driver package が入っている可能性を優先して見る。

Mac 側は推測ではなく、user-space driver binary の文字列からかなり裏が取れている。

- `CGVirtualDisplayDescriptor`
- `CGVirtualDisplayMode`
- `CGVirtualDisplaySettings`
- `CGVirtualDisplay`
- `CGDisplayStream`
- `CGRequestScreenCaptureAccess`
- screen recording consent UI
- `MCTCoreGraphicsVirtualDisplayFramebuffer`
- `MCTDisplayStreamFBImageSource`
- `startDisplayStream`

つまり Mac 版は「CoreGraphics の virtual display を作る -> display stream / screen capture で frame を得る -> T6 用に encode して DriverKit DEXT へ渡す」という構成らしい。

さらに user-space driver 側には T6 画像処理の文字列が大量にある。

- `t6_submit_frame_surface_with_compressed_dirty_rects`
- `t6_submit_frame_surface_whole_screen_compressed`
- `t6_submit_frame_surface_compressed_flip`
- `submitEncodedFrame`
- `t6_upload_uncompressed_yuv420`
- `t6_upload_uncompressed_yuv444`
- `t6_compress_yuv420`
- `t6_compress_yuv420_gpurle`
- `mct_jpeg_videotoolbox_encode`
- `MCTT6Device JPEG Encoder`
- `reservePayloadVramAndFenceIdAsync`
- `t6_vram_payload_zone_alloc`
- `MCTT6Device::handleVideoInterrupt: JPEG decoder error!`

依存 framework も `CoreGraphics`, `IOSurface`, `CoreVideo`, `Metal`, `VideoToolbox`, `OpenCL` を含む。DExt は USB transport と control request を担当し、payload の encode / VRAM / fence 管理は主に user-space driver 側にある可能性が高い。

Windows との差分からの仮説:

- Mac は OS 標準でこの USB display adapter を直接 display output として扱えないため、仮想ディスプレイ + userspace/DriverKit 転送の構成になっている可能性が高い。
- Windows は in-box driver、Windows Update 経由の自動 driver、または Windows 表示スタックに統合された USB display 経路で動いている可能性がある。
- ただし USB 上の T6 control request / bulk video payload は同じ系統のはずなので、Windows capture は protocol 確認にかなり有用。

「追加ドライバ無し」は、手動インストール不要という意味か、Windows が完全に標準クラスドライバだけで認識しているという意味かで解釈が変わる。Windows のデバイスマネージャで driver provider / driver file を確認したい。

既存 Linux 実装は raw RGB565 風の全画面 payload を送ろうとしているが、公式 Mac 実装は JPEG/YUV/VRAM/fence を使う経路が中心に見える。このため、Linux 側は framebuffer 変換だけでなく T6 payload header / VRAM upload / flip command の構造を再現する必要がある可能性が高い。

## 割り込み endpoint

Mac DEXT には interrupt pipe read がある。

- `StartReadingInterrupts`
- `InterruptPipeReadCompletion_Impl`
- read size は 64 bytes に見える。
- `MCTTrigger6VideoClient::HandleVideoInterrupt(unsigned char, unsigned int)` へ状態を渡している。

`InterruptPipeReadCompletion_Impl` の静的解析結果:

- 完了 byte count は `<= 0x40` を assert。
- status が `kIOReturnAborted` らしき値の場合、停止中でなければ read を再開する。
- status 成功かつ `actualByteCount == 0x40` のときだけ packet を処理する。
- interrupt buffer 先頭 byte の bit 2 が立っていると video interrupt として扱う。
- video interrupt は以下だけを userspace video client に渡す。
  - `packet[0x13]` の 1 byte
  - `*(uint32_t *)&packet[0x0c]`
- 先頭 byte の bit 5 が立っていると audio interrupt として扱い、64 byte packet 全体を audio handler に渡す。
- 処理後、停止中でなければ interrupt read を再投入する。

公開リポジトリ側では、interrupt packet type として status change / current state / firmware update らしき情報が推測されている。connector hotplug, output status, audio などに関係しそうだが、まだ未確定。

video interrupt の2値は、user-space 側の文字列から見ると `JPEG_ERROR`, `FENCE_ID`, `PORT_0_CONNECTION`, `PORT_1_CONNECTION` のどれかに対応している可能性がある。`MCTT6Device::handleVideoInterrupt: JPEG decoder error! (fence ID %08x)` という文字列があり、少なくとも JPEG decoder error と fence ID は interrupt 経由で来るらしい。

## Windows capture: initial USBPcap probe

Windows 上で `tshark -D` を確認したところ、JUA365 は `USBPcap1` 側にいた。

短時間 probe:

```text
file: captures/probe_usbpcap1.pcapng
interface: \\.\USBPcap1
duration: 5s
packets: 2293
```

`tools/t6_pcap_summary.py --summary-only` の結果:

```text
packets 2293
commands 66
video_payloads 11
interrupts 22
video_count type=0x3 format=0xd count=11
interrupt_count flags=0x04 event=0x04 count=22
```

既存 Linux usbmon capture では主に `type=0x4 format=0x0d` の JPEG payload が見えていたが、Windows MCT driver 経路の probe では `type=0x3 format=0x0d` が出た。どちらも JPEG format `0x0d` なので、Windows でも raw RGB ではなく JPEG/VRAM/fence 系の T6 payload を使っていることはかなり確度が高い。

次に取るべき capture:

- HDMI 未接続からの full enumeration。
- HDMI hotplug。
- solid color full-screen。
- partial update。

追加で取得済み:

```text
file: captures/2026-06-19_jua365_win_01_enumeration_no_hdmi_retry.pcapng
interface: \\.\USBPcap1
packets: 9904
commands: 0
video_payloads: 0
interrupts: 2
interrupt_count flags=0x20 event=0x00 count=2

file: captures/2026-06-19_jua365_win_02_hdmi1_hotplug.pcapng
interface: \\.\USBPcap1
packets: 14378
commands: 935
video_payloads: 148
interrupts: 288
video_count type=0x3 format=0x0d count=40
video_count type=0x7 format=0x0d count=108
interrupt_count flags=0x04 event=0x01 count=2
interrupt_count flags=0x04 event=0x04 count=283
interrupt_count flags=0x20 event=0x00 count=3

file: captures/2026-06-19_jua365_win_04_solid_colors.pcapng
interface: \\.\USBPcap1
packets: 23036
commands: 1722
video_payloads: 124
interrupts: 457
video_count type=0x7 format=0x0d count=124
interrupt_count flags=0x04 event=0x04 count=457

file: captures/2026-06-19_jua365_win_05_partial_update.pcapng
interface: \\.\USBPcap1
packets: 26841
commands: 2090
video_payloads: 95
interrupts: 649
video_count type=0x7 format=0x0d count=95
interrupt_count flags=0x04 event=0x04 count=649

file: captures/2026-06-19_jua365_win_03_resolution_change.pcapng
interface: \\.\USBPcap1
packets: 20488
commands: 2016
video_payloads: 137
interrupts: 454
video_count type=0x7 format=0x0d count=137
interrupt_count flags=0x04 event=0x04 count=454

file: captures/2026-06-19_jua365_win_06_4k_hotplug.pcapng
interface: \\.\USBPcap1
packets: 186500
note: 4K display was attached first, then a 1920x1080 display was also attached during the capture.

file: captures/2026-06-19_jua365_win_07_4k_only.pcapng
interface: \\.\USBPcap1
packets: 124049
note: 4K display only.
```

観察:

- Windows MCT driver 経路では `type=0x3 format=0x0d` が hotplug 前後の JPEG upload に出る。
- `type=0x7` は `type=0x3/0x4` と header layout が少し違い、offset `0x14` に `0x07800780` のような packed field が入る。これは `canvas=1920x1920` のように見える。
- `type=0x7` の VRAM start/end と image format は `type=0x3/0x4` より 4 byte 後ろにずれる。summary tool はこの layout 差を反映済み。
- 修正後の解釈では `type=0x7 format=0x0d` で、payload offset `0x30` に JPEG SOI が来る。つまり `type=0x7` も JPEG tile/dirty update と見てよい。
- `type=0x7` の詳細行では `jpeg=128x576`, `128x544`, `64x544`, `64x928`, `64x1080` などの細長い tile が見えており、dirty rect / tiled update の可能性が高い。`width_field` / `height_field` は JPEG SOF の width / height と一致する。
- `type=0x7` の例:
  - `type=0x7 data_len=0x2fb0 seq=7998 hint=0x6 width_field=0x40 height_field=0x220 canvas=1920x1920 start=0x30 end=0x1fe030 format=0xd jpeg=64x544`
  - `type=0x7 data_len=0x123b0 seq=8008 hint=0x6 width_field=0x720 height_field=0x60 canvas=1920x1920 start=0x18aab50 end=0x1aa8b50 format=0xd jpeg=1824x96`
- 1080p header CSV / extracted samples:
  - `captures/1080p_video_headers.csv`
  - `captures/1080p_frame4650_type7_64x1080.jpg`
  - `captures/1080p_frame4650_type7_64x1080.header.bin`
- Representative type 7 header fields for `64x1080` JPEG:
  - `w0=0x00000007`: type
  - `w1=0x000053b0`: JPEG payload length + 0x30 header length
  - `w2`: sequence/fence-like counter
  - `w3=0x00000006`: flags/hint
  - `w4=0x04380040`: `height << 16 | width`
  - `w5=0x07800780`: `canvas_height << 16 | canvas_width` = `1920x1920`
  - `w6/w7`: VRAM start/end
  - `w9=0x0000000d`: JPEG format
- Current Linux-side 1080p implementation status:
  - Bulk command layout has been aligned with capture (`more_fragments` byte at offset 20).
  - `0x31` software-ready request is sent during probe.
  - A `trigger6_type7_video_header` struct has been added for the JPEG tile path.
  - The actual framebuffer update path still sends BGR24-style full frames. The next functional step is to replace or bypass it with a JPEG encoder/test payload path that emits `type=0x7 format=0x0d` payloads.
- interrupt はほぼ `flags=0x04 event=0x04` で、`value` は `0x21b8`, `0x21b9`, ... のように増える。fence ID 仮説と整合する。
- `2026-06-19_jua365_win_03_resolution_change.pcapng` は resolution change attempt。capture 内には `0x12` detailed timing や `0x08` set resolution by index は出ていない。control transfer も descriptor/config 系だけで、Windows driver がモード変更を vendor control request として流している様子はこの capture には見えない。
- 同 capture の video tile は全て `canvas=1920x1920`。前半は `64x1080` の縦 tile が中心で、32s 付近に `1216x192`, `320x224`, `64x288` などの小さい dirty update がまとまって出る。これは Windows の設定画面/確認 UI の描画変化を反映した可能性が高く、出力 timing の切替そのものを示す packet はまだ特定できていない。
- `tools/t6_type7_timeline.py` で type 7 tile を pcap/session/sequence ごとに時系列 grouping できるようにした。`captures/1080p_video_headers.csv` は複数 pcap の行を含むため、時刻だけで grouping すると別 capture が混ざる。
- Windows 1080p capture の type 7 tile は、1更新が複数 tile で構成されることがある。例: `64x96`, `1824x96`, `64x1016` の3 tile が約 1.6ms 内に連続し、同一の上帯/中央帯/左帯のような dirty 分割に見える。
- `start_addr/end_addr` は tile JPEG の byte range や画面座標から単純に計算できる値ではない。同じ `start=0x18aaaf0 end=0x1aa8af0 span=0x1fe000` が `64x64`, `64x96`, `64x544`, `64x1080`, `128x576` など複数サイズで再利用される。したがって、この2値は tile-local offset ではなく、surface / VRAM allocation / decoder target zone の状態を指す可能性が高い。
- `captures/mctt6.pcapng` の type 7 は少数だが、tile 直後 1-数 ms に interrupt `flags=0x04 event=0x04 value=<sequence相当>` が返る例がある。type 7 実送信には JPEG header だけでなく、この ack/fence と target surface state の扱いが必要な可能性が高い。
- Mac 実機で full-frame `VideoFlipHeader.fence_id` を非ゼロ連番にしたところ、interrupt raw packet は `packet[0]=0x04`, `packet[19]=0x04`, `packet[0x0c..0x10]=fence_id` になった。これは Windows pcap の type 7 `sequence` ack と同じ形。
- `t6-virtual-display --wait-interrupt-ms` の観測では、full-frame path の ack は送信直後に必ず取れるわけではなく、安定時でも `ack_lag=1`、つまり1 frame前の fenceまで読めることが多い。type 7 実験では現在 tile の immediate ack ではなく、次回送信後に前回 sequence の ack を確認する設計が必要。
- 4K + 1080p 混在 capture は巨大なため、先頭/中盤を `editcap` で切り出して解析した。control transfer 側では `0x10` timing/read 系が `wValue=0..4` に対して繰り返し出ており、単一出力時より多くの output/timing slot を見ている。`0x80` EDID read は output 0/1 で確認できる。
- 同 capture の中盤切り出しでは、拾えた video payload は `type=0x4 format=0x0d jpeg=1920x1080` と `type=0x7 format=0x0d jpeg=1920x736`。現時点の parser で見えている video stream は 4K full frame ではなく、1080p 系の面を送っているように見える。4K 側が別 session / 別 header layout / 別 output slot で流れている可能性が残る。
- 4K only capture では `session=7` の bulk command が継続的に出る。既存の 1080p/JPEG path と違い、command 直後の payload は `6144` bytes や `1044480` bytes の大きな blob で、JPEG SOI は見えない。例:
  - command frame `24935`: `07 00 00 00 00 18 00 00 ...` の直後に `6144` bytes payload。
  - command frame `24939`: `07 00 00 00 00 e4 1b 00 ...` の直後に `1044480` bytes payload。
- したがって 4K path は、少なくとも Windows capture では従来の `session=0 type=0x7 format=0x0d JPEG tile` ではなく、`session=7` の別 payload format を使っている可能性が高い。Linux driver 側で 4K 対応するなら、JPEG path だけでなくこの session 7 path の解読が必要になる。
- `session=7` command の 6th dword は packed width/height と見てよい。`low16=width`, `high16=height` とすると、確認した全 command で `total_len == width * height * 3 / 2` が成立した。つまり payload は YUV420/NV12/I420 系の raw rectangle である可能性が高い。
- 4K only mid cut で見えた `session=7` dimensions:
  - `64x64`, `total=0x1800`, count 35
  - `2720x448`, `total=0x1be400`, count 33
  - `2656x64`, `total=0x3e400`, count 16
  - `384x384`, `total=0x36000`, count 2
  - `32x64`, `total=0x0c00`, count 1
- 例: command raw `07 00 00 00 00 e4 1b 00 30 a3 5f 00 30 63 ae 00 00 0f 00 0f a0 0a c0 01 ...` は `total=0x1be400`, `width=0x0aa0=2720`, `height=0x01c0=448`。`2720 * 448 * 3 / 2 = 0x1be400`。
- USBPcap/tshark default capture は large payload を `frame.cap_len=65535` で切る。`tshark -s 0` でも USBPcap extcap 経由では改善しなかった。full payload を取るには `USBPcapCMD.exe` を直接使い、`-A -s 2000000 -b 134217728` のように snaplen を指定する必要がある。
- `USBPcapCMD.exe` high-snaplen capture では `frame.cap_len == frame.len` の full payload が取れる。確認例: `test_usbpcapcmd_snap2m_motion.pcap` の frame `151186` は `session=7`, `total=0x15c00`, packed size `64x928`; 直後の frame `151188` は payload `89088` bytes で、`64 * 928 * 3 / 2` と一致する。抽出して Y plane を grayscale preview すれば、NV12/I420 のどちらかを判定できるはず。
- frame `151188` payload を抽出して preview 済み:
  - raw: `captures/session7_frame151188_64x928_yuv420.raw`
  - Y plane: `captures/session7_frame151188_64x928_y_plane.png`
  - NV12 color: `captures/session7_frame151188_64x928_nv12.png`
  - I420 color: `captures/session7_frame151188_64x928_i420.png`
- Y plane には文字/背景が正常に見える。NV12 color は自然、I420 color は色ノイズが強い。したがって `session=7` 4K path の payload format は NV12 raw rectangle と見るのが現時点で最も妥当。
- `test_usbpcapcmd_snap2m_motion.pcap` から `session=7` command を抽出し、CSV/preview を作成:
  - command TSV: `captures/session7_commands_test_usbpcapcmd_snap2m_motion.tsv`
  - rect CSV: `captures/session7_rects_test_usbpcapcmd_snap2m_motion.csv`
  - second preview: `captures/session7_frame203538_64x928_nv12.png`
- high-snaplen motion capture 内の `session=7` は 2 件だけで、どちらも `64x928`, `total=0x15c00`, NV12 size と一致する。2枚の preview はほぼ同じ縦長 UI 領域で、異なる `addr1/addr2` はダブルバッファ/リングバッファ上の別領域を指している可能性が高い。
- default-snaplen の `captures/2026-06-19_jua365_win_07_4k_only_mid40k.pcapng` から command metadata を CSV 化:
  - `captures/session7_commands_07_4k_only_mid40k.csv`
  - rows: 87
  - dimensions: `64x64` count 35, `2720x448` count 33, `2656x64` count 16, `384x384` count 2, `32x64` count 1
  - all command sizes satisfy `total_len == width * height * 3 / 2`
- address groups show repeated address pairs:
  - `2720x448`: `addr1=0x05fa330 addr2=0x0ae6330` count 16, and `addr1=0x124f890 addr2=0x173b890` count 17; both have `addr2-addr1=0x4ec000`
  - `2656x64`: `addr1=0x0ccd6b0 addr2=0x147a6b0`, `addr2-addr1=0x7ad000`
  - many `64x64` rects have `addr2-addr1=0x735000` or `0x7ad000`
- `addr1/addr2` are not yet proven screen coordinates. Treat them as VRAM/buffer addresses first. Subtracting the minimum `addr1=0x78150` and testing stride candidates gives some plausible alignments, but no single stride cleanly explains all rectangles. More full-snaplen rect previews are needed to infer exact placement/stride.
- Existing captures are not enough to reconstruct a complete 4K frame. The only full high-snaplen `session=7` payloads currently available are two `64x928` rects from `test_usbpcapcmd_snap2m_motion.pcap`.
- Default-snaplen 4K captures still contain useful prefixes of large rects. Preview artifacts:
  - `captures/session7_truncated_large_rect_previews.csv`
  - `captures/session7_truncated_frame24941_2720x448_a_y_rows24.png`: first 24 Y rows of a `2720x448` NV12 rect; visible text confirms the rect interpretation.
  - `captures/session7_truncated_frame37947_384x384_a_y_rows170.png`: first 170 Y rows of a `384x384` NV12 rect; UI card/text are visible.
- The `2656x64` command at frame `37591` (`total=0x3e400`, packed size `2656x64`) does not have an obvious immediate payload in the truncated capture, so either payload association is more complex for that packet, the transfer was skipped/completed differently, or the current command pairing heuristic is incomplete.

Deferred 4K follow-up:

- Do not block the 1080p/JPEG path on 4K. Treat 4K as a separate `session=7` NV12 path.
- When the device is available again, capture 4K with `USBPcapCMD.exe` directly, not Wireshark extcap:
  - `USBPcapCMD.exe -d \\.\USBPcap1 -A -s 2000000 -b 134217728 -o captures/<name>.pcap`
  - Move a large high-contrast window on the 4K output during capture to force `2720x448` / larger rects.
- For analysis, extract full `session=7` rects, render both Y plane and NV12 color, then infer:
  - whether `addr1` or `addr2` is the destination VRAM address
  - screen coordinate / stride mapping for rect placement
  - fence interrupt mapping for `session=7`
- A complete 4K frame reconstruction needs enough full-snaplen rects to cover the screen or a known full-screen update. Current captures are sufficient for format identification, not full-frame assembly.

### Windows enumeration no HDMI control sequence

`captures/2026-06-19_jua365_win_01_enumeration_no_hdmi_retry.pcapng` は JUA365 を抜いた状態から挿し直した full enumeration。USB address は `25`。HDMI 未接続なので session 0 bulk video は出ず、interrupt は audio/status 系らしい `flags=0x20 event=0x00` が 2 回だけ出た。

descriptor 後の vendor/control sequence:

```text
IN  0xb0 wValue=0x0000 wIndex=1 len=4
IN  0xb0 wValue=0x0000 wIndex=2 len=4
IN  0xb0 wValue=0x0000 wIndex=4 len=4
IN  0xb0 wValue=0x0000 wIndex=0 len=4
IN  0xb0 wValue=0x0000 wIndex=5 len=8
IN  0xb4 wValue=0x0000 wIndex=0 len=4
IN  0xb0 wValue=0x0000 wIndex=3 len=16
IN  0xcc wValue=0x0001 wIndex=0 len=104
IN  0xb1 wValue=0x0000 wIndex=0 len=132
IN  0xb1 wValue=0x0000 wIndex=3 len=132
IN  0xa1 wValue=0x0000 wIndex=0 len=16
IN  0xa4 wValue=0x0000 wIndex=0 len=16
IN  0xa1 wValue=0x0000 wIndex=1 len=16
IN  0xa5 wValue=0x0000 wIndex=0 len=32
IN  0xa2 wValue=0x0000 wIndex=0 len=16
IN  0xa3 wValue=0x0000 wIndex=0 len=40
OUT 0x23 wValue=0x0000 wIndex=0 len=40
IN  0xb3 wValue=0x0000 wIndex=0 len=112
IN  0x88 wValue=0x0000 wIndex=0 len=1
OUT 0x1c wValue=0x0000 wIndex=0 len=0
OUT 0x1c wValue=0x0100 wIndex=0 len=0
OUT 0x31 wValue=0x0000 wIndex=0 len=0
OUT 0x24 wValue=0x0000 wIndex=0 len=16
IN  0x89 wValue=0x0000 wIndex=0 len=512
IN  0x89 wValue=0x0000 wIndex=512 len=512
IN  0x89 wValue=0x0000 wIndex=1024 len=128
IN  0x89 wValue=0x0001 wIndex=0 len=512
IN  0x89 wValue=0x0001 wIndex=512 len=448
IN  0x87 wValue=0x0000 wIndex=0 len=1
IN  0x87 wValue=0x0001 wIndex=0 len=1
```

観察:

- `0x31` software ready は Windows でも display session 用に `wValue=0`, `wIndex=0` で送られる。
- `0x89` timing table は output 0 と output 1 の両方を読む。output 0 は `512 + 512 + 128 = 1152` bytes、output 1 は `512 + 448 = 960` bytes。
- `0x87` connector status も output 0 / output 1 の両方を見る。
- HDMI 未接続のため EDID request `0x80` はこの capture には見えていない。

## Software ready request

`T6_send_software_ready_commands` から、`0x31` request は少し確度が上がった。

- display ready:
  - `bmRequestType = 0x40`
  - `bRequest = 0x31`
  - `wValue = 0`
  - `wIndex = 0`
  - `wLength = 0`
- audio ready, audio 対応時のみ:
  - `bmRequestType = 0x40`
  - `bRequest = 0x31`
  - `wValue = 3`
  - `wIndex = 0`
  - `wLength = 0`

既存 Linux 実装にはまだ入っていない。起動時の初期化 sequence に必要か、Windows/Mac capture で確認したい。

## 現時点の不明点

- 32 byte bulk command phase の offset 20 以降の正確な構造。
- `destination address` の値の意味。
  - 例: `0x00000030`, `0x03000000`
- video payload header の正確な構造。
- 実際に JUA365 が受け付ける pixel format / compression。
- full-frame update と partial update の切り替え条件。
- multi-output 時の output index の置き場所。
- Mac DEXT の `0x31` software ready request の正確な引数。
- interrupt packet の bit layout。先頭 byte の bit 2 = video, bit 5 = audio までは見えたが、`packet[0x13]` と `packet[0x0c..0x0f]` の意味は未確定。
- SuperSpeed / HighSpeed での正しい bulk fragment size。
- Windows が使っている driver が本当に in-box なのか、Windows Update 由来なのか。
- Windows と Mac で video payload format が同じか。Windows が画面キャプチャ型でなくても、USB payload は JPEG/YUV/VRAM/fence 系の可能性が高い。
- 既存 Linux 実装の `trigger6_set_resolution()` に渡す 32 byte timing table の byte layout。
- EDID / mode table 取得失敗時の error handling。
- `trigger6_transfer.c` をどう扱うべきか。今は build されていない。

## 次に見るべき場所

1. Mac DEXT の `QueueVideoBulkCommand` 呼び出し元。
   - 呼び出し時の第1引数が `destination address` か command kind かを確定する。
2. Mac DEXT の `InterruptPipeReadCompletion_Impl`。
   - 64 byte interrupt packet の decode を進める。
3. Windows capture。
   - Mac とは違う display pipeline でも、USB protocol の実例として有用。
4. 既存 `captures/mctt6.pcapng` の再解析。
   - command phase と payload header を Wireshark dissector の推測と照合する。
## Ubuntu T6 official driver package notes

`ubuntu_T6_260223.run` is a self-extracting POSIX shell installer. The first
114 lines are installer logic; line 115 onward is a gzip tar archive containing:

- `evdi.tar.gz`
- `evdi_t6.tar.gz`

The vendor Linux stack is not a native T6 DRM/KMS kernel driver. It builds and
installs EVDI, then runs a user-space daemon (`T6evdi`) that connects EVDI
updates to the T6 device through libusb.

Important protocol definitions from `evdi_t6_1/t6.h` and `t6bulkdef.h`:

- Bulk OUT endpoint: `0x02`
- Bulk IN endpoint: `0x81`
- Interrupt IN endpoint: `0x83`
- `VENDOR_REQ_SET_SOFTWARE_READY = 0x31`
- `VENDOR_REQ_SET_RESOLUTION_DETAIL_TIMING = 0x12`
- `VENDOR_REQ_GET_EDID = 0x80`
- `VENDOR_REQ_QUERY_MONITOR_CONNECTION_STATUS = 0x87`
- `VENDOR_REQ_QUERY_VIDEO_RAM_SIZE = 0x88`
- `VENDOR_REQ_QUERY_SECTION_DATA = 0xb3`
- `VIDEO_COLOR_JPEG = 13`
- `VIDEO_COLOR_NV12 = 6`
- `VIDEO_COLOR_YV12 = 4`
- `VIDEO_CMD_FLIP_PRIMARY = 3`
- `VIDEO_CMD_FLIP_SECONDARY = 4`

The 32-byte bulk command header matches the current reverse-engineered command
shape:

```c
struct bulk_cmd_header {
    u32 signature;
    u32 payload_length;
    u32 payload_address;
    u32 packet_length;
    u32 reserved2;
    u32 reserved3;
    u8 padding[8];
} __packed;
```

`t6_libusb_FilpJpegFrame()` sends a bulk command with `signature=0`,
`payload_address=t6dev->cmdAddr`, then sends a 48-byte `VIDEO_FLIP_HEADER`
followed by JPEG bytes and 1024 bytes of padding. The flip header sets
`TargetFormat=VIDEO_COLOR_NV12`, `SourceFormat=VIDEO_COLOR_JPEG`,
`Y_RGB_Pitch=align32(width)`, `UV_Pitch=align32(width)`, Y offset to
`t6dev->fbAddr`, UV offset to `fbAddr + aligned_y_plane_size + 1024`, and uses
`Flag=0x80` as the JPEG reset flag during initial frames / command ring wrap.

For non-4K mode in the default build, `usb_process()` uses JPEG:

- `!bRun4K30` -> `t6_libusb_FilpJpegFrame(...)`
- `bRun4K30` -> `t6_libusb_FilpYV12Frame(...)`

The source comments document VRAM layout. For one-port 1080p output:

- `cmdAddr = 0x0000000`
- `fbAddr1 = (ramsize - 12) MiB`
- `fbAddr2 = (ramsize - 8) MiB`
- `fbAddr3 = (ramsize - 4) MiB`

For the observed 58 MiB RAM size this corresponds to:

- `fbAddr1 = 0x2e00000`
- `fbAddr2 = 0x3200000`
- `fbAddr3 = 0x3600000`

This explains the Windows capture addresses around `0x32xxxxxx` and confirms
that the earlier BGR/RGB565 full-frame Linux path is not the right 1080p path.
The next implementation target should be an EVDI/libusb-style user-space path,
or at least a Linux kernel bulk sender that emits the same `VIDEO_FLIP_HEADER +
JPEG + padding` payload.

## 2026-06-21 公式 macOS ドライバ Ghidra 解析メモ

対象:

- `/Applications/USB Display.app/Contents/Resources/USB Display Device Driver.app/Contents/MacOS/USB Display Device Driver`
- version: driver app 4.2.0
- Ghidra 12.1.2 で x86_64 slice を import 済み。
- project/output は `ghidra_projects/`, `ghidra_out/` に生成。再実行用 script は `ghidra_scripts/`。

### 入口として有用な関数

Ghidra 上の自動名なので再解析で変わる可能性はあるが、現時点の xref は以下。

- `FUN_100030444`: `t6_compress_yuv420`
- `FUN_100030b7c`: `t6_compress_yuv420_gpurle`
- `FUN_100031b55`: `t6_metaljpeg_compress_texture_async`
- `FUN_100032889`: `t6_compress_yuv420_gpu_dctquantrlehuff_whole_image`
- `FUN_1000473ab`: `t6_submit_frame_surface_with_compressed_dirty_rects`
- `FUN_1000487e0`: dirty rect encode completion callback
- `FUN_1000489d7`: VRAM/fence request phase
- `FUN_100048d32`: rect device submission phase
- `FUN_10004947c`: compressed flip command submission
- `FUN_1000b8d6a`: `syncDirtyRectsFromSurface` らしき大きい経路選択関数

### 公式 macOS ドライバの画面送信経路

公式ドライバは単純な ImageIO/JPEG 送信ではない。少なくとも以下の複数経路を持つ。

- VideoToolbox JPEG encoder:
  - `mct_jpeg_videotoolbox_encode`
  - `mct_jpeg_videotoolbox_encoder`
  - `t6_submit_frame_surface_whole_screen_compressed_vt_jpeg`
- Metal / OpenCL 系の T6 専用 JPEG/YUV420 encoder:
  - `t6_metaljpeg_compress_texture_async`
  - `t6_compress_yuv420`
  - `t6_compress_yuv420_gpurle`
  - `t6_compress_yuv420_gpu_dctquantrlehuff_whole_image`
- uncompressed YUV upload:
  - `t6_upload_uncompressed_yuv420`
  - `t6_upload_uncompressed_yuv444`
- dirty rect compressed submission:
  - `t6_submit_frame_surface_with_compressed_dirty_rects`
  - rect ごとに encode completion -> VRAM/fence request -> device submission -> flip submission へ進む。

`syncDirtyRectsFromSurface` らしき関数では、dirty rect がある通常経路で
`t6_submit_frame_surface_with_compressed_dirty_rects` を呼ぶ分岐と、
`this->vt_yuv420_encoder` を使う VideoToolbox 系分岐がある。つまり公式は
「全画面 JPEG を毎回送る」だけではなく、dirty rect と T6 専用 GPU encoder を主経路にしている可能性が高い。

### アラインメントと内部ブロック

公式の T6 専用経路はアラインメント制約が強い。

- `t6_compress_yuv420`
  - `source_region.width() % 16 == 0`
  - `source_region.height() % 16 == 0`
- `t6_metaljpeg_compress_texture_async`
  - `dev_update_rect.width() % 32 == 0`
  - `dev_update_rect.height() % macroblock_size_px == 0`
  - `macroblock_size_px` は引数上 `8 << yuv420_subsample` 相当。
- `t6_compress_yuv420_gpu_dctquantrlehuff_whole_image`
  - `source_region.width() % 64 == 0`
  - `source_region.height() % 16 == 0`

`t6_metaljpeg_compress_texture_async` は coeff / macroblock size / bytestream buffer を分けて扱う。
420 系では macroblock あたりの係数 buffer 使用量が `0xc0 * num_macroblocks * 2`、
bytestream spacing が `0x4e4 * num_macroblocks` らしい。420 でない場合は係数側 `0x180`、
bytestream 側 `0x9c4` へ増える。

### 420 なのに公式/Windows がきれいに見える理由の更新

Windows/IP P JPEG と TurboJPEG q95 は量子化テーブルが一致していた。

- Windows IPP q95: luma sum 369, chroma sum 558
- TurboJPEG q95 420: luma sum 369, chroma sum 558

したがって、Windows がきれいに見える理由は量子化テーブルではなさそう。
公式 macOS の kernel source から見ると、差分候補は以下。

- RGB/BGRX から YUV への変換と 420 downsample の実装差。
- 公式 kernel は Cb/Cr を 2x2 平均している:
  - Y: `0.299 R + 0.587 G + 0.114 B - 128`
  - Cb: `-0.168736 R - 0.331264 G + 0.5 B`
  - Cr: `0.5 R - 0.418688 G - 0.081312 B`
  - Cb/Cr は 4 pixel の合計に係数 `* 0.25f` をかける。
- DCT/quantize を GPU kernel 内で直接行い、係数を zig-zag して RLE/Huffman へ渡している。
- 公式は type4/full frame JPEG だけでなく、type7/dirty rect 風の T6 専用送信経路も持つ。

次に試す価値が高いもの:

1. Rust 側の 420 変換を公式 kernel と同じ 2x2 average / BT.601 係数に寄せる。
2. TurboJPEG に渡す入力を RGB 直接ではなく、公式相当の Y/Cb/Cr plane から作れるか確認する。
3. full frame JPEG の改善だけで限界がある場合、type7 はいったん低優先度でも、公式 dirty rect 送信の構造メモは維持する。

### Ghidra 作業メモ

Ghidra 起動:

```sh
JAVA_HOME=/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home ghidraRun
```

Headless import:

```sh
JAVA_HOME=/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home \
/opt/homebrew/Cellar/ghidra/12.1.2/libexec/support/analyzeHeadless \
  ghidra_projects USBDisplayDriver \
  -import "/Applications/USB Display.app/Contents/Resources/USB Display Device Driver.app/Contents/MacOS/USB Display Device Driver" \
  -overwrite \
  -analysisTimeoutPerFile 1200 \
  -log ghidra_projects/import.log
```

関連文字列 xref:

```sh
JAVA_HOME=/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home \
/opt/homebrew/Cellar/ghidra/12.1.2/libexec/support/analyzeHeadless \
  ghidra_projects USBDisplayDriver \
  -process "USB Display Device Driver" \
  -scriptPath ghidra_scripts \
  -postScript T6StringXrefs.java \
  -noanalysis
```

関数 decompile:

```sh
JAVA_HOME=/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home \
/opt/homebrew/Cellar/ghidra/12.1.2/libexec/support/analyzeHeadless \
  ghidra_projects USBDisplayDriver \
  -process "USB Display Device Driver" \
  -scriptPath ghidra_scripts \
  -postScript T6DecompileAddrs.java 100030444 100031b55 100032889 1000473ab 1000b8d6a \
  -noanalysis
```
