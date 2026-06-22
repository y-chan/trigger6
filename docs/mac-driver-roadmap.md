# macOS Trigger 6 実装ロードマップ

このロードマップは、2026-06 時点の実機検証結果を反映したもの。目的は protocol の完全解明ではなく、Mac で 1080p 60 Hz 相当の仮想ディスプレイを安定表示し、公式 Mac ドライバにない回転対応を実用化すること。

## 現在地

すでにできていること:

- `0711:5601` の検出、interface claim、EDID、monitor status、video RAM size の取得。
- HDMI 接続先の EDID 読み出し。
- 静止画、PNG などの入力画像を T6 に送信。
- MP4 を decode して送信。
- `CGVirtualDisplay` + `CGDisplayStream` による DeskPad 風の仮想ディスプレイ prototype。
- cursor 表示。
- 90 度回転表示。
- `jpeg`, `nv12`, `yv12`, `yuv444` raw transport の実験。
- JPEG 4:4:4 + T6 target `yuv444` で、JPEG 4:2:0 由来の信号機ボタン滲みを大きく改善。
- USB 転送の header/data 時間を分けた profile。
- capture callback と sender を分ける `--async-send` 実験。
- TurboJPEG direct BGRA encode と vImage 回転による変換時間削減。
- `CGDisplayStreamUpdateGetRects` による dirty rect 観測。
- dirty rect 個別 tile の crop + JPEG encode probe。
- `Type7JpegTilePacket` / `Type7JpegTileHeader` の protocol builder。
- `--reset-jpeg-engine` による復旧用 vendor request。

現在の主な問題:

- 全画面 JPEG 4:4:4 encode が重く、平均は 60 fps 圏内でも `encode=100ms+` 級のスパイクが出る。
- 画面全体が大きく変わる場合に encode 負荷が跳ねる。
- adaptive quality は平均負荷には効くが、長いスパイクを根本的には消せない。
- raw `nv12` / `yv12` は速いが、4:2:0 のため UI の赤/緑/青の細部が滲む。
- `rgb24` は大きく、連続転送で I/O error が出やすい。
- `yuv444` raw は画質は良いが帯域が大きく、常用 path としては厳しい。
- type 7 tile 実送信は、部分的な色ズレやデバイス側 stall を起こした。現時点では実送信 path を止め、probe / builder のみに戻している。
- dirty bounding box 1枚方式は、離れた小矩形を巨大 bbox にまとめるため効率が悪く、表示更新にも不安定さが見えた。

現時点の判断:

- 画質優先の本命は `jpeg --subsamp 444 --jpeg-target yuv444`。
- 速度優先の fallback は `nv12` または `yv12`。
- dirty rect 個別 tile encode は非常に有望。実測では full-frame JPEG が数百 KB / encode spike 大なのに対し、tile probe は数 KB から十数万 byte 程度、encode も軽い。
- 次の最重要課題は type 7 実送信を急ぐことではなく、Windows capture から「type 7 tile がどの表示面へ、どの手順で反映されるか」を詰めること。

## 短期ロードマップ

### 1. dirty rect 観測

目的は、macOS の仮想ディスプレイ更新が実際にどの程度の矩形として通知されるかを測ること。

実装済み:

- `CGDisplayStreamUpdateGetRects(update_ref, kCGDisplayStreamUpdateDirtyRects, &count)` を Objective-C bridge に追加した。
- Rust callback へ dirty rect count、bounding box、dirty area、個別 dirty rect 配列を渡している。
- `--profile` に以下を追加した。
  - dirty rect count
  - dirty area ratio
  - bounding box width/height
  - full-frame 扱いになった回数
- `--dirty-mode log|bbox|tile-send` で dirty rect の測定を切り替えられる。

実測から分かったこと:

- 通常操作では dirty area が小さいことが多く、tile JPEG 化の価値は高い。
- ただし fullish update も混ざるため、全面更新時は full-frame fallback が必要。
- dirty bbox は dirty area より大きくなりやすい。特に離れた小矩形があると、bbox 方式は実効面積を大きく見積もる。

### 2. dirty rect を使った encode 範囲削減

