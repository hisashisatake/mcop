# 38x6 音源仕様

## チャンネルIDとキーオン契約（sound-core共通）

`Ym38x6Engine::note_on(channel, frequency, velocity, patch)`で発音し、`channel`は呼び出し側（VST/gesture-app等）が指定する安定したIDとして扱う。

- 同じ`channel`へ再度`note_on`すると、`env_level`を0（無音）に落とさず**残響レベルからAttackを再開**する（実機OPMのKey-On挙動。EGは減衰量をリセットせず現在値からアタックするため、前の音が消えきる前のキーオンではARが本来の立ち上がりをせず、モジュレーターのエンベロープが残響に引きずられる＝FMらしい再アタックの明るさが出る）。これにより同音連打でもプチノイズが出ない
  - ※以前は「即座にカットしてAttackから再開する同音チョーク」を実機準拠としていたが、これは実機EGの誤解に基づくものだった。実機は切らずに残響から再アタックする
- ピッチベンドは`set_pitch_bend(channel, cents)`、または`set_pitch_bend_group(group, cents)`で`channel >> 7`が一致する全ボイスへ一括適用する（MIDIチャンネル単位ベンド）
- VST（ym38x6-vst）はボイスIDを`midi_ch*128 + note`で符号化する（一意性＝Note Off/同音再アタックの突き合わせ、グループ性＝`id >> 7`でMIDIチャンネルを復元しベンド一括適用、を両立）
- gesture-appはコードの声部インデックス（0〜N-1の固定スロット）を`channel`として使う（[spec-app.md](spec-app.md)参照）

## 波形メモリ専用音色バンク（38x6のOP1のみ有効）

フェーズ1ではプロトタイプとして独立クレートのWMS-1（波形オシレーター + ADSR）を用いていたが、
その実態は38x6の1オペレーター相当であり、波形フォーマット・チャンネル契約・パフォーマンスLFO（現: FG／質感LFO）は
すでに38x6と共通化されていた。そのため**WMS-1（wms1-core / wms1-vst）は廃止**し、同等の音色は
38x6の**「波形メモリ専用音色バンク」**として提供する。これは38x6エンジンの専用モードではなく、
予約バンク`WAVEFORM_MEMORY_BANK`に「OP1のみ可聴・OP2〜4はTL=0」という通常のパッチをまとめた
取り決めにすぎない（エンジン側に特別な分岐はない）。

### 位置づけ

```
波形メモリ音色 ＝ 38x6の1オペレーター（OP1）のみを鳴らした状態

波形メモリ音色:  Algorithm 7（全並列・変調なし）/ OP1のみ可聴 / OP2〜4はTL=0でミュート
38x6（通常）:  （波形スロット + AR/D1R/D2R/RR/D1L） × 4op + FM変調 = 1音色
```

エンジンは38x6そのものなので専用の音源実装は不要。`ym38x6-core`の`waveform_memory_patch(waveform, adsr)`
（[preset.rs](ym38x6-core/src/preset.rs)）がこの音色のパッチを生成する。

### 音色の構成

- Algorithm = 7（全並列・FM変調なし）
- OP1（`operators[0]`）：指定 waveform + ADSR のみが可聴
- OP2〜4：`TL = 0`（≈ −95dB、実質無音）でミュート
- ADSRマッピング（`AdsrParams`の各値0〜255 → OP1のEG）: AR=attack / D1R=decay / D1L=sustain / RR=release（D2R=0で第2減衰なし、MUL=1 / DT1=128）
- チャンネル側はデフォルト（フィルター全開・チップ内LFO無効）

WMS-1の単一指数ADSRとはOPM準拠カーブを通る分だけ触感がわずかに変わる（許容）。

### ビルトイン波形（スロット0〜31）

38x6のビルトイン波形は **4基本波 × 8変換の32種**（[waveform.rs](ym38x6-core/src/waveform.rs)）。
**0〜7のサイン系のみOPZ由来**（ymfm OPZ実装準拠）で、**8〜31の3基本波はOPN/OPM/OPZには無い独自拡張**。

- 0〜7: サイン系（OPZ由来）
- 8〜31: 独自拡張（OPN/OPM/OPZには無い基本波）
  - 8〜15: ノコギリ系（saw基本波 × OPZ8変換スキーム）
  - 16〜23: 矩形系（PWMファミリー。矩形は2乗/絶対値変換が縮退するため、デューティ
    50/33/25/16.7/12.5/6.25% ＋ ハーフ矩形 ＋ 2倍速矩形前半 の独自8種）
  - 24〜31: 三角系（triangle基本波 × OPZ8変換スキーム）

サイン系8波形（0〜7、OPZ由来。8/16/24はそれぞれノコギリ/矩形/三角の同じ並びの先頭）:

```
0: サイン波（フル）
1: sin²（フル、符号付き）
2: ハーフサイン（前半のみ）
3: ハーフsin²（前半のみ）
4: 2倍速サイン前半（正負両方含む）
5: 2倍速sin²前半（符号付き）
6: 2倍速絶対値サイン前半（常に正）
7: 2倍速正sin²前半（常に正）
```

### 専用バンクによる選択（Bank Select / Program Change）

波形メモリ音色は専用のBank Select番号 `WAVEFORM_MEMORY_BANK`（= 16383）で選ぶ。
このバンクでは Program Change（0〜127）から`waveform_memory_params_for_program`で
(波形, ADSR)を決定し、1オペレーター音色を生成する。

波形メモリ音色（1オペレーター）は素直なオシレーターとして使うため、ビルトイン32種のうち
**基本4波形（sine=0 / saw=8 / square=16 / triangle=24、各基本波の variant0）のみ**を提供する
（OPZ8変換やPWMバリエーションは含めない）。これを ピアノ風 / リード風 の2 ADSR と組み合わせ、
Program 0〜7 に配置する（8以降は同じ並びを繰り返す = `program % 8`）。

- Program 0〜3: sine / saw / square / triangle + **ピアノ風ADSR**（attack=255 / decay=80 /
  sustain=120 / release=130。減衰して中程度のサスティンレベルへ落ち着く）
- Program 4〜7: sine / saw / square / triangle + **リード風ADSR**（attack=255 / decay=0 /
  sustain=255 / release=130。減衰なしの無限サスティンで、RRはピアノ風ADSRと同じ値にして
  キーオフ後の余韻の長さを揃える）
- Program 8以降: 上記（Program 0〜7）の繰り返し

`PresetBank::patch_for_program`の解決順は「ユーザープリセット(.38x6) > 波形メモリバンク > GM2 Bank0 > プレースホルダー」。
これにより gesture-app・ym38x6-vst の双方が Program 経由で同一の波形メモリ音色を選べる。

### 内部波形フォーマット

```
内部表現: 1024エントリ × uint16_t（2KB / 1波形）
  bit14〜0: -log|amplitude|（4.8 fixed point）
  bit15:    符号フラグ（負の半周期）
```

### ユーザー入力フォーマット（変換パイプライン）

```
ユーザー入力:
  32サンプル × int8（線形振幅、-128〜127）

          ↓ 変換パイプライン（sound-coreに実装）

  1. 32サンプル → 1024サンプルにリサンプリング
  2. 線形振幅 → 4.8固定小数点対数減衰フォーマットに変換

内部表現: 1024 × uint16_t
```

ユーザー入力は32サンプルの単純なテーブルで済む。高品質な内部フォーマットへの変換は自動。

### 波形スロット番号

```
0〜7:    ビルトイン・サイン系（OPZ由来）
8〜31:   ビルトイン・独自拡張波形（8〜15ノコギリ / 16〜23矩形 / 24〜31三角）
32〜63:  ノイズ（実機SSG準拠32段階、ランタイム生成。下記参照）
64〜255: ユーザー定義波形スロット
```

### ノイズ波形（番号32〜63、実機SSG準拠32段階）

テーブルルックアップ方式（32サンプルループ）では周期性で必ずピッチ感が出てしまい
ノイズにならない。そのため波形番号 **32〜63** はオシレーターがテーブル参照を行わず、
**17bit LFSR（AY-3-8910互換）でランタイム生成**する特別レンジとして予約する。

実機SSGのノイズは「1種類の白色LFSRの**クロックレート**をノイズ周期 NP(0〜31) で変える」もの。
38x6でもこれに倣い、波形番号で NP を指定する：

- `color = waveform - 32 = NP`（0〜31）。NP小=高速更新=広帯域（白）、NP大=低速更新=低域寄り。
- 生成方式: LFSRを **サンプル&ホールド** で更新する。更新レート `noise_clock_rate(NP) =
  NOISE_BASE_CLOCK / (16 × max(NP,1))`。`NOISE_BASE_CLOCK = 2MHz`（YM2203/YM2608の内部SSG
  標準クロック相当）を固定基準とする（エンジンはチップ固有の ssg_clock を知らないため）。
- ピッチレス（note周波数/FM変調入力に依存しない）。ADSR・TL・AM・ベロシティの音量制御は通常どおり効く。
- 実装: [waveform.rs](ym38x6-core/src/waveform.rs)（`is_noise_waveform`/`noise_color`/`noise_clock_rate`）、
  [operator.rs](ym38x6-core/src/operator.rs)（`next_noise_sample`、`tick`内分岐）。

#### トーン+ノイズ混合（実機SSGの同時出力）

実機SSGは1チャンネルでトーンとノイズを同時に出せる（ミキサーR7のbit0-2=tone, bit3-5=noise が
各々独立、両方有効可）。38x6では **Algorithm 7（全並列）** で OP1=矩形波（トーン）＋OP3=ノイズ
の**加算混合**で近似する（OP2/4はTL=0でミュート）。実機の「トーン AND ノイズ（トーンでノイズを
ゲート）」ではなく加算だが、両方が同時に聞こえる。vgm2x6 がトーン・ノイズ両有効を検出して
この混合パッチ（`mix_patch`）を使う。

---

## 38x6（FMオペレーター音源）

---

## 基本構成

| 項目 | 値 | 備考 |
|------|-----|------|
| チャンネル数 | 無制限 | ソフトウェアなので制約なし |
| オペレーター | 4op / ch | |
| アルゴリズム | 8種類（固定） | OPQ由来、将来的にルーティング拡張モード追加可 |
| 出力 | ステレオ | |

---

## 周波数・音程

38x6では常にOp0〜3それぞれが独立したオクターブ/F-Numberを持つ（OPNの拡張モード相当を標準仕様として全チャンネルに適用）。

| 項目 | 値 | 備考 |
|------|-----|------|
| オクターブ（指数部） | 3bit（0〜7、OP単位 × 4） | |
| F-Number（仮数部） | 13bit、OP単位 × 4 | OPQの12bitより精密。オクターブと合わせて16bit |

通常のノートはNote-On時に全Opへ同一のオクターブ/F-Numberが設定されるが、NRPN「Operator F-Number」でOP単位にF-Numberを上書き可能（オクターブは全Op共通のまま、詳細はNRPNセクション参照）。
OPQ由来の「1ch2周波数」（Op0/2ペアとOp1/3ペアが独立した周波数を持つ仕様）は、このOP単位F-Numberに内包される形で一般化された。

オペレーター間の周波数比が整数比（MUL/DT由来）に縛られなくなり、インハーモニックなFM変調（ベル系・金属的な音色）が可能になる。
アルゴリズム「全並列（4オペレーターが全てキャリア）」と組み合わせれば、独立した波形・周波数・エンベロープを持つ4オシレーターとしても利用できる。

