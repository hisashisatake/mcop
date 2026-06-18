# 38x6 音源仕様

## チャンネルIDとキーオン契約（sound-core共通）

`SoundEngine`トレイトは`note_on(channel, wave_slot, frequency, adsr)`で発音し、`channel`は呼び出し側（VST/gesture-app等）が指定する安定したIDとして扱う。

- 同じ`channel`へ再度`note_on`すると、発音中・リリース中を問わずエンベロープを即座にカットしてAttackから再開する（実機のKey-On挙動に準拠した「同音チョーク」、insert-or-replace契約）
- 呼び出し側が「離す→再度押す」を同じ`channel`で行えば、直前のリリーステールが自動的にチョークされ次の発音にかぶらない
- VST（ym38x6-vst）はMIDIノート番号をそのまま`channel`として使う
- gesture-appはコードの声部インデックス（0〜N-1の固定スロット）を`channel`として使う（[spec-app.md](spec-app.md)参照）

## 波形メモリ専用音色バンク（38x6のOP1のみ有効）

フェーズ1ではプロトタイプとして独立クレートのWMS-1（波形オシレーター + ADSR）を用いていたが、
その実態は38x6の1オペレーター相当であり、波形フォーマット・チャンネル契約・パフォーマンスLFOは
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
- チャンネル側はデフォルト（フィルター全開・音色LFO無効）

WMS-1の単一指数ADSRとはOPM準拠カーブを通る分だけ触感がわずかに変わる（許容）。

### ビルトイン波形（38x6と共通、スロット0〜7）

波形メモリ音色のビルトイン波形は38x6ネイティブの8波形（[waveform.rs](ym38x6-core/src/waveform.rs)）を使う。
旧WMS-1の三角波は廃止した。

```
0: サイン波
1: ハーフサイン
2: 絶対値サイン（全波整流）
3: 矩形波
4: ノコギリ波
5: クワンタイズドサイン（暫定）
6: パルスサイン（暫定）
7: オクターブサイン（暫定）
```

### 専用バンクによる選択（Bank Select / Program Change）

波形メモリ音色は専用のBank Select番号 `WAVEFORM_MEMORY_BANK`（= 16383）で選ぶ。
このバンクでは Program Change（0〜127）から`waveform_memory_params_for_program`で
(波形, ADSR)を決定し、1オペレーター音色を生成する。

- Program 0〜7: ビルトイン波形（Program番号） + **ピアノ風ADSR**（attack=255 / decay=80 /
  sustain=120 / release=130。減衰して中程度のサスティンレベルへ落ち着く）
- Program 8〜15: ビルトイン波形（Program-8） + **リード風ADSR**（attack=255 / decay=0 /
  sustain=255 / release=130。減衰なしの無限サスティンで、RRはピアノ風ADSRと同じ値にして
  キーオフ後の余韻の長さを揃える）
