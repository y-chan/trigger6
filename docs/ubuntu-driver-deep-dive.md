# Ubuntu 公式 T6 ドライバ深掘りメモ

このメモは `extracted/ubuntu_T6_260223/evdi_t6/evdi_t6_1` の公式 Ubuntu ドライバを、macOS/Rust 実装へ反映する観点で読んだもの。

## 全体構成

公式ドライバは大きく次の分担になっている。

- EVDI から画面更新を受ける。
- 画面更新を JPEG または YUV へ変換する。
- 変換済み frame を `jpg_queue` に積む。
- USB 送信スレッドが queue から取り出して T6 へ送る。
- cursor は `HW_CURSOR` 有効時に別 queue / 別スレッドで扱う。

重要なのは、画面取得 callback 内で USB bulk 送信まで完結させていない点。Rust の現状実装は `CGDisplayStream` callback 内で変換と USB 送信まで行っているため、WindowServer 側の callback queue を詰まらせやすい。

## frame queue と backpressure

`main.c` では frame queue 長を見ている。

- 生成側: `queue_length(pt6evdi->jpg_queue) > 5` なら `usleep(10000)` して新しい update 要求を抑える。
- 変換投入側: `queue_length(pt6evdi->jpg_queue) < 5` の場合だけ JPEG/YUV 変換して queue へ追加する。
- USB 側: `queue_length(pt6evdi->jpg_queue) == 0` なら sleep して待つ。

つまり公式は「最新 frame を必ず送る」より、「queue を浅く保って USB 送信側に合わせる」設計。Rust 版も callback で直接送るのではなく、capture thread と sender thread を分離する方が公式に近い。

## JPEG 経路

`t6_libusb_FilpJpegFrame` は次の形。

- bulk header:
  - `Signature = 0`
  - `PayloadLength = jpgsize + 48 + 1024`
  - `PayloadAddress = cmdAddr`
- video flip header:
  - `TargetFormat = VIDEO_COLOR_NV12`
  - `SourceFormat = VIDEO_COLOR_JPEG`
  - `Y_RGB_Pitch = align(width, 32)`
  - `UV_Pitch = align(width, 32)`
  - `Y_RGB_Data_FB_Offset = fbAddr`
  - `U_UV_Data_Offset = fbAddr + align(width,32) * align(height,32) + 1024`
  - `Flag = resetflag`
- payload:
  - 48 byte header
  - JPEG body
  - 1024 byte padding

JPEG は 4:2:0 固定で生成されている。

```c
tjCompress2(..., TJSAMP_420, 95, TJFLAG_FASTDCT)
```

そのため、macOS 版で `quality=100` にしても信号機ボタンの色滲みが残る理由は説明できる。T6 が JPEG 4:2:2 / 4:4:4 を受けないなら、JPEG 経路での根本改善は難しい。

## YV12 / NV12 raw 経路

公式には `t6_libusb_FilpYV12Frame` と `t6_libusb_FilpNV12Frame` がある。

共通点:

- bulk header:
  - `Signature = 0`
  - `PayloadLength = yuv_size + 48 + 1024`
  - `PayloadAddress = fbAddr`
- video flip header:
  - `Y_RGB_Data_FB_Offset = fbAddr + 48`
  - `PayloadSize = len - 48`
- payload:
  - 48 byte header
  - YUV body
  - 1024 byte padding

JPEG と違って `cmdAddr` ではなく `fbAddr` へ直接送る。

### NV12

`t6_libusb_FilpNV12Frame`:

- `TargetFormat = VIDEO_COLOR_NV12`
- `SourceFormat = VIDEO_COLOR_NV12`
- `Y_Pitch = align(width, 16)`
- `UV_Pitch = align(width, 16)`
- `U_UV_Data_Offset = Y_RGB_Data_FB_Offset + Y_Pitch * height`
- `V_Data_Offset = 0`

Rust 版の `--transport nv12` はこの構造に合わせている。

### YV12

`t6_libusb_FilpYV12Frame`:

- `TargetFormat = VIDEO_COLOR_YV12`
- `SourceFormat = VIDEO_COLOR_YV12`
- `Y_Pitch = align(width, 16)`
- `UV_Pitch = align(width / 2, 16)`
- `U_UV_Data_Offset = Y_RGB_Data_FB_Offset + Y_Pitch * height`
- `V_Data_Offset = U_UV_Data_Offset + UV_Pitch * height / 2`

