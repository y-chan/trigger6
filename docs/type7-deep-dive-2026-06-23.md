# Type7 詳細解析メモ

Date: 2026-06-23 JST

## 範囲

既存の Type7 capture とリポジトリ内メモをもう一段深く見直した結果をまとめる。

このワークスペース内では Windows driver binary は見つからなかった。`*.exe`, `*.dll`, `*.sys`, `*.inf`, `*.msi`, `*.cab`, `*.pdb` を検索したが該当ファイルはなかった。そのため、このメモの Windows バイナリ解析部分は、driver package を入手した後の具体的な引き継ぎとして書いている。

## パケット解析から分かること

### Type7 header field

現在の parser とローカル header 定義は、Type7 を次の形として扱っている。

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

今回確認した 1080p Windows capture では、`canvas_width/canvas_height` は `1920x1080` ではなく、だいたい `1920x1920` になっている。これは Type7 の宛先が単純な可視画面の矩形ではなく、内部の正方形 surface または atlas 的な領域である可能性を強く示している。

### JPEG format は安定している

確認した Type7 capture では、Type7 JPEG はすべて次の特徴だった。

- SOF marker は `0xc0`、つまり baseline JPEG
- components は `id0:2x2:q0,id1:1x1:q1,id2:1x1:q1`
- したがって 4:2:0。progressive でも 4:4:4 でもない
- `image_format=0x0d`

このため、Type7 の位置ズレや色ズレの主因として JPEG sampling を疑う優先度は下がる。より疑うべきなのは、target surface の placement、stride/canvas の解釈、または Type7 upload 周辺の device state である。

### `cmd_dest` は payload ring cursor

もっとも強い不変条件は、引き続き次の式で説明できる。

```text
next_cmd_dest = cmd_dest + align1024(payload_len) - 32
```

連続した Type7 row では、clean な capture でこの式が完全に成立している。

- `win_type7_addr_xscan_64x64.pcapng`: 連続 pair 9/9
- `type7_motion_vertical_bands.pcapng`: 連続 pair 1/1
- `win_type7_addr_yscan_64x64.pcapng`: 連続 pair 225/225

`cmd_dest` field は画面位置として扱うべきではない。これは device 側 command/payload ring 内の JPEG payload upload 先である。

### `sequence_counter` は fence/ack 値

`flags=0x04`, `event=0x04` の interrupt packet は、interrupt payload offset `0x0c` に Type7 の `sequence_counter` を返している。

観測例:

- `win_type7_addr_xscan_64x64.pcapng`: Type7 tile 10 個、ack match 10 個、p50 latency は約 2.2 ms
- `win_type7_addr_yscan_64x64.pcapng`: Type7 tile 248 個、ack match 247 個、p50 latency は約 1.9 ms
- `type7_motion_horizontal_bands.pcapng`: Type7 tile 4 個、ack match 4 個

少なくとも現在の parser で見えている範囲では、Type7 tile と ack の間に、独立した明確な commit/flip packet は必要なさそうに見える。

## 更新サイズと更新領域の挙動

可視画面上の dirty rectangle は、そのまま送られていない。Windows driver は Type7 を出す前に、dirty 領域を量子化、拡張、または分解しているように見える。

### X scan: 可視 64x64 rect が全高の縦帯になる

刺激:

黒背景上の白い `64x64` rectangle を、画面上端に沿って横方向に移動。

`win_type7_addr_xscan_64x64.pcapng` での観測:

- Type7 row は 10 個
- すべての Type7 JPEG が `192x1080`
- address pair は 1 種類だけ: `0x2500430-0x26fe430`
- span は `0x1fe000`
- `cmd_dest` は `0x23e0` ずつ進む。これは `align1024(9184)-32` と一致する

解釈:

上端付近で小さい rectangle を横に動かしても、`64x64` の Type7 tile にはならない。幅が `192` に丸められた、全高の縦帯として送られている。

### Y scan: 可視 64x64 rect は小 tile + 全幅帯になりやすい

刺激:

黒背景上の白い `64x64` rectangle を、画面左端に沿って縦方向に移動。

`win_type7_addr_yscan_64x64.pcapng` での観測:

- Type7 row は 248 個、update group は 159 個
- 頻出サイズ:
  - `1920x56`: 97 rows
  - `64x64`: 59 rows
  - `64x56`: 39 rows
  - `1920x32`: 20 rows
  - `384x224`: 13 rows
  - `1920x96`: 11 rows
- 多くの group は 2 tile を含む。例: `64x64` と `1920x56`

解釈:

縦方向の移動では、局所 tile と横帯の両方が発生している。横帯は、古い rectangle の行帯を消す処理、compositor 側の damage 拡張、または driver 側 dirty rect merge/collapse heuristic の結果である可能性が高い。

### 横帯/縦帯 capture は Type7 row が少ない

band pattern の pcap では、Type7 row は少なかった。

