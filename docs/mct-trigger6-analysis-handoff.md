# MCT Trigger 6 デバイス解析の引き継ぎ書

Date: 2026-06-23 JST

## この文書で引き継ぐ範囲

この文書は、MCT 社製 Trigger 6 系 USB display adapter の解析を次の担当者へ渡すための作業メモである。

対象は手元の JUA365 で、USB ID は `0711:5601`、USB 表示名は `T6 USB Station` である。

主な未解決点は、`type=0x07` の画面一部更新で、どの画面領域をどのサイズで送るかを Windows driver がどう決めているかである。

Ghidra はインストール済みとして扱う。

## 最初に読む資料

解析の全体像は `docs/reverse-engineering-notes.md` にある。

Type7 の現時点の整理は `docs/type7-deep-dive-2026-06-23.md` にある。

Windows capture の取り方は `docs/windows-capture-guide.md` にある。

Mac 側の実装経緯と Type7 replay 実験は `docs/mac-driver-roadmap.md` にある。

Ubuntu 公式 driver 由来の JPEG と YUV 経路は `docs/ubuntu-driver-deep-dive.md` にある。

この引き継ぎ書は、それらの詳細メモを読む前に作業の向きを合わせるための入口である。

## 現在分かっていること

T6 の bulk OUT は、32 byte の command phase の後に video payload phase が続く。

command phase の `dest` は、video payload の device 側書き込み先として使われる。

video payload には少なくとも `type=0x03`、`type=0x04`、`type=0x07` が出る。

`image_format=0x0d` は JPEG を表す。

Windows 公式 driver の JPEG は、既存 capture では baseline JPEG 4:2:0 だった。

Type7 の `sequence_counter` は、interrupt IN の ack または fence 値として返る。

Type7 の `cmd_dest` は画面位置ではなく、payload ring cursor と見るのが自然である。

## まだ分かっていないこと

Type7 の `start_address/end_address` が、どの surface、VRAM window、placement を指すのかは分かっていない。

Windows driver が dirty rect をどの規則で `192x1080`、`1920x56`、`64x64` などへ変換しているのかも分かっていない。

Type7 を Mac や Linux 側で安定再現するために、単独 tile で足りるのか、capture 由来 group 全体の replay が必要なのかも未確定である。

大きい更新時に Windows driver が Type7 を続ける条件と Type4 へ切り替える条件も、まだ切り分けられていない。

## Type4 の位置づけ

**Type4** は、全画面 JPEG 更新として扱える可能性が高い経路である。

Windows capture では、大きい画面変化や fullscreen colors のような刺激で Type4 が主体になることがある。

見る field は `type=0x04`、`image_format=0x0d`、JPEG payload、典型的な `1920x1080` の画像サイズである。

Type4 は、まず画面を出すための fallback として価値がある。

Type7 を解く途中でも、Type4 の実装は残すべきである。

## Type7 の位置づけ

**Type7** は JPEG tile update と見ている。

ただし、画面上の dirty rect がそのまま tile になるわけではない。

現在の Type7 header 解釈は次のとおりである。

```c
struct trigger6_type7_video_header {
    u32 type;             // 0x07
    u32 data_length;
    u32 sequence_counter;
    u32 flags;
    u16 width;
    u16 height;
    u16 canvas_width;
    u16 canvas_height;
    u32 start_address;
    u32 end_address;
    u32 reserved0;
    u32 image_format;     // 0x0d for JPEG
    u32 reserved1;
    u32 reserved2;
};
```

既存の 1080p capture では、`canvas_width/canvas_height` が `1920x1080` ではなく `1920x1920` になりやすい。

この値は、Type7 の宛先が単純な可視画面ではなく、内部 surface または atlas のような領域であることを示している可能性がある。

## Type7 で信頼できる不変条件

`cmd_dest` は payload ring cursor として説明できる。

連続する Type7 row では、次の式が clean な capture で成立している。

```text
next_cmd_dest = cmd_dest + align1024(payload_len) - 32
```

