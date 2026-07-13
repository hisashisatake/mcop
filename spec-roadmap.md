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
ステップ4「LFOカットオフ行先(オートワウ)」・ステップ5「チャンネルLFO三層再編（設計・spec改訂）」完了、
次はステップ5.5「VCF/VCAファンクションジェネレーター統合（設計・spec改訂）」→ステップ6「コア実装」）

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

フェーズ2: パフォーマンスLFO（現: チャンネルLFO）・マスターエフェクト実装
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
  → ステップ5.5「VCF/VCAファンクションジェネレーター統合（設計・spec改訂）」【次はここ・設計中】:
    VCF/VCAのモジュレーション源を「一発(EG)にもループ(LFO)にもなるファンクションジェネレーター」に
    統一する設計を詰めてspec改訂する（コード実装はステップ6へ）。背景=VCF/VCAはループさせればLFOであり、
    アナログシンセ的なスイープ/うねりを一次源として持てる。主要論点:
    (a) sound-core::Egにループモードを追加（D1R⇄D2Rの往復。底値の定義=D1R戻り再利用 or 新規D2L、
        ノートオフで現在位置からRRへ離脱する連続性）。ADSR形状ループがチャンネルLFO(8波形/Offset)で
        代替できない具体音を1つ確定してから採否を決める
    (b) チャンネルLFOのCutoff/Volume行きをVCF/VCA側のファンクションジェネレーターへ合流(統合)するか併存か。
        Pitch/TL行きはVCF/VCAという帰る家が無くVCO側に残る（TLはFM固有、Pitchは全VCO共通）
    (c) 音色LFO(tone_lfo.rs、チップ内蔵でVCO固有＝VCO差し替えで消える)の再定義・改名
        （「TONE LFO」は実態=チップ副次パラメーター以上に重い。opz2x6/opm2x6の写像先波及に注意）
    → ステップ5で確定したチャンネルLFOのDestination構成を部分的に見直す可能性がある（先に確定させる理由）
  → ステップ6「コア実装」: ChannelParamsへLFO1/2の全8項目を完全所有させ、実効値=三層加算、
    Volume行きをVcaゲインへ合流、旧perf_lfo_shapeのserde互換移行。**加えてEG/KSRレート倍率の共通化**
    （`sound-core::Eg::tick`にレート倍率引数`rate_scale`を追加し、FM側=`ksr_mul`/VCF・VCA=`1.0`を渡す形へ
    統一。`operator.rs`が独自に持つ複製状態機械`EnvPhase`/`tick_envelope`を削除し`Eg`へ一本化する。
    既存operator.rsテスト群で「音が1サンプルもズレない」ことを担保しながら進める）（未着手）
  → ステップ7「VST配線」: CC76/77/78をパート補正（Rate/Delay=64中心相対・Depth=0起点加算）化、
    新NRPN番地(0,23〜0,35)、DAWパラメーター37個化、shadow/effectiveの二重ソースを層分離で解消（未着手）
  → ステップ8「UI/gesture-app」: ym38x6-ui共有パネルのLFO2対応、ホイール/VキーのLFO1接続、
    IPC/editor-wasm追随（未着手）
  → ステップ9「smf2wav・変換ツール」: LFO系CC/NRPNの解釈対応（マルチティンバーでパッチ外の揺れを再現、未着手）
  → 以降の未着手: 手動ワウ・表情ルーティング（CC1/CC2/CC4/AT × 音量/TL/カットオフ/LFOデプス等）・
    velocity→音量「量」（ChannelParams、既定255）・Pitch EG
  → VST/NRPN配線（差分検知方式に追加、各ステップ内で随時）
  → スコープ外: SSG-EGループ・汎用モッドマトリクス・テンポ同期・ポリAT/MPE。
    チャンネルLFOのLFO×2は固定2基であり、モッドマトリクスではない（配線先はDestination enumの4種に固定）

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
