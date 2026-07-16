# 38x6 実装ロードマップ

このファイルは 38x6 プロジェクトの実装フェーズを管理する。全体像は [spec.md](spec.md)、
音源エンジン仕様は [spec-sound.md](spec-sound.md)、アプリUI仕様は [spec-app.md](spec-app.md) を参照。

各フェーズの細かい設計経緯は `docs/session_history.txt` と memory/ に分担して残す
（書き分け方針は memory の「設計経緯の書き分け方針」を参照）。

---

## 現在地

**フェーズ5：実機音色資産の取り込みと音作り基盤**と**フェーズ7：MC-505風モジュレーション拡張**を並行進行中
（フェーズ7はステップ1「VCO抽象境界の確立」・ステップ2「EG形式の確定とVcf/Vca定義」・
ステップ3「LFO波形8種/Fade/Offset拡張（VST/UI/gesture-app配線含む）」・
ステップ4「LFOカットオフ行先(オートワウ)」・ステップ5「チャンネルLFO三層再編（設計・spec改訂）」・
ステップ5.5「VCF/VCAファンクションジェネレーター統合（設計・spec改訂）」・
ステップ6「コア実装」・ステップ7「VST配線」・ステップ8「UI/gesture-app」完了、次はステップ9「smf2wav・変換ツール」)

---

## フェーズ一覧

