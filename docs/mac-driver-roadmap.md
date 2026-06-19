# macOS Trigger 6 実装ロードマップ

macOS 向け実装では、まず「画面をどこから取得するか」と「Trigger 6 へ USB でどう送るか」を分けて考える。最初から DriverKit の表示ドライバを作るのではなく、ユーザー空間で USB 転送とフレーム生成を検証してから判断する。

## 段階 1: USB 転送の実証

目的は、ユーザー空間のコマンドラインツールで USB デバイスを確保し、既知の 1 フレームを送れる状態にすること。

範囲:

- `0711:5601` を確保する。
- 必要な vendor control request を送る。
  - software ready `0x31`
  - monitor status `0x87`
  - EDID `0x80`
  - video RAM size `0x88`
  - monitor power `0x03`
  - timing setup `0x12` または resolution index `0x08`
- 公式 `T6BULKDMAHDR` layout に従って display bulk transfer を送る。
- interrupt endpoint `0x83` から display fence と JPEG error event を読む。
- 静止 JPEG フレームを upload する。

依存なしの protocol core は `mac/t6proto-rs` から始める。低レベル比較や C/DriverKit 実験用に `mac/t6proto` の C 版も残す。最初の USB 向け tool は `t6-probe`。

```sh
cd mac/t6proto-rs
cargo run --features usb --bin t6-probe -- --claim
```

## 段階 2: フレーム取得元の実証

macOS 上でフレームを取得し、段階 1 の転送 prototype に渡す。

推奨順:

1. 静的に生成したフレーム。
2. 画像ファイル。
3. ScreenCaptureKit によるメインディスプレイの mirror。
4. `CGVirtualDisplay` を使う DeskPad 風の仮想ディスプレイ取得元。

この段階はユーザー空間のまま進める。JPEG encode、回転、フレーム間隔制御、USB scheduling は DriverKit 外で debug する方が圧倒的に扱いやすい。

## 段階 3: 仮想ディスプレイ prototype

仮想ディスプレイの OSS 参考実装としては DeskPad が最有力。想定する prototype の流れは次の形。

```text
CGVirtualDisplay
  -> ScreenCaptureKit frame stream
  -> BGRA frame buffer
  -> TurboJPEG JPEG または NV12/YV12
  -> T6 bulk transfer
  -> interrupt fence wait
```

リスク:

- `CGVirtualDisplay` は private API 寄り。
- entitlement の扱いが通常の public app entitlement と違う。
- ローカル prototype には有用でも、再配布可能な driver として成立するとは限らない。

## 段階 4: DriverKit 判断

DriverKit/System Extension の display path は、ユーザー空間 prototype で USB 転送とフレーム処理 pipeline が成立してから検討する。

確認すべきこと:

- public な DriverKit display extension で必要な表示挙動を出せるか。
- 必要な entitlement が何で、それが取得可能か。
- USB device を DEXT が所有しつつ、フレーム encode をユーザー空間 agent に残せるか。
- install / signing / notarization の実運用フロー。

## 段階 5: 性能と回転

まず正しさを優先し、その後で最適化する。

- full-frame JPEG
- TurboJPEG transform による JPEG 回転
- USB3 / 高解像度向け NV12/YV12 path
- dirty rectangle upload
- VRAM allocator
- fence に基づく queue depth 制御
- USB2 / USB3 での送信方式切替

## 難易度マップ

目標は protocol を完全解明することではない。まず 1 つの表示 path を安定させ、必要になった部分だけ広げる。

### 低から中程度

#### USB protocol probe

取り組みやすい理由:

- vendor request は Linux、macOS 静的解析、capture からかなり特定できている。
- `rusb` で同じ control transfer をユーザー空間から送れる。
- 失敗は `busy`、`timeout`、`pipe` error として見えることが多い。

残るリスク:

- 公式 macOS DEXT が USB device を排他的に掴む可能性がある。
- request 順序への依存は実機で初めて見える可能性がある。

#### 静止 JPEG フレーム upload

取り組みやすい理由:

- Ubuntu 公式実装に `t6_libusb_FilpJpegFrame()` がある。
- bulk DMA header layout が分かっている。
- `VIDEO_FLIP_HEADER` layout が分かっている。
- JPEG source format、NV12 target format、1024 byte padding、`cmdAddr` の使い方が vendor code に出ている。

残るリスク:

- `cmdAddr` / `fbAddr` 初期化は device RAM size と port layout に合わせる必要がある。
- 最初の数フレームでは JPEG reset flag の挙動が効く可能性がある。

#### 1080p single-output full-frame display

取り組みやすい理由:

- 最も単純な既知 path である full-frame JPEG を使える。
- 1 port 1080p の VRAM layout は Ubuntu source にコメントと実装がある。
- dirty rect、multi-output 調停、4K bandwidth 問題を避けられる。