目的は、送信 protocol をまだ変えずに、CPU 負荷と payload がどこまで下がるかを見ること。

実装済み:

- `--dirty-mode bbox` で dirty rect tile の crop + JPEG encode probe を行う。
- `tile_probe avg_ms convert/encode`, `avg_payload`, `max_payload` を profile に出す。
- `bbox` という名前は残っているが、現状の実体は個別 dirty rect の tile encode probe。

実測:

- full-frame JPEG は `300KB` から `400KB` 程度になりやすく、`encode=100ms+` の spike が出る。
- tile probe は小更新で数 KB、重めの更新でも十数万 byte 程度に収まることが多い。
- tile probe encode は平均数 ms 以下になりやすく、full-frame よりかなり軽い。
- 一方で probe自体も追加処理なので、profile時の総負荷には注意する。

### 3. Windows 1080p 型 `type=0x7` JPEG tile path

目的は、Windows capture で見えている 1080p の dirty/tile JPEG upload を Mac 実装に移植すること。

根拠:

- Windows 1080p capture では `type=0x7 format=0x0d` が多く、`64x1080`, `128x576`, `1824x96` などの JPEG tile が見えている。
- `width_field` / `height_field` は JPEG SOF と一致する。
- 公式 Mac user-space driver 文字列にも `t6_submit_frame_surface_with_compressed_dirty_rects` がある。

実装済み:

- Rust protocol core に type 7 JPEG tile packet builder を追加した。
- `t6-send-type7` を追加した。通常の virtual-display path から切り離した、1 tile だけ送る危険な実験用ツール。
- header fields は capture 由来の layout を使う。
  - `w0 = 0x7`
  - `w1 = jpeg_len + 0x30`
  - `w4 = height << 16 | width`
  - `w5 = canvas_height << 16 | canvas_width`
  - `w6/w7 = VRAM start/end`
  - `w9 = 0x0d`
- `--dirty-mode tile-send` で一度 type 7 実送信を試した。
- 実送信は部分的な色ズレと device stall を起こしたため、現在は送信を止め、probe専用へ戻している。

分かったこと:

- type 7 header の形だけでは不十分。tile後に必要な flip/fence/commit 手順、または target surface の選び方が未解明。
- `w6/w7` は単純な screen coordinate ではなく、VRAM allocator / surface state / fence と結びついている可能性が高い。
- Windows 1080p capture では、1更新が複数の type 7 tile に分割されることがある。例: `64x96`, `1824x96`, `64x1016` が約 1.6ms 内に連続する。
- `start_addr/end_addr` は tile サイズと独立して再利用される。`0x1fe000` span の pair が多数の JPEG tile サイズに現れるため、tile-local byte range ではなく target surface / VRAM zone を示す可能性が高い。
- pcap で見える type 7 tile の後には interrupt `event=0x04` が返る例があり、sequence/fence ack として扱う必要がありそう。
- Mac 実機でも `VideoFlipHeader.fence_id` を非ゼロにすると interrupt packet offset `0x0c` に同じ値が返ることを確認した。
- full-frame JPEG path では ack が1 frame程度遅れて返ることが多い。`target_data` と `last_data` の差は通常 `ack_lag=1` と見てよいが、初回や重い frame では大きくなる。
- device stall を起こすため、type 7 実送信は安全策なしに繰り返さない。
- 回転時は dirty rect と VRAM offset の対応が入れ替わる。

次に必要なこと:

- `tools/t6_type7_timeline.py` で Windows capture の type 7 周辺に出る command / interrupt / fence を時系列で見る。
- `cmd_dest`, bulk payload address, header `start_addr/end_addr`, fence ID の関係を整理する。
- type 7が「表示中surfaceへ直接patch」なのか、「別surface/VRAM payload zoneへuploadして別commandでcommit」なのかを切り分ける。
- 実送信再開時は、Windows capture に近い固定 tile set から始める。単発 tile ではなく、同一 group の複数 tile + 1 frame 遅れの interrupt ack 確認を再現する。
- 最初の実験は `--frames 1`、固定背景、device reset/unplug 前提、直後に `--reset-jpeg-engine` できる状態で行う。
- `t6-send-type7` の初期値は Windows capture の代表例に寄せている。
  - `64x1080`
  - `canvas=1920x1920`
  - `start=0x30`
  - `end=0x1fe030`
  - `payload_addr=0x02d00000`