`win_type7_addr_xscan_64x64.pcapng` では、連続 pair 9 個すべてで一致した。

`win_type7_addr_yscan_64x64.pcapng` では、連続 pair 225 個すべてで一致した。

`type7_motion_vertical_bands.pcapng` でも、確認できた連続 pair 1 個で一致した。

このため、`cmd_dest` を画面位置として扱う実装は捨ててよい。

## Type7 の ack

`sequence_counter` は interrupt packet の ack 値として返る。

対象は `flags=0x04`、`event=0x04` の interrupt packet である。

値は interrupt payload offset `0x0c` に little endian で入っている。

`win_type7_addr_xscan_64x64.pcapng` では、Type7 tile 10 個すべてに ack が対応した。

`win_type7_addr_yscan_64x64.pcapng` では、Type7 tile 248 個のうち 247 個に ack が対応した。

少なくとも現在の parser で見えている範囲では、Type7 tile の後に独立した commit packet や flip packet は見えていない。

## 更新領域は dirty rect そのものではない

Type7 の更新サイズは、可視画面上の dirty rect と一対一に対応しない。

この点が、現在の解析で最も重要である。

X scan では、白い `64x64` rectangle を画面上端に沿って横方向に動かした。

しかし `win_type7_addr_xscan_64x64.pcapng` に出た Type7 JPEG は、すべて `192x1080` だった。

address pair も `0x2500430-0x26fe430` の 1 種類だけだった。

この結果は、小さい矩形の横移動が、幅 `192` の全高縦帯に拡張されていることを示している。

Y scan では、白い `64x64` rectangle を画面左端に沿って縦方向に動かした。

`win_type7_addr_yscan_64x64.pcapng` では、`64x64` tile だけでなく、`1920x56` の横帯が頻出した。

多くの group は `64x64` と `1920x56` のような 2 tile 構成だった。

この結果は、縦方向の移動で、局所 tile と全幅の cleanup band が同時に作られている可能性を示している。

## Type7 生成の作業仮説

現時点では、Type7 は次の流れで生成されていると見るのがよい。

```text
Windows compositor dirty rect
  -> driver 側で merge / collapse / align
  -> 固定 strip / band / tile へ分解
  -> JPEG encode
  -> Type7 header 作成
  -> payload ring へ upload
  -> interrupt ack で fence 完了
```

この仮説では、更新領域の決定は USB protocol の最後の段階ではなく、Windows driver の dirty rect processing にある。

したがって、Type7 のサイズ決定を解くには、pcap 解析だけでなく Windows driver binary を見る必要がある。

## pcap 解析で使うコマンド

Type7 timeline は次で見る。

```powershell
python tools\t6_type7_timeline.py captures\win_type7_addr_xscan_64x64.pcapng --limit-groups 120 --verbose --ack-summary --cmd-dest-summary --cmd-dest-payload-correlation --address-summary --address-transition-summary --jpeg-summary
```

pcap 全体の概要は次で見る。

```powershell
python tools\t6_pcap_summary.py captures\win_type7_addr_xscan_64x64.pcapng --summary-only
```

JPEG 抽出は次で行う。

```powershell
python tools\t6_reassemble_video.py captures\type7_motion_horizontal_bands.pcapng --summary-only --export-jpegs captures\type7_motion_horizontal_bands_reassembled --salvage-eoi --report-csv captures\type7_motion_horizontal_bands_reassembled.csv --report-html captures\type7_motion_horizontal_bands_reassembled.html
```

JPEG の SOF、component、DQT、comment は次で見る。

```powershell
python tools\t6_jpeg_inspect.py <jpeg-file>
```

## pcap 解析で見る値

Type7 row 数と group 数を見る。

`width/height` の分布を見る。

`canvas` が `1920x1920` になっているかを見る。

`cmd_dest` delta が `align1024(payload_len)-32` に合うかを見る。

`sequence_counter` と interrupt `value` が合うかを見る。

`start_address/end_address` の pair が、位置、tile size、group 内順序のどれと相関するかを見る。

## Windows driver binary の探し方

