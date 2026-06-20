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