- `t6-send-type7 --image` は、crop 指定なしだと画像全体を tile サイズへリサイズする。細長い tile では画像が潰れて見えるため、表示位置確認には `--crop-x/--crop-y` で元画像から tile サイズを切り出す。
- 色ズレ確認用に `--subsamp 420|422|444` と `--quality` を切り替えられる。まずは Windows capture に近い `--subsamp 420` と、Mac full-frame で綺麗だった `--subsamp 444` を比較する。

### 4. full-frame path の安定化

dirty rect と並行して、現行の全画面 path も fallback として維持する。

作業:

- `jpeg 444 -> yuv444` の推奨引数を固定する。
- `quality 85` 付近を default 候補にする。
- `--drop-late-frames` は「遅延蓄積を避ける」目的で残す。
- async sender の callback copy 時間を継続測定する。
- encode spike が TurboJPEG 側か input frame lock/IOSurface 側かを切り分ける。

判断:

- full-frame JPEG 4:4:4 単体で安定 60 fps は厳しい前提で見る。
- ただし dirty rect が効かない場面の fallback としては必要。

### 5. raw YUV fallback

目的は、品質より滑らかさを優先するモードを残すこと。

候補:

- `nv12`: CGDisplayStream から直接 `420f/420v` 取得できるため、変換を減らせる。
- `yv12`: T6 側で動作確認済み。
- raw `yuv444`: 画質は高いが帯域が大きいため、限定用途。

方針:

- `nv12` / `yv12` は低負荷 fallback。
- UI の色滲みが問題になる通常利用では JPEG 4:4:4 を優先。
- 全面動画など、画質劣化が目立ちにくく encode 負荷が高い場面では raw YUV へ切り替える余地を残す。

## 中期ロードマップ

### fence / queue 制御

公式 Ubuntu ドライバは capture callback 内で USB 送信まで完結させず、浅い queue と sender thread で backpressure をかけている。Rust 版もこの方向に寄せる。

作業:

- interrupt endpoint `0x83` の fence ID を送信 frame と対応付ける。
- queue depth を 1 から 3 程度で制御する。
- JPEG decoder error interrupt 時に JPEG engine reset / frame drop / resync を行う。
- type 7 実験時は、JPEG error / fence interrupt を必ず同時に見る。

難しい理由:

- interrupt event format はある程度見えているが、正確な queue policy は仕様化されていない。
- 待ちすぎると latency が増え、待たなすぎると VRAM 上書きや decoder error の原因になる。

### 回転と dirty rect の統合

現行の 90 度回転は full-frame では成立している。dirty rect 対応後は rect mapping が追加で必要。

作業:

- `Deg90`, `Deg180`, `Deg270` それぞれで dirty rect 座標を出力座標へ変換する。
- tile の width/height と VRAM offset を回転後の canvas に合わせる。
- cursor 合成/表示位置が破綻しないか確認する。

難しい理由:

- full-frame 回転と違い、部分矩形は crop、rotate、placement の順序を間違えると斜め崩れや位置ずれになる。

### encoder 改善

候補:

- TurboJPEG の thread-local compressor / preallocated output buffer を維持する。
- tile を複数 worker で並列 encode する。
- VideoToolbox JPEG は probe 済みだが、sampling / latency / hardware support が不安定なら主経路にしない。
- Metal/CoreImage で BGRA crop/rotate/color conversion を GPU 側に寄せる。

判断:

- tile probe で「全画面を encode しない」効果は確認できた。
- 先に type 7 の正しいcommit手順を解く。
- 正しい送信手順が分かった後で、tile encode 並列化やGPU cropを追加する。

## 長期ロードマップ

### DriverKit / System Extension

当面は userspace `rusb` で進める。製品化や常駐化が必要になった場合だけ、USB ownership を DriverKit へ移す。

確認事項:

