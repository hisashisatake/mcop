# opm2x6

VOPM形式の`.opm`音色ファイル（YM2151/OPM）を、38x6の`.38x6`プリセットJSON
（`ym38x6-core::PresetFile`形式、`{"bank":.., "presets"/"programs": [...]}`）に
変換するツール。

実機OPMの音/エミュレーターの音と38x6の音を聴感で比較するための、
パラメーターの「当てはめ」変換を行う。38x6本体（Cargoワークスペース）には
依存しない独立ツールで、Dockerコンテナ上のPythonで動作する。

## 使い方

```powershell
# イメージのビルド（初回のみ）
docker build -t opm2x6 tools/opm2x6

# 変換（inputディレクトリの voice.opm を読み込み、同ディレクトリに*.38x6を出力）
docker run --rm -v ${PWD}:/work opm2x6 /work/voice.opm

# 出力先を指定する場合
docker run --rm -v ${PWD}:/work opm2x6 /work/voice.opm /work/out

# bank番号を指定する場合（デフォルト1）
docker run --rm -v ${PWD}:/work opm2x6 /work/voice.opm /work/out --bank 2

# 音色ごとに個別ファイルへ分割する場合
docker run --rm -v ${PWD}:/work opm2x6 /work/voice.opm /work/out --split
```

デフォルトでは、`.opm`ファイル内の全`@:`音色定義を1つの`.38x6`ファイル
（入力ファイル名と同じstem、例: `voice.opm` → `voice.38x6`）にまとめて出力する。

```json
{
  "bank": 1,
  "presets": [
    { "program": 0, "name": "音色名1", "patch": { "operators": [...], "channel": {...} } },
    { "program": 1, "name": "音色名2", "patch": { "...": "..." } }
  ]
}
```

`program`は`.opm`の`@:`番号(0-127)を継承する。`bank`は`--bank`で指定する
（デフォルト1。Bank0はGM2準拠のため通常は使わない）。

`presets`形式は読み込み時にそのbankのプリセットを丸ごと初期化・再構築する
（[spec-sound.md](../../spec-sound.md)参照）。`--split`を指定すると、音色ごとに
`<番号>_<音色名>.38x6`という個別ファイルへ分割される。各ファイルは
`{"bank":.., "programs": [{"program":.., "name":.., "patch":{...}}]}`形式
（`programs`形式）になる。複数の個別ファイルをpresetsディレクトリに置いても、
`programs`形式は読み込み時に他ファイルのプリセットを初期化しないため、
互いのbankを上書きし合わない。

## オペレーター順序の注意

`.opm`ファイルの`M1:`/`C1:`/`M2:`/`C2:`の並びを、38x6の`operators[0..3]`
（アルゴリズム結線表`ALGORITHMS`が参照するOp0-3）にどう対応させるかは
未検証。デフォルト(`--operator-order direct`)はファイル記載順そのまま
（M1→Op0, C1→Op1, M2→Op2, C2→Op3）。

アルゴリズム0系（直列接続）の音が構造的に違う（モジュレーターとキャリアが
入れ替わっている）と感じたら、`--operator-order register`
（M1→Op0, M2→Op1, C1→Op2, C2→Op3、YM2151のレジスタ順）を試すこと。

```powershell
docker run --rm -v ${PWD}:/work opm2x6 /work/voice.opm /work/out --operator-order register
```

## 変換式

各式は`ym38x6-core/src/mapping.rs`・`tone_lfo.rs`で定義された
「実機OPM理論値アンカー+指数カーブ」のパラメーター空間に、`.opm`の
レジスタ値（離散値）を当てはめたもの。レジスタの最小値/最大値はym38x6側
の0/255に厳密に一致させ、中間のレジスタ段はym38x6の指数カーブ上で
等間隔になるように配置している。

| .opmフィールド | 範囲 | 38x6パラメーター | 変換 |
| --- | --- | --- | --- |
| `@:`番号 | 0-127 | program | 直接コピー（PresetEntryの`program`） |
| (CLIの`--bank`) | 0-65535 | bank | `.opm`に相当フィールドは無く、`--bank`オプション（デフォルト1）で指定する |
| AR / D1R / D2R | 0-31 (5bit) | ar / d1r / d2r | `eg_rate=reg*2`として`reg=0`→0(フリーズ)、`reg=1..31`→1..255に等間隔配置 |
| RR | 0-15 (4bit) | rr | `eg_rate=reg*4+2`として`reg=0..15`→0..255に等間隔配置（実機RRはフリーズしないため0も有限値） |
| D1L | 0-15 (4bit) | d1l (SL) | `reg=0..14`は-3dB/step、`reg=15`は-93dBへジャンプ（実機の不連続をそのまま反映） |
| TL | 0-127 (7bit, 0.75dB/step) | tl | `(127-reg)*255/127`（0dB↔255、-95.25dB↔0に厳密一致） |
| KS | 0-3 (2bit) | ksr | `reg*255/3`（レジスタ4値を等間隔配置） |
| MUL | 0-15 (4bit) | mul | 直接コピー（OPM/OPN/OPQ/OPZ共通のMultipleテーブルと同一） |
| DT1 | 0-7 (3bit大きさ+符号) | dt1 | 簡易近似: `128 ± (大きさ/3)*127`（大きさ=`reg&3`、符号=`reg&4`） |
| DT2 | 0-3 | - | 対応なし（無視） |
| FL (feedback) | 0-7 (3bit) | feedback | `reg*255/7`（feedback_to_scaleの「約36刻みごとに2倍」と1段=1FBステップが一致） |
| CON (algorithm) | 0-7 | algorithm | 直接コピー（OPMのCONとym38x6のALGORITHMSは同一トポロジー） |
| AMS | 0-3 (2bit) | ams | `reg=0`→0(無効)、`reg=1..3`→`1 + 127*(reg-1)`（AMS=1↔23.9dB、AMS=3↔95.6dBに厳密一致） |
| PMS | 0-7 (3bit) | pms | `reg=0`→0(無効)、`reg=1..7`→`1 + 254*(reg-1)/6`（PMS=1↔5cent、PMS=7↔700centに厳密一致） |
| AMS-EN (各OP) | 0/1 | am_enable | 直接コピー |
| LFRQ | 0-255 (8bit) | tone_lfo_freq | 直接コピー（簡易近似、Hzの厳密対応はしない） |
| AMD / PMD | 0-127 (7bit) | tone_lfo_amd / tone_lfo_pmd | `reg*255/127`（PMD=127↔tone_lfo_pmd=255=PMSで決まる変調幅いっぱい） |
| LFO WF | 0-3 | - | 対応なし（38x6の音色LFOは三角波固定。WF!=2の場合は警告を出力） |
| NFRQ / NE / PAN / SLOT | - | - | 対応なし（無視。SLOTで一部OPが無効化されている音色は警告を出力） |

velocity_sensitivity・waveform（OP波形）は、OPMに相当パラメーターが無いため
全オペレーターで0（明るさのベロシティ感度なし・サイン波）固定。
※velocity_sensitivity=0でも音量はベロシティに従う（音量は常時ベロシティ担当、明るさのみがopt-in）。filter_*はデフォルト値
（Cutoff全開・Self-Oscillation有効・Filter EG無効）固定。
