# audition

音色試聴スキル。単音 C2-C5 とストラムアルペジオの WAV を生成する。

## 使い方

```
/audition                          # piano_template.py の全8音色
/audition 4                        # piano_template.py の prog 4 のみ
/audition private/foo.op505        # バンクファイルの全プリセット
/audition private/foo.op505 --prog 4  # バンクファイルの prog 4 のみ
```

## 手順

引数を解析して以下を実行する。

1. `op505/tools/patchlab` ディレクトリで `uv run python` を使って
   `python\audition.py` を実行する。
   - 引数が数字のみ → `--prog <N>`
   - 引数が `.op505` ファイルパス → `--bank <path>`
   - 引数が `.op505` ファイルパス + `--prog N` → `--bank <path> --prog <N>`
   - 引数なし → 引数なし（全音色）

2. 実行コマンド例:
   ```powershell
   cd "c:\Users\satake\source\repos\ym38x6\op505\tools\patchlab"
   uv run python python\audition.py --prog 4
   uv run python python\audition.py --bank private/piano_template.op505
   ```

3. 出力先 `private/audition_wav/<label>/` に生成された WAV ファイルのパスを報告する。

## 出力ファイル

- `single_C2toC5.wav` : C2→C3→C4→C5 の単音（各2秒）
- `strum.wav` : C2→C5 のストラムアルペジオ（4分音符×3 + 全音符）