- Program 16〜127: ユーザー波形スロット（Program番号） + デフォルトADSR
  （`AdsrParams::default()` = attack=200 / decay=150 / sustain=180 / release=100）

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
0〜7:   ビルトイン波形（固定）
8〜255: ユーザー定義波形スロット
```

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
| SL（サスティンレベル） | 4bit | 0〜255 | 0〜255をenv_level閾値に線形マッピング(d1l/255)。dBリニアエンベロープにより出力は-96dB(0)〜0dB(255)のdB線形（255=減衰なし）。※フィルターEGのSLは別途sl_to_level(-93dB(0)〜0dB(255))を使用 |
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

### 波形8種類（OPZ由来、番号0〜7固定）
```
0: サイン波
1: ハーフサイン
2: 絶対値サイン
3: 矩形波
4: ノコギリ波
5〜7: 変形波形
```

### ユーザー定義波形（番号8〜255）

波形番号8〜255はユーザー定義波形スロットとして使用可能。

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
スロット番号: 8〜255（248スロット）
1スロット:   2KB（1024 × uint16_t）
合計最大:    248 × 2KB = 496KB
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
| フィードバック | 3bit | 0〜255 | 指数カーブ（0は完全オフ、1〜255で7oct、255で最大1.8サイクル）。OPN実機理論値は±0.5サイクルだが、1サンプル帰還では自己変調が弱くFBリッチ音色のノイズを再現できないため、fmgen原音と聴き比べて1.8に調整 |
| AM感度 | 2bit | 0〜255 | 指数カーブ（0は完全オフ、1〜255でAMS=1〜3相当の23.9〜95.6dBをdepth=1-10^(-dB/20)で振幅深度に変換） |
| PM感度 | 3bit | 0〜255 | 指数カーブ（0は完全オフ、1〜255でPMS=1(+/-5cents)〜PMS=7(+/-700cents)相当、約7.13oct） |

### フィルター（State Variable Filter、ボイス単位）

FM合成出力にかけるアナログシンセ的なVCF相当。OPQ由来パラメーターとは独立した38x6独自拡張。

| パラメーター | 値域 | 備考 |
|------------|------|------|
| Cutoff | 0〜255 | カットオフ周波数。対数スケール（0≒20Hz、255≒20kHz） |
| Resonance | 0〜255 | レゾナンス。Self-Oscillation ON時は255でカットオフ周波数のサイン波が自己発振 |
| Self-Oscillation | 0 or 1（8bitで保持） | デフォルト=1（ON）。OFF時は255でも発振寸前で安定動作 |
| Filter Type | 0〜2（8bitで保持） | 0=LP、1=HP、2=BP |
| Filter EG A（Attack） | 0〜255 | キーオンからピークまでの時間 |
| Filter EG D（Decay） | 0〜255 | ピークからサスティンレベルまでの時間 |
| Filter EG S（Sustain） | 0〜255 | キーオン中に保持するレベル |
| Filter EG R（Release） | 0〜255 | キーオフから0までの時間 |
| Filter EG Depth | 0〜255 | Filter EGがCutoffに与える変調量 |

**Filter EGの加算モデル：**
```
実効Cutoff = clamp(Cutoffベース値 + Filter EG出力 × Filter EG Depth, 0, 255)
```
キーオンでA→D→Sの順に推移し、キーオフでRに移行する（オペレーターのエンベロープと同様の挙動、MC-404等のフィルターエンベロープ相当）。

**実装方式：** State Variable Filter（SVF）
- LP/HP/BPを同一回路から同時出力できる構造で、Filter Typeによる切り替えと相性が良い
- 高Resonanceでも数値的に安定（Self-Oscillation時の発振も含めて安定動作）

Self-Oscillation ON + Filter EGでCutoffをスイープすると、発振に突入する効果音的な表現が可能。

**OPQコンバーターとの関係：**
フィルターはOPQ由来パラメーターではないため、OPQ変換対象外。38x6独自フォーマット（.38x6）にのみ保存される。

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
- `sound-core`に依存ゼロのDSPモジュールとして実装し、各`SoundEngine::render()`の出力に対してapp/plugin側のレンダリング後段で適用する

**OPQコンバーターとの関係：**
エフェクトはOPQ由来パラメーターではないため、OPQ変換対象外。38x6独自フォーマット（.38x6）にのみ保存される。

---

## キーオン（OPQ由来）

- オペレーターごとに独立してキーオン/オフ可能
- **Op3がマスター**：Op3がOffになると全Op強制Off
- Op3がOnの間、Op0/1/2は個別に制御可能
- 作曲支援アプリでのアボイドノート音量制御に応用可能
- CC102〜105でOP単位のキーオン/オフを制御可能（詳細はMIDI実装方針のOperator Key On/Offセクション参照）

---

## 音色LFO

プリセット・NRPNで設定する「音作り」用のLFO。MIDI CC（後述のパフォーマンスLFO）からは独立しており、演奏時のビブラート/トレモロには影響しない。

| 項目 | 値 | 備考 |
|------|-----|------|
| 波形 | 三角波 | OPQ由来・固定 |
| 周波数 | 3bit → 8bit（0〜255） | |
| PMD（ピッチ変調深さ） | 0〜255 | |
| AMD（振幅変調深さ） | 0〜255 | |
| PM感度（PMS） | チャンネルごと 3bit → 8bit | 指数カーブ（0=オフ、1〜255でOPM PMS=1(+/-5cents)〜PMS=7(+/-700cents)を両端アンカーとした指数補間） |
| AM感度（AMS） | チャンネルごと 2bit → 8bit | 指数カーブ（0=オフ、1〜255でOPM AMS=1(23.9dB)〜AMS=3(95.6dB)を両端アンカーとした指数補間をdepth=1-10^(-dB/20)で振幅深度に変換） |
| Delay | 0〜255 | キーオンからLFO効果開始までの遅延時間。38x6独自拡張 |
| AMオン/オフ | オペレーターごと | |

周波数/PMD/AMD/Delay/PMS/AMSの6項目は、チャンネル単位のDAWパラメーターとして公開する（MIDI実装方針セクション参照）。

---

## パフォーマンスLFO（ビブラート/トレモロ）

GM2のCC1/76/77/78に対応する、演奏時のビブラート/トレモロ専用LFO。
音色LFO（PMD/AMD/PMS/AMS）とは完全に独立しており、音色設計には影響しない。

| 項目 | 制御 | 備考 |
|------|-----|------|
| Rate | CC76（Vibrato Rate） | 0〜255 → 0.01Hz〜20Hz（指数マッピング） |
| Depth | CC77（ベース値）+ CC1（加算分） | Destinationにより単位・モデルが異なる（下記） |
| Delay | CC78（Vibrato Delay） | キーオンから効果開始までの遅延。0〜255 → 0〜10秒（線形マッピング） |
| 波形 | NRPN「Performance LFO Waveform」で選択（下記） | デフォルト = 三角波 |
| Destination | NRPN「Performance LFO Destination」で選択（下記） | デフォルト = Pitch（ビブラート） |

**Waveform enum：**

| 値 | 波形 |
|---|---|
| 0（デフォルト） | 三角波 |
| 1 | サイン波 |
| 2 | 矩形波 |
| 3 | S&H（ランダム） |

**Destination enum：**

| 値 | 宛先 | Depthのモデル | 対応エンジン |
|---|---|---|---|
| 0（デフォルト） | Pitch（ビブラート） | `実効Depth(セント) = CC77値 + CC1値 × RPN0,5値 / 127` をピッチに加算 | F-Number全Op |
| 1 | Volume（トレモロ） | `実効Depth = clamp(CC77値 + CC1値, 0, 255)` を各ノートの実効音量（ベロシティ音量適用後）に加算 | TL全オペレーター一括 |
| 2 | TL（キャリア一括、トレモロ） | 同上（キャリアのみ） | 38x6拡張のみ |

トレモロ（Destination=1/2）は各ノートの実効音量に対して相対的に作用するため、ベロシティによる音量差は維持されたまま揺れる。
RPN 0,5（Modulation Depth Range）はDestination=Pitchの場合のみ意味を持つ（詳細はRPNセクション参照）。

**実装方式：**
`PerformanceLfo`（Rate/Depth/Delay/Waveform）はエンジン非依存の共通コンポーネントとして`sound-core`に実装する。
適用先は`PerformanceLfoTarget`トレイトとして定義し、共通Destination（0=Pitch、1=Volume）に加え、38x6は拡張Destination（2=TLキャリア一括）も実装する。

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
| CSMモード（タイマー駆動の自動キーオン） | 需要なし。OP単位F-Number＋CC102〜105によるOP単位キーオン/オフ（MIDI実装方針参照）をシーケンサーから高速送信することでCSM風の効果を代替可能 |

---

## MIDI実装方針

### DAWオートメーション（nice-plugパラメーター）

以下の全パラメーターをnice-plugのParam として公開し、Cubase・Logic等でのDAWオートメーションに対応する。

**プリセット選択（1個）：**
Program（0=Manual：DAWパラメーター/NRPNで手動チューニングしたパッチを使う。1〜128=Program 0〜127：CC0/CC32で選択中のbankの該当プリセットへ切り替える。VST3でMIDI Program Changeの代替として使う、Bank Select / Program Changeセクション参照）

**チャンネル単位（20個）：**
Algorithm / Feedback / パフォーマンスLFO Rate / パフォーマンスLFO Depth（ベース値）/ パフォーマンスLFO Delay / 音色LFO Freq / 音色LFO PMD / 音色LFO AMD / 音色LFO Delay / PMS / AMS / Filter Cutoff / Filter Resonance / Filter EG A / Filter EG D / Filter EG S / Filter EG R / Filter EG Depth / Reverb Send / Chorus Send

**オペレーター単位（12個 × 4op = 48個）：**
TL / AR / D1R / D2R / D1L / RR / MUL / DT1 / KS / AME / Velocity Sensitivity / OP Fine Tune

**マスター単位（5個）：**
Reverb Time / Chorus Mod Rate / Chorus Mod Depth / Chorus Feedback / Chorus Send To Reverb

**離散パラメーター（NRPN専用、DAWオートメーション対象外）：**
以下はnice-plugパラメーターを持たず、NRPN/GUI操作でのみ設定する
（CC91/93やマスターエフェクトの「マスター単位」パラメーターのように、
NRPN/CCとnice-plugパラメーターを併用する項目とは異なる点に注意）。

Algorithm / Waveform（WF）per op / Filter Type（LP/HP/BP）/ Filter Self-Oscillation / AT Destination / Poly AT Destination / Performance LFO Destination / Performance LFO Waveform / Reverb Type / Chorus Type

※「Algorithm」は例外的に、NRPN(0,9)に加えて上記チャンネル単位のnice-plugパラメーターとしても公開する。

### MIDI CC（GM2準拠）

| CC | GM2定義 | 38x6割り当て | GM2との関係 |
|---|---|---|---|
| CC 1 | Modulation Wheel | パフォーマンスLFO Depth加算分 | 準拠（パフォーマンスLFO参照） |
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
| CC 76 | Vibrato Rate | パフォーマンスLFO Rate | 完全準拠 |
| CC 77 | Vibrato Depth | パフォーマンスLFO Depthベース値 | 完全準拠 |
| CC 78 | Vibrato Delay | パフォーマンスLFO Delay | 完全準拠 |
| CC 91 | Reverb Send Level | Reverb Send | 完全準拠（マスターエフェクト参照） |
| CC 93 | Chorus Send Level | Chorus Send | 完全準拠（マスターエフェクト参照） |
| CC 120 | All Sound Off | All Sound Off | 完全準拠 |
| CC 121 | Reset All Controllers | Reset All Controllers | 完全準拠 |
| CC 123 | All Notes Off | All Notes Off | 完全準拠 |
| CC 126 | Mono Mode On | Mono Mode | 完全準拠 |
| CC 127 | Poly Mode On | Poly Mode | 完全準拠 |

**Portamento（CC5/CC65）：**

CC65 ON時、新しいノート（新チャンネル）のF-Numberは、同一MIDIチャンネルで直前に発音したノートのF-Numberから、CC5（Portamento Time、0=即座〜127=数秒程度）で指定した時間をかけて目標値へ線形にグライドする。
直前のノートは別チャンネルで独立してリリース/サステインペダル等の影響を受けながら鳴り続けるため、グライドとの相互作用は発生しない。
作曲支援アプリのジェスチャーUIの「ゆっくり移動 → ポルタメント」（ジェスチャーレパートリー参照）も、この仕組み（Note-On + CC65 ON + CC5）で実現する。

**Sostenuto（CC66）：**

CC66 ON時点で発音中（Note-On済みかつNote-Off未到達）の全チャンネルに「サステイン保持」フラグを立てる。該当チャンネルはNote-OffされてもCC66 OFFまでReleaseに入らない（CC64と同じ仕組みを対象チャンネルのみに適用）。CC66 ON以降に新規キーオンしたノートは対象外。

**Soft Pedal（CC67）：**

CC67 ON中に新規キーオンしたノートに対してのみ、実効TLとFilter Cutoffを減算する。
```
実効TL = clamp(TLベース値 - CC67値, 0, 255)
実効Cutoff = clamp(Cutoffベース値 - CC67値, 0, 255)
```

**サステインペダル（CC64）の実装方針（未実装・将来対応）：**

「ペダル対応」は2つの独立した関心事に分かれる。(a)はペダル本体、(b)はオプション拡張。

**(a) ホールドフラグ方式（第一候補、まずこれを実装する）：**

普通のサステイン（離鍵してもReleaseに入らない）は、チャンネルを新規確保せず、現状の1ノート=1チャンネル（[キーオン契約](#チャンネルidとキーオン契約sound-core共通)）のまま実現できる。VST側に `pedal_down: bool` と `pending_release: HashSet<note>`（離鍵済みだがペダル保持中のノート）を持つ。

```
Note-On(note):   engine.note_on(note as id, ...)         // 現状どおり（チョーク＋デクリック）
                 pending_release.remove(note)            // 弾き直したら保持解除