このワークスペースには、まだ Windows driver binary がない。

driver package を入手したら、まず package を展開して `*.sys`、`*.dll`、`*.inf` を保存する。

installer 由来の cab や extracted directory も残す。

symbol や pdb があれば必ず残す。

Type7 の dirty rect 処理は user-mode component にある可能性が高い。

ただし USB bulk submit は kernel と user の境界をまたぐ可能性があるので、`*.sys` と `*.dll` の両方を見る。

## Ghidra project の作り方

Ghidra で新規 project を作り、driver package 内の `*.sys` と `*.dll` をすべて import する。

PE の auto-analysis を有効にする。

C++ RTTI、exception、string reference、decompiler parameter ID は有効にする。

最初は decompile 結果を読むより、strings、imports、constant xref から候補関数を絞る。

## Ghidra で最初に検索する文字列

ASCII と UTF-16LE の両方で検索する。

```text
t6_submit_frame_surface_with_compressed_dirty_rects
t6_submit_frame_surface_whole_screen_compressed
t6_submit_frame_surface_compressed_flip
MCTT6Device JPEG Encoder
JPEG_ERROR
fence
dirty
rect
compressed
TurboJPEG
tjCompress2
IPP
JPEG encoder
```

Windows capture の JPEG comment には次が入っていた。

```text
Intel(R) IPP JPEG encoder [7.1.37466] - Sep 25 2012;
```

この文字列や IPP 関連 import が見つかれば、JPEG encode 関数の近くに Type4 または Type7 header construction がある可能性が高い。

## Ghidra で探す数値定数

小さい値は誤爆が多いので、複数の値を組み合わせて見る。

```text
0x00000007  Type7
0x00000004  Type4
0x0000000d  JPEG format
0x00000780  1920
0x00000438  1080
0x001fe000
0x001ef000
0x0010e000
0x03200000
0x03a00000
0x00019000  known fragment size
```

`0x07` や `0x04` だけでは候補が多すぎる。

`0x0d`、`1920`、`1080`、`1920x1920`、`0x1fe000` を同じ関数内で見る方が効率がよい。

## 同定したい関数

最初に探すべき関数は、dirty rect を受け取る関数である。

次に、dirty rect を merge または collapse する関数を見る。

その次に、crop して JPEG encode する関数を見る。

JPEG encode 後に、Type7 header を組み立てる関数を見る。

その周辺で、`cmd_dest` ring cursor を進める関数、`sequence_counter` を採番する関数、`start_address/end_address` を決める allocator または placement 関数を探す。

最終的に取りたい対応は次である。

```text
input dirty rects
  -> emitted Type7 width/height
  -> start_address/end_address
  -> cmd_dest
  -> sequence_counter
```

この対応が取れれば、Type7 の更新部分とサイズ決定の本体に届く。

## JPEG encode 関数から辿る

JPEG 関連 import が見つかった場合は、その xref から調べる。

JPEG encode 呼び出しの直前には、crop 後の width、height、source pointer、stride、quality、subsampling が出るはずである。

見る変数は、crop x/y、crop width/height、source stride、JPEG output buffer、JPEG output size、subsampling、quality である。

JPEG output buffer の使い先を辿ると、Type4 または Type7 payload に詰める処理に着くはずである。

## Type7 header construction の見つけ方

Type7 header は、連続した store として現れる可能性が高い。

特に `u32 0x07` と `u32 0x0d` を同じ小さな構造体に書いている箇所を見る。

候補が見つかったら、Ghidra で Type7 header の構造体を作り、該当 pointer に apply する。

構造体を apply すると、`width/height`、`canvas_width/canvas_height`、`start_address/end_address` の流れを追いやすくなる。

## `cmd_dest` 更新の見つけ方

pcap では `cmd_dest` が `align1024(payload_len)-32` で進む。

Ghidra では、`+ 0x3ff` と `& 0xfffffc00` のような align 処理を探す。

その近くに `- 0x20` があれば、Type7 payload ring cursor の更新処理である可能性が高い。

