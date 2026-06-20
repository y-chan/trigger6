# Windows Codex Next Task: Type7 Motion Capture

## 目的

公式 Windows driver が `type=0x7` dirty update をどう送っているかを、既知の画面変化と USB capture の対応で確認する。

今回知りたいこと:

- 実際に送られている JPEG の sampling。特に 4:2:0 / 4:4:4 のどちらか。
- 大きい画面変化で `type=0x7` を続けるのか、`type=0x3/type=0x4` full-frame に切り替えるのか。
- `start_addr/end_addr` が 3 つ程度の address zone / surface を回しているだけなのか、画面位置にも対応しているのか。
- YouTube capture で見えた `type7` JPEG の block 配置が、制御したパターンでも起きるのか。

小さい矩形、帯、全画面更新は必ず別 pcap にする。大きい更新を混ぜると巨大 payload と大量 tile が混ざり、解析しづらくなる。

## 表示設定

- JUA365 / T6 側ディスプレイは横向き `1920x1080`
- スケーリング `100%`
- 拡張ディスプレイでよい
- HTML は必ず JUA365 側ディスプレイで全画面表示
- capture 中は通知、マウス移動、他ウィンドウの重なりを避ける

縦向きは後段でよい。まず横向きで address / JPEG / 大更新の挙動を固める。

## 使うスクリプト

HTML と対応 CSV はリポジトリ側で生成する。

```sh
python3 tools/generate_type7_address_patterns.py
```

生成先:

```text
captures/type7_address_patterns/
```

既存の小矩形パターン:

- `type7_addr_xscan_64x64.html`
- `type7_addr_yscan_64x64.html`
- `type7_addr_grid_64x64.html`

今回追加した motion / 大更新パターン:

- `type7_motion_horizontal_bands.html`
- `type7_motion_vertical_bands.html`
- `type7_motion_large_rects.html`
- `type7_motion_fullscreen_colors.html`

各 HTML と同名の `.csv` も保存する。CSV は step 名、座標、サイズ、色、停止時間の対応表。

## Capture セット

### 1. 小矩形 address scan

目的: 画面座標と `start_addr/end_addr`、address zone、ack/fence の関係を見る。

取得ファイル:

```text
win_type7_addr_xscan_64x64.pcapng
win_type7_addr_yscan_64x64.pcapng
win_type7_addr_grid_64x64.pcapng
```

対応 HTML:

```text
type7_addr_xscan_64x64.html
type7_addr_yscan_64x64.html
type7_addr_grid_64x64.html
```

### 2. 帯パターン

目的: 横帯/縦帯のような大きめの dirty rect が、type7 tile と address zone にどう分解されるかを見る。

取得ファイル:

```text
win_type7_motion_horizontal_bands.pcapng
win_type7_motion_vertical_bands.pcapng
```

対応 HTML:

```text
type7_motion_horizontal_bands.html
type7_motion_vertical_bands.html
```

### 3. 大矩形と全画面

目的: 大きい画面変化で `type7` 継続か、`type3/type4` full-frame へ切り替わるかを見る。

取得ファイル:

```text
win_type7_motion_large_rects.pcapng
win_type7_motion_fullscreen_colors.pcapng
```

対応 HTML:

```text
type7_motion_large_rects.html
type7_motion_fullscreen_colors.html
```

## tshark / USBPcap

USBPcap interface は環境に合わせて変更する。

```powershell
tshark -D
tshark -i \\.\USBPcap1 -w win_type7_motion_horizontal_bands.pcapng
```

手順:

1. HTML を JUA365 側ディスプレイで開く
2. USB capture を開始
3. HTML をクリックまたは Space で開始
4. HTML 完了後、1 秒程度待って capture を停止
5. pcapng と対応 CSV を同じフォルダに置く

JPEG payload を完全に見たい capture だけ high snaplen 版も別ファイルで取る。

```powershell
USBPcapCMD.exe -d \\.\USBPcap1 -A -s 2000000 -b 134217728 -o win_type7_motion_fullscreen_colors_fullsnap.pcap
```

通常 capture と fullsnap capture は混ぜない。

## 解析コマンド

capture を `captures/` に置いたら、まず timeline を見る。

```sh
python3 tools/t6_type7_timeline.py captures/win_type7_motion_horizontal_bands.pcapng \
  --limit-groups 300 \
  --verbose \
  --ack-summary \
  --cmd-dest-summary \
  --cmd-dest-payload-correlation \
  --address-summary \
  --address-transition-summary \
  --jpeg-summary
```

JPEG を抽出して可視化する。

```sh
python3 tools/t6_reassemble_video.py captures/win_type7_motion_horizontal_bands.pcapng \
  --summary-only \
  --export-jpegs captures/win_type7_motion_horizontal_bands_reassembled \
  --salvage-eoi \
  --report-csv captures/win_type7_motion_horizontal_bands_reassembled.csv \
  --report-html captures/win_type7_motion_horizontal_bands_reassembled.html
```

同じコマンドを各 pcap に対して実行する。

## 見るポイント

- `type=0x7 format=0x0d` の JPEG sampling が 4:2:0 か 4:4:4 か。
- `type=0x3/type=0x4` がいつ出るか。
- `start_addr/end_addr` が `0x025...`, `0x029...`, `0x02d...` のような zone を回るか。
- `cmd_dest` が `align1024(payload_len) - 32` で進む payload ring として説明できるか。
- sequence / fence と interrupt `event=0x04` の `offset=0x0c` が対応するか。
- 抽出 JPEG が最終画面位置の画像か、内部 atlas / dirty surface の断片か。
- 制御した帯/全画面パターンでも、YouTube capture と同じような block 配置の崩れが出るか。

## 返してほしいもの

- 各 pcapng
- 対応する `.csv`
- `t6_type7_timeline.py` の出力
- `t6_reassemble_video.py` の `.csv` / `.html`
- 抽出 JPEG ディレクトリは必要なら zip などの外部 artifact として渡す
- Windows 表示設定メモ
  - 解像度
  - 向き
  - スケーリング
  - JUA365 が何番目のディスプレイだったか
  - USBPcap interface 名

抽出 JPEG / PNG / HTML report はローカル生成物なので、通常は git commit しない。