- public entitlement で成立するか。
- 公式 Mac DEXT と競合しない install / uninstall 手順。
- user-space encode process と DEXT の IPC。
- sleep/wake、hotplug、crash recovery。

### 4K path

4K は 1080p JPEG tile path と別物として扱う。

現時点の仮説:

- Windows 4K only capture では `session=7` の raw NV12 rectangle らしき payload が見えている。
- `total_len == width * height * 3 / 2` が成立する。
- 1080p の `session=0 type=0x7 JPEG tile` とは別系統。

方針:

- 1080p の安定化を先に行う。
- 4K は high-snaplen capture を増やして、rect placement / destination VRAM / fence を別途解く。

### two-output support

single-output が安定するまで後回し。

難しい理由:

- VRAM を port 間で分割する必要がある。
- 4K-capable output と 1080p-only output で allocation strategy が違う。
- USB bandwidth と frame queue を 2 display で調停する必要がある。

## 難易度マップ

### 低から中

- USB probe / EDID / monitor status。
- 静止 JPEG upload。
- 1080p single-output full-frame JPEG。
- CGVirtualDisplay prototype。

理由:

- 実機で動作確認済みの部分が多い。
- 失敗時も timeout、I/O error、表示崩れとして観測しやすい。

### 中

- `nv12` / `yv12` raw transport。
- software rotation。
- dirty rect 観測。
- full-frame JPEG 4:4:4 の tuning。

理由:

- path 自体は動いている。
- ただし 60 fps の安定性、回転、画質、latency spike が絡む。

### 中から高

- type 7 JPEG tile upload。
- fence-based queue。
- dirty rect + rotation。
- tile encode 並列化。

理由:

- Windows capture から形は見えているが、VRAM placement と fence の正確な意味が未確定。
- 失敗時に表示崩れ、stale tile、decoder error、stall が起きうる。

### 高

- 4K `session=7` NV12 rectangle path。
- two-output support。
- DriverKit 化。

理由:

- 1080p path と別 protocol になる可能性が高い。
- entitlement、install、VRAM allocation、multi-output scheduling の実装面積が大きい。

### 非常に高い / 初期 scope 外

- 公式 driver parity。
- audio。
- firmware / protocol の完全仕様化。

理由:

- 全 mode、USB2/USB3、hotplug、sleep/wake、cursor、audio、error recovery、device-specific quirk を含むため、回転対応付き 1080p 表示という当面の目的を超える。

## 次の作業

次にやることは、type 7 実送信の再挑戦ではなく、Windows capture から commit 手順を詰めること。

1. Windows 1080p capture から type 7 payload の前後数十 packet を抽出する。
2. 各 type 7 について、bulk command の `cmd_dest`、payload address、header `start_addr/end_addr`、interrupt fence ID を並べる。
3. 複数tileが1つの画面更新を構成している箇所を探し、最後に commit / flip 相当の packet があるか確認する。
4. `Type7JpegTilePacket` builderは残すが、Mac実機での送信は再度条件を絞るまで封印する。
5. 実機が固まった場合は `--reset-jpeg-engine`、それでも駄目ならUSB抜き差しで復旧する。

## 2026-06-21 type 7 再解析メモ

`tools/t6_type7_timeline.py` に `--ack-summary` と `--cmd-dest-summary` を追加した。

`captures/2026-06-19_jua365_win_05_partial_update.pcapng`:

- type7 rows: 95
- groups: 87
- interrupt rows: 649
- ack match: 82 / 95
- ack latency: min 0.170ms, p50 1.657ms, p90 4.758ms, max 7.566ms
- negative ack dt: 0
- duplicate sequence matches: 0
- `cmd_dest` range: `0x03244be0` - `0x039d27a0`
- `cmd_dest` wraps: 12
- common `cmd_dest` delta: `0x33e0` が最多

`captures/2026-06-19_jua365_win_02_hdmi1_hotplug.pcapng`:

- type7 rows: 108
- groups: 94
- interrupt rows: 288
- ack match: 80 / 108
- ack latency: min 0.234ms, p50 3.350ms, p90 7.819ms, max 7.941ms
- negative ack dt: 0
- duplicate sequence matches: 0
- `cmd_dest` range: `0x03200000` - `0x039ea060`
- `cmd_dest` wraps: 3
- common `cmd_dest` delta: `0xb3e0` が最多

