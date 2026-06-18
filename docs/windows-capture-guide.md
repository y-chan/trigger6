# Windows / Mac handoff guide for JUA365 / Trigger6

JUA365 が Windows で追加ドライバ無しに動くなら、Windows 側の USB traffic は protocol 確認にかなり有用。Mac のような仮想ディスプレイ転送とは上位の作りが違っても、USB device に流れる control / bulk / interrupt は同じ T6 系 protocol のはず。

## Working environment policy

このプロジェクトの作業環境は、当面 **Windows または Mac を優先**する。

最終的にやりたいことは、既存 Mac ドライバが対応していない画面回転に対応した新規ドライバを書くこと。ただし最終ゴールは Mac 専用ではなく、Mac / Windows / Linux を含む all-platform な T6 display stack。

当面 Mac を優先する理由は、ユーザー側の一番強い課題が「Mac で回転できない」ことだから。Windows は USBPcap による protocol capture 環境として強く、Mac は最終ターゲット兼静的解析対象として強い。Linux は既存実装と usbmon capture があるため protocol 検証に有用で、最終的には Linux DRM/KMS 実装にも戻したい。

各環境の役割:

- Windows
  - USBPcap / Wireshark で公式または標準ドライバ動作中の USB traffic を取る。
  - 追加ドライバ無しで動く経路を観察し、T6 control / bulk / interrupt protocol を確認する。
  - 将来的には Windows 側 display capture / indirect display driver / USB transport 実装の候補も検討する。
- Mac
  - 既存 MCT DriverKit / user-space driver を静的解析する。
  - 最初の実装ターゲット。仮想ディスプレイ、回転、encode、T6 USB transport を再実装する候補。
- Linux
  - 既存 `trigger6` リポジトリと `usbmon` capture は protocol 解析の参考にする。
  - 最終的には Linux DRM/KMS driver として同じ T6 protocol core を使うのが理想。

設計方針:

- T6 control / bulk / interrupt protocol の知識は OS 非依存の core として整理する。
- Mac / Windows / Linux で違うのは、仮想ディスプレイ作成、画面取得、USB device 取得、権限、driver packaging。
- まずは Mac の回転対応を最短で進めるが、protocol notes と capture tools は Windows/Linux でも再利用できる形に保つ。

つまり、当面の引き継ぎ時は「WindowsでUSB trafficを取る」か「Macで静的解析/新規実装を進める」のどちらかを優先する。ただし成果物は Linux 実装にも戻せる形で残す。

## 目的

確認したいこと:

- control request の実引数
- bulk video command phase 32 byte の正確な構造
- video payload header と pixel/compression format
- multi-fragment 時の flag と offset
- HDMI output index の扱い
- hotplug 時の interrupt packet
- Windows が使っている driver の実体

加えて、Windows では任意の MP4 を JUA365 側ディスプレイで再生するデモも候補にする。

この MP4 デモの位置づけは2通りある。

- 解析用デモ
  - 既存 Windows 表示経路で MP4 を全画面再生し、その USB traffic を USBPcap で取る。
  - 動きの多い映像により、JPEG payload、dirty rect、full-frame update、fence、flip の挙動を観察しやすくする。
- 解析後デモ
  - T6 protocol が固まった後、自前実装で MP4 を decode して JUA365 に直接送る。
  - all-platform display stack の分かりやすい成果物として使う。

もし既存 Windows 経路の capture が解析しやすければ前者を優先する。Windows 側が抽象化されすぎて解析に向かない場合でも、後者の「解析後の任意動画再生デモ」として残す。

## 事前準備

- Windows PC
- JUA365
- HDMI monitor 1 台以上
- 可能なら 2 台目の HDMI monitor
- Wireshark
- USBPcap
- USB Device Tree Viewer または同等の USB descriptor 確認ツール

画面内容が capture に含まれる可能性があるので、個人情報や通知が出ない状態で作業する。検証用の単色画像やテストパターンだけを表示するのが安全。

## Driver 情報の記録

まず Windows で JUA365 を接続し、デバイスマネージャで以下を記録する。

- 表示アダプター、USB デバイス、モニター、オーディオデバイスのどこに出るか
- device name
- hardware id
- compatible id
- driver provider
- driver version
- driver date
- driver files
- Windows version

「追加ドライバ無し」でも、Windows Update 経由で自動取得された driver が入っている場合がある。`driver provider` と `driver files` が重要。

特に確認したい点:

- driver provider が Microsoft か MCT/j5create 系か
- driver file 名に `mct`, `trigger`, `usbdisplay`, `udl`, `displaylink` などが含まれるか
- 表示アダプターとして見えているのか、USB device としてだけ見えているのか
- Windows の「設定 > ディスプレイ」に通常の外部ディスプレイとして出ているか

Windows では Mac のような screen recording consent / virtual display capture が見えないとのことなので、driver 情報は特に重要。Windows 側が OS の display pipeline により近い位置で frame を受け取っていても、USBPcap では最終的な T6 USB 転送を観察できるはず。

## USB descriptor の保存

USB Device Tree Viewer などで JUA365 の descriptor を保存する。

記録したい項目:

- VID/PID
- manufacturer / product / serial
- USB speed
- configuration
- interface
- endpoint address
- endpoint type
- max packet size

保存ファイル名例:

```text
2026-xx-xx_jua365_windows_descriptors.txt
```

## USBPcap の基本方針

Wireshark 起動時に USBPcap interface が複数出る。JUA365 が接続されている host controller を選ぶ。

確実に取るなら、capture 開始後に JUA365 を挿す。これで enumeration から control request まで残る。

推奨 display filter:

```text
usb.idVendor == 0x0711 && usb.idProduct == 0x5601
```

Wireshark の版によって field 名が違うことがある。うまく filter できない場合は、まず USB device address を特定してから `usb.device_address == N` で絞る。

control request を見る filter 例:

```text
usb.setup.bRequest == 0x80 || usb.setup.bRequest == 0x87 || usb.setup.bRequest == 0x89 || usb.setup.bRequest == 0x12 || usb.setup.bRequest == 0x03
```

bulk/interrupt は endpoint address や transfer type で絞る。field 名が合わない場合は Wireshark の packet detail に出ている名前を使う。

## Capture scenario

各 scenario は短めに分けて保存する。巨大な連続 capture にすると後で解析しづらい。

### 1. Enumeration only

1. JUA365 を抜く。
2. Wireshark / USBPcap capture を開始する。
3. JUA365 を挿す。
4. HDMI は未接続のまま 10 秒待つ。
5. capture を停止して保存する。

目的:

- descriptor
- 初期 control request
- software ready らしき `0x31`
- monitor 未接続時の status

ファイル名例:

```text
2026-xx-xx_jua365_win_01_enumeration_no_hdmi.pcapng
```

### 2. HDMI hotplug

1. JUA365 を接続済みにする。
2. capture を開始する。
3. HDMI monitor を port 1 に接続する。
4. 10 秒待つ。
5. HDMI monitor を抜く。
6. 5 秒待つ。
7. capture を停止する。

目的:

- interrupt packet
- connector status request `0x87`
- EDID request `0x80`
- mode table request `0x84`, `0x85`, `0x89`

ファイル名例:

```text
2026-xx-xx_jua365_win_02_hdmi1_hotplug.pcapng
```

### 3. Resolution change

1. HDMI monitor を接続する。
2. capture を開始する。
3. Windows の display settings で 1920x1080 60 Hz にする。
4. 5 秒待つ。
5. 可能なら別解像度に変える。
6. capture を停止する。

候補:

- 1920x1080 60 Hz
- 1280x720 60 Hz
- 3840x2160 30 Hz, 対応している場合

目的:

- `0x08` set resolution by index
- `0x12` detailed timing
- mode table と実設定値の対応

ファイル名例:

```text
2026-xx-xx_jua365_win_03_resolution_change.pcapng
```

### 4. Solid color full-screen

1. HDMI monitor を接続する。
2. capture を開始する。
3. 外部ディスプレイに黒一色を全画面表示する。
4. 3 秒待つ。
5. 白一色を全画面表示する。
6. 3 秒待つ。
7. 赤、緑、青も同様に表示する。
8. capture を停止する。

目的:

- pixel format の推定
- full-frame payload の header 確認
- compression 有無の確認

ファイル名例:

```text
2026-xx-xx_jua365_win_04_solid_colors.pcapng
```

### 5. Partial update

1. HDMI monitor を接続する。
2. capture を開始する。
3. 黒背景の上で小さい白い window または四角形だけを動かす。
4. 5 秒ほど記録する。
5. capture を停止する。

目的:

- partial update packet
- dirty rectangle
- video type `4` / `7` らしき payload

ファイル名例:

```text
2026-xx-xx_jua365_win_05_partial_update.pcapng
```

### 5b. MP4 playback

任意の MP4 を JUA365 側ディスプレイで再生する。

