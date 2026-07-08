# 38x6 実装ロードマップ

このファイルは 38x6 プロジェクトの実装フェーズを管理する。全体像は [spec.md](spec.md)、
音源エンジン仕様は [spec-sound.md](spec-sound.md)、アプリUI仕様は [spec-app.md](spec-app.md) を参照。

各フェーズの細かい設計経緯は `docs/session_history.txt` と memory/ に分担して残す
（書き分け方針は memory の「設計経緯の書き分け方針」を参照）。

---

## 現在地

**フェーズ5：実機音色資産の取り込みと音作り基盤**と**フェーズ7：MC-505風モジュレーション拡張**を並行進行中
（フェーズ7はステップ1「VCO抽象境界の確立」・ステップ2「EG形式の確定とVcf/Vca定義」・
ステップ3「LFO波形8種/Fade/Offset拡張（VST/UI/gesture-app配線含む）」完了、
次はLFOのカットオフ行先(オートワウ)/LFO×2・表情ルーティング・Pitch EG）

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

フェーズ2: パフォーマンスLFO・マスターエフェクト実装
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
  → 次のステップ: LFOのカットオフ行先(オートワウ)・手動ワウ・LFO×2・表情ルーティング・
    velocity→音量の量・Pitch EG（いずれも未着手）
  → LFO拡張の残り: カットオフ行先(オートワウ)・手動ワウ・LFO×2
  → velocity→音量「量」（ChannelParams、既定255）
  → 表情コントローラー・ルーティング: CC1/CC2/CC4/AT × 音量/TL/カットオフ/LFOデプス等
  → VST/NRPN配線（差分検知方式に追加）
  → スコープ外: SSG-EGループ・汎用モッドマトリクス・テンポ同期・ポリAT/MPE

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