観測:

- type7 header の `sequence` は interrupt packet の `value` と一致する。interrupt は `flags=0x04`, `event=0x04`。
- 単発 tile でも複数 tile group でも、各 tile sequence ごとに ack が返る例がある。
- 複数 tile group の後に独立した commit / flip packet は、少なくともこの解析範囲では見えていない。
- missing ack は capture window 外、interrupt window 8ms 超過、または frame skip / capture truncation の可能性がある。即座に「ack が存在しない」とは扱わない。
- `cmd_dest` は command/payload ring 上の書き込み先と見るのが自然。差分は payload size を 0x400 境界に丸めた値に近い。
- `start_addr/end_addr` は画面座標ではなく、decoder target / surface / VRAM zone の state を表す可能性が高いまま。

次の解析:

- `cmd_dest_delta` と `payload_len` / `data_len` の対応を CSV 出力し、`align1024(payload_len)` と一致するか確認する。
- `start_addr/end_addr` pair の切り替わりと `cmd_dest` wrap、JPEG reset、fence ack の関係を見る。
- Mac 側の再実験は単発 tile ではなく、Windows capture の group replay に寄せる。ただし現時点では実送信封印を維持する。

### `cmd_dest` と payload 長の対応

`tools/t6_type7_timeline.py --cmd-dest-payload-correlation` を追加し、
`cmd_dest` の次値との差分を前 tile の payload 長と比較した。

結果:

- `05_partial_update`
  - non-wrap pair: 82
  - wrap: 12
  - `delta == align1024(prev_payload_len) - 32`: 35
  - sequence が連続する pair: 35
  - 連続 sequence では `align1024(prev_payload_len) - 32` が 35 / 35 で一致
- `02_hdmi1_hotplug`
  - non-wrap pair: 104
  - wrap: 3
  - `delta == align1024(prev_payload_len) - 32`: 84
  - sequence が連続する pair: 84
  - 連続 sequence では `align1024(prev_payload_len) - 32` が 84 / 84 で一致

解釈:

- `cmd_dest` は type7 JPEG payload ring の書き込み先でほぼ確定。
- 次の `cmd_dest` は、前回 payload transfer の 1024 byte 境界丸めから bulk command header 32 byte を引いた分だけ進む。
- sequence が飛んでいる pair で大きな delta になるのは、解析対象外の packet、capture window、または同 ring 内の他 payload が間に入っているためと見るのが自然。
- type7 group replay を Mac で試す場合、`cmd_dest` は固定値ではなく ring cursor として更新する必要がある。少なくとも連続 tile では:

```text
next_cmd_dest = cmd_dest + align1024(payload_len) - 32
```

ここで `payload_len` は type7 payload transfer の長さで、Windows capture 上では `cmd_total_len` と同じ値になっている。

### `start_addr/end_addr` pair の遷移

`tools/t6_type7_timeline.py --address-transition-summary` を追加し、type7
header の `start_addr/end_addr` pair を target zone ID として集計した。

`05_partial_update`:

- address pair: 45 種
- steps: 94
- same pair steps: 32
- pair changes: 62
- consecutive sequence の same pair: 19
- consecutive sequence の pair change: 16
- 主な pair:
  - `0xcafcd0-0xe80cd0`: 17回、`64x704`
  - `0x1932190-0x1aec990`: 7回、`64x224`
  - `0xc55590-0xe53590`: 6回、`64x224`, `320x224`, `1216x224`, `1216x928`
  - `0xd45cb0-0xecbcb0`: 6回、`64x64`, `96x64`

`02_hdmi1_hotplug`:

- address pair: 12 種
- steps: 107
- same pair steps: 62
- pair changes: 45
- consecutive sequence の same pair: 56
- consecutive sequence の pair change: 29
- 主な pair:
  - `0x30-0x1fe030`: 54回、`64x544`, `1920x736`
  - `0x18aaaf0-0x1aa8af0`: 16回、`64x64`, `64x96`, `64x544`, `64x1080`, `256x32`, `1248x64`
  - `0x18ab210-0x1aa9210`: 11回、`64x96`, `96x96`
  - `0x18aab10-0x1aa8b10`: 9回、`1888x96`
  - `0x18c8af0-0x1ab7af0`: 7回、`64x480`, `64x1016`