Note-Off(note):  if pedal_down { pending_release.insert(note) }  // 離鍵してもReleaseしない
                 else         { engine.note_off(note) }
CC64 ON:         pedal_down = true
CC64 OFF:        pedal_down = false
                 pending_release中の全ノートをengine.note_off()してdrain
```

鍵がまだ押されている音は `pending_release` に入らない（ペダルを離しても鳴り続ける）。同じ鍵を弾き直すと保持中の音は自然にチョークされる（ピアノは1鍵=1組の弦なので物理的に正しい）。エンジンは無改造で済む（必要なら `set_sustain(channel, bool)` のようなAPIを足す案もあるが、まずはVST完結）。Sostenuto（CC66）は対象を限定した同じ仕組み。

**(b) ペダルON時の同音重ね発音（スウェル、オプション拡張）：**

パッド系などで「ペダルを踏んで同じ音を重ねて独立に減衰させる」表現が欲しい場合のみ必要。ピアノでは物理的に起きないので(a)とは別物。チャンネルIDをノート番号から切り離し、ペダルON時のみ新規ユニークIDを割り当てて重ね発音を許す。38x6はチャンネル無制限のため「空きチャンネル検索」は不要で、IDを払い出すだけ（カウンター採番や `note番号 + 世代` 等）でよい。choke/overlapの判定はVST側のID採番ポリシーのみで決まり、エンジンのinsert-or-replace契約は変更不要：

- ペダルUP → `id = ノート番号`（前をチョーク、現状の挙動）
- ペダルDOWN → `id = 新規ユニークID`（重なる）

代償はVST側に `note番号 → 複数channel ID` の多重マップが要ること（Note-Off／ペダル解放時に対象IDをまとめてReleaseする）。現スコープ（ペダル無しのgesture-app・ピアノ系音色）では不要のため未実装。

### Pitch Bend

0xEn（14bit、中央値8192 = ベンドなし）でF-Numberを直接変化させる。
ベンドレンジはRPN 0,0（Pitch Bend Sensitivity）で設定する。

### RPN（GM2準拠）

| RPN (MSB,LSB) | 内容 | デフォルト | 備考 |
|---|---|---|---|
| 0, 0 | Pitch Bend Sensitivity | ±2半音 | Pitch BendのF-Number換算レンジ（半音 + セント） |
| 0, 1 | Channel Fine Tuning | 0セント | F-Numberオフセット（±100セント） |
| 0, 2 | Channel Coarse Tuning | 0半音 | F-Numberオフセット（±64半音） |
| 0, 5 | Modulation Depth Range | 64（約50セント相当） | パフォーマンスLFO Destination=Pitchの場合のCC1セント換算係数（パフォーマンスLFOセクション参照） |
| 127, 127 (7F,7F) | RPN/NRPN Null | - | 選択解除（誤操作防止のため必須） |

### Bank Select / Program Change

CC 0（MSB）+ CC 32（LSB）によるBank SelectとProgram Changeを実装する。
GM2のプログラム番号定義（0〜127の楽器カテゴリ）に準拠したバンク構成を採用する。

**バンク構成：**

| バンク | 内容 |
|---|---|
| Bank 0 | GM2プログラムマップ準拠（0〜127）。ym38x6-mlによるインバース合成でML自動生成 |
| Bank 1以降 | ユーザー定義プリセット |
| WAVEFORM_MEMORY_BANK+1以降 | OPQ/PSR-70の変換音色（tools/psr2x6で生成、ym38x6-mlのシード/教師） |

**音色作成方針（フェーズ5で実施）：**
- ym38x6-mlで目標音（GM2リファレンス音源等）からFMパラメーターを逆算し、Bank 0の128音色を自動生成
- FMが苦手なカテゴリ（アコースティックピアノ・弦楽器・合唱等）は最近似音色で代替
- 実際のGM2→音色マッピング表はフェーズ5で別途作成
- Bank 0の自動生成にはOPQ/PSR-70実機プリセットを直接流用しない（ML逆算で生成する）。実機音色の変換はtools/psr2x6で別バンク（WAVEFORM_MEMORY_BANK+1以降）へ行う

**実装状況（フェーズ5着手時点）：**
- CC0+CC32によるバンク選択とProgram Changeの受信を実装済み。
- Bank0のうちProgram 0（Acoustic Grand Piano）/4（Electric Piano 1）/80（Lead 1 Square）は、
  動作確認用に手動チューニングしたパッチ（`gm2_bank0_patch`）を使用する。設計概要：
  - Program 0: Algorithm 4（(O1→O2)+(O3→O4)）で2つの倍音グループを構成。各グループの
    モジュレーター(O1/O3)をキャリアより速く減衰させ、打鍵直後だけ倍音が立つハンマー
    アタックを表現。片方のグループをわずかにデチューンしコーラス感を付加
  - Program 4: Algorithm 4で、O1→O2をベル成分（MUL=14 + 強いフィードバックでメタリックな
    質感）、O3→O4をメインのトーン成分とするDX7系E.PIANOの定番構成
  - Program 80: Algorithm 7（全並列）で矩形波(waveform=3)を3本デチューンして重ね、O4を
    1オクターブ上の矩形波で薄く重ねるデチューンユニゾン構成。d1l=255+d2r=0でキーオン中は
    無限サスティンとする
- 上記以外のbank・program番号は、bank・program番号から決定的に生成する暫定パッチ
  （`placeholder_patch`）を使用する（ym38x6-ml生成・プリセットライブラリができるまでの
  暫定構成。優先順位はユーザープリセット(.38x6) > Bank0手動チューニング > `placeholder_patch`）。
- nice-plugの制約により、MIDI Program ChangeイベントはCLAPでのみ受信可能（VST3では
  受信不可、`MidiConfig::MidiCCs`の仕様）。CC0/CC32（Bank Select）はVST3でも受信可能。
- VST3でもプリセットを切り替えられるよう、nice-plugパラメーター「Program」
  （0=Manual/1〜128=Program 0〜127）を公開する。CC0/CC32で選択中のbankと組み合わせて
  Program Changeと同じロジック（`patch_for_program`）でパッチを解決する。
- Program Change受信時、または「Program」パラメーター変更時（0=Manual以外）は、
  選択したパッチがDAWパラメーターに優先して使われる（新規ノートおよび発音中
  チャンネルの両方）。DAWパラメーターでの編集に戻すには「Program」を0=Manualに戻す
  （MIDI Program Change経由で切り替えた場合はプラグインのリロードが必要、暫定。
  本格的なプリセット編集UIはフェーズ5「パラメーターUI」で別途検討）。

**ユーザープリセット（Bank1以降）の読み込み（フェーズ5その2）：**

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
            "filter_eg_attack": 0, "filter_eg_decay": 0, "filter_eg_sustain": 0, "filter_eg_release": 0, "filter_eg_depth": 0
          }
        }
      },
      { "program": 1, "name": "音色名2", "patch": { "...": "..." } }
    ]
  }
  ```
  `operators`は配列要素0〜3がOp0〜3に対応する（4要素固定）。各フィールドの意味・数値範囲は
  「オペレーターパラメーター」「チャンネルパラメーター」「フィルター」「音色LFO」の各節を参照。
  現在は`Ym38x6Patch`のフィールドのみが対象で、拡張波形スロット(8〜255、未実装)のデータ埋め込みは
  当該機能実装後に別途対応する。`program`は`.opm`（VOPM）の`@:`番号(0-127)を継承可能な識別子。

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

