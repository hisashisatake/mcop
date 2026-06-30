# _archive — 休眠ツール置き場

ML逆算（教師あり回帰モデル）系のツール群。2026-06-25 に退避。

## 退避理由

A-by-S（録音→FMパラメーター逆算）アプローチが原理的に行き止まりと判明したため
（FMとPCMの音響空間が非重なり）。現在の音色設計は「人間が印象を記述子で指定 →
probe地図で最近傍検索 → 耳で手動チューニング」のワークフローに移行した。

現役ツールは `../features.py` / `../probe.py` / `../analyze.py` / `../lookup.py`。

## 中身

| ファイル | 役割 |
|---|---|
| `dataset.py` | 自己教師ありデータセット生成 |
| `generate.py` | ランダムパッチ→WAVレンダリング |
| `model.py` | InverseNet（mel→パラメーター回帰モデル）|
| `train.py` | モデル学習 |
| `eval.py` | モデル評価 |
| `infer.py` | 学習済みモデルで逆算推論 |

## 注意

- 実行には `torch` が必要だが、現在の venv には未導入。動かすには再インストールが必要。
- `abys.py` は ML warm-start で `from model import InverseNet` を遅延importするが、
  try/except + モデルファイル存在チェックで保護されているため、archive後も abys.py は
  警告を出すだけで正常動作する。
- 本格的に使わないことが確定したら、このディレクトリごと削除してよい。