観測:

- 同じ address pair が異なる tile size に再利用されるため、pair は tile の画面座標ではない。
- hotplug 側は少数の pair が繰り返され、特に `0x30-0x1fe030` は大きめの `1920x736` にも使われる。
- `64x96`, `1824x96`, `64x480/1016` のような複数 tile group では、pair が左帯/中央帯/残り帯のように切り替わる。
- `05_partial_update` は pair の種類が多く、UI の細かい dirty では target zone が細分化されるように見える。
- address pair は `cmd_dest` ring とは独立しており、同じ pair でも `cmd_dest` は広い範囲で変化する。

現時点の実装仮説:

- type7 の実送信では、`cmd_dest` は ring cursor として自前計算できる。
- 一方で `start_addr/end_addr` は dirty rect の placement / target zone allocator で、まだ自前生成は危険。
- Mac 側の次の安全な実験は、Windows capture 由来の group を `cmd_dest` 更新込みで replay すること。`start_addr/end_addr` はまず capture 値をそのまま使う。

### type7 JPEG の種類

`tools/t6_type7_timeline.py --jpeg-summary` を追加し、type7 payload の JPEG SOF
marker と component sampling を集計した。

確認した capture:

- `02_hdmi1_hotplug`: 108 / 108
- `03_resolution_change`: 137 / 137
- `04_solid_colors`: 124 / 124
- `05_partial_update`: 95 / 95

結果:

- すべて `SOF0 = 0xc0`。つまり baseline JPEG。
- すべて component sampling は `id0:2x2:q0,id1:1x1:q1,id2:1x1:q1`。
- これは 4:2:0 sampling と見てよい。
- Windows type7 dirty tile は、少なくとも既存 1080p capture では progressive / 4:2:2 / 4:4:4 を使っていない。

Mac 側への反映:

- type7 replay / generation の初期値は baseline JPEG 4:2:0 に固定する。
- full-frame path で有効だった JPEG 4:4:4 + yuv444 target は、type7 の再現実験には混ぜない。
- type7 の色ズレ調査では、まず sampling ではなく `start_addr/end_addr` / target surface / expected output format を疑う。

### 2026-06-21 niconico fullsnap の反映

YouTube と niconico の fullsnap capture を比較した結果、type7 は任意 dirty rect というより、公式 driver が動画領域や画面下端の帯を固定パターンで送る経路に見える。

観測値:

- YouTube:
  - type7: `1376x800`
  - `delta_start=0x3c000`, `delta_end=0x1e000`, `span=0x1e0000`
- niconico:
  - type7: `832x480` / `832x544` / `832x800`
  - `delta_start=0x78260`, `delta_end=0x3c260`, `span=0x1c2000`
  - 追加で Windows taskbar 相当の `1920x56`
  - `delta_start=0x1e0000`, `delta_end=0xf0000`, `span=0x10e000`

次の実装方針:

- `t6-virtual-display` の type7 は、任意 dirty bbox から即生成しない。
- まず `t6-replay-video` / `t6-send-type7` で、capture 由来の type4 初期化直後に動画領域 type7 を安定 replay できる状態を基準にする。
- 生成 type7 を戻す場合は、`--unsafe-generated-type7` のままにし、動画領域を明示指定した固定パターンから始める。
- 色ズレや表示ズレが出る場合は、JPEG sampling より address range と surface state を優先して疑う。

### Windows full-frame JPEG の sampling

`tools/t6_pcap_summary.py --jpeg-summary --jpeg-summary-only` を追加し、type7 以外の
JPEG payload も SOF marker / component sampling で集計した。

確認結果:

- `2026-06-19_jua365_win_02_hdmi1_hotplug.pcapng`
  - `type=0x3`, `1920x1080`, 40 frames
  - すべて `SOF0 = 0xc0` baseline JPEG
  - components は `id0:2x2:q0,id1:1x1:q1,id2:1x1:q1`
  - これは 4:2:0 sampling と見てよい。