この処理を binary 側で確認できれば、pcap 側の `cmd_dest` 解釈はかなり固まる。

## dirty rect merge の見つけ方

Type7 のサイズ決定を理解する本命は dirty rect merge である。

探す処理は、rect list の配列走査、left/top/right/bottom の比較、隣接 rect の結合、min/max による bounding box 作成である。

さらに、width/height を `32`、`56`、`64`、`96`、`192`、`224` などへ丸める処理を見る。

面積や tile 数のしきい値で Type4 に fallback する処理も探す。

pcap に出た特徴的なサイズは次である。

```text
192x1080
32x1080
32x344
1920x56
64x64
64x56
1920x32
384x224
1920x96
448x64
1184x1080
```

これらの値が immediate として直接出るとは限らない。

MCU block、surface tile grid、alignment、display height から計算されている可能性がある。

## `start_address/end_address` の生成元

`start_address/end_address` は placement に関係するが、screen x/y そのものではない。

探すべき処理は、VRAM base address に offset を足す処理、複数 surface の ring、triple buffering、zone base の選択である。

pcap 上の代表 pair は次である。

```text
0x2500430-0x26fe430
0x29556f0-0x2b536f0
0x26e0430-0x27ee430
0x2b356f0-0x2c436f0
0x2f8a9b0-0x30989b0
```

span は `0x1fe000`、`0x1ef000`、`0x10e000` 付近に固まる。

address pair が allocator output なら、dirty rect 座標とは別の state machine を持っているはずである。

## 次に取るべき capture

既存の browser fullscreen capture は、compositor timing や browser UI 混入の影響を受ける。

Type7 のサイズ決定を調べるには、native Win32/DXGI の小さい test app が望ましい。

test app は JUA365 側 display にだけ描画する。

背景は黒にする。

1 frame ごとに 1 rectangle だけを描く。

frame time、rect、color を CSV に記録する。

mouse cursor、通知、window animation は入れない。

推奨 matrix は次である。

- 左上固定：`16x16`, `32x32`, `64x64`, `96x96`, `128x128`, `192x192`, `224x224`, `256x256`
- X scan：x=`0`, `32`, `64`, `128`, `192`, `256`, `512`, `1024`, `1856`
- Y scan：y=`0`, `32`, `56`, `64`, `96`, `112`, `224`, `448`, `728`, `1016`

family ごとに pcap を分ける。

capture 後は、対応 CSV と表示設定メモを必ず残す。

表示設定メモには、解像度、scaling、JUA365 が Windows 上で何番目の display か、USBPcap interface 名、capture 開始時刻と終了時刻を書く。

## 作業上の注意

Type7 synthetic send は device stall を起こす可能性がある。

無制限に繰り返すべきではない。

`cmd_dest` を固定値にしてはいけない。

連続 tile では ring cursor として進める必要がある。

`start_address/end_address` は既知 capture 由来の値から始めるべきである。

自前生成はまだ危険である。

Type7 を単独 tile で試すより、capture 由来 group replay を基準にする。

high snaplen capture は必要な短時間だけにする。

巨大 pcap は解析効率を落とす。

個人情報や通知が capture に混じらないよう、専用テスト画面で取る。

## 次の担当者の最短手順

1. Windows driver package を入手し、`*.sys` と `*.dll` を Ghidra に import する。
2. JPEG encoder 文字列、IPP 文字列、`0x0d`、`1920`、`1080`、`0x1fe000` の xref を探す。
3. JPEG encode 呼び出しの直後から Type7 header construction までを辿る。
4. `cmd_dest` ring update の式を binary 側で確認する。
5. dirty rect merge または collapse 関数を見つけ、入力 rect と出力 Type7 size の対応を取る。
6. 既存 capture の `win_type7_addr_xscan_64x64.pcapng` と `win_type7_addr_yscan_64x64.pcapng` の結果と照合する。
7. 必要なら native Win32/DXGI test app で追加 capture を取る。

この順で進めると、Type7 の「どこを、どのサイズで更新するか」を決めている層に届く。
