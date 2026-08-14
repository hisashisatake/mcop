# phrase

定型フレーズ試聴スキル。ストラム / ファンク / バロック / タンゴの4種を任意音色で生成する。

## 使い方

```
/phrase <prog>                          # 全3フレーズ
/phrase <prog> <type>                   # 指定タイプのみ
/phrase <bank_path> <prog>              # バンクファイルから
/phrase <bank_path> <prog> <type>
```

フレーズタイプ:
- `strum`   : C2→C5 ストラムアルペジオ（120 BPM、4分音符×3 + 全音符）
- `funk`    : Eマイナーペンタ トライアドコード スタッカート（112 BPM）
- `baroque` : Aマイナー バロック風アルペジオプレリュード（72 BPM）
- `tango`   : Aマイナー ハバネラリズム（130 BPM）

## 手順

1. 引数を解析して `--prog` / `--bank` / `--type` を決定する

2. `op505/tools/patchlab` ディレクトリで以下を実行:
   ```powershell
   cd "c:\Users\satake\source\repos\ym38x6\op505\tools\patchlab"
   # prog 番号指定
   uv run python python\phrase.py --prog <prog> [--type <type>]
   # バンクファイル指定
   uv run python python\phrase.py --bank <path> --prog <prog> [--type <type>]
   ```

3. 出力先 `private/audition_wav/<label>/phrase_<type>.wav` のパスを報告する。
