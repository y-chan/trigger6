# macOS prototype path

このディレクトリは macOS 向け実装 path の作業場所。最初の milestone は signed DriverKit display driver ではなく、次を確認する userspace prototype。

1. macOS 上で仮想/副 display 相当の framebuffer を作る、または取得する。
2. framebuffer を JPEG/NV12/YV12 に encode する。
3. encode 済み frame を、Linux/Windows/macOS の vendor driver と同じ USB protocol で Trigger 6 device へ送る。

## 想定構成

```text
Trigger6.app または Trigger6Agent
  - settings / diagnostics
  - virtual display lifecycle
  - ScreenCaptureKit frame source
  - JPEG/NV12/YV12 encode と rotation
  - frame pacing

T6 transport core
  - vendor control request
  - bulk DMA header / video header
  - payload fragmentation
  - interrupt/fence parse
  - VRAM address planning

USB backend
  - 初期 prototype は libusb/rusb
  - 必要なら後で USBDriverKit または DriverKit user client
```

## 仮想ディスプレイ参考実装

現時点で最も参考になる OSS は DeskPad。private `CGVirtualDisplay` API family と `com.apple.VirtualDisplay` entitlement を使っている。prototype には有用だが、public DriverKit display driver と同じものではない。

当面の推奨順:

1. static frame USB sender を作る。
2. ScreenCaptureKit source を追加する。
3. DeskPad 風 virtual display source を追加する。
4. その後で DriverKit display path が必要か判断する。

## 現在の中身

- `t6proto/`: 依存なしの C protocol helper と test。
- `t6proto-rs/`: 依存なしの Rust protocol helper と test。macOS userspace tool の主実装にする。

## Rust USB probe

最初の実 USB tool は `rusb` 依存なので feature gate している。

```sh
cd mac/t6proto-rs
cargo run --features usb --bin t6-probe -- --help
```

default mode は matching する MCT device を列挙するだけ。vendor driver を止めた状態で prototype が最初の `0711:5601` device を open する場合は `--claim` を付ける。

## 静止 JPEG 送信

`t6-send-jpeg` は Ubuntu 公式 driver の 1080p JPEG path に寄せた実験用 sender。

```sh
cd mac/t6proto-rs
cargo run --features usb --bin t6-send-jpeg -- \
  --jpeg /path/to/image.jpg \
  --display 1 \
  --ready \
  --power-on
```

今回の JUA365/VA24D では `display 1` が接続済み。default layout は `58 MB` RAM の 2 port 1080p secondary 想定で、`cmdAddr = (ram - 18) MB`, `fbAddr = (ram - 8) MB` を使う。違う挙動が出る場合は `--layout one-port-1080p` または `--cmd-addr` / `--fb-addr` で上書きする。

## 仮想ディスプレイ送信

`t6-virtual-display` は DeskPad と同じ方向の prototype。macOS 側で private `CGVirtualDisplay` を作り、`CGDisplayStream` で BGRA frame を受け取り、Rust 側で JPEG 4:2:0 に encode して T6 へ送る。

Xcode 26 SDK では Homebrew clang/SDK path の都合で、次の環境変数を付けるのが安全。

```sh
cd mac/t6proto-rs

SDKROOT=/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk \
BINDGEN_EXTRA_CLANG_ARGS="-isysroot /Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk" \
cargo run --features usb --bin t6-virtual-display -- \
  --display 1 \
  --width 1920 \
  --height 1080 \
  --fps 60 \
  --ready \
  --power-on
```

USB へ送らず、仮想ディスプレイ作成と capture/encode/packetize だけ確認する場合:

```sh
SDKROOT=/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk \
BINDGEN_EXTRA_CLANG_ARGS="-isysroot /Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk" \
cargo run --features usb --bin t6-virtual-display -- \
  --dry-run \
  --frames 1 \
  --dump-first-frame /tmp/t6-vd-first.png
```

確認済みの dry-run では、`1920x1080` の仮想 display が作成され、最初の frame を `/tmp/t6-vd-first.png` に保存し、JPEG packet 化まで成功した。

縦置き monitor を横向き timing のまま使う場合は、仮想 display を portrait にして、送信直前に回転する。

```sh
SDKROOT=/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk \
BINDGEN_EXTRA_CLANG_ARGS="-isysroot /Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk" \
cargo run --release --features usb --bin t6-virtual-display -- \
  --display 1 \
  --width 1080 \
  --height 1920 \
  --rotate 90 \
  --fps 60 \
  --ready \
  --power-on
```

`--rotate 90` / `--rotate 270` は送信 frame の width/height を入れ替える。たとえば `--width 1080 --height 1920 --rotate 90` は T6 へ `1920x1080` として送る。
