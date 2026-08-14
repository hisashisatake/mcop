---
paths:
  - "sound/**"
  - "ym38x6/core/**"
  - "op505/core/**"
  - "op505/midi/**"
  - "op505/tools/**"
---

# 音源コアクレート 依存ガード

このルールは `sound-core` / `sound-fm`（`sound/`配下）と `ym38x6-core` / `op505-core`、
`op505-midi`（`op505/midi/`）、および `op505/tools/*` を編集するときだけ読み込まれる。

- これらのクレートは **nice-plug・Tauri・cpal に依存してはならない**。純粋なRustライブラリ（標準＋必要最小限のクレートのみ）を保つ。
- 音源エンジンの変更はこれらのクレートに閉じる。MIDI・ジェスチャー解釈・UI・音声出力はコアの外側で行う。
- これらに新機能を実装したら、同じタイミングで `op505-vst`（2026-08-12以降の主力。`ym38x6-vst`は凍結のため対象外）に配線し、VST単体でも機能が使える状態を保つ（MIDI CC/RPN/NRPNの受信処理やパラメーター追加を含む）。
- パラメーターは0〜255（8bit）統一。例外は周波数（オクターブ3bit + F-Number 13bit = 16bit、常にOP単位×4）とMUL（0〜15、Multiple 4bit準拠）。
- 波形フォーマットは1024×uint16_t対数で統一。線形↔対数の変換パイプラインは `sound-core` 側に置く。
- **`op505-core`と`op505/tools/*`はym38x6グループ（`ym38x6-core`・`ym38x6/tools/*`）に依存してはならない**（2026-08-11 op505デフォークで達成した状態を維持する）。唯一の例外は`op505-core`の`[dev-dependencies]`（`examples/op505_probe.rs`が既存`.38x6`との聴き比べに使う`legacy_convert_patch`のみ）。`src/`への`use ym38x6_core`混入は`[dev-dependencies]`降格によりコンパイルエラーで防げる設計になっている。共有したくなった処理は中立クレート化せず、該当ファイルを複製する（fork-on-write。詳細はspec-fm.md 8章）。
- **`op505-midi`はop505-coreに置けないMIDI解釈層（CC/RPN/NRPN解釈・ペダル状態機械・Pitch FG演奏補正）の正規の置き場所**（フェーズ5.5、2026-08-14新設）。`op505-vst`と`op505/tools/smf2op505`の両方が参照する、fork-on-write方針の**限定的な例外**（理由: VSTと参照実装が食い違うと「どちらが正しいか」を決める基準そのものが消えるため。詳細はspec-fm.md 8章⑤）。依存は`op505-core`と`sound-fm`の2本のみに絞り、`sound-core`型（`MasterEffects`等）をAPIに出さない。新しいCC/NRPN解釈を追加するときは、ここに実装した上で`op505-vst`・`smf2op505`の両方から呼び出し、`_ =>`を使わない全列挙（`ControlTarget`）を保つ。
