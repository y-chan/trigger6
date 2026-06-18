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
- Windows では追加ドライバ無しでも動き、Mac のような明示的な画面キャプチャ処理は発生していなさそう。

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