```
フェーズ1: 波形メモリ音源とTauriデスクトップアプリの基盤（完了）
  → プロトタイプとしてWMS-1（波形メモリ音源 + ADSR）をwms1-coreに実装
    （フェーズ5以降に38x6へ統合し、wms1-core/wms1-vstは廃止。
     役割はym38x6の「波形メモリ専用音色バンク」＝Algorithm 7・OP1のみ有効が引き継ぐ）
  → 内部波形フォーマット（1024サンプル対数）と変換パイプラインを実装
  → cpalで音声出力
  → マウスによる2Dジェスチャー入力UIの実装
  → キャリブレーション（C-F-G基準点）の実装

フェーズ2: パフォーマンスLFO（現: FG／質感LFO）・マスターエフェクト実装
  → PerformanceLfo / PerformanceLfoTarget をsound-coreに実装
  → MasterEffects（Reverb/Chorus）をsound-coreに実装

フェーズ3: 38x6 FMエンジン導入、波形選択・デチューン拡張（完了）
  → OPZ系の音色表現を取り込む

フェーズ4: OP単位F-Number・独立キーオンを実装（完了）
  → OPQ由来の音楽的表現を一般化して活用

フェーズ5: 実機音色資産の取り込みと音作り基盤（進行中）
  → 変換ツール群（tools/）: opz2x6 / vgm2x6 / psr2x6 / mucom2x6 / opm2x6
    実機・既存資産（OPZ/OPM/OPN/OPQ/PSR-70/MUCOM88）を .38x6 バンクへ変換
    （WAVEFORM_MEMORY_BANK+1以降。Bank 0には流用しない）
  → 試聴・検証ツール: smf2wav（SMF→WAV）/ wavetest（波形試聴）/
    opzref（ymfm参照レンダラ、音程・配線の検算用）
  → エンジン忠実度チューニング（feedback・KSR・EG曲線等）。
    OPN/OPM忠実を保ちつつ拡張軸（8bit・非サイン波形・フィルター等）へ投資

フェーズ6: 音色設計（patchlab）でGM2準拠Bank0を生成
  → 当初は目標音声からのFMパラメーター逆算（ML/A-by-S インバース合成）を計画していたが、
    FM/PCMの音響空間が重ならず行き止まりと判明（詳細はdocs/session_history.txt参照）
  → 知覚記述子（明るさ・金属度・歪み等7軸）の自動分析＋probe探索による近傍パラメーターの
    ルックアップを叩き台に、人手で族ごとにテンプレート設計する方式へ転換
  → 38x6エンジンのPythonバインディング（PyO3 + maturin）経由でテンプレートを試聴・反復
  → GM2プログラムマップ準拠のBank 0音色セットを族ごとに手動設計
    （Bank 0には実機プリセットを直接流用せず、フェーズ5の変換音色は参考程度に用いる）
  → 同一リポジトリ内の tools/patchlab/ に収録

フェーズ7: MC-505風モジュレーション拡張（承認済みプラン Part 2・2026-06-19、進行中）
  → 実装層は sound-core。FMをVCOと見なし後段にアナログシンセ的なVCF/VCA/VCO変調層を被せる
    （決め台詞「EG＝一発の形 / LFO＝持続する揺れ」。実機への忠実ではなく拡張軸）
  → アーキテクチャ方針: モジュレーション層は sound-core に置き、発振源（VCO）を差し替え可能にする。
    ym38x6-core のFMエンジンはVCO実装の一つとして扱い、将来は別の発振源へ置換できる抽象境界を用意する
    （詳細は spec.md「クレート構成／VCO抽象」を参照）
  → ステップ1「VCO抽象境界の確立」完了（Vco/AudioProcessorトレイトをsound-coreに実装）
  → ステップ2「EG形式の確定とVcf/Vca定義」完了: 5段OPM形式（AR/D1R/D1L/D2R/RR）を採用。
    状態機械`Eg`・トレイト`Vcf`/`Vca`をsound-coreに実装（具象実装`VoiceFilter`/`VoiceAmp`）。
    旧フィルターEG（4段ADSR、ym38x6-core/src/filter.rs）は撤去しVcfへ統合済み。
    Pitch EG（EnvelopeによるピッチModulation）は今回のスコープ外で未着手のまま残る
  → ステップ3「LFO拡張（波形8種/Fade/Offset）」完了: `LfoWaveform`にSaw/Trapezoid/Random/Chaosを追加し
    8種に、`LfoFadeMode`(ON-IN/ON-OUT/OFF-IN/OFF-OUT)・`PerformanceLfoShape`(waveform/fade_mode/
    fade_time/offset)を新設。`ChannelParams.perf_lfo_shape`として発音中もリアルタイム反映。
    VST（NRPN+DAWパラメーター両対応）・ym38x6-ui共有パネル・gesture-app（editor-wasm/Tauri IPC）まで配線済み
  → ステップ4「LFOカットオフ行先(オートワウ)」完了: `Ym38x6LfoDestination`に`Cutoff`を追加し、
    `Channel::tick()`でLFOがシフトした基準CutoffをVcfへ渡すことでFilter EG Depth（キーオン一発の変調）
    と独立に積み重なる持続的な変調を実現。Depthは`cutoff_depth(cc77,cc1)`（sound-core）で
    Filter EG Depthと同じ0〜255単位系に統一。VST NRPN(0,0)=3・gesture-app Tauri IPCまで配線済み
    （UIトグルは既存のTLキャリア一括同様スコープ外のまま）
  → ステップ5「チャンネルLFO三層再編（設計・spec改訂）」完了: 旧「パフォーマンスLFO」の帰属分裂
    （波形/Fade/Offsetはパッチ所有・Rate/Delay/Destination/Depthはランタイム専用）を解消し、三層モデル
    （①音色/②パート状態/③ジェスチャー）で再編する設計を確定。LFO×2対称・完全パッチ所有・実効Depth=三層加算・
    Volume→Vca合流を spec-sound.md/spec.md/spec-app.md に明文化（コード変更なし）
  → ステップ5.5「VCF/VCAファンクションジェネレーター統合（設計・spec改訂）」完了:
    VCF/VCAのモジュレーション源を「一発(EG)にもループ(LFO)にもなるファンクションジェネレーター(FG)」に
    統一する設計を確定しspec改訂した（コード実装はステップ6へ）。決着内容:
    (a) `sound-core::Eg`にLoop/Floor/Curveを追加（既存5段EG＋3項目の拡張、新規部品ではない）。
        Loop=1でFloor⇄peakをAR/D1Rが独立レートで往復（膝なし）、Curveは線形/サイン風の2択で出力レベルのみ整形、
        ノートオフで現在位置からRRへ離脱する連続性を持つ。決め手はアシッド風フィルター（開閉非対称スイープ）の
        A/B検証・実測グラフで、チャンネルLFOのサイン波では出せない固有価値と確認した
    (b) 3FGスロット（Pitch新規／Cutoff=旧Filter EG／Gain=旧VCA EG）へ集約。チャンネルLFOのCutoff/Volume/Pitch行きは
        「軌跡で表せる」三角/のこぎり/サイン波の範囲でFGへ畳み、「軌跡で表せない」矩形/台形/S&H/ランダム/カオスの
        5波形だけを**質感LFO1基**（焼き込み専用・set-and-forget）に隔離した。境界規則「パラメーター化された
        時間軌跡ならFG、そうでない（生成器/固定波形）なら質感LFO」で重なりゼロに整理。演奏CC(CC1/76/77/78)は
        Pitch FGへ配線（質感LFOはCC補正を受けない）
    (c) 音色LFO(tone_lfo.rs)を「チップ内LFO」へ改名（spec表記のみ、コード改名はステップ6）。VCO固有＝VCO
        差し替えで消えるレイヤー帰属を名指しする呼称とした
    → 詳細はspec-sound.mdの「ファンクションジェネレーター」節・「質感LFO」節を参照
  → ステップ6「コア実装」完了（`sound-core`/`ym38x6-core`に閉じた4コミット、2026-07-14）:
    (1) `sound-core::Eg`にLoop/Floor/Curve・`retrigger()`（残響レベル保持での再Attack）・
        `rate_scale`引数を追加。`Vcf`/`Vca`の`process()`を`EgParams`ベースの引数へ変更
    (2) `operator.rs`が独自に持っていた複製状態機械`EnvPhase`/`tick_envelope`を削除し`sound_core::Eg`
        （KSRは`rate_scale`として適用）へ一本化。既存テスト群を出力振幅ベースの検証に書き換えて
        「音が1サンプルもズレない」ことを確認
    (3) `tone_lfo.rs`を`chip_lfo.rs`（`ToneLfo`→`ChipLfo`）へ改称。JSON上の旧フィールド名
        （`tone_lfo_freq`等）は`#[serde(rename)]`で維持し既存`.38x6`は無変更で読める
    (4) `ChannelParams`から`filter_eg_*`/`vca_eg_*`/`perf_lfo_shape`（12個）を削除し、
        `pitch_fg`/`cutoff_fg`/`gain_fg`（`sound-core::BipolarFg`/`GainFg`、共通EG=`EgParams`）・
        `texture_lfo`（5波形専用8項目、完全パッチ所有）へ再編。`ChannelParams`に手動`Deserialize`
        （`ChannelParamsWire`シャドー構造体経由）を実装し、旧`.38x6`ファイルを新スキーマへ自動移行する
        後方互換レイヤーを追加（回帰テストで検証）。`Ym38x6Engine::set_performance_lfo`ランタイムAPIは
        質感LFOの完全パッチ所有化により廃止。`ym38x6-vst`/`gesture-app`/`tools/wavetest`は新スキーマへ
        機械的に追従済み（新パラメーターのUI/NRPN露出はステップ7/8）
    → `cargo test --workspace`で全クレート0 failedを確認
  → ステップ7「VST配線」完了（`sound-core`/`ym38x6-core`/`ym38x6-vst`/`ym38x6-ui`、2026-07-16）:
    (1) 実装中に発覚した仕様矛盾を解決：CC78「Pitch FG Delayへの64中心相対補正」の対応先が
        `EgParams`に存在しなかったため、`sound-core::EgParams`にDelayフィールドを新設
        （Pitch/Cutoff/Gain FG共通、`#[serde(default)]`で後方互換）。`sound-core::Eg`に
        `EgPhase::Delay`と`elapsed`カウンタを追加し、`delay=0`は同一tick内でAttackへフォールスルーする
        設計で既存パッチのサンプル精度互換を維持。DAWパラメーターは仕様書記載の45個ではなく
        Pitch/Cutoff/Gain FG各Delay追加分を含め**48個**に確定（spec-sound.md更新済み）
    (2) CC76（Vibrato Rate、「AR/D1Rを一括スケール」）はAR/D1Rの指数マッピング特性上、生コードへの
        加算では成立しないと判明し、`sound-core::Eg::tick`の`rate_scale`引数（KSRと同じ仕組み）を
        経由する方式に設計変更。`cc76_to_rate_scale`（sound-core::eg純粋関数）・
        `Ym38x6Engine::set_pitch_fg_rate_scale`（`Vco`トレイトではなく`set_operator_f_number`と
        同じ38x6固有の単一ボイスsetter）・`Channel::pitch_fg_rate_scale`を新設し、
        `pitch_bend`/`channel_volume`と同じnote_on直後+毎ブロック適用パターンで配線
    (3) `ym38x6-vst`のチャンネルDAWパラメーターを48個へ全面再構成（`feg_*`→`cutoff_fg_*`、
        `vca_*`→`gain_fg_*`、`lfo_*`→`texture_lfo_*`とリネームしつつPitch FG一式・Floor/Loop/Curve/Delay
        を新設）。`cutoff_fg_depth`は旧unipolar→bipolar変換式を撤去し直接コピーに簡素化。
        `gain_fg_rr`のVST既定値を255→0に修正（`default_gain_fg()`の透過的既定と不整合だった）
    (4) NRPN(0,1)質感LFO Waveformを8波形経由の変換なし0〜4直接値へ簡素化。新規NRPN(0,23)〜(0,33)
        （質感LFO Rate/Depth/Delay/FadeTime/Offset・FG Loop/Curve×3）を追加。CC1/76/77/78は
        質感LFOから完全に切り離しPitch FGのみを補正するよう再配線（具体式はspec-sound.md参照）
    (5) 質感LFOのRate/Depth/DelayをNRPN直書き込み＋DAW差分検知の1シャドウへ統一し、旧
        `effective_lfo_*`の2シャドウ（CC76/77/78とDAWパラメーターの二重ソース）を廃止
        （＝「shadow/effectiveの二重ソースを層分離で解消」の実体）
    (6) `ym38x6-ui::LFO_WAVEFORM_NAMES`を8波形から質感LFOの5波形（Square/Trapezoid/S&H/Random/Chaos）
        へ修正し、GUIの質感LFO波形ドロップダウンが新NRPN(0,1)仕様と一致するようにした
    → `cargo test --workspace`で全クレート0 failedを確認
  → ステップ8「UI/gesture-app」完了（`ym38x6-ui`/`ym38x6-vst`/`gesture-app`、2026-07-16）:
    (1) `ym38x6-ui::PanelParams`を新世代スロット名へ改名（`lfo_*`→`texture_lfo_*`／`feg_*`→
        `cutoff_fg_*`／`vca_*`→`gain_fg_*`／`tone_*`→`chip_lfo_*`）。新設`FgEgPanelParams`/
        `BipolarFgPanelParams`で共通EG（AR/D1R/D1L/D2R/RR/FLOOR/DLY/LOOP/CURVE）を構造化し、
        **PITCH FGパネルを新規追加**、CUTOFF FG/GAIN FGパネルにFloor/Delay/Loop/Curveノブを追加。
        ラベルも"TEXTURE LFO"/"CHIP LFO"/"CUTOFF FG"/"GAIN FG"へ改名
    (2) `eg_preview()`のシグネチャを`sound_core::eg::EgParams`ベースへ統一し、Delay（線形0〜10秒の
        無音リードイン）・Loop（Floor⇄peak 2サイクル往復＋RR離脱）・Curve（レイズドコサイン整形）を
        描画に反映。OPパネルの既存FLOOR/LOOP/CURVEノブがプレビュー未反映だった不整合も解消
    (3) VST `editor.rs`・gesture-app `editor-wasm`（state/handle/app/ipc）を新スロットへ結線。
        `ipc.rs`/`ym38x6_dto.rs`のDTOがPitch FG・各FGのFloor/Delay/Loop/Curve・質感LFOの
        rate/depth/delay/destinationを往復で破棄していた問題を修正（`ChannelParamsDto`拡張）。
        質感LFO波形が0〜7のままだったgesture-app側のスタールバグ（新5波形はmax=4）も修正。
        旧8波形経由の変換ロジック（`perf_lfo_waveform_to_texture_lfo_index`等）は撤去し
        直接値渡しに簡素化
    (4) gesture-appのVキー（ビブラート⇔トレモロ切替、既存トグルUX維持）を再配線: Pitch時は
        `pitch_fg`（Loop=1・Curve=1・AR/D1R初期値で往復ビブラート、CC76相当は`set_pitch_fg_rate_scale`
        でVSTと同じ経路により発音中チャンネルへ個別反映）、Volume時は従来通り質感LFOへ書き込む
    → `cargo test --workspace`0 failed・VST3/CLAPバンドルビルド・gesture-app実機起動（スクリーン
      ショットでPITCH FG/CUTOFF FG/GAIN FGパネル表示とV/C/Bキー操作の無クラッシュを確認）で検証済み
  → ステップ9「smf2wav・変換ツール」: FG/質感LFO系CC/NRPNの解釈対応（マルチティンバーでパッチ外の揺れを再現、未着手）
  → 以降の未着手: 手動ワウ・表情ルーティング（CC1/CC2/CC4/AT × 音量/TL/カットオフ/FG Depth等）・
    velocity→音量「量」（ChannelParams、既定255）
  → VST/NRPN配線（差分検知方式に追加、各ステップ内で随時）
  → スコープ外: SSG-EGループ・汎用モッドマトリクス・テンポ同期・ポリAT/MPE。
    質感LFOは固定1基であり、モッドマトリクスではない（配線先はDestination enumの4種に固定）

フェーズ8: パラメーターUI・音色運用
  → パラメーターUI・音色保存・プリセットライブラリ（.38x6 の書き出しUI）
  → 実装形態（VST製 or Tauri製）は未決定
  → Bank Select / Program Change（受信は実装済み、UI・運用が残課題）

フェーズ9: スケール判定・アボイド挙動の検証

フェーズ10: タブレット対応（Tauri v2 iOS/Android）
  → マルチタッチ入力の実装（UIロジックは共通）

フェーズ11: アルゴリズム拡張モード（オプション）
  → SY77スタイルのルーティングレジスタ公開
```