公式の生成側では raw 経路に入ると `tjEncodeYUV2(..., TJSAMP_420, 0)` を使っている。コメントや関数名は `YV12` 寄りだが、実際の memory layout と `tjEncodeYUV2` の出力順は注意が必要。Rust 版で `nv12` と `yv12` の両方が表示できたため、T6 側はどちらの header も受けられる。

## USB 転送方法の差

公式 raw YUV/RGB24 は、libusb bulk transfer を概ね次の2回で送っている。

1. 32 byte bulk header
2. `VIDEO_FLIP_HEADER + frame body + padding` 全体

Rust 版は JPEG と同じ fragment helper で、複数 chunk に分けて `bulk header + chunk data` を繰り返している。

観測:

- `jpeg`: 分割転送で動く。
- `nv12`: 分割転送でも 60fps で動く。
- `yv12`: 分割転送でも動く。
- `rgb24`: chunk 1 の data 後、chunk 2 の header で I/O Error。

推測:

- T6 は raw YUV では複数 chunk を許容するが、RGB24 は firmware 側の扱いが違う可能性がある。
- 公式 RGB24 は初期化/全画面塗り用途に近く、通常動画経路ではない可能性が高い。
- 実用経路としては RGB24 を追うより NV12/YV12 を主経路にする方が良い。

## VRAM アドレス計画

公式は triple buffer を使う。

今回の 58MB / 2port / 1080p secondary 相当では、Rust 版が使っている以下と概ね一致する。

- `cmdAddr = (ram - 18) MB = 0x02800000`
- `fbAddr1 = (ram - 12) MB = 0x02e00000`
- `fbAddr2 = (ram - 8) MB = 0x03200000`
- `fbAddr3 = (ram - 4) MB = 0x03600000`

公式は raw YUV でも `fbAddr` を frame ごとにローテーションする。一方で `cmdAddr` は JPEG 用で、raw YUV では基本的に使わない。

## 回転

公式の JPEG 経路は `tjTransform` で JPEG 自体を rotate している。

- `jpg_rotate == 1`: `TJXOP_ROT90`
- `jpg_rotate == 2`: `TJXOP_ROT180`
- `jpg_rotate == 3`: `TJXOP_ROT270`

raw YUV 経路では `pt6evdi->jpg_rotate = 0; // no rotation first` としており、少なくとも該当箇所では raw YUV の回転を積極的に使っていない。macOS 版の raw NV12/YV12 回転は独自実装。

## macOS/Rust 版へ反映すべき点

優先度高:

1. capture callback と USB sender を分離する。
   - callback は最新 frame 参照だけ更新する。
   - sender thread が fps pacing しながら最新 frame を変換/送信する。
   - 公式の shallow queue と同じ目的で遅延蓄積を避ける。

2. raw YUV を主経路にする。
   - `--transport nv12` を常用候補にする。
   - `jpeg` は fallback / 低帯域用に残す。

3. raw YUV の転送形式を公式に寄せる実験を追加する。
   - 現状の分割転送でも NV12/YV12 は動く。
   - ただし公式は bulk header 1回 + payload 1回。
   - macOS/libusb が大きい transfer を安定して扱えるか `--raw-single-bulk` のような実験 option を作る価値はある。

優先度中:

4. YUV 変換の高速化。
   - 現状の Rust scalar 変換は十分動くが、60fps では重い場面がある。
   - SIMD/NEON 化、または Accelerate/vImage/CoreImage/Metal 経由を検討する。

5. 色差処理のモード調整。
   - 4:2:0自体は避けられない。
   - `--chroma-mode saturated` は UI の小さい高彩度色には効く可能性があるが、副作用もある。

優先度低:

6. RGB24 経路。
   - 公式に関数はあるが、通常 frame path ではなさそう。
   - 1 frame 約6MBで帯域も厳しい。
   - 現状では chunk 2 header で I/O Error のため、深追い優先度は低い。

## 結論

公式 Ubuntu ドライバから見ると、Mac 版の本命は `NV12/YV12 raw + shallow queue + sender thread`。JPEG 4:2:0 の画質限界は公式実装にも存在するため、信号機ボタンの滲みを改善するなら JPEG ではなく raw YUV 経路を磨くべき。

次の実装タスクは、`CGDisplayStream` callback 内で USB 送信しない構造への変更。これはフレーム落ち対策として公式実装に最も近い。