- `type7_motion_horizontal_bands.pcapng`: Type7 row 4 個。サイズは `192x1080`, `32x1080`, `32x344`
- `type7_motion_vertical_bands.pcapng`: Type7 row 2 個。どちらも `192x1080`

これは、browser fullscreen 表示経路や Windows compositor のタイミングによって、期待した更新が抑制または合成される可能性を示している。これらの capture は量子化例としては有用だが、単独では全体の policy を推定するには足りない。

## Address pair の解釈

`start_address/end_address` は placement に関係している可能性がもっとも高い field だが、単純な画面座標 encoding ではない。

根拠:

- X scan では可視 rectangle を複数の X 位置に動かしたが、観測された Type7 row はすべて同じ pair `0x2500430-0x26fe430` を使っていた。
- Y scan では複数の pair が出る。ただし pair は可視位置だけでなく、tile class や zone とも相関している。
- span は `0x1fe000`, `0x1ef000`, `0x10e000` 付近に固まる。これは JPEG byte size よりかなり大きく、tight な可視 rectangle とは合わない。
- `canvas=1920x1920` は、より大きい内部 surface 上の placement を示しているように見える。

作業仮説:

- `cmd_dest` は upload ring の位置。
- `start_address/end_address` は device VRAM 内の destination zone または surface window を選ぶ。
- `width/height` は driver 側 dirty processing 後の JPEG tile size。
- 可視位置は `width/height` だけで決まるのではなく、destination surface state と `start_address/end_address` の組から導かれる。

## 現時点の仮説

1. Type7 生成は Windows compositor の dirty rect から始まるが、driver が encode 前に固定 strip/band へ拡張している。
2. 幅と高さは decoder または surface 制約に合わせて align されている。観測された代表的な粒度は、幅が `32`, `64`, `192`, `384`, `448`, `1920`、高さが `32`, `56`, `64`, `96`, `224`, `1080`。
3. `1920x1920` canvas は実際の protocol state と見るべき。Type7 は内部の rotated/atlas surface を patch し、その surface が hardware/display state によって可視 `1920x1080` 出力へ mapping されている可能性がある。
4. address pair は raw x/y ではなく、allocator/placement の出力である。単発 synthetic tile replay が失敗するのは、同じ allocator state や前段 surface setup が欠けているためかもしれない。
5. replay で最も忠実な単位は capture 由来の group である。`cmd_dest` の進行、sequence 値、address pair、group 内の全 tile を維持する必要がある。

## Windows バイナリ解析の次手順

Windows driver package が入手できたら、次を優先して調べる。

### 対象ファイル

installed driver package 内の display miniport / user-mode component を探す。候補は `*.sys`, `*.dll`, `*.inf`。元 installer と展開後の package tree は両方残す。

### 検索する文字列

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
```

数値定数も検索する。

```text
0x00000007  Type7
0x0000000d  JPEG format
0x00000780  1920
0x00000438  1080
0x00000780  1920 canvas width
0x00000780  1920 canvas height
0x001fe000
0x001ef000
0x0010e000
0x03200000
0x03a00000
```

### 同定したい関数

重要な control flow target は、dirty rect を受け取り、1 個以上の Type7 header を生成する関数である。Ghidra で string xref または constant xref を見つけたら、次の役割の周辺関数を label する。

- dirty rect collection / merge / collapse
- tile width/height quantization
- JPEG crop / encode call
- Type7 header construction
- `cmd_dest` ring cursor update
- fence / sequence allocation
- destination VRAM address allocation

最も価値が高い breakpoint / logging point は、Type7 header を submit する直前と、dirty rect を merge した直後である。この 2 点で次の対応を見る。

```text
input dirty rects -> emitted Type7 width/height/start/end/cmd_dest/sequence
```

この対応が取れれば、全高/全幅 band が Windows dirty rect 由来なのか、driver の merge heuristic 由来なのか、device 固定 tiling policy 由来なのかを切り分けられる。

## 次に取るべき capture

現在の browser capture より正確に size policy を切り分けるには、browser fullscreen ではなく、小さい native Win32/DXGI test app を JUA365 側 display で動かすのがよい。1 frame ごとに 1 rectangle を描画し、正確な frame time、rect、color を log する。

推奨 matrix:

- 左上固定 rect: `16x16`, `32x32`, `64x64`, `96x96`, `128x128`, `192x192`, `224x224`, `256x256`
- 同じ rect を X 位置 `0`, `32`, `64`, `128`, `192`, `256`, `512`, `1024`, `1856` に置く
- 同じ rect を Y 位置 `0`, `32`, `56`, `64`, `96`, `112`, `224`, `448`, `728`, `1016` に置く
- family ごとに pcap を分ける。mouse movement と通知は入れない

重要なのは、browser fullscreen UI の混入と compositor animation を避けること。high snaplen は JPEG payload decode が必要な短い subset にだけ使う。