PSR-70ファームウェアの周波数テーブル（OPQプログラマーズガイドより）：
```
C  → 4CAH, C# → 513H, D  → 560H, D# → 5B2H
E  → 609H, F  → 665H, F# → 6C6H, G  → 72DH
G# → 79AH, A  → 80EH, A# → 889H, B  → 90AH
```

---

## パラメーター設計方針

- **全パラメーター 0〜255（8bit統一）**
- 元チップ値域の単純スケーリングではなく、0〜255全域に対して独自の指数カーブを設計
- AR/D1R/D2R/RR・フィードバック・KSR・AM感度・PM感度・SL・TLは、実機OPM/OPN（ymfm実装）の
  理論値（min/max）を両端のアンカーとした指数カーブとする（実機の離散的なカーブをそのまま
  再現するのではなく、よりなだらかな変化にする）。指数カーブは出力値（時間・ゲイン等）が
  0〜255に対して指数的に変化する補間方式で、人間の時間・音量知覚が対数的であることから
  ノブ全域で変化が均等に感じられる。SL/TLの「dB単位で線形補間」は、dBがゲインの対数表現
  であることから同じ指数カーブの一種である。詳細は各パラメーターの備考を参照
- OPQオリジナル値からの**可逆変換（コンバーター）**を提供
- 周波数（F-Number + オクターブ）は**16bitのまま例外**
- マルチプル（MUL）は**0〜15（4bit）のまま例外**。OPM/OPN/OPQ/OPZ共通のMultiple仕様に準拠し、
  実機音色からの変換・パラメーターUIでの分かりやすさを優先する

---

## オペレーターパラメーター（基本8bit、MULのみ4bit例外）

| パラメーター | 元bit幅 | 8bit設計 | 備考 |
|------------|--------|---------|------|
| デチューン（DT1） | 6bit（OPQ）| 0〜255、中心128 | OPQ中心値32→128にマッピング。両端±50セント（微細デチューン、実機慣習に準拠）。これを超える範囲はOP Fine Tune参照 |
| マルチプル | 4bit | 0〜15（そのまま） | 8bit統一の例外。OPM/OPN/OPQ/OPZ共通のMultiple(0=0.5倍、1〜15=整数倍)と同一表現 |
| トータルレベル | 7bit | 0〜255 | 0=-95.25dB相当 |
| 波形選択 | 3bit | 0〜255（実質0〜7使用） | OPZ由来の8波形 |
| AR（アタックレート） | 5bit | 0〜255 | 指数カーブ（0.68ms〜20.2秒、0はフリーズ） |
| DR（ディケイレート） | 5bit | 0〜255 | 指数カーブ（8.71ms〜284.9秒、0はフリーズ） |
| SR（サスティンレート） | 5bit | 0〜255 | OPQ由来（DR2相当）。DRと同じ指数カーブ |
| RR（リリースレート） | 4bit | 0〜255 | 指数カーブ（8.71ms(255)〜284.9秒(0)、全域を指数補間。実機reg=15/reg=0に厳密一致。RRは実機にもフリーズが存在しないためAR/DRと異なり0でも有限値） |
| SL（サスティンレベル） | 4bit | 0〜255 | 0〜255をenv_level閾値に線形マッピング(d1l/255)。dBリニアエンベロープにより出力は-96dB(0)〜0dB(255)のdB線形（255=減衰なし）。※VCF/VCAのD1L（sound-core::Eg）も同じ線形マッピング(d1l/255)を使用 |
| KSR | 2bit | 0〜255 | 指数カーブ（1octあたり約1.09倍(0)〜2倍(255)、1段ごとに倍） |
| AMオン/オフ | 1bit | 0 or 1（8bitで保持） | |
| Velocity Sensitivity | なし | 0〜255（デフォルト0） | 38x6独自拡張（DX7/OPS由来）。OPQ/OPZ系チップにはハードウェア機能として存在しない。音の明るさ（モジュレーターの変調量）にのみ作用し、音量には作用しない（音量はベロシティが常時担当） |
| OP Fine Tune | なし | 0〜255、中心128（デフォルト128） | 38x6独自拡張。中心128＝±0、両端±1オクターブ（±1200セント）。DT1（±50セント）で足りない広いデチューンや、インハーモニックなOP周波数比を音色として静的に持たせる。DT1とはセントで加算。既存`.38x6`にフィールドが無い場合は128（オフセットなし）として読む |

**デチューンの2段構成（DT1 + OP Fine Tune）：**
DT1（±50セント・微細）とOP Fine Tune（±1オクターブ・広域）はセントで加算され、`実効周波数 = ノート周波数 × MUL比 × 2^((DT1セント + OP Fine Tuneセント)/1200)` として作用する（ともにノート相対＝音程比）。
通常はDT1のみで足り、OPQの広いデチューンやインハーモニックな倍音を再現する場合にOP Fine Tuneを併用する。OP単位F-Number上書き（NRPN、ランタイム・Note-Onでリセット）とは別系統で、こちらは音色（パッチ）として保存される。

**ベロシティの2系統（音量と明るさを分離）：**

38x6ではベロシティを「音量」と「明るさ」の2系統に分けて扱う。両者は独立しており、混ざらない。

- **音量（ベロシティ本来の役割・常時ON）：** 通常のMIDI楽器と同じく、ベロシティはチャンネル出力の音量に作用する。`実効音量ゲイン = Velocity / 127`（音色は一切変えない）。Velocity Sensitivityの設定値に関わらず常に効く。
- **明るさ（Velocity Sensitivity・opt-in）：** Velocity Sensitivityは「強く弾くと音色が明るくなる」ための拡張で、**モジュレーターのTL（=変調量）にのみ**作用する。キャリアでは無視する（音量は上記で一本化しているため）。デフォルト0＝明るさ変化なし。

```
実効TL（モジュレーターのみ） = clamp(TLベース値 + (Velocity / 127) × VelocitySensitivity, 0, 255)
```

### ビルトイン波形32種（番号0〜31）

4基本波 × 8変換 = 32種。**0〜7のサイン系のみOPZ由来**（ymfmの`opz_registers`実装に完全準拠）、
**8〜31の3基本波（ノコギリ/矩形/三角）はOPN/OPM/OPZには無い独自拡張**。
波形番号 = 基本波index×8 + 変換index。

#### 0〜7: サイン系（OPZ由来）
全波形のパターン: wf2〜7 はすべて「後半（p∈[0.5,1)）= 無音」、前半の内容で差異化。

```
0: サイン波（フル）              sin(2πp)
1: sin²（フル、符号付き）        sin(2πp)·|sin(2πp)|  — 奇数次倍音が緩く乗る
2: ハーフサイン                  sin(2πp) for p∈[0,0.5), 0
3: ハーフsin²                    sin²(2πp) for p∈[0,0.5), 0
4: 2倍速サイン前半               sin(4πp) for p∈[0,0.5), 0  — 正負両方含む
5: 2倍速sin²前半（符号付き）     sin(4πp)·|sin(4πp)| for p∈[0,0.5), 0
6: 2倍速絶対値サイン前半         |sin(4πp)| for p∈[0,0.5), 0  — 常に正
7: 2倍速正sin²前半               sin²(4πp) for p∈[0,0.5), 0  — 常に正
```

#### 8〜15: ノコギリ系（独自拡張）
基本波 saw（`2p-1`、上昇ランプ）に、サイン系と同じ8変換スキーム（full / squared / half /
half-squared / 2x-half / 2x-squared-half / 2x-abs-half / 2x-pos-squared-half）を適用。

#### 16〜23: 矩形系（独自拡張・PWMファミリー）
矩形波は b=±1 のため2乗/絶対値変換が縮退して重複する。代わりに**パルス幅変調（PWM）の
デューティスイープ ＋ OPZ由来のhalf/2x形**で8種すべてを別物にする。
```
16: デューティ50%（矩形）   20: デューティ12.5%
17: デューティ33%          21: デューティ6.25%
18: デューティ25%          22: ハーフ矩形（前半+1/後半0）
19: デューティ16.7%        23: 2倍速矩形の前半（+1,-1を前半に収め後半無音）
```

#### 24〜31: 三角系（独自拡張）
基本波 triangle（サイン位相に揃えた 0→+1→0→-1→0、p=0.25で+1ピーク）に、サイン系と同じ8変換を適用。

実装: [waveform.rs](ym38x6-core/src/waveform.rs)。

### ユーザー定義波形（番号32〜255）

波形番号32〜255はユーザー定義波形スロットとして使用可能（ビルトインは0〜31を占有）。

**内部波形テーブルフォーマット（ymfm準拠）：**
```
サイズ:   1024エントリ × uint16_t = 2KB / 1波形
フォーマット: 4.8固定小数点の対数減衰値
  bit14〜0: -log|amplitude|（4.8 fixed point）
  bit15:    符号フラグ（負の半周期）
```
これはYAMAHAのOPN系ダイショットから実測された値と同一フォーマット。
エンベロープ適用が加算のみで済む（乗算不要）という設計上の利点がある。

**ユーザー入力フォーマット（線形サンプル）：**
```
入力:  32サンプル × int8（線形振幅、-128〜127）
       1024サンプル × int16_t（線形振幅、-32768〜32767）
       または任意サンプル数（内部で1024点にリサンプリング）
変換:  線形振幅 → 4.8対数減衰フォーマットに自動変換
ソース: 波形エディタUIで直接描画
       WAVファイルからインポート（1周期分を切り出し）
```

**スロット仕様：**
```
スロット番号: 32〜255（224スロット。0〜31はビルトインが占有）
1スロット:   2KB（1024 × uint16_t）
合計最大:    224 × 2KB = 448KB
             → ソフトウェアなので問題なし
```

**利用例：**
```
スロット8:  ユーザー描画の倍音豊富な波形
スロット9:  WAVからインポートしたピアノの1周期
スロット10: プログラムで生成したチェビシェフ多項式波形
  ...
→ これらをFMのモジュレーターやキャリアとして使用可能
```

**プリセットへの保存：**
```
音色ファイル（.38x6）に波形データも埋め込み
→ 音色ファイル単体で完全再現可能
```

---

## チャンネルパラメーター（全8bit）

| パラメーター | 元bit幅 | 8bit設計 | 備考 |
|------------|--------|---------|------|
| アルゴリズム | 3bit | 0〜7（8bitで保持） | |
| フィードバック | 3bit | 0〜255 | 指数カーブ（0は完全オフ、1〜255で7oct、255で最大1.8サイクル）。帰還方式は実機OPM/OPN/OPZ準拠の2サンプル平均`(out[n-1]+out[n-2])/2`が既定（詳細・上限1.8の経緯は[spec-fm.md](spec-fm.md)「フィードバック帰還」参照） |
| AM感度 | 2bit | 0〜255 | 指数カーブ（0は完全オフ、1〜255でAMS=1〜3相当の23.9〜95.6dBをdepth=1-10^(-dB/20)で振幅深度に変換） |
| PM感度 | 3bit | 0〜255 | 指数カーブ（0は完全オフ、1〜255でPMS=1(+/-5cents)〜PMS=7(+/-700cents)相当、約7.13oct） |

### フィルター（Vcf: State Variable Filter、ボイス単位）

FM合成出力にかけるアナログシンセ的なVCF相当（`sound-core::Vcf`トレイト、具象実装`VoiceFilter`）。
OPQ由来パラメーターとは独立した38x6独自拡張。

| パラメーター | 値域 | 備考 |
|------------|------|------|
| Cutoff | 0〜255 | カットオフ周波数。対数スケール（0≒20Hz、255≒20kHz） |
| Resonance | 0〜255 | レゾナンス。Self-Oscillation ON時は255でカットオフ周波数のサイン波が自己発振 |
| Self-Oscillation | 0 or 1（8bitで保持） | デフォルト=1（ON）。OFF時は255でも発振寸前で安定動作 |
| Filter Type | 0〜2（8bitで保持） | 0=LP、1=HP、2=BP |