プリセットの保存（書き出し）UI・操作は未実装（フェーズ5「パラメーターUI」で対応予定）。
`PresetFile::to_json`は今後gesture-app等から呼び出して書き出す想定。

### NRPN

DAWオートメーション非対応の離散パラメーターおよびハードコントローラー向けの詳細制御に使用。

CC99（NRPN MSB）/CC98（NRPN LSB）でパラメーター番号を選択し、CC6（Data Entry MSB）で値を設定する（GM2準拠の標準的なNRPN手順）。
CC99/98またはCC101/100（RPN）に127,127（Null）を送ると選択解除される。

| 対象 | 備考 |
|---|---|
| Algorithm（CON） | 8種類、信号ルーティングが変わるため離散制御 |
| Waveform（WF）per op | 0〜7（ビルトイン）+ 8〜255（ユーザー定義） |
| Filter Type | 0=LP / 1=HP / 2=BP |
| AT Destination | Channel Pressureの加算先（destination enum、下記） |
| Poly AT Destination | Poly Key Pressureの加算先（destination enum、下記） |
| Performance LFO Destination | パフォーマンスLFOの加算先（destination enum、パフォーマンスLFOセクション参照） |
| Performance LFO Waveform | パフォーマンスLFOの波形（waveform enum、パフォーマンスLFOセクション参照） |
| Reverb Type | Reverbのタイプ（type enum、マスターエフェクトセクション参照） |
| Chorus Type | Chorusのタイプ（type enum、マスターエフェクトセクション参照） |
| Operator F-Number (Op0〜3) | OP単位F-Numberの上書き（13bit × 4、下記参照） |

