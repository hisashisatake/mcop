---
paths:
  - "sound-core/**"
  - "ym38x6/core/**"
---

# sound-core / ym38x6-core 依存ガード

このルールは `sound-core` / `ym38x6-core` を編集するときだけ読み込まれる。

- `sound-core` と `ym38x6-core` は **nice-plug・Tauri・cpal に依存してはならない**。純粋なRustライブラリ（標準＋必要最小限のクレートのみ）を保つ。
- 音源エンジンの変更はこの2クレートに閉じる。MIDI・ジェスチャー解釈・UI・音声出力はコアの外側で行う。
- これらに新機能を実装したら、同じタイミングで `ym38x6-vst` に配線し、VST単体でも機能が使える状態を保つ（MIDI CC/RPN/NRPNの受信処理やパラメーター追加を含む）。
- パラメーターは0〜255（8bit）統一。例外は周波数（オクターブ3bit + F-Number 13bit = 16bit、常にOP単位×4）とMUL（0〜15、Multiple 4bit準拠）。
- 波形フォーマットは1024×uint16_t対数で統一。線形↔対数の変換パイプラインは `sound-core` 側に置く。
