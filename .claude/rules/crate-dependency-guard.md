---
paths:
  - "sound/**"
  - "ym38x6/core/**"
  - "op505/core/**"
  - "op505/midi/**"
  - "op505/tools/**"
  - "op505/editor/**"
---

# 音源コアクレート 依存ガード

このルールは `sound-core` / `sound-fm`（`sound/`配下）と `ym38x6-core` / `op505-core`、
`op505-midi`（`op505/midi/`）、`op505/tools/*`、および `op505-editor`（`op505/editor/`）を
編集するときだけ読み込まれる。

- これらのクレートは **nice-plug・Tauri・cpal に依存してはならない**。純粋なRustライブラリ（標準＋必要最小限のクレートのみ）を保つ。
- 音源エンジンの変更はこれらのクレートに閉じる。MIDI・ジェスチャー解釈・UI・音声出力はコアの外側で行う。
- これらに新機能を実装したら、同じタイミングで `op505-vst`（2026-08-12以降の主力。`ym38x6-vst`は凍結のため対象外）に配線し、VST単体でも機能が使える状態を保つ（MIDI CC/RPN/NRPNの受信処理やパラメーター追加を含む）。
- パラメーターは0〜255（8bit）統一。例外は周波数（オクターブ3bit + F-Number 13bit = 16bit、常にOP単位×4）とMUL（0〜15、Multiple 4bit準拠）。
- 波形フォーマットは1024×uint16_t対数で統一。線形↔対数の変換パイプラインは `sound-core` 側に置く。
- **`op505-core`と`op505/tools/*`はym38x6グループ（`ym38x6-core`・`ym38x6/tools/*`）に依存してはならない**（2026-08-11 op505デフォークで達成した状態を維持する）。唯一の例外は`op505-core`の`[dev-dependencies]`（`examples/op505_probe.rs`が既存`.38x6`との聴き比べに使う`legacy_convert_patch`のみ）。`src/`への`use ym38x6_core`混入は`[dev-dependencies]`降格によりコンパイルエラーで防げる設計になっている。共有したくなった処理は中立クレート化せず、該当ファイルを複製する（fork-on-write。詳細はspec-fm.md 8章）。
- **`op505-midi`はop505-coreに置けないMIDI解釈層（CC/RPN/NRPN解釈・ペダル状態機械・Pitch FG演奏補正）の正規の置き場所**（フェーズ5.5、2026-08-14新設）。`op505-vst`と`op505/tools/smf2op505`の両方が参照する、fork-on-write方針の**限定的な例外**（理由: VSTと参照実装が食い違うと「どちらが正しいか」を決める基準そのものが消えるため。詳細はspec-fm.md 8章⑤）。依存は`op505-core`・`sound-fm`・`sound-midi`の3本に絞り、`sound-core`型（`MasterEffects`等）を自身のAPIには出さない（`sound-midi`経由で運ぶ`EffectControlTarget`は例外として再エクスポートする、後述）。新しいCC/NRPN解釈を追加するときは、ここに実装した上で`op505-vst`・`smf2op505`の両方から呼び出し、`_ =>`を使わない全列挙（`ControlTarget`）を保つ。
- **`sound-midi`（`sound/midi/`）はop505に依存しない中立なMIDI解釈クレート**（2026-09-04新設、GM2マスターボリューム実装の一環）。GM2 Universal SysExパーサ（`universal_sysex`）と、エフェクト系NRPN/CCの`MasterEffects`への適用（`effect_control`の`EffectControlTarget`/`apply_effect_control`）を持つ。依存は`sound-core`のみ。`op505-midi`はこれに依存し`EffectControlTarget`を`pub use`で再エクスポートする（依存の向きは`op505-midi → sound-midi → sound-core`）。standalone/vst/smf2op505の3ホストは`sound-midi::apply_effect_control()`を直接呼び、`EffectControlTarget::X => fx.set_x(...)`という機械的な写像を各ホストで複製しない。
- **`op505-editor`はop505-vst/standaloneのエディタ重複（PRESETSパネル・パネル組み立て・min/max/default定義）を吸収する正規の置き場所**（2026-09-02新設）。`op505-vst`と`op505-standalone`の両方が参照する、fork-on-write方針の**限定的な例外**（理由は⑤・⑥と同型：複製するとSave/Delete等の意味論がホスト間で食い違ったときの正誤の基準が消えるため。詳細はspec-fm.md 8章⑧）。依存は`egui`/`op505-core`/`op505-ui`/`ui-core`/`sound-core`/`rfd`の6本（両ホストが既に持つ辺）のみに絞り、**`nice-plug`・`eframe`・`winit`・`cpal`・`midir`・`serde`・`Tauri`には依存してはならない**。特に`serde`非依存は、VSTの`#[persist = "op505_egs"]`対象である`Op505EgBank`をうっかり移設できない構造上の防御でもある。新しい共有パネル/プリセット操作を追加するときは、ここに`PresetHost`/`PanelParamSource`トレイト経由で実装した上で`op505-vst`・`op505-standalone`の両方から呼び出す。
