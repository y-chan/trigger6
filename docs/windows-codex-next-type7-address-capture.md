# Windows Codex Next Task: Type7 Address Capture

## 目的

JUA365 / T6 の type7 dirty tile address 対応を解析する。

見るもの:

- 白 `64x64` 矩形の画面座標と type7 header の `start_addr/end_addr` の対応
- type7 group 内の tile 数と address pair の切り替わり
- `cmd_dest` が payload ring として進むか

今回は大きい画面変化、動画再生、マウス高速移動は混ぜない。

## 表示設定

- JUA365 側ディスプレイを横向き `1920x1080` にする
- スケーリングは `100%`
- 拡張ディスプレイでよい
- HTML は必ず JUA365 側ディスプレイで全画面表示する
- capture 中は通知や他ウィンドウを出さない

まずは横向き `1920x1080` で取る。縦向きは第2段階。

## 使うファイル

- `captures/type7_address_patterns/type7_addr_xscan_64x64.html`
- `captures/type7_address_patterns/type7_addr_yscan_64x64.html`
- `captures/type7_address_patterns/type7_addr_grid_64x64.html`

同名の `.csv` も pcap と一緒に保存する。

HTML はクリックまたは Space で開始する。画面には黒背景と白 `64x64` 矩形だけが出る。

## 取得する Capture

### 1. X Scan

出力ファイル:

```text
win_type7_addr_xscan_64x64.pcapng
```

手順:

1. `type7_addr_xscan_64x64.html` を JUA365 側ディスプレイで開く
2. USBPcap / tshark capture を開始
3. HTML をクリックまたは Space で開始
4. HTML 終了後、1秒程度待ってから capture を停止

### 2. Y Scan

出力ファイル:

```text
win_type7_addr_yscan_64x64.pcapng
```

手順は X Scan と同じ。HTML は `type7_addr_yscan_64x64.html` を使う。

### 3. Grid

出力ファイル:

```text
win_type7_addr_grid_64x64.pcapng
```

手順は X Scan と同じ。HTML は `type7_addr_grid_64x64.html` を使う。

## tshark 例

USBPcap interface は環境に合わせて変更する。

```powershell
tshark -i \\.\USBPcap1 -w win_type7_addr_xscan_64x64.pcapng
```

JUA365 がどの USBPcap interface にいるか分からない場合は、JUA365 を挿抜して packet が出る interface を確認する。

## High Snaplen Capture

可能なら high snaplen 版も別ファイルで取る。

```powershell
USBPcapCMD.exe -d \\.\USBPcap1 -A -s 2000000 -b 134217728 -o win_type7_addr_xscan_64x64_fullsnap.pcap
USBPcapCMD.exe -d \\.\USBPcap1 -A -s 2000000 -b 134217728 -o win_type7_addr_yscan_64x64_fullsnap.pcap
USBPcapCMD.exe -d \\.\USBPcap1 -A -s 2000000 -b 134217728 -o win_type7_addr_grid_64x64_fullsnap.pcap
```

通常 pcap と fullsnap は混ぜない。

## 注意

- xscan / yscan / grid は必ず別 pcap にする
- 大きい画面変化を混ぜない
- 動画再生を混ぜない
- マウス高速移動を混ぜない
- HTML 開始前に 1-2 秒黒画面を capture する
- HTML 終了後も 1 秒程度黒画面を capture してから停止する
- 取得後、pcap と対応する CSV を同じフォルダに置く

## 取得後の解析

各 pcap に対して実行する。

```sh
python3 tools/t6_type7_timeline.py captures/win_type7_addr_xscan_64x64.pcapng \
  --limit-groups 200 \
  --verbose \
  --ack-summary \
  --cmd-dest-summary \
  --cmd-dest-payload-correlation \
  --address-summary \
  --address-transition-summary \
  --jpeg-summary
```

`yscan` / `grid` に対しても同じオプションで実行する。

## 返してほしいもの

- `win_type7_addr_xscan_64x64.pcapng`
- `win_type7_addr_yscan_64x64.pcapng`
- `win_type7_addr_grid_64x64.pcapng`
- 可能なら `*_fullsnap.pcap`
- 対応する CSV
- Windows 表示設定メモ
  - 解像度
  - 向き
  - スケーリング
  - JUA365 が何番目のディスプレイだったか
  - 使用した USBPcap interface 名
- 上記解析コマンドの出力