**NRPN番号（MSB,LSB）：**

NRPN番号は本実装（パフォーマンスLFO）で初めて定義する。MSB=0を「離散パラメーター」用に予約し、LSBを実装順に割り当てる。他の離散パラメーターのNRPN番号は実装時に追記する。

| 対象 | NRPN (MSB,LSB) | 値 |
|---|---|---|
| Performance LFO Destination | 0, 0 | 0=Pitch（ビブラート） / 1=Volume（トレモロ） / 2=TL（キャリア一括、トレモロ、38x6拡張のみ） |
| Performance LFO Waveform | 0, 1 | 0=三角波 / 1=サイン波 / 2=矩形波 / 3=S&H |
| Reverb Type | 0, 2 | 0〜7（マスターエフェクトセクションのenum参照） |
| Chorus Type | 0, 3 | 0〜7（マスターエフェクトセクションのenum参照） |
| Reverb Time | 0, 4 | 0〜255 |
| Chorus Mod Rate | 0, 5 | 0〜255 |
| Chorus Mod Depth | 0, 6 | 0〜255 |
| Chorus Feedback | 0, 7 | 0〜255 |
| Chorus Send To Reverb | 0, 8 | 0〜255 |
| Algorithm | 0, 9 | 0〜7 |
| Waveform Op0〜3 | 0, 10〜13 | 0〜255（0〜7=ビルトイン、8〜255=ユーザー波形スロット） |
| Filter Type | 0, 14 | 0=LP / 1=HP / 2=BP |
| Filter Self-Oscillation | 0, 15 | 0=OFF / 1=ON |
| AT Destination | 0, 16 | 0〜5（destination enum、下記参照） |
| Poly AT Destination | 0, 17 | 0〜5（destination enum、下記参照） |
| Operator F-Number Op0 | 0, 18 | 0〜8191（13bit、CC6=MSB+CC38=LSBで送信、下記参照） |
| Operator F-Number Op1 | 0, 19 | 同上 |
| Operator F-Number Op2 | 0, 20 | 同上 |
| Operator F-Number Op3 | 0, 21 | 同上 |

