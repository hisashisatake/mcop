# fm-compare

FMパラメーター比較スキル。1つのパラメーターを複数値に振って比較WAVを一括生成する。

## 使い方

```
/fm-compare <prog> <param> <値1,値2,値3...>
```

例:
```
/fm-compare 4 brightness 138,155,168
/fm-compare 7 feedback 40,72,108
/fm-compare 4 mod_wf 0,1,8,16,19
/fm-compare 0 car_ksr 0,85,100,170
```

## 手順

1. 引数を解析:
   - 第1引数: プログラム番号（整数）
   - 第2引数: パラメーター名（piano_template.py の PIANO_FAMILY キー名）
   - 第3引数: カンマ区切りの値リスト

2. `tools/patchlab` ディレクトリで以下を実行:
   ```powershell
   cd "c:\Users\satake\source\repos\ym38x6\tools\patchlab"
   uv run python python\fm_compare.py --prog <prog> --param <param> --values <values>
   ```

3. 出力先 `private/audition_wav/compare_<prog>_<param>/` のファイル一覧を報告する。

## 注意

- `param` は `make_piano_patch()` の引数名と一致する必要がある
- 値が整数でない場合も対応（例: 0.5 など）