残るリスク:

- 連続表示では queue 制御と error recovery が必要。
- macOS フレーム取得と USB scheduling で latency spike を抑える必要がある。

### 中程度

#### ScreenCaptureKit mirror source

中程度で済む理由:

- ScreenCaptureKit は public API。
- true display driver を作らなくても、取得したフレームを encode して送れる。

簡単ではない理由:

- 権限と初回 UX が必要。
- pixel format、timing、backpressure を丁寧に扱う必要がある。
- mirror は本物の extended desktop ではない。

#### software rotation

中程度で済む理由:

- Ubuntu 公式実装でも TurboJPEG transform による JPEG 回転を使っている。
- full-frame update なら 90/180/270 度の考え方は単純。

簡単ではない理由:

- 回転後の width/height、pitch、cursor 座標、EDID/mode selection を揃える必要がある。
- dirty rect 対応時は rect の変換が難しくなる。

#### dirty rectangle / clip upload

中程度で済む理由:

- `VIDEO_CMD_CLIP_PRIMARY` / `VIDEO_CMD_CLIP_SECONDARY` が定義されている。
- Linux の EVDI は dirty rect を返すので、vendor がこの pipeline を想定していた可能性が高い。

リスクが残る理由:

- clip payload の正確な挙動は実機検証が必要。
- 回転時は rect mapping が変わる。
- rect と fence の順序を誤ると表示崩れが起きる。

### 中から高

#### 4K30

難しくなる理由:

- monitor EDID、T6 timing table、T6 4K capability、USB SuperSpeed の全てに依存する。
- Ubuntu 公式実装でも build option と USB speed によって JPEG / YV12 / NV12 strategy が変わる。
- VRAM 領域が大きくなり、余裕が減る。

主なリスク:

- bandwidth と encoder latency。
- mode programming の正確性。
- 大きな payload fragmentation と転送失敗時の recovery。

#### fence-based frame queue

難しくなる理由:

- interrupt event format は分かっているが、正確な queue-depth policy は仕様化されていない。
- overload 時の firmware behavior は実機で詰める必要がある。

主なリスク:

- 待ちが少なすぎると buffer を早く上書きしすぎる。
- 待ちが多すぎると latency が増えるか stall する。
- JPEG decoder error 時の recovery 方針が必要。

### 高

#### two-output support

難しい理由:

- VRAM を port 間で分割する必要がある。
- output capability が port ごとに違う。
- 4K-capable output と 1080p-only output で allocation strategy が違う。
- USB bandwidth、frame queue、timing を 2 display で調停する必要がある。

現実的な進め方:

- single-output が安定するまで後回し。
- 既知の two-port device layout を 1 つずつ追加する。

#### macOS の本物の仮想ディスプレイ / extended desktop

難しい理由:

- 簡単な OSS path は private `CGVirtualDisplay` API と特殊な entitlement に依存する。
- 再配布可能な DriverKit display path には、一般取得できない entitlement が必要な可能性がある。
- display source の問題は USB transport の問題とは別。

現実的な進め方:

- まず ScreenCaptureKit mirror。
- 次に DeskPad 風 virtual display prototype。
- prototype が有用で entitlement path が現実的な場合だけ DriverKit を検討する。

#### DriverKit/System Extension による USB ownership

難しい理由:

- 開発中は userspace `rusb` が簡単だが、製品化では DEXT が必要になる可能性がある。
- DEXT install、承認、signing、user-client IPC が大きな実装面積を持つ。
- 公式 DEXT と同じ USB device を取り合う。

現実的な進め方:

- transport core はまず userspace から使える状態を維持する。
- 必要になった時だけ、USB ownership の最小層を DriverKit に移す。

### 非常に高い / 初期 scope 外

#### 公式 driver parity

非常に難しい理由:

- 全 mode、全 port、USB2/USB3 behavior、hotplug、sleep/wake、cursor、dirty rect、error recovery、device-specific quirk が必要。
- closed な macOS/Windows driver には Ubuntu source にない挙動が含まれる可能性がある。

#### audio

別扱いにすべき理由:

- audio は別の vendor request、session/signature、engine state、timing constraint を持つ。
- display path の実証には不要。
- audio と display を同時に debug すると failure mode が増えすぎる。

#### protocol / firmware の完全理解

良い目標ではない理由:

- firmware 内部は見えない。
- 一部 field は古い製品や未使用 path 向けの可能性がある。
- この project に必要なのは安定した表示 path であり、歴史的 command の完全仕様ではない。

実用上の最初の目標:

```text
single output -> 1080p -> USB3 -> full-frame JPEG -> software rotation
```

それが動いてから広げる対象:

```text
virtual display source -> frame pacing/fence -> 4K or dirty rects
```