**AT Destination / Poly AT Destination（アフタータッチの加算先）：**

Channel PressureとPoly Key Pressureは、それぞれ独立に「揺らぎ系」パラメーターへ加算するモデルで実装する。
加算先（Destination）はNRPN（AT Destination / Poly AT Destination）で選択可能。デフォルトはLFO PMD。

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
実効値 = clamp(ベース値 + プレッシャー値, 0, 255)
```

Channel PressureとPoly Key Pressureが同じdestinationを指す場合、両方の値が加算される。
Poly Key Pressure対応コントローラーは少数（MPE等）のため、多くの環境ではChannel Pressureのみが機能する。

**Operator F-Number（OP単位F-Number上書き）：**

NRPN(0,18)〜(0,21)がOp0〜Op3に対応する。CC6（Data Entry MSB）+ CC38（Data Entry LSB）で14bit値（0〜16383）として送信し、13bit（0〜8191）にclampして使用する（NRPNのデータエントリ精度14bitに対し1bit余裕がある）。CC38を送らないコントローラーではCC6のみ（128単位の粗い精度）でも動作する。

F-Number値はNote-On時の周波数（全Op共通）に対する比率として作用する：`周波数比 = F-Number / 4096`（4096 = 2^12が比率1.0=上書きなしに相当、13bit全域で約0〜2倍≒2オクターブ分の可変範囲）。

デフォルトはNote-Onで設定された値（全Op共通、比率1.0）。NRPN送信時点から、該当オペレーターの周波数のみを独立して上書きする（オクターブ＝他Opとの基準周波数は変化しない）。

### Operator Key On/Off（OP単位キーオン/オフ、CC102〜105）

CC102=Op0、CC103=Op1、CC104=Op2、CC105=Op3に、オペレーター単位のキーオン/オフを割り当てる。
CC66/67と同じ閾値判定（値≧64でキーオン、値<64でキーオフ）を採用し、NRPNの3メッセージ手順より応答性の高いCC単発メッセージで即時反映する。

38x6はチャンネル数無制限のため、1ノート=1チャンネルとして扱うことで、チャンネル単位のCCがそのままノート単位のOP制御になる。

- CC105（Op3）< 64 → Op3がマスターのため全OP強制キーオフ（そのノートのNote-Off相当）
- CC102〜104（Op0〜2）< 64 → 該当オペレーターのみキーオフ（Op3は鳴り続ける）

未定義領域（CC102〜119、GM2にコントローラー定義のないCC）を使用し、GM2標準コントローラーとの意味的な衝突を避ける。

主な用途：シーケンサーから各CCを高速かつ周期的に送ることで、Op単位のエンベロープを繰り返しトリガーし、OPN系実機のCSMモード（タイマー駆動の自動キーオンによるフォルマント的効果）に近い効果をシミュレートする（演奏時のリアルタイム操作ではなく、シーケンサーによる自動化を想定）。

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