- `2026-06-19_jua365_win_06_4k_hotplug_mid40k.pcapng`
  - `type=0x4`, `1920x1080`, 1 frame
  - 同じく baseline JPEG 4:2:0。

現時点の結論:

- 既存 Windows capture では、type7 dirty tile だけでなく、観測済み full-frame
  JPEG path でも 4:4:4 は使われていない。
- Mac 側で有効だった `jpeg 444 -> yuv444` は画質改善用の独自 fallback として扱い、
  Windows 再現 path の初期値にはしない。

### JPEG 4:2:0 の画質差調査

Windows 公式 driver の JPEG 4:2:0 が Mac/TurboJPEG より綺麗に見える理由は未解明。

現時点の観測:

- Windows fullsnap 抽出 JPEG は `Intel(R) IPP JPEG encoder [7.1.37466]` の comment を持つ。
- Windows JPEG は baseline 4:2:0、component id は `0,1,2`。
- macOS `sips` / ImageIO の quality 95 JPEG も baseline 4:2:0 だが、component id は `1,2,3`。
- ImageIO の量子化テーブルは Windows IPP より軽いので、画質差は量子化だけでは説明しにくい。
- VideoToolbox JPEG は probe 上 `VTCompressionSessionCreate(JPEG) failed: -12903` で、この環境では使えていない。

次の切り分け:

1. Windows 抽出 JPEG を `t6-send-jpeg --jpeg` でそのまま送る。
2. 同じ画像を ImageIO/sips quality 95 で再エンコードした JPEG を送る。
3. 同じ画像を TurboJPEG `--image` で再エンコードして送る。
4. 1 が綺麗で 2/3 が滲むなら encoder/downsample 差が主因。
5. 1 も滲むなら、公式 driver の表示経路は JPEG そのもの以外、たとえば type7 領域選択、raw NV12 path、target format、または T6 state の違いが効いている。

### group replay 用 export

`tools/t6_type7_timeline.py --export-groups-json <path>` を追加した。

出力には group ごとに以下を含める:

- pcap 名、session、time range
- 各 tile の `sequence`, `cmd_dest`, `cmd_total_len`, `payload_len`, `data_len`
- tile header fields: size, canvas, `start_addr/end_addr`, format
- JPEG SOF / sampling
- type7 payload 全体の base64 (`payload_b64`)
- 近傍 interrupt packets

生成済み sample:

- `captures/type7_hotplug_groups_sample.json`
- `captures/type7_partial_groups_sample.json`

これは Mac 側 `t6-send-type7` に capture group replay mode を追加するための入力形式として使う。

### type7 address pair scan 実験

`t6-send-type7` に、単色白 tile を生成して既知の Windows capture 由来
`start_addr/end_addr` pair へ順番に送る実験モードを追加した。

目的:

- type7 decoder path では JPEG 4:2:0 が前提か確認する。
- `start_addr/end_addr` pair ごとに、描画される、色が壊れる、何も出ない、固まる、を人間が観察して分類する。
- `cmd_dest` は固定せず、連続 tile ごとに ring cursor として進める。

実行例:

```sh
cargo run --features usb --bin t6-send-type7 -- \
  --solid-white \
  --width 64 \
  --height 64 \
  --quality 90 \
  --subsamp 420 \
  --zero-based-component-ids \
  --scan-known-addresses \
  --scan-sleep-ms 100 \
  --wait-interrupt-ms 100
```

`--scan-known-addresses` では次の pair を順に送る。

- `0x00000030-0x001fe030`
- `0x018aaaf0-0x01aa8af0`
- `0x018ab210-0x01aa9210`
- `0x018aab10-0x01aa8b10`
- `0x018c8af0-0x01ab7af0`
- `0x00cafcd0-0x00e80cd0`
- `0x01932190-0x01aec990`
- `0x00c55590-0x00e53590`
- `0x00d45cb0-0x00ecbcb0`

確認点:

