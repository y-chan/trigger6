# Windows Type7 Capture Handoff

目的は、JUA365 / T6 の 1080p JPEG dirty tile path である `type=0x7` について、次を明確にすること。

- 画面上の位置と USB header field の対応。
- 実際に送られている JPEG の種類、特に baseline/progressive と chroma subsampling。
- 大きい画面変化時に、type7 tile を大量に送るのか、full-frame JPEG path に切り替えるのか。

Mac 側では次が確認済み。

- `type=0x7 format=0x0d` tile は実機に受理される。
- interrupt `event=0x04` の offset `0x0c` に sequence / fence ID が返る。
- `start_addr/end_addr` を変えると表示の出方は変わる。
- ただし `start_addr/end_addr` だけではまだ正しい表示位置を再現できていない。
- `crop-x` を変えても表示位置は動かず、tile 内の内容だけが変わる。
- 2026-06-21 の再解析では、`cmd_dest` は表示位置ではなく type7 JPEG payload ring の書き込み先と見るのが自然。連続 tile では概ね `next_cmd_dest = cmd_dest + align1024(payload_len) - 32` で進む。
- 複数 tile group の後に独立した commit / flip packet は、既存解析範囲では見えていない。

したがって Windows 側では、既知位置の小さい矩形を動かす capture、JPEG 種類を見る capture、大きい画面変化を見る capture を必ず分けて取る。大きい payload が混ざると解析しづらいので、通常の位置対応 capture に full-screen update や動画再生を混ぜない。位置対応では主に `start_addr/end_addr` pair、tile group 内の pair 切り替わり、sequence ack を見る。

## 必要なもの

- Windows PC
- JUA365 / T6 device
- 1920x1080 HDMI monitor
- Wireshark + USBPcap
- `tshark`
- 可能なら `USBPcapCMD.exe`

capture 中は個人情報や通知を出さない。外部ディスプレイにはテストパターンだけを表示する。

## 画面設定

- JUA365 側ディスプレイを 1920x1080 にする。
- 拡張ディスプレイでよい。
- スケーリングは 100% が望ましい。
- 背景は黒。
- テスト用ウィンドウは JUA365 側ディスプレイにだけ表示する。

## 表示するテストパターン

HTML でも小さい native app でもよい。重要なのは、黒背景上に単色矩形を段階的に表示すること。

address 対応に集中する場合は、リポジトリ側でテスト HTML を生成できる。

```sh
python3 tools/generate_type7_address_patterns.py
```

生成物:

- `captures/type7_address_patterns/type7_addr_xscan_64x64.html`
- `captures/type7_address_patterns/type7_addr_yscan_64x64.html`
- `captures/type7_address_patterns/type7_addr_grid_64x64.html`
- 各 HTML と同名の `.csv`

Windows 側では HTML を JUA365 側ディスプレイに置き、クリックまたは Space で開始する。画面には黒背景と白い矩形だけが出る。`.csv` は各 step の位置と停止時間の対応表として、pcap と一緒に保存する。

### Capture A: 小さい矩形の位置対応

推奨パターン:

1. 黒背景のみで 2 秒停止。
2. 赤い `64x64` 矩形を `(0, 0)` に表示し 1 秒停止。
3. 同じ矩形を `(64, 0)` に移動し 1 秒停止。
4. 同じ矩形を `(256, 0)` に移動し 1 秒停止。
5. 同じ矩形を `(0, 64)` に移動し 1 秒停止。
6. 同じ矩形を `(0, 256)` に移動し 1 秒停止。
7. 同じ矩形を `(1856, 0)` に移動し 1 秒停止。
8. 同じ矩形を `(0, 1016)` に移動し 1 秒停止。
9. 黒背景に戻して 1 秒停止。

追加で取れるなら、同じ手順を `96x96` と `64x224` でも行う。既存 capture に近い tile size なので解析しやすい。

### Capture B: JPEG 種類の確認

目的は、Windows driver が dirty tile に使う JPEG が 4:2:0 / 4:2:2 / 4:4:4 のどれか、baseline か progressive かを見ること。

推奨パターン:

1. 黒背景のみで 2 秒停止。
2. 赤・緑・青・白の縦帯を含む `256x256` 矩形を `(0, 0)` に表示し 1 秒停止。
3. 同じ矩形を `(512, 256)` に移動し 1 秒停止。
4. 文字を含む `512x256` 矩形を表示し 1 秒停止。
5. 黒背景に戻して 1 秒停止。

色帯と文字を入れる理由は、chroma subsampling と decoder target format のズレが見えやすいから。

### Capture C: 大きい画面変化

目的は、大きい更新で Windows driver がどの戦略を使うかを見ること。

これは Capture A/B とは必ず別ファイルにする。大きい画面変化は巨大 payload や大量 tile を発生させ、位置対応や JPEG sampling の解析を邪魔するため。

推奨パターン:

1. 黒背景のみで 2 秒停止。
2. 画面全体を赤にして 1 秒停止。
3. 画面全体を緑にして 1 秒停止。
4. 画面全体を青にして 1 秒停止。
5. 画面全体を白黒チェッカーボードにして 1 秒停止。
6. 黒背景に戻して 1 秒停止。

ここでは `type=0x7` tile が大量に出るのか、`type=0x3/type=0x4` full-frame JPEG が出るのか、または別 session/path に切り替わるのかを見る。

## Capture 方法

まず `tshark -D` で USBPcap interface を確認する。

```powershell
tshark -D
```

JUA365 がいる controller が分からない場合は、Wireshark で USBPcap interface を短時間見て、JUA365 挿抜時に packet が出る interface を確認する。

通常 capture:

```powershell
tshark -i \\.\USBPcap1 -w win_type7_rect_64x64_positions.pcapng
```

`USBPcap1` は環境に合わせて変更する。

手順:

1. テストパターンを黒背景のみで待機させる。
2. capture 開始。
3. 上の矩形移動シナリオを実行。
4. capture 停止。
5. pcapng を保存。

ファイル名:

```text
win_type7_rect_64x64_positions.pcapng
win_type7_rect_96x96_positions.pcapng
win_type7_rect_64x224_positions.pcapng
win_type7_jpeg_sampling_patterns.pcapng
win_type7_large_changes.pcapng
```

## 高 snaplen capture

通常の位置対応 capture は header と interrupt が主目的なので、Wireshark/tshark の通常 capture でよい。巨大 payload を完全取得しようとしない。

JPEG payload の完全取得が必要な場合だけ、別ファイルとして `USBPcapCMD.exe` の high snaplen capture を取る。通常 capture と high snaplen capture を混ぜない。

例:

```powershell
USBPcapCMD.exe -d \\.\USBPcap1 -A -s 2000000 -b 134217728 -o win_type7_jpeg_sampling_patterns_fullsnap.pcap
```

大きい画面変化の high snaplen capture が必要な場合も、短時間で別ファイルにする。

```powershell
USBPcapCMD.exe -d \\.\USBPcap1 -A -s 2000000 -b 134217728 -o win_type7_large_changes_fullsnap.pcap
```

## Capture 後の確認

リポジトリ側に capture を置いたら、まず概要を見る。

```sh
python3 tools/t6_pcap_summary.py captures/win_type7_rect_64x64_positions.pcapng --summary-only
```

type7 timeline を見る。

```sh
python3 tools/t6_type7_timeline.py captures/win_type7_rect_64x64_positions.pcapng --limit-groups 80 --verbose --ack-summary --cmd-dest-summary --cmd-dest-payload-correlation --address-summary --address-transition-summary
```

JPEG 種類を見る capture では、sampling を必ず集計する。

```sh
python3 tools/t6_type7_timeline.py captures/win_type7_jpeg_sampling_patterns.pcapng --jpeg-summary --limit-groups 80
```

大きい画面変化 capture は、まず video type の比率を見る。

```sh
python3 tools/t6_pcap_summary.py captures/win_type7_large_changes.pcapng --summary-only
python3 tools/t6_pcap_summary.py captures/win_type7_large_changes.pcapng | rg "video|type=|jpeg="
```

見る点:

- `type7_rows` が出ているか。
- `width/height` が `64x64` など意図した矩形に近いか。
- 矩形位置を変えたタイミングで `start_addr/end_addr` がどう変わるか。
- `cmd_dest` が `align1024(payload_len) - 32` に従って進むか。
- sequence が連続している tile で、`cmd_dest` delta と payload 長の対応が崩れる例があるか。
- sequence と interrupt `event=0x04 value=...` が一致するか。
- ack latency が数 ms 程度に収まるか。missing ack は capture window 外や interrupt window 超過の可能性もあるので、単独では失敗扱いにしない。
- 1回の矩形移動が単独 tile か、複数 tile group か。
- 複数 tile group 内で `start_addr/end_addr` pair が、左帯/中央帯/残り帯のように切り替わるか。
- JPEG SOF が `0xc0` baseline か、`0xc2` progressive か。
- JPEG sampling が `id1:2x2,id2:1x1,id3:1x1` など 4:2:0 か、`1x1` の 4:4:4 か。
- 大きい画面変化で `type=0x7` が継続するか、`type=0x3/type=0x4` full-frame JPEG に切り替わるか。
- 大きい画面変化時の payload bytes、tile count、ack/fence の増え方。

## 期待する成果物

最低限ほしいもの:

- `captures/win_type7_rect_64x64_positions.pcapng`
- `captures/win_type7_jpeg_sampling_patterns.pcapng`
- `captures/win_type7_large_changes.pcapng`
- 実行した矩形位置リスト
- Windows の表示設定メモ
  - 解像度
  - スケーリング
  - JUA365 が何番目のディスプレイだったか
  - USBPcap interface 名

あるとよいもの:

- `win_type7_rect_96x96_positions.pcapng`
- `win_type7_rect_64x224_positions.pcapng`
- high snaplen 版 capture
- テストパターンの HTML / app / script

注意:

- `win_type7_rect_*` には大きい画面変化を入れない。
- `win_type7_jpeg_sampling_patterns` には小-中サイズの色帯/文字矩形だけを入れる。
- `win_type7_large_changes` は大きい更新専用にする。
- high snaplen は必要時だけ、通常 capture と別名で保存する。

## 解析で判定したいこと

この capture で答えたい質問:

- `start_addr/end_addr` は tile の画面座標を直接表しているのか。
- 画面 x/y の変化に対応して変わる field はどれか。
- `cmd_dest` は payload ring buffer として説明しきれるか。
- 複数 tile group の中で、tile の並び順と画面上の位置に規則があるか。
- capture 由来の group replay に必要な最小 field は、`cmd_dest` ring cursor と `start_addr/end_addr` pair と sequence だけで足りるか。
- Windows driver の type7 JPEG は 420/422/444 のどれか。
- Mac 側で赤/緑だけに見えている原因は JPEG sampling か、decoder target surface format か、placement/stride か。
- 大きい画面変化では dirty tile path を使い続けるのか、full-frame path を使うのか。
- Mac 側 `t6-send-type7` で再現すべき最小単位は、単独 tile か、group 全体か。

この対応が見えたら、Mac 側では `t6-send-type7` に capture 由来の tile group replay mode を追加し、次に `t6-virtual-display` の dirty rect path へ戻す。