**Cutoffへの変調：** カットオフのキーオン連動スイープ／オートワウ／アシッドは **Cutoff FG**
（[ファンクションジェネレーター](#ファンクションジェネレーターfgpitchcutoffgain)節）が担う。旧「Filter EG」の後継で、
5段EG＋Loop/Floor/Curveを持ち、Depthは**バイポーラ**（中心128、カットオフを開く/閉じる両方向）。
質感LFO（Destination=Cutoff）の5波形変調とも加算的に共存する（決め台詞「FG＝軌跡の変調／LFO＝5波形の飛び道具」）。

**実装方式：** State Variable Filter（SVF、`sound-core::Svf`）
- LP/HP/BPを同一回路から同時出力できる構造で、Filter Typeによる切り替えと相性が良い
- 高Resonanceでも数値的に安定（Self-Oscillation時の発振も含めて安定動作）

Self-Oscillation ON + Cutoff FGでCutoffをスイープすると、発振に突入する効果音的な表現が可能。

**OPQコンバーターとの関係：**
フィルターはOPQ由来パラメーターではないため、OPQ変換対象外。38x6独自フォーマット（.38x6）にのみ保存される。

### VCA（Vca: ボイス単位の振幅オーバーレイ）

FM合成 → Vcf通過後の信号に乗算するボイス単位の振幅オーバーレイ（`sound-core::Vca`トレイト、
具象実装`VoiceAmp`）。振幅の時間変化は **Gain FG**（[ファンクションジェネレーター](#ファンクションジェネレーターfgpitchcutoffgain)節）が担う。
旧「VCA EG」の後継で、5段EG＋Loop/Floor/Curveを持つ。音量に負値は無いためDepthは持たず、
**Floorがうねりの深さ**を担う（出力＝ゲイン直結、ループ=トレモロ／ワンショット=通常アンプEG）。

既定（Loop=0・AR=255・D1L=255・RR=0）ではアタックは数サンプルで完了しゲイン1.0に張り付き、
リリースは最遅（RR=0≈284.9秒＝実質ゲートを閉じない）ため、離鍵後の減衰は各オペレーター本来のRRに委ねられる。
これによりFM本来のキャリアEG（`operator.rs`のオペレーター単位EG）のアタックもリリースも変えない**透過的レイヤー**
として働く（二重EG化を避ける既定設計）。発音終了時のチャンネル回収はオペレーターのidle判定のみで行うため、
VCAが閉じなくても問題ない。**注意：** RRを速くすると（例：RR=255＝8.71ms）離鍵時に全チャンネルを短時間で
ゼロへ閉じ、各オペレーター本来のリリース尾を打ち消す（リリース瞬断）。ユーザーがGain FGのAR/RR等を変えることで、
キャリアEGとは独立にアタック/リリースの「量」を上書きする効果音的な表現に使える（その際は上記の瞬断に留意）。

**質感LFOのVolume行きの合流点：** [質感LFO](#質感lfo5波形専用焼き込み)の
Destination=Volume（5波形の飛び道具）は、このVcaゲインへ**乗算合流**する
（`Vcaゲイン = Gain FG出力 × (1 + LFOデルタ)`）。適用点はSVFフィルターの**後**（Vca段）である。

> **自励発振時の挙動差：** Volume変調の適用点がVCF前（旧方式のVCF前volume_gain）からVCF後（Vca段）へ
> 変わったため、Self-Oscillation ONでフィルターが自己発振している音に対しても、新方式では変調が掛かる
> （旧方式では自励発振成分はVCF後に生じるため、VCF前のVolume変調が乗らなかった）。

---

## ファンクションジェネレーター（FG：Pitch / Cutoff / Gain）

38x6のボイス単位モジュレーションは、**3つのファンクションジェネレーター（FG）スロット**——
Pitch / Cutoff / Gain——に集約する。3スロットは共通の部品「ループ可能EG」を持ち、**行き先とDepthの
性質だけが異なる**。MC-505のPitch / TVF（Filter）/ TVA（Level）3エンベロープに対応する構成である。

FGは「**一発（ワンショット）にもループにもなる**」変調源で、アナログシンセ的なスイープ／うねり
（アシッドフィルター・シンセタム・トレモロ）を一次源として持てる。ループさせればLFOに相当するが、
LFOのサイン波では出せない「上りと下りが非対称な軌跡」を出せる点が固有価値（[質感LFO](#質感lfo5波形専用焼き込み)との境界参照）。

### 共通部品：ループ可能EG

FGの本体は、38x6本体のFM EGと同じ**5段OPM形式EG**（`sound-core::Eg`、AR→D1R→D1L→D2R→RR＋Idle）に、
次の4項目を足した拡張である。**新規の別部品は作らない**（＝Loop=0かつDelay=0で既存の5段EGと完全に一致し、後方互換）。

| 追加項目 | 値域 | 役割 |
|---|---|---|
| Loop | 0/1（既定0） | 0=ワンショット（従来のADSR挙動そのまま）／1=ループ |
| Floor | 0〜255（既定0） | ループ時の折り返しの底レベル（0=完全開閉、上げるほど浅い連続的なうねり） |
| Curve | 0/1（既定0） | 0=線形（角の立つ三角）／1=サイン風（レイズドコサイン `0.5-0.5cos(π·進行度)` で角を丸める） |
| Delay | 0〜255（既定0） | キーオンからAR開始までの遅延。0〜10秒（線形、チップ内LFO/質感LFOのDelayと同じマッピング）。CC78の補正対象（下記） |

- **Loop=0（既定）**：キーオンで AR→D1R→D1L→D2R と推移し、キーオフで RR。**従来の5段EGと完全に同一**
  （既存パッチは Loop=0 扱いで挙動不変）。
- **Loop=1**：**Floor⇄peak を AR（開く/戻り）と D1R（閉じる/下降）が独立レートで往復**するループ
  （LoopToD1L相当・膝なし）。上りと下りの非対称を保てる点が、対称波形しか出せないLFOサイン波との
  決定的な差。**キーオフで現在位置から RR へ離脱**する（連続性）。
- **Curve** はEGのタイミング計算（一定レート折り返し）は変えず、**出力レベルだけを整形**する。上り(AR)と
  下り(D1R)の非対称を保ったまま角を丸められる（アシッド系のうねりに有効。プロトタイプA/B検証で確定）。

### 3スロット

| スロット | 行き先 | Depth | 適用点 | ループ用途 / ワンショット用途 |
|---|---|---|---|---|
| **Pitch FG** | F-Number（全Op） | **バイポーラ**（中心128、±） | ピッチ | ビブラート / シンセタム（一発のピッチ下降・上昇） |
| **Cutoff FG** | Filter Cutoff（0〜255） | **バイポーラ**（中心128、±） | Cutoff（VCF前） | オートワウ・アシッド / フィルタースイープ |
| **Gain FG** | Vcaゲイン | なし（Floorが深さ役） | Vcaゲイン（VCF後） | トレモロ / 通常アンプEG |

各スロットは共通のループ可能EG（AR/D1R/D1L/D2R/RR/Loop/Floor/Curve/Delay）を持ち、加えて Pitch/Cutoff は
**バイポーラDepth**（0〜255、中心128＝変調なし。128超＝＋方向、128未満＝−方向）を持つ。
Gain は音量に負値が無いためDepthを持たず、Floor が深さを担う。

- **Pitch FG（新規）**：ピッチ変調の一次源。バイポーラDepthにより、キーオン一発の**ピッチ下降
  （上から落とす）と上昇（下から上げる）の両方**を作れる（シンセタム）。ループ時はビブラート。
- **Cutoff FG**：旧「Filter EG」の後継。Depthを**バイポーラ化**（旧0〜255単極→中心128の±。
  MC-505のFilter Env Depth −63〜+63相当）。カットオフを開く/閉じる両方向のスイープが可能。
  `実効Cutoff = clamp(Cutoffベース + FG出力 × (Depth-128)/128, 0, 255)`。
- **Gain FG**：旧「VCA EG」の後継。出力はVcaゲインへ直結（ループ=トレモロ／ワンショット=通常アンプEG）。

各行き先では、FG・[チップ内LFO](#チップ内lfo)・[質感LFO](#質感lfo5波形専用焼き込み)が
**加算的に共存**する（例: Cutoffは Cutoff FGのスイープ＋質感LFOのS&H が積み重なる。Pitchは
Pitch FG＋チップ内LFO(PMS/PMD)＋質感LFO(Dest=Pitch) が積み重なる）。

### 演奏層による補正（Pitch FGのみ）

②パート状態と③ジェスチャーのCCは **Pitch FGのみ** を補正する（Cutoff/Gain FGはパッチ/NRPN専用）。
モジュレーションホイール＝ビブラート＝Pitch FGという古典的対応に従う。

| CC | 層 | 作用 |
|---|---|---|
| CC1（Modulation Wheel） | ③ジェスチャー | Pitch FG Depthへの**瞬間加算**（×RPN(0,5)/127でセント換算） |
| CC77（Vibrato Depth） | ②パート状態 | Pitch FG Depthへの**0起点パート加算** |
| CC76（Vibrato Rate） | ②パート状態 | Pitch FGの速さ（AR/D1Rを一括スケール）への**64中心相対補正**（64=無補正） |
| CC78（Vibrato Delay） | ②パート状態 | Pitch FG Delayへの**64中心相対補正**（64=無補正） |

**実効Depth（三層加算）：** ①パッチが定義する基準Depthに②③を**加算**する（乗算ではない）。②③がゼロでも
①のビブラートは鳴る（GM2互換：ホイールを触らなくてもパッチのビブラートは効く）。Pitch FGはループ時に
AR/D1R の2レートを持つため、CC76「Vibrato Rate」は両レートを一括でスケールする（全体の速さ）。

**具体式（実装確定、ym38x6-vst）：**

- **CC76（Rate）**：AR/D1Rは指数マッピング（`rate_to_delta`）のため、生コードへの単純加算では
  ベース値によって体感速度が大きく変わってしまう（「一括スケール」の語義に反する）。そのため
  `sound-core::Eg::tick`の`rate_scale`引数（時間軸への乗算係数、KSRと同じ仕組み）を経由する：
  `rate_scale = cc76_to_rate_scale(CC76生値0〜127)`（64→1.0倍、0→0.25倍、127→4.0倍の指数カーブ）。
  `ChannelParams`を経由せず、`pitch_bend`/`channel_volume`と同じ単一ボイス直接setter
  （`set_pitch_fg_rate_scale`）でエンジンへ渡す
- **CC78（Delay）**：`delay_to_seconds`が線形（0〜255が0〜10秒に比例）のため、生コード空間での
  加算がそのままスケールと等価になる。`実効Delay = clamp(Delayベース値 + (CC78生値-64), 0, 255)`
- **CC77（Depth）**：`実効Depth内訳 += cc_to_u8(CC77生値)`（0起点、そのまま加算）
- **CC1（Depth、セント換算）**：`セント = (CC1生値/127) × (RPN0,5生値 × 50/64)`で求めたセント量を、
  Pitch FGの`(Depth-128)/128×1200`という既存のDepth→セント変換式の逆算で0〜255単位空間へ戻し
  （`depth_units = round(セント/1200×128)`）、実効Depthへ加算する

### 実装状況

FGは`sound-core::Eg`にLoop/Floor/Curveを追加した拡張として実装する（ステップ6）。Loop=0既定で既存の
5段EGテスト群が「音が1サンプルもズレない」ことを担保しながら進める。旧`Filter EG`（`VoiceFilter`）／
`VCA EG`（`VoiceAmp`）はそれぞれ Cutoff FG／Gain FG へ移行し、Pitch FGを新設する。パッチの
FGフィールド名（`pitch_fg`/`cutoff_fg`/`gain_fg`）と旧フィールド（`filter_eg_*`/`vca_eg_*`）の
serde互換移行はステップ6で実施する。

Delayはステップ7でCC78「Pitch FG Delayへの64中心相対補正」の実装対象が存在しないという矛盾が
見つかったため追加した項目（`EgParams::delay`、`#[serde(default)]`で後方互換）。Pitch/Cutoff/Gain
FG共通の`EgParams`拡張のため3スロットとも「効果開始までの遅延」を持つ（オペレーター単位のFM EGへは
展開せず、`OperatorParams`は12個の公開パラメーターのまま据え置く）。

---

## マスターエフェクト（Reverb / Chorus）

GM2準拠のセンドエフェクト2系統。各ボイス（FM合成 → SVFフィルター後の信号）からセンドレベルでReverb/Chorusバスに送り、マスターでミックスする。

| パラメーター | 制御 | 値域 | 備考 |
|------------|------|------|------|
| Reverb Send | CC91 | 0〜255 | チャンネル単位 |
| Reverb Type | NRPN | 0〜7 | enum（下記） |
| Reverb Time | NRPN | 0〜255 | マスター |
| Chorus Send | CC93 | 0〜255 | チャンネル単位 |
| Chorus Type | NRPN | 0〜7 | enum（下記） |
| Chorus Mod Rate | NRPN | 0〜255 | マスター |
| Chorus Mod Depth | NRPN | 0〜255 | マスター |
| Chorus Feedback | NRPN | 0〜255 | マスター |
| Chorus Send To Reverb | NRPN | 0〜255 | マスター。GM2準拠、ChorusバスからReverbバスへの送り量 |

※「Reverb Time」「Chorus Mod Rate/Depth/Feedback」「Chorus Send To Reverb」は、
NRPNに加えてnice-plugのマスターパラメーターとしても公開する
（MIDI実装方針のDAWオートメーション参照）。
「Reverb Type」「Chorus Type」はNRPN専用（DAWオートメーション対象外）。

**Reverb Type enum（GM2/GS準拠）：**

| 値 | タイプ |
|---|---|
| 0 | Room1 |
| 1 | Room2 |
| 2 | Room3 |
| 3 | Hall1（デフォルト） |
| 4 | Hall2 |
| 5 | Plate |
| 6 | Delay |
| 7 | Panning Delay |

**Chorus Type enum（GM2/GS準拠）：**

| 値 | タイプ |
|---|---|
| 0 | Chorus1（デフォルト） |
| 1 | Chorus2 |
| 2 | Chorus3 |
| 3 | Chorus4 |
| 4 | Feedback Chorus |
| 5 | Flanger |
| 6 | Short Delay |
| 7 | Short Delay (FB) |

**信号フロー：**
```
[各ボイス: FM合成 → SVF] → Dry ──┬─ ×Reverb Send(CC91) ──→ Reverbバス ─┐
                                  │                                     ├→ Master Out
                                  └─ ×Chorus Send(CC93) ──→ Chorusバス ─┤
                                                  │                     │
                                                  └ ×Chorus Send To Reverb → Reverbバスへ
```

**実装方式：**
- Reverb：コムフィルタ＋オールパスフィルタ構成のアルゴリズミックリバーブ（Room1〜Plate）。Delay/Panning Delayタイプはフィードバックディレイラインで実現
- Chorus：LFO変調ディレイライン（Chorus1〜4、Flanger、Feedback Chorus）。Short Delay系タイプは変調なしの短ディレイ
- `sound-core`に依存ゼロのDSPモジュールとして実装し、各エンジンの`render()`出力に対してapp/plugin側のレンダリング後段で適用する

**OPQコンバーターとの関係：**
エフェクトはOPQ由来パラメーターではないため、OPQ変換対象外。38x6独自フォーマット（.38x6）にのみ保存される。

---

## キーオン（OP単位キーオン/オフ）

- オペレーター0〜3を**それぞれ独立に**キーオン/オフできる（38x6独自拡張。OPN/OPM実機はチャンネル単位のみ）
- **全OP独立・特別なマスターOPは持たない**。あるOPだけ消す／鳴らすが自由にできる
- ノート全体の停止は通常のNote-Off（または CC120/CC123）で行う
- 作曲支援アプリでのアボイドノート音量制御に応用可能
- 制御は CC103〜106（Op0〜Op3）。詳細はMIDI実装方針のOperator Key On/Offセクション参照

> 補足: 以前は「Op3がマスター（Op3 Off で全OP強制Off）」という特別扱いがあったが、CC103〜106による
> OP独立キーオン/オフと通常のNote-Offで役割が満たせること、また Op3=最終キャリアという前提が
> Algorithm 7（全並列）等では成立しないことから廃止し、全OPを対等に扱う方式へ統一した。

---

## モジュレーションの三層モデル（音色・パート状態・ジェスチャー）

38x6のモジュレーション（揺れ・表情）は、値の**帰属**を3つの層に分けて管理する。決め台詞は
「**パッチが定義し、CCが補正し、ジェスチャーが今を動かす**」。

| 層 | 正体 | 保存先 | 設定経路 |
|---|---|---|---|
| ①音色 | パッチ（.38x6）が定義する音そのもの | `.38x6`ファイル（`Ym38x6Patch`） | DAWパラメーター・NRPN |
| ②パート状態 | MIDIチャンネル（パート）ごとの持続的な補正 | 曲データ（SMF等）のCC | MIDI CC |
| ③ジェスチャー | 今この瞬間の揮発的な動き | どこにも保存しない（揮発） | Pitch Bend・AT・一部CC |

### 三層帰属表

| パラメーター群 | 層 | 保存先 | 設定経路 |
|---|---|---|---|
| オペレーター12種×4 / Algorithm / Feedback / チップ内LFO6項目 / フィルターDSP4項目 / **Pitch/Cutoff/Gain FG 各項目** / **質感LFO8項目** | ①音色 | `.38x6`（`Ym38x6Patch`） | DAWパラメーター・NRPN |
| CC7/10/11（音量・パン・エクスプレッション）/ CC91/93（センド）/ CC71/74（レゾナンス・カットオフ）/ CC72/73/75（RR/AR/D1R）/ **CC76/77/78（Pitch FG補正）** / **CC2/CC4（Expression Destination加算、下記）** / CC5/65（ポルタメント）/ RPN群 / Bank・Program | ②パート状態 | 曲データ（SMF等）のCC | MIDI CC |
| **CC1（モジュレーションホイール）** / Pitch Bend / アフタータッチ / CC64/66/67（ペダル）/ CC103〜106（OP単位キーオン） | ③ジェスチャー | 保存しない（揮発） | MIDI |

**補強規則：**
- **NRPN＝①の編集経路**（パッチそのものを書き換える）／**CC＝②③の補正**（パッチは書き換えない）。
- **CC121（Reset All Controllers）は③層のみをリセットする**（②パート状態・①音色は保持）。
- アフタータッチのDestination加算（AT Destination／Poly AT Destination、NRPN節参照）は③層に分類され、
  「ベース値＋補正値」という三層加算モデルの先行例である。

**実効値＝三層加算：** あるモジュレーション量の実効値は、①パッチが定義する基準値に、②パート補正と
③ジェスチャーを**加算**して求める（乗算ではない）。したがって②③がゼロでも①が定義した揺れは鳴り続ける
（GM2互換：モジュレーションホイールを触らなくてもパッチのビブラート/トレモロは効く）。具体式はFG節・CC表を参照。

### 単一カレントパッチ前提とマルチティンバー（将来）

現状の38x6は「エンジン全体で1つのカレントパッチ」を前提としており、全MIDIチャンネルが同じ音色を共有する
（VST/gesture-appとも）。三層モデルはこの前提を**マルチパート化（1エンジンで複数パートが別音色・別CC状態を
持つ）への前提条件**として整理したものである。②パート状態をMIDIチャンネル単位で独立に持てるようにすれば、
マルチティンバー音源へ拡張できる。マルチパート実装自体は本改訂のスコープ外（将来のフェーズ）。

---

## チップ内LFO

FMチップに内蔵された「音作り」用のLFO（プリセット・NRPNで設定）。VCO層（`ym38x6-core`）に属する
**チップ固有の変調源で、VCOを差し替えると消える**（旧称「音色LFO」。「チップ内」はこのレイヤー帰属を
名指しした呼称で、opz2x6/opm2x6が写像する先はこのチップ内LFOである）。演奏用のモジュレーション（[FG](#ファンクションジェネレーターfgpitchcutoffgain)／[質感LFO](#質感lfo5波形専用焼き込み)）とは完全に独立しており、演奏時のビブラート/トレモロには影響しない。

> **チップ内LFOと演奏系モジュレーションの違い（混同防止）：** チップ内LFO＝チップ忠実のPMS/AMS×PMD/AMD（音作り・パッチ内で完結、VCO差し替えで消える）／FG・質感LFO＝38x6拡張の永続層（ビブラート/トレモロ/オートワウ等の軌跡はFG、5波形の飛び道具は質感LFO）。両系統は独立して同時に効く。

| 項目 | 値 | 備考 |
|------|-----|------|
| 波形 | 三角波 | OPQ由来・固定 |
| 周波数 | 3bit → 8bit（0〜255） | |
| PMD（ピッチ変調深さ） | 0〜255 | |
| AMD（振幅変調深さ） | 0〜255 | |
| PM感度（PMS） | チャンネルごと 3bit → 8bit | 指数カーブ（0=オフ、1〜255でOPM PMS=1(+/-5cents)〜PMS=7(+/-700cents)を両端アンカーとした指数補間） |
| AM感度（AMS） | チャンネルごと 2bit → 8bit | 指数カーブ（0=オフ、1〜255でOPM AMS=1(23.9dB)〜AMS=3(95.6dB)を両端アンカーとした指数補間をdepth=1-10^(-dB/20)で振幅深度に変換） |
| Delay | 0〜255 | キーオンからLFO効果開始までの遅延時間。OPZ(TX81Z)のLFO DELAY由来（opz2x6変換で使用。手設計音色では現状常に0） |
| AMオン/オフ | オペレーターごと | |

周波数/PMD/AMD/Delay/PMS/AMSの6項目は、チャンネル単位のDAWパラメーターとして公開する（MIDI実装方針セクション参照）。

---

## 質感LFO（5波形専用・焼き込み）

旧「チャンネルLFO（LFO1/LFO2）」を再編し、**軌跡（Floor⇄peak）では表せない波形だけ**を担う1基のLFO
（旧称: パフォーマンスLFO → チャンネルLFO → 質感LFO）。ビブラート/トレモロ/オートワウの持続的な揺れは
[FG](#ファンクションジェネレーターfgpitchcutoffgain)のループモードへ移行したため、質感LFOが担うのは
**FGで出せない5波形**——矩形・台形・S&H・ランダム・カオス——に限定される。

> **境界規則（FG↔質感LFO）：** 「**パラメーター化された時間軌跡なら FG、そうでない（固定ルックアップ
> 波形／生成器）なら 質感LFO**」。三角・のこぎり・サインはFGのループ（Floor⇄peakをAR/D1Rで往復）で
> 出せるため質感LFOは持たない。両者の重なりはゼロ。S&H/ランダム/カオスは値が乱数抽選や写像反復で
> 決まる「生成器」で、FGの軌跡パラメーター（Floor/AR/D1R/Curve）が生かせないためLFO側に隔離する。

**焼き込み専用：** 質感LFOは**①音色（パッチ）が所有**し、演奏CC（CC1/76/77/78）による補正は受けない
（それらは[Pitch FG](#ファンクションジェネレーターfgpitchcutoffgain)へ）。既定 Depth=0 で不可視＝初心者の
既定経路には現れない飛び道具。5波形は予測不能系のため手で動かす必要がなく、旧LFO2的な set-and-forget として扱う。

### パッチ所有パラメーター（8項目）

全項目を`ChannelParams`が所有する（`.38x6`に保存）。既定はDepth=0（鳴らない）＝旧パッチ挙動と互換。

| 項目 | 値域 | 既定 | 備考 |
|------|------|------|------|
| Waveform | 0〜4（5種） | 0=矩形波 | 下記Waveform enum |
| Destination | 0〜3 | 0=Pitch | 下記Destination enum |
| Rate | 0〜255 | 0 | 0.01Hz〜20Hz（指数） |
| Depth | 0〜255 | 0 | 揺れ量（既定0＝鳴らない） |
| Delay | 0〜255 | 0 | キーオンから効果開始までの遅延。0〜10秒（線形）。Fadeとは独立 |
| Fade Mode | 0〜3 | 0=ON-IN | 下記Fade Mode enum |
| Fade Time | 0〜255 | 0 | 0〜10秒（線形）。0=フェード無効 |
| Offset | 0〜255（中心128） | 128 | 波形の中心シフト-100〜100（生値-1.0〜1.0に`offset/100.0`を加算してクランプ） |

**Waveform enum（5種）：**

旧チャンネルLFOの8波形から、FGへ移行する三角/サイン/のこぎりを除いた5波形に絞り、0起点で再番号付けした。

| 値 | 波形 |
|---|---|
| 0（デフォルト） | 矩形波 |
| 1 | 台形波 |
| 2 | S&H（ランダム、階段状） |
| 3 | Random（周期ごとの目標値を位相で線形補間する滑らかなランダムウォーク） |
| 4 | Chaos（ロジスティック写像`x=3.9x(1-x)`による決定論的カオス、周期内はホールド） |

**Destination enum：**

| 値 | 宛先 | 適用点 |
|---|---|---|
| 0（デフォルト） | Pitch | F-Number全Op |
| 1 | Volume | **Vcaゲイン乗算合流・VCF後** |
| 2 | TL（キャリア一括） | TL（VCF前）・38x6拡張のみ |
| 3 | Cutoff | Cutoff（VCF前）・38x6拡張のみ |

各行き先ではFG・チップ内LFOと**加算的に共存**する（例: Cutoffは Cutoff FGのスイープ＋質感LFOのS&H が
積み重なる）。Volume（Destination=1）は`Vcaゲイン = Gain FG出力 × (1 + LFOデルタ)`でVca段（VCF後）へ乗算合流、
TL（Destination=2）はキャリアTL（VCF前）へ加算する（明るさ方向）。Depthは全行き先で`clamp(Depth, 0, 255)`
（質感LFOは焼き込み専用のためCCによる加算補正はなく、演奏系CC1/76/77/78はPitch FGへ行く）。

**Fade Mode enum：**

Delay（既存のハードカットオーバー）とは独立して共存する。DelayがゲートそのものでFadeはゲート解除後の振幅ゲインのランプ。

| 値 | 意味 |
|---|---|
| 0（デフォルト） | ON-IN：note_on後、Delay経過を起点に振幅ゲインが0→1にフェードイン |
| 1 | ON-OUT：note_on後、Delay経過を起点に振幅ゲインが1→0にフェードアウト |
| 2 | OFF-IN：note_off前は振幅ゲイン0、note_off後にFade Timeかけて0→1にフェードイン（リリーステール中のみ効く） |
| 3 | OFF-OUT：note_off前は振幅ゲイン1、note_off後にFade Timeかけて1→0にフェードアウト（リリーステール中のみ効く） |

### 実装状況

既存`sound-core/src/lfo.rs`（`PerformanceLfo`、8波形実装済み）から三角/サイン/のこぎりを除いた
**5波形に絞った1基**を`ChannelParams`が所有する。適用先は`PerformanceLfoTarget`トレイト
（Destination 0=Pitch/1=Volume/2=TLキャリア一括/3=Cutoff）。`set_channel_params`が他のフィールドと
同様に毎ブロック発音中ボイスへ伝播するため、NRPN/DAWパラメーター変更は発音中のノートにもリアルタイムに
反映される。演奏系CC（CC1/76/77/78）は質感LFOには繋がらず[Pitch FG](#ファンクションジェネレーターfgpitchcutoffgain)へ行く。

> **後方互換規則：** 質感LFOの各フィールドは`#[serde(default)]`とする。旧`ChannelParams.perf_lfo_shape`
> （波形/Fade Mode/Fade Time/Offset）や旧`lfo1`/`lfo2`を持つ既存`.38x6`は、波形が5波形（矩形/台形/S&H/
> Random/Chaos）に該当すれば質感LFOへ、三角/サイン/のこぎりなら対応する[FG](#ファンクションジェネレーターfgpitchcutoffgain)の
> ループ設定へ移行読み込みする。型名`PerformanceLfo`・`perf_lfo_shape`等のコード名の整合は実装フェーズ（ステップ6）で解消する。

---

## アルゴリズム拡張モード（将来実装）

SY77/TG77（AFM音源, 1989年）の設計を参考に：
- 表向き：固定8アルゴリズムから選択（初期UI）
- 内部：オペレーターごとのルーティングレジスタ
- 将来的にルーティングビットを公開する拡張モードを追加可能

---

## あえて省いたもの

| 機能 | 理由 |
|------|------|
| SSG-EG | 需要なし |
| ノイズ | 需要なし |
| DT2 | 需要なし |
| CSMモード（タイマー駆動の自動キーオン） | 需要なし。OP単位F-Number＋CC103〜106によるOP単位キーオン/オフ（MIDI実装方針参照）をシーケンサーから高速送信することでCSM風の効果を代替可能 |

---

## MIDI実装方針

### DAWオートメーション（nice-plugパラメーター）

以下の全パラメーターをnice-plugのParam として公開し、Cubase・Logic等でのDAWオートメーションに対応する。

**プリセット選択（1個）：**
Program（0=Manual：DAWパラメーター/NRPNで手動チューニングしたパッチを使う。1〜128=Program 0〜127：CC0/CC32で選択中のbankの該当プリセットへ切り替える。VST3でMIDI Program Changeの代替として使う、Bank Select / Program Changeセクション参照）

**チャンネル単位（48個）：**
Algorithm / Feedback / 質感LFO Rate / 質感LFO Depth / 質感LFO Delay / 質感LFO Waveform / 質感LFO Fade Mode / 質感LFO Fade Time / 質感LFO Offset / チップ内LFO Freq / チップ内LFO PMD / チップ内LFO AMD / チップ内LFO Delay / PMS / AMS / Filter Cutoff / Filter Resonance / Pitch FG AR / Pitch FG D1R / Pitch FG D1L / Pitch FG D2R / Pitch FG RR / Pitch FG Depth / Pitch FG Floor / Pitch FG Delay / Pitch FG Loop / Pitch FG Curve / Cutoff FG AR / Cutoff FG D1R / Cutoff FG D1L / Cutoff FG D2R / Cutoff FG RR / Cutoff FG Depth / Cutoff FG Floor / Cutoff FG Delay / Cutoff FG Loop / Cutoff FG Curve / Gain FG AR / Gain FG D1R / Gain FG D1L / Gain FG D2R / Gain FG RR / Gain FG Floor / Gain FG Delay / Gain FG Loop / Gain FG Curve / Reverb Send / Chorus Send

（質感LFOは7個＝Rate/Depth/Delay/Waveform/Fade Mode/Fade Time/Offset。DestinationのみNRPN専用のためDAWパラメーターには含めない。焼き込み専用のため演奏CC補正は受けない。3つのFG〈Pitch/Cutoff/Gain〉は共通EG=AR/D1R/D1L/D2R/RR＋Loop/Floor/Curve/Delayで、加えてPitch/Cutoffはバイポーラ Depth を持つ〈Gainは Floor が深さ役でDepth無し〉。Loop/CurveはAlgorithm同様、離散トグルだがNRPNとDAWパラメーターの両方で公開する。DelayはAR等と同じ連続値のためNRPN専用ではなくDAWパラメーターのみ〈NRPN番地は持たない〉。仕様上の当初案は45個だったが、ステップ7実装時にCC78の対応先が存在しない矛盾が見つかりDelayを新設した結果48個になった。）

**オペレーター単位（12個 × 4op = 48個）：**
TL / AR / D1R / D2R / D1L / RR / MUL / DT1 / KS / AME / Velocity Sensitivity / OP Fine Tune

**マスター単位（5個）：**
Reverb Time / Chorus Mod Rate / Chorus Mod Depth / Chorus Feedback / Chorus Send To Reverb

**離散パラメーター（NRPN専用、DAWオートメーション対象外）：**
以下はnice-plugパラメーターを持たず、NRPN/GUI操作でのみ設定する
（CC91/93やマスターエフェクトの「マスター単位」パラメーターのように、
NRPN/CCとnice-plugパラメーターを併用する項目とは異なる点に注意）。

Algorithm / Waveform（WF）per op / Filter Type（LP/HP/BP）/ Filter Self-Oscillation / AT Destination / Poly AT Destination / 質感LFO Destination / Reverb Type / Chorus Type

※「Algorithm」「質感LFO Waveform」「質感LFO Fade Mode」「Pitch/Cutoff/Gain FG Loop」「Pitch/Cutoff/Gain FG Curve」は例外的に、対応するNRPNに加えて上記チャンネル単位のnice-plugパラメーターとしても公開する（DAWオートメーション/GUIノブ操作とNRPN/外部MIDIコントローラーの両方から設定できる）。

### MIDI CC（GM2準拠）

| CC | GM2定義 | 38x6割り当て | GM2との関係 |
|---|---|---|---|
| CC 1 | Modulation Wheel | ③ジェスチャー：Pitch FG Depthへ瞬間加算（×RPN(0,5)/127でセント換算） | 準拠（FG参照） |
| CC 5 | Portamento Time | Portamento Time | 完全準拠（下記参照） |
| CC 7 | Channel Volume | Volume | 完全準拠 |
| CC 10 | Pan | Pan | 完全準拠 |
| CC 11 | Expression | Expression | 完全準拠 |
| CC 64 | Damper Pedal | Sustain | 完全準拠 |
| CC 65 | Portamento On/Off | Portamento | 完全準拠（下記参照） |
| CC 66 | Sostenuto | Sostenuto | 完全準拠（下記参照） |
| CC 67 | Soft Pedal | Soft Pedal | 完全準拠（下記参照） |
| CC 71 | Resonance | Filter Resonance | 完全準拠 |
| CC 72 | Release Time | RR（キャリア一括） | 準拠 |
| CC 73 | Attack Time | AR（キャリア一括） | 準拠 |
| CC 74 | Brightness | Filter Cutoff | 完全準拠 |
| CC 75 | Decay Time | D1R（キャリア一括） | 準拠 |
| CC 76 | Vibrato Rate | ②パート状態：Pitch FGの速さ（AR/D1R一括）へ64中心相対補正 | 完全準拠 |
| CC 77 | Vibrato Depth | ②パート状態：Pitch FG Depthへ0起点加算 | 完全準拠 |
| CC 78 | Vibrato Delay | ②パート状態：Pitch FG Delayへ64中心相対補正 | 完全準拠 |
| CC 91 | Reverb Send Level | Reverb Send | 完全準拠（マスターエフェクト参照） |
| CC 93 | Chorus Send Level | Chorus Send | 完全準拠（マスターエフェクト参照） |
| CC 102 | （未定義） | Program Change 代替（VST3用。下記 Bank Select / Program Change 参照） | 38x6独自拡張（GM2未定義領域 CC102〜119 の先頭） |
| CC 103〜106 | （未定義） | Operator Key On/Off（Op0〜Op3。下記 Operator Key On/Off 参照） | 38x6独自拡張（GM2未定義領域） |
| CC 120 | All Sound Off | All Sound Off | 完全準拠 |
| CC 121 | Reset All Controllers | Reset All Controllers | 完全準拠 |
| CC 123 | All Notes Off | All Notes Off | 完全準拠 |
| CC 126 | Mono Mode On | Mono Mode | 完全準拠 |
| CC 127 | Poly Mode On | Poly Mode | 完全準拠 |

**GM2未定義領域（CC102〜119）の独自割り当て：** GM2にコントローラー定義のないCC102〜119を、38x6独自機能に使う（標準コントローラーとの意味的衝突を避けるため）。先頭のCC102をProgram Change代替（VST3ではMIDI Program Changeが受信できないため）、続くCC103〜106をOperator Key On/Off（Op0〜Op3）に割り当てる。旧実装はVOPMex互換でCC92をProgram Change代替に使っていたが、CC92はGM2でEffects 2 Depth（トレモロ）に予約され衝突するため廃止し、未定義領域の先頭CC102へ移した。

**Portamento（CC5/CC65）：**

CC65 ON時、新しいノート（新チャンネル）のF-Numberは、同一MIDIチャンネルで直前に発音したノートのF-Numberから、CC5（Portamento Time、0=即座〜127=数秒程度）で指定した時間をかけて目標値へ線形にグライドする。
直前のノートは別チャンネルで独立してリリース/サステインペダル等の影響を受けながら鳴り続けるため、グライドとの相互作用は発生しない。
作曲支援アプリのジェスチャーUIの「ゆっくり移動 → ポルタメント」（ジェスチャーレパートリー参照）も、この仕組み（Note-On + CC65 ON + CC5）で実現する。

**Sostenuto（CC66）：**

CC66 ON時点で発音中（Note-On済みかつNote-Off未到達）の全チャンネルに「サステイン保持」フラグを立てる。該当チャンネルはNote-OffされてもCC66 OFFまでReleaseに入らない（CC64と同じ仕組みを対象チャンネルのみに適用）。CC66 ON以降に新規キーオンしたノートは対象外。実装済み（smf2wav `render.rs`・ym38x6-vst `lib.rs`）、詳細は下記「サステインペダル（CC64）の実装方針」参照（`sostenuto`をCC64の`pending_release`と組み合わせて解放判定する）。

**Soft Pedal（CC67）：**

CC67 ON中に新規キーオンしたノートに対してのみ、実効TL（**キャリアのみ**、`ALGORITHMS[alg].carriers`）とFilter Cutoffを減算する。モジュレーターは対象外（音量＝キャリアTL、音色の丸まり＝Cutoffと役割を分離するため）。
```
実効TL(キャリアのみ) = clamp(TLベース値 - CC67値, 0, 255)
実効Cutoff = clamp(Cutoffベース値 - CC67値, 0, 255)
```
実装済み（smf2wav `midi.rs`の`apply_soft_pedal`／ym38x6-vst `midi.rs`の同名関数、複製）。減算はNote-On時点のCC67深さで焼き込み、以降にCC67の値が変わっても既に鳴っているノートには（簡易実装として）ライブ伝播ループ経由で現在値が再適用される（Note-Onのタイミングでsoft対象と判定されたノートに限る。ペダルの深さそのものが発音中に変化するのは実運用上まれなため許容）。

**All Sound Off（CC120）/ Reset All Controllers（CC121）/ All Notes Off（CC123）：**

GM2の定義に合わせ、CC120とCC123を区別する：CC120は**リリースを経ず即座に消音**（`Ym38x6Engine::silence_group`でボイスマップから即除去、残響も無い）、CC123は**通常のNote-Off相当**（リリースして自然減衰）。CC121は「モジュレーションの三層モデル」の**③ジェスチャー層のみ**リセットする（②パート状態・①音色は保持、上記「補強規則」参照）。対象はCC64/66/67ペダル・Pitch Bend・CC1(Mod Wheel)・アフタータッチ（Channel Pressure/Poly Key Pressure）。CC2/CC4/CC7/CC11/CC76〜78/センドレベル/RPN選択等（②パート状態）は保持する。実装済み（smf2wav `render.rs`・ym38x6-vst `lib.rs`）。

**サステインペダル（CC64）の実装方針：**

「ペダル対応」は2つの独立した関心事に分かれる。(a)はペダル本体、(b)はオプション拡張。

**(a) ホールドフラグ方式（実装済み）：** smf2wav（`render.rs`、e828515）・ym38x6-vst（`lib.rs`）とも実装済み。CC66(Sostenuto)/CC67(Soft Pedal)/CC120/CC121/CC123も同じホールドフラグ方式を拡張して実装している。

普通のサステイン（離鍵してもReleaseに入らない）は、チャンネルを新規確保せず、現状の1ノート=1チャンネル（[キーオン契約](#チャンネルidとキーオン契約sound-core共通)）のまま実現できる。`pedal_down: [bool; 16]` と `pending_release: [u128; 16]`（ビットN=ノート番号Nが離鍵済みだがいずれかのペダルで保持中）に加え、`keys_down: [u128; 16]`（物理的に押下中の鍵）・`sostenuto: [u128; 16]`（CC66 ON時点でのkeys_downスナップショット）・`cc67: [u8; 16]`／`soft_notes: [u128; 16]`（Soft Pedal深さと対象ノート）をMIDIチャンネルごとに持つ。

```
Note-On(note):   engine.note_on(id, ...)                 // 残響再アタックがデフォルト
                 pending_release.remove(note)            // 弾き直したら保持解除
                 keys_down.insert(note)
Note-Off(note):  keys_down.remove(note)
                 if pedal_down || sostenuto.contains(note) { pending_release.insert(note) }
                 else                                     { engine.note_off(note) }
CC64 ON:         pedal_down = true
CC64 OFF:        pedal_down = false
                 pending_release中、sostenuto保持中でもkeys_down中でもないノートをengine.note_off()してdrain
CC66 ON:         sostenuto = keys_down のスナップショット
CC66 OFF:        sostenuto中、pedal_down中でもkeys_down中でもないノートをengine.note_off()してdrain
                 sostenuto = 空
```

鍵がまだ押されている音は `pending_release` に入らない（ペダルを離しても鳴り続ける）。エンジンは無改造で済む（必要なら `set_sustain(channel, bool)` のようなAPIを足す案もあるが、まずはVST完結）。CC64/CC66 OFF・CC121の解放判定は共通ヘルパー（VST `release_unheld`／smf2wav同名関数）に集約し、「他のペダルに保持されておらず、かつ再押下中でもない」ノートだけを解放する。

> **要再検討（残響再アタック化に伴う）：** デフォルトの同一ID再キーオンは「同音チョーク」から「残響からの再アタック」に変わった。ピアノの弦再打弦としてはむしろ残響再アタックの方が物理的に自然だが、ペダルON時に「同じ音を重ねて独立に減衰」させたい場合は、**ペダルON中は同一ノートでも同一チャンネルIDを使わない**フラグを立て、新規ユニークIDを払い出せばよい（下記(b)と同じ仕組み）。これによりデフォルトの残響再アタック挙動を汚さずにスウェルを実現できる。

**(b) ペダルON時の同音重ね発音（スウェル、オプション拡張）：**

パッド系などで「ペダルを踏んで同じ音を重ねて独立に減衰させる」表現が欲しい場合のみ必要。ピアノでは物理的に起きないので(a)とは別物。チャンネルIDをノート番号から切り離し、ペダルON時のみ新規ユニークIDを割り当てて重ね発音を許す。38x6はチャンネル無制限のため「空きチャンネル検索」は不要で、IDを払い出すだけ（カウンター採番や `note番号 + 世代` 等）でよい。choke/overlapの判定はVST側のID採番ポリシーのみで決まり、エンジンのinsert-or-replace契約は変更不要：

- ペダルUP → `id = 通常のボイスID`（同音は残響再アタック、デフォルト挙動）
- ペダルDOWN → `id = 新規ユニークID`（重なって独立に減衰）

代償はVST側に `note番号 → 複数channel ID` の多重マップが要ること（Note-Off／ペダル解放時に対象IDをまとめてReleaseする）。現スコープ（ペダル無しのgesture-app・ピアノ系音色）では不要のため未実装。

### Pitch Bend

0xEn（14bit、中央値8192 = ベンドなし）をセント換算し、`set_pitch_bend_group(midi_ch, cents)`で
**MIDIチャンネル単位**で全ボイスへ一括適用する（VSTのボイスID `midi_ch*128+note` の `id >> 7` でグループ判定）。
和音は全ノートが一緒に滑らかに上下する（VGM/OPMのソフトビブラート＝チャンネル全体のピッチ移動に一致）。
ベンドレンジはRPN 0,0（Pitch Bend Sensitivity、半音）で設定する。現状は全MIDIチャンネル共通。
セント変換は `cents = (value/16383 - 0.5) * 2 * range半音 * 100`（VST側はnice-plugの正規化値0〜1を使用）。

### RPN（GM2準拠）

| RPN (MSB,LSB) | 内容 | デフォルト | 備考 |
|---|---|---|---|
| 0, 0 | Pitch Bend Sensitivity | ±2半音 | Pitch BendのF-Number換算レンジ（半音 + セント） |
| 0, 1 | Channel Fine Tuning | 0セント | F-Numberオフセット（±100セント） |
| 0, 2 | Channel Coarse Tuning | 0半音 | F-Numberオフセット（±64半音） |
| 0, 5 | Modulation Depth Range | 64（約50セント相当） | Pitch FGのCC1セント換算係数（FGセクション参照） |
| 127, 127 (7F,7F) | RPN/NRPN Null | - | 選択解除（誤操作防止のため必須） |

### Bank Select / Program Change

CC 0（MSB）+ CC 32（LSB）によるBank SelectとProgram Changeを実装する。
GM2のプログラム番号定義（0〜127の楽器カテゴリ）に準拠したバンク構成を採用する。

**バンク構成：**

| バンク | 内容 |
|---|---|
| Bank 0 | GM2プログラムマップ準拠（0〜127）。patchlabでの知覚記述子探索＋手動テンプレート設計で族ごとに作成 |
| Bank 1以降 | ユーザー定義プリセット |
| WAVEFORM_MEMORY_BANK+1以降 | OPQ/PSR-70の変換音色（tools/psr2x6で生成） |

**音色作成方針（フェーズ6・patchlabで実施）：**
- 当初はGM2リファレンス音からのML逆算合成（A-by-S）でBank 0を自動生成する計画だったが、
  FM/PCMの音響空間が重ならず行き止まりと判明（詳細はdocs/session_history.txt参照）
- 知覚記述子（明るさ・金属度・歪み等7軸）の自動分析＋probe探索による近傍パラメーターのルックアップを
  叩き台に、人手で族ごとにテンプレート設計（piano/brass/organ_template等）する方式に転換
- FMが苦手なカテゴリ（アコースティックピアノ・弦楽器・合唱等）は最近似の音色設計で代替
- 実際のGM2→音色マッピング表はフェーズ6で別途作成
- Bank 0の作成にはOPQ/PSR-70実機プリセットを直接流用しない。実機音色の変換はtools/psr2x6で別バンク（WAVEFORM_MEMORY_BANK+1以降）へ行う

**実装状況（フェーズ8・パラメーターUI・音色運用）：**
- CC0+CC32によるバンク選択とProgram Changeの受信を実装済み。
- Bank0のうちProgram 0（Acoustic Grand Piano）/4（Electric Piano 1）/80（Lead 1 Square）は、
  動作確認用に手動チューニングしたパッチ（`gm2_bank0_patch`）を使用する。設計概要：
  - Program 0: Algorithm 4（(O1→O2)+(O3→O4)）で2つの倍音グループを構成。各グループの
    モジュレーター(O1/O3)をキャリアより速く減衰させ、打鍵直後だけ倍音が立つハンマー
    アタックを表現。片方のグループをわずかにデチューンしコーラス感を付加
  - Program 4: Algorithm 4で、O1→O2をベル成分（MUL=14 + 強いフィードバックでメタリックな
    質感）、O3→O4をメインのトーン成分とするDX7系E.PIANOの定番構成
  - Program 80: Algorithm 7（全並列）でハーフsin²(waveform=3)を3本デチューンして重ね、O4を
    1オクターブ上で薄く重ねるデチューンユニゾン構成のシンセリード。d1l=255+d2r=0でキーオン中は
    無限サスティンとする
- 上記以外のbank・program番号は、bank・program番号から決定的に生成する暫定パッチ
  （`placeholder_patch`）を使用する（patchlabでのBank0設計・プリセットライブラリができるまでの
  暫定構成。優先順位はユーザープリセット(.38x6) > Bank0手動チューニング > `placeholder_patch`）。
- nice-plugの制約により、MIDI Program ChangeイベントはCLAPでのみ受信可能（VST3では
  受信不可、`MidiConfig::MidiCCs`の仕様）。CC0/CC32（Bank Select）はVST3でも受信可能。
- VST3でMIDIファイル等からProgram Changeを行うため、CC102（GM2未定義領域の先頭）を
  Program Change代替として受信する（値0〜127=プログラム番号、CC0/CC32バンクと組み合わせて
  `patch_for_program`で解決）。CLAPでは本来のMIDI Program Changeが届くため代替は不要だが、
  両対応のため変換ツール（tools/vgm2x6等）はProgram ChangeとCC102を併送する。
  旧実装はVOPMex互換のCC92を使っていたが、CC92はGM2のEffects 2 Depth（トレモロ）と
  衝突するため廃止した（MIDI CCセクション参照）。
- VST3でもプリセットを切り替えられるよう、nice-plugパラメーター「Program」
  （0=Manual/1〜128=Program 0〜127）を公開する。CC0/CC32で選択中のbankと組み合わせて
  Program Changeと同じロジック（`patch_for_program`）でパッチを解決する。
- Program Change受信時、または「Program」パラメーター変更時（0=Manual以外）は、
  選択したパッチがDAWパラメーターに優先して使われる（新規ノートおよび発音中
  チャンネルの両方）。DAWパラメーターでの編集に戻すには「Program」を0=Manualに戻す
  （MIDI Program Change経由で切り替えた場合はプラグインのリロードが必要、暫定。
  本格的なプリセット編集UIはフェーズ8「パラメーターUI」で別途検討）。

**ユーザープリセット（Bank1以降）の読み込み（フェーズ8・パラメーターUI・音色運用）：**

`.38x6`ファイル形式（JSON、`ym38x6-core::PresetFile`に定義）。常に1つの`bank`
（Bank Select相当、CC0×128+CC32、0〜16383）と、`presets`/`programs`いずれかの
プリセット配列を持つ（単数/複数の区別はなく、すべて以下のいずれかの形式）。

- **`presets`形式**：`bank`のプリセットを丸ごと定義する。
  ```json
  {
    "bank": 1,
    "presets": [
      {
        "program": 0,
        "name": "音色名1",
        "patch": {
          "operators": [
            { "tl": 0, "ar": 0, "d1r": 0, "d2r": 0, "d1l": 0, "rr": 0, "mul": 0, "dt1": 0, "ksr": 0, "am_enable": false, "velocity_sensitivity": 0, "waveform": 0 },
            { "...": "Op1" }, { "...": "Op2" }, { "...": "Op3" }
          ],
          "channel": {
            "algorithm": 0, "feedback": 0,
            "tone_lfo_freq": 0, "tone_lfo_pmd": 0, "tone_lfo_amd": 0, "tone_lfo_delay": 0, "pms": 0, "ams": 0,
            "filter_cutoff": 255, "filter_resonance": 0, "filter_type": 0, "filter_self_oscillation": true,
            "pitch_fg": { "ar": 0, "d1r": 0, "d1l": 255, "d2r": 0, "rr": 255, "depth": 128, "floor": 0, "loop": 0, "curve": 0 },
            "cutoff_fg": { "ar": 0, "d1r": 0, "d1l": 0, "d2r": 0, "rr": 0, "depth": 128, "floor": 0, "loop": 0, "curve": 0 },
            "gain_fg": { "ar": 255, "d1r": 0, "d1l": 255, "d2r": 0, "rr": 0, "floor": 0, "loop": 0, "curve": 0 },
            "texture_lfo": { "waveform": 0, "destination": 0, "rate": 0, "depth": 0, "delay": 0, "fade_mode": 0, "fade_time": 0, "offset": 128 }
          }
        }
      },
      { "program": 1, "name": "音色名2", "patch": { "...": "..." } }
    ]
  }
  ```
  `operators`は配列要素0〜3がOp0〜3に対応する（4要素固定）。各フィールドの意味・数値範囲は
  「オペレーターパラメーター」「チャンネルパラメーター」「フィルター」「チップ内LFO」「ファンクションジェネレーター」「質感LFO」の各節を参照。
  `pitch_fg`/`cutoff_fg`/`gain_fg`は3つのFGスロット（共通EG=ar/d1r/d1l/d2r/rr＋loop/floor/curve、
  Pitch/Cutoffはバイポーラ depth〈中心128〉、Gainはdepth無し）、`texture_lfo`は5波形専用質感LFOの8項目。
  いずれも`#[serde(default)]`により省略時は既定値（FG: loop=0で従来の5段EG挙動・depth=128でピッチ/カットオフ変調なし、
  texture_lfo: depth=0で無効）で読み込む。旧`filter_eg_*`/`vca_eg_*`/`lfo1`/`lfo2`/`perf_lfo_shape`フィールドを持つ
  既存`.38x6`は、それぞれ`cutoff_fg`/`gain_fg`/質感LFO・FGへ移行して読み込む（後方互換規則、FG節・質感LFO節「実装状況」参照）。
  現在は`Ym38x6Patch`のフィールドのみが対象で、ユーザー定義波形スロット(32〜255)の波形データ埋め込みは
  当該機能実装後に別途対応する（未実装）。`program`は`.opm`（VOPM）の`@:`番号(0-127)を継承可能な識別子。

- **`programs`形式**：`bank`内の一部の`program`だけを差分で追加・上書きする。
  ```json
  {
    "bank": 1,
    "programs": [
      { "program": 5, "name": "差し替え音色", "patch": { "...": "presets形式のpatchと同じ構造" } }
    ]
  }
  ```

**`PresetBank::load_from_dir`のロード処理：**

読み込み元ディレクトリは`%APPDATA%\ym38x6\presets`が存在すればそちらを使い、
無ければ`%USERPROFILE%\Documents\ym38x6\presets`にフォールバックする
（Explorerで見つけやすい場所を優先するための暫定措置）。

ディレクトリ内の`.38x6`ファイルをファイル名の昇順で読み込み、順に適用する
（ファイル名自体はbank/program番号を持たず、命名は自由）。
- `presets`形式：そのファイルが指定する`bank`のプリセットを一旦すべて削除し、
  このファイルのエントリーで再構築する（他bankのプリセットは保持される）
- `programs`形式：初期化せず、`(bank, program)`単位でこのファイルのエントリーを
  上書きマージする
- 同じ`(bank, program)`が複数ファイルで指定された場合、後から読み込まれた
  ファイルの内容が優先する

Program Change受信時に該当する`(bank, program)`のプリセットがあればその音色を、
なければ`placeholder_patch`を使う。

プリセットの保存（書き出し）UI・操作は未実装（フェーズ8「パラメーターUI」で対応予定）。
`PresetFile::to_json`は今後gesture-app等から呼び出して書き出す想定。

### NRPN

DAWオートメーション非対応の離散パラメーターおよびハードコントローラー向けの詳細制御に使用。

CC99（NRPN MSB）/CC98（NRPN LSB）でパラメーター番号を選択し、CC6（Data Entry MSB）で値を設定する（GM2準拠の標準的なNRPN手順）。
CC99/98またはCC101/100（RPN）に127,127（Null）を送ると選択解除される。

| 対象 | 備考 |
|---|---|
| Algorithm（CON） | 8種類、信号ルーティングが変わるため離散制御 |
| Waveform（WF）per op | 0〜31（ビルトイン）+ 32〜255（ユーザー定義） |
| Filter Type | 0=LP / 1=HP / 2=BP |
| AT Destination | Channel Pressureの加算先（destination enum、下記） |
| Poly AT Destination | Poly Key Pressureの加算先（destination enum、下記） |
| CC2 Destination | CC2（ブレス）の加算先（destination enum、下記。既定TLキャリア一括） |
| CC4 Destination | CC4（フット）の加算先（destination enum、下記。既定Filter Cutoff＝手動ワウ） |
| 質感LFO Destination | 質感LFOの加算先（destination enum、質感LFOセクション参照） |
| 質感LFO Waveform | 質感LFOの波形（0〜4、質感LFOセクション参照） |
| 質感LFO Fade Mode | 質感LFOのFadeモード（fade mode enum、質感LFOセクション参照） |
| Pitch/Cutoff/Gain FG Loop | 各FGのループON/OFF（0/1、FGセクション参照） |
| Pitch/Cutoff/Gain FG Curve | 各FGのカーブ（0=線形/1=サイン風、FGセクション参照） |
| Reverb Type | Reverbのタイプ（type enum、マスターエフェクトセクション参照） |
| Chorus Type | Chorusのタイプ（type enum、マスターエフェクトセクション参照） |
| Operator F-Number (Op0〜3) | OP単位F-Numberの上書き（13bit × 4、下記参照） |

**NRPN番号（MSB,LSB）：**

NRPN番号は旧チャンネルLFO（＝旧パフォーマンスLFO）実装で初めて定義し、本改訂で質感LFO＋FG Loop/Curveへ引き直した。MSB=0を「離散パラメーター」用に予約し、LSBを割り当てる。FGの連続パラメーター（AR/D1R/D1L/D2R/RR/Depth/Floor）は旧Filter/VCA EGと同様DAWオートメーション用パラメーターで、このNRPN離散表には含めない（Pitch FGの演奏補正はCC1/76/77/78で行う、FG節参照）。他の離散パラメーターのNRPN番号は実装時に追記する。

**データ入力方式（0〜255値）：** CC6（Data Entry MSB、0〜127）を2倍スケール（`cc_to_u8` ＝ `min(cc6 × 2, 255)`）して0〜255値として受け取る。列挙値（Destination/Waveform/Fade Mode/Type等）はCC6の値をそのまま使う。より高精度が必要な項目（Operator F-Number）はCC6=MSB+CC38=LSBで14bit送信する（下記参照）。

| 対象 | NRPN (MSB,LSB) | 値 |
|---|---|---|
| 質感LFO Destination | 0, 0 | 0=Pitch / 1=Volume / 2=TL（キャリア一括、38x6拡張のみ） / 3=Cutoff（38x6拡張のみ） |
| 質感LFO Waveform | 0, 1 | 0=矩形波 / 1=台形波 / 2=S&H / 3=Random / 4=Chaos |
| Reverb Type | 0, 2 | 0〜7（マスターエフェクトセクションのenum参照） |
| Chorus Type | 0, 3 | 0〜7（マスターエフェクトセクションのenum参照） |
| Reverb Time | 0, 4 | 0〜255 |
| Chorus Mod Rate | 0, 5 | 0〜255 |
| Chorus Mod Depth | 0, 6 | 0〜255 |
| Chorus Feedback | 0, 7 | 0〜255 |
| Chorus Send To Reverb | 0, 8 | 0〜255 |
| Algorithm | 0, 9 | 0〜7 |
| Waveform Op0〜3 | 0, 10〜13 | 0〜255（0〜31=ビルトイン、32〜255=ユーザー波形スロット） |
| Filter Type | 0, 14 | 0=LP / 1=HP / 2=BP |
| Filter Self-Oscillation | 0, 15 | 0=OFF / 1=ON |
| AT Destination | 0, 16 | 0〜5（destination enum、下記参照） |
| Poly AT Destination | 0, 17 | 0〜5（destination enum、下記参照） |
| Operator F-Number Op0 | 0, 18 | 0〜8191（13bit、CC6=MSB+CC38=LSBで送信、下記参照） |
| Operator F-Number Op1 | 0, 19 | 同上 |
| Operator F-Number Op2 | 0, 20 | 同上 |
| Operator F-Number Op3 | 0, 21 | 同上 |
| 質感LFO Fade Mode | 0, 22 | 0=ON-IN / 1=ON-OUT / 2=OFF-IN / 3=OFF-OUT |
| 質感LFO Rate | 0, 23 | 0〜255（焼き込み専用。CC補正なし） |
| 質感LFO Depth | 0, 24 | 0〜255（焼き込み専用。CC補正なし） |
| 質感LFO Delay | 0, 25 | 0〜255（焼き込み専用。CC補正なし） |
| 質感LFO Fade Time | 0, 26 | 0〜255（0=フェード無効） |
| 質感LFO Offset | 0, 27 | 0〜255（中心128=オフセットなし） |
| Pitch FG Loop | 0, 28 | 0=ワンショット / 1=ループ |
| Pitch FG Curve | 0, 29 | 0=線形 / 1=サイン風 |
| Cutoff FG Loop | 0, 30 | 0=ワンショット / 1=ループ |
| Cutoff FG Curve | 0, 31 | 0=線形 / 1=サイン風 |
| Gain FG Loop | 0, 32 | 0=ワンショット / 1=ループ |
| Gain FG Curve | 0, 33 | 0=線形 / 1=サイン風 |
| CC2 Destination | 0, 34 | 0〜5（destination enum、下記参照。既定5=TLキャリア一括） |
| CC4 Destination | 0, 35 | 0〜5（destination enum、下記参照。既定2=Filter Cutoff＝手動ワウ） |

**Expression Destination（表情コントローラーの加算先、AT/CC2/CC4共通）：**

Channel Pressure・Poly Key Pressure・CC2（ブレス）・CC4（フット）は、いずれも同じ
「揺らぎ系パラメーターへ非破壊的に加算する」モデルで実装する（旧称「AT Destination」を
表情ソース全体へ一般化したもの）。加算先（Destination）はソースごとに独立してNRPNで選択可能。

| ソース | NRPN | デフォルト |
|---|---|---|
| Channel Pressure（AT Destination） | 0, 16 | LFO PMD |
| Poly Key Pressure（Poly AT Destination） | 0, 17 | LFO PMD |
| CC2（ブレス、Breath Controller） | 0, 34 | TL（キャリア一括） |
| CC4（フット、Foot Controller） | 0, 35 | Filter Cutoff（**手動ワウ**） |

destination enum（共通）：

| 値 | 宛先 |
|---|---|
| 0 | LFO PMD（デフォルト） |
| 1 | LFO AMD |
| 2 | Filter Cutoff |
| 3 | Filter Resonance |
| 4 | TL（全オペレーター一括） |
| 5 | TL（キャリア一括） |

加算モデル：
```
実効値 = clamp(ベース値 + Σ(同じdestinationを指す全ソースの値), 0, 255)
```

複数の表情ソース（Channel Pressure・Poly Key Pressure・CC2・CC4）が同じdestinationを指す場合、
全ソースの値が加算される。Poly Key Pressure対応コントローラーは少数（MPE等）のため、
多くの環境ではChannel Pressure（またはCC2/CC4）のみが機能する。

**手動ワウ：** CC4（フット）のデフォルト行先をFilter Cutoffに設定しているため、
フットコントローラーで直接カットオフを開閉する古典的な「手動ワウ」がデフォルトで有効になる。
チップ内LFOのCutoff行先（質感LFO Destination=3、[質感LFO](#質感lfo5波形専用焼き込み)参照）による
「オートワウ」（自動で周期的に開閉）とは独立して積み重なり、演奏者がリアルタイムに手で
ワウ効果を制御しつつ、パッチ側の自動オートワウも同時に効かせられる。

**Operator F-Number（OP単位F-Number上書き）：**

NRPN(0,18)〜(0,21)がOp0〜Op3に対応する。CC6（Data Entry MSB）+ CC38（Data Entry LSB）で14bit値（0〜16383）として送信し、13bit（0〜8191）にclampして使用する（NRPNのデータエントリ精度14bitに対し1bit余裕がある）。CC38を送らないコントローラーではCC6のみ（128単位の粗い精度）でも動作する。

F-Number値はNote-On時の周波数（全Op共通）に対する比率として作用する：`周波数比 = F-Number / 4096`（4096 = 2^12が比率1.0=上書きなしに相当、13bit全域で約0〜2倍≒2オクターブ分の可変範囲）。

デフォルトはNote-Onで設定された値（全Op共通、比率1.0）。NRPN送信時点から、該当オペレーターの周波数のみを独立して上書きする（オクターブ＝他Opとの基準周波数は変化しない）。

### Operator Key On/Off（OP単位キーオン/オフ、CC103〜106）

CC103=Op0、CC104=Op1、CC105=Op2、CC106=Op3に、オペレーター単位のキーオン/オフを割り当てる。
CC66/67と同じ閾値判定（値≧64でキーオン、値<64でキーオフ）を採用し、NRPNの3メッセージ手順より応答性の高いCC単発メッセージで即時反映する。

38x6はチャンネル数無制限のため、1ノート=1チャンネルとして扱うことで、チャンネル単位のCCがそのままノート単位のOP制御になる。

- CC103〜106（Op0〜3）≧ 64 → 該当オペレーターのみキーオン
- CC103〜106（Op0〜3）< 64 → 該当オペレーターのみキーオフ（他のOPは影響を受けない・全OP対等）
- ノート全体の停止は通常のNote-Off（または CC120/CC123）で行う（Op3マスターのような特別扱いは持たない）

未定義領域（CC102〜119、GM2にコントローラー定義のないCC）を使用し、GM2標準コントローラーとの意味的な衝突を避ける。先頭のCC102はProgram Change代替に割り当てているため、OP単位キーオンはCC103〜106を用いる。

主な用途：シーケンサーから各CCを高速かつ周期的に送ることで、Op単位のエンベロープを繰り返しトリガーし、OPN系実機のCSMモード（タイマー駆動の自動キーオンによるフォルマント的効果）に近い効果をシミュレートする（演奏時のリアルタイム操作ではなく、シーケンサーによる自動化を想定）。

CSM的フォルマント合成は **Algorithm 7（全並列）が事実上の唯一の選択肢**である。各フォルマントは「変調を受けないクリーンなサインキャリア」である必要があり、4オペレーターすべてが独立した並列キャリアになるのはAlgorithm 7だけ（他のアルゴリズムは最低1つがモジュレーター役になり、再キーオンしてもフォルマントでなくFMサイドバンドが出る）。レシピは「Algorithm 7 ＋ OP単位F-Number（NRPN 0,18〜0,21で各フォルマント周波数を設定）＋ CC103〜106の高速送出」。なお全OPを対等に扱う（特定OPをマスターにしない）方式は、4本のフォルマント・キャリアを独立にキーイングできるという点でこの用途と整合する（「キーオン（OP単位キーオン/オフ）」節参照）。

---

## OPQから38x6へのコンバーター設計

PSR-70のサウンドROM（`Software/ROM2`、JKN0/PSR70-reverse）に格納されたOPQ音色データ（実使用は約32音色、テーブル上の定義は約80）を架空音源プリセット形式（`.38x6`）へ変換する。実装は`tools/psr2x6`。
変換先は`WAVEFORM_MEMORY_BANK + 1`以降のバンク（Programは0〜127、128件ごとに連番バンクへ分割）。
（OPQボイスは0x60以降がOPM互換のレジスタ配置。デチューンは6bit。正確なビット配置は`Guides/OPQ_ProgGuide.pdf`を出典とする。
なお「`def_seqs.h`」「450エントリ」はシーケンス／自動伴奏データを指し、音色データではない。）

スケーリング方針（線形・可逆）：
```
5bit（0〜31）  → 8bit（0〜255）: × 8
4bit（0〜15）  → 8bit（0〜255）: × 17（RR/SL等）
3bit（0〜7）   → 8bit（0〜255）: × 36
2bit（0〜3）   → 8bit（0〜255）: × 85
6bit（0〜63）  → 8bit（0〜255）: × 4（デチューン：中心32→128）
マルチプル（4bit、0〜15）→ MUL（0〜15）: そのまま（変換不要）
```

**トータルレベル（7bit, 0〜127）の変換：極性反転 + × 2**
```
38x6_TL = (127 - OPQ_TL) × 2
```
OPQのTLレジスタは「0=0dB（最大音量）、127=-95.25dB（最小音量）」という減衰量。
38x6のTLは「0=-95.25dB（最小音量）、254=0dB（最大音量）」という音量ノブ的な極性（オペレーターパラメーター参照）のため、単純な×2ではなく反転が必要。

逆変換：`OPQ_TL = 127 - (38x6_TL / 2)`（38x6_TLが奇数または255の場合は丸め誤差あり）

可逆変換が保証されるため、OPQ実機で再生できる形式に戻すことも可能。

**Velocity Sensitivity（38x6独自拡張）：**
OPQ/OPZにはベロシティ感度のレジスタが存在しないため、変換時は全オペレーターVelocity Sensitivity = 0とする。
これによりベロシティで音色（明るさ）が変化しなくなり、OPQ実機と同じ音色挙動を再現できる。
ただし音量はベロシティが常時担当するため、OPQ実機の「ベロシティに依らず一定音量」まで厳密に再現したい場合は、変換後のシーケンスを固定ベロシティで再生する。

---

## 実装参照元

| 資料 | 内容 | ライセンス |
|------|------|-----------|
| ymfm（Aaron Giles） | OPQ/OPZ/OPN実装 | BSD 3-Clause |
| OPQプログラマーズガイド V1.1（Jari Kangas） | レジスタ仕様・周波数テーブル | 変更なしなら自由配布可 |
| PSR-70 サウンドROM（ROM2、JKN0/PSR70-reverse） | OPQ音色テーブル（実使用約32／定義約80） | LICENSEファイルなし→要確認 |

**注記**：PSR-70音色データの利用にはJari Kangasへの許諾確認を推奨。
連絡先：https://github.com/JKN0/PSR70-reverse（Issues）
または https://hackaday.io/project/177168

---