- 白 tile がどこかに出るか。
- 赤/緑崩れが 4:2:0 でも残るか。
- 何も出ない場合、Windows capture と同じ `id0:2x2,id1:1x1,id2:1x1` になるよう `--zero-based-component-ids` を付けて再確認する。
- pair ごとに描画範囲や overlay の出方が変わるか。
- ack は返るが描画されない pair があるか。
- 送信後にデバイスが固まる pair があるか。

注意:

- これは placement を自動解決するものではなく、有効 zone を分類するための実験。
- デバイスが固まった場合は抜き差しで復旧する前提。
- `--dry-run` を付けると、送信せず JPEG sampling と ring cursor 更新だけ確認できる。

2026-06-21 追記:

- `--subsamp 420` と `--zero-based-component-ids` でも、単発 synthetic tile の address scan では表示変化が見えなかった。
- したがって、少なくとも単発生成 tile だけでは type7 表示条件を満たしていない可能性が高い。
- 次は Windows capture の `payload_b64` を含む group export を、そのまま raw type7 payload として replay する。

実行例:

```sh
python3 tools/t6_type7_timeline.py captures/<windows_capture>.pcapng \
  --export-groups-json captures/type7_groups.json \
  --limit-groups 20

cargo run --features usb --bin t6-send-type7 -- \
  --replay-groups-json captures/type7_groups.json \
  --replay-group 1 \
  --scan-sleep-ms 100 \
  --wait-interrupt-ms 100
```

`--replay-groups-json` では、生成 JPEG ではなく export JSON 内の type7
`payload_b64` をそのまま bulk payload として送る。`cmd_dest` も capture 値を使う。
これで表示が出るなら、synthetic tile 側の header/JPEG/group 条件が不足している。
これでも出ないなら、type7 の前段 state、surface 初期化、または Windows capture 前後の別 command が必要。

## 2026-06-21 公式 macOS ドライバ Ghidra 解析後の見直し

type7 は Windows capture の replay で動画らしい更新までは確認できたが、色ズレ・位置ズレ・灰色欠けが残る。
現時点では優先度を下げ、記録を残したうえで 420 画質差の切り分けを優先する。

公式 macOS ドライバには次の経路がある。

- VideoToolbox JPEG encoder
- Metal/OpenCL 系の T6 専用 YUV420/JPEG encoder
- dirty rect compressed submission
- uncompressed YUV420/YUV444 upload

重要なのは、公式の高速経路が ImageIO/TurboJPEG のような汎用 JPEG API だけではなさそうな点。
埋め込み kernel では RGB/BGRX から YUV へ変換し、Cb/Cr を 2x2 平均してから DCT/quant/RLE/Huffman に進む。
Windows/IP P JPEG と TurboJPEG q95 の量子化テーブルは一致したので、画質差の主因候補は量子化ではなく
RGB->YUV 変換、420 downsample、または T6 専用 submit/target format の差。

短期ロードマップ:

1. Rust 側の JPEG420 前処理を公式 kernel 相当に寄せる。
   - BGRX/RGB の扱いを明示する。
   - Y: `0.299R + 0.587G + 0.114B - 128`
   - Cb: `-0.168736R - 0.331264G + 0.5B`
   - Cr: `0.5R - 0.418688G - 0.081312B`
   - Cb/Cr は 2x2 average。
2. 同じ capture frame から以下を保存・比較する。
   - 現行 TurboJPEG 420
   - 公式相当 downsample 420
   - ImageIO 420
   - Windows capture から抽出した IPP JPEG
3. 信号機部分の crop/zoom を並べて、キャプチャ自体、JPEG decode 後、実機表示後のどこで滲むかを確認する。
4. 改善しない場合、公式の `t6_compress_yuv420_gpu_dctquantrlehuff_whole_image` 相当を CPU/Rust で再現するか、
   VideoToolbox JPEG 出力を T6 に渡す実験へ進む。

中期ロードマップ:

1. type7 は docs に残しつつ、Windows capture 由来 payload replay を基準にする。
2. synthetic type7 は、公式 submit 経路のアラインメント制約と state がもう少し分かるまで保留。
3. dirty rect 送信は、まず type4/full frame の画質と安定性が十分になってから再開する。