1. HDMI monitor を接続し、JUA365 出力を Windows の外部ディスプレイとして認識させる。
2. 個人情報が映らない検証用 MP4 を用意する。
3. capture を開始する。
4. MP4 を JUA365 側ディスプレイで全画面再生する。
5. 10-30 秒程度で capture を停止する。

目的:

- 動画更新時の JPEG payload サイズ分布
- full-frame と partial update の比率
- dirty rect / tile update の有無
- fence interrupt の頻度
- type `0x4` / type `0x7` の関係

ファイル名例:

```text
2026-xx-xx_jua365_win_05b_mp4_playback.pcapng
```

メモに残すもの:

- MP4 の解像度 / fps / codec
- 再生時の Windows display mode
- 全画面再生か window 再生か
- JUA365 側の表示解像度 / refresh rate

解析上の注意:

- MP4 再生は traffic が多くなりやすいので、長時間取りすぎない。
- 圧縮後 payload は映像内容に依存するため、まずは単色/partial update capture のほうが protocol field の切り分けには向く。
- ただし実運用に近い挙動、frame pacing、fence 頻度を見るには MP4 capture が有用。

### 6. Dual output

2 台の HDMI monitor がある場合だけ実施する。

1. capture を開始する。
2. HDMI 1 を接続する。
3. 10 秒待つ。
4. HDMI 2 を接続する。
5. 10 秒待つ。
6. Windows で extend / duplicate を切り替える。
7. capture を停止する。

目的:

- output index の確認
- port 1 / port 2 の status bit
- EDID と mode table の output index

ファイル名例:

```text
2026-xx-xx_jua365_win_06_dual_output.pcapng
```

### 7. HDMI audio

必要なら実施する。

1. Windows の音声出力を JUA365 / HDMI audio に切り替える。
2. capture を開始する。
3. 短いテスト音を再生する。
4. capture を停止する。

目的:

- audio session
- video session との bulk command phase 差分

ファイル名例:

```text
2026-xx-xx_jua365_win_07_audio.pcapng
```

## 保存時のメモ

各 capture と一緒に、以下を短くメモする。

- Windows version
- JUA365 VID/PID
- USB port が USB 2 / USB 3 どちらか
- hub 経由か直挿しか
- monitor model
- HDMI port 番号
- 解像度 / refresh rate
- extend / duplicate / single display
- driver provider / version
- 実施した操作

例:

```text
file: 2026-xx-xx_jua365_win_04_solid_colors.pcapng
device: JUA365, 0711:5601
windows: Windows 11 xxH2
driver provider: ...
driver version: ...
usb: USB 3 direct port
monitor: ...
mode: 1920x1080 60 Hz, extend
steps: black 3s, white 3s, red 3s, green 3s, blue 3s
```

## 受け渡し

`.pcapng` は大きくなりやすいので、scenario ごとに分けて `pcapng.gz` などで圧縮する。

最低限ほしいもの:

- enumeration no HDMI
- HDMI hotplug
- resolution change
- solid colors
- partial update
- MP4 playback, 解析に使えそうなら
- driver details
- USB descriptors

dual output と audio は後回しでもよい。MP4 playback は capture サイズが大きくなりやすいので、基本セットを取った後に短時間だけ取る。

## After capture: local summary

このリポジトリに capture を置いたら、まず以下で概要を見る。

```sh
python3 tools/t6_pcap_summary.py captures/<capture>.pcapng --summary-only
```

詳細を見る場合:

```sh
python3 tools/t6_pcap_summary.py captures/<capture>.pcapng
```

見るポイント:

- `command_count session=0`: video bulk command
- `command_count session=3`: audio らしき bulk command
- `video_count type=0x4 format=0xd`: JPEG payload
- `video_count type=0x7`: flip/partial update 候補
- `interrupt_count flags=0x04 event=0x04`: fence ID らしき video interrupt
- command の `more=1` が multi-fragment で出るか
- `jpeg=WxH` が外部ディスプレイ解像度や dirty rect とどう対応するか

Windows capture でここが Mac/既存 capture と一致すれば、Linux 実装は OS 側の違いより T6 USB protocol 再現に集中できる。一致しない場合は、Windows 固有の別経路がある可能性を優先して見る。

MP4 capture を解析するときは、まず summary で `video_count`, `command_count session=0`, `interrupt_count` を見る。詳細行で JPEG size と fence ID が frame ごとに自然に増えるなら、解析用 capture として使える。payload が大きすぎて field の切り分けが難しい場合は、MP4 capture は後のデモ確認用として扱い、protocol 解析は solid color / partial update / resolution change を優先する。
