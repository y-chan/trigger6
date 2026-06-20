# Windows Type7 Capture Handoff

目的は、JUA365 / T6 の 1080p JPEG dirty tile path である `type=0x7` の「画面上の位置」と USB header field の対応を明確にすること。

Mac 側では次が確認済み。

- `type=0x7 format=0x0d` tile は実機に受理される。
- interrupt `event=0x04` の offset `0x0c` に sequence / fence ID が返る。
- `start_addr/end_addr` を変えると表示の出方は変わる。
- ただし `start_addr/end_addr` だけではまだ正しい表示位置を再現できていない。
- `crop-x` を変えても表示位置は動かず、tile 内の内容だけが変わる。

したがって Windows 側では、既知位置の小さい矩形を動かして USBPcap/tshark capture を取り、`type=0x7` の `start_addr/end_addr/cmd_dest/sequence/width/height` と画面座標の対応を見る。

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
```

## 高 snaplen capture

今回の主目的は header と interrupt なので通常 capture で足りる可能性が高い。ただし JPEG payload も完全に取りたい場合は `USBPcapCMD.exe` を使う。

例:

```powershell
USBPcapCMD.exe -d \\.\USBPcap1 -A -s 2000000 -b 134217728 -o win_type7_rect_64x64_positions_fullsnap.pcap
```

## Capture 後の確認

リポジトリ側に capture を置いたら、まず概要を見る。

```sh
python3 tools/t6_pcap_summary.py captures/win_type7_rect_64x64_positions.pcapng --summary-only
```

type7 timeline を見る。

```sh
python3 tools/t6_type7_timeline.py captures/win_type7_rect_64x64_positions.pcapng --limit-groups 80 --verbose --address-summary
```

見る点:

- `type7_rows` が出ているか。
- `width/height` が `64x64` など意図した矩形に近いか。
- 矩形位置を変えたタイミングで `start_addr/end_addr` がどう変わるか。
- `cmd_dest` が単なる payload buffer か、位置や surface と連動するか。
- sequence と interrupt `event=0x04 value=...` が一致するか。
- 1回の矩形移動が単独 tile か、複数 tile group か。

## 期待する成果物

最低限ほしいもの:

- `captures/win_type7_rect_64x64_positions.pcapng`
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

## 解析で判定したいこと

この capture で答えたい質問:

- `start_addr/end_addr` は tile の画面座標を直接表しているのか。
- 画面 x/y の変化に対応して変わる field はどれか。
- `cmd_dest` は payload ring buffer だけか、表示位置にも関係するか。
- 複数 tile group の中で、tile の並び順と画面上の位置に規則があるか。
- Mac 側 `t6-send-type7` で再現すべき最小単位は、単独 tile か、group 全体か。

この対応が見えたら、Mac 側では `t6-send-type7` に capture 由来の tile group replay mode を追加し、次に `t6-virtual-display` の dirty rect path へ戻す。
