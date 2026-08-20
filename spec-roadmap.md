# op505 実装ロードマップ

このファイルは実装フェーズを管理する。全体像は [spec.md](spec.md)、
音源エンジン仕様は [spec-sound.md](spec-sound.md)、アプリUI仕様は [spec-app.md](spec-app.md)、
FM音源変換ツール群の横断知見は [spec-fm.md](spec-fm.md) を参照。

各フェーズの細かい設計経緯は `docs/session_history.txt` と memory/ に分担して残す
（書き分け方針は memory の「設計経緯の書き分け方針」を参照）。

---

## ⚠️ 方針転換（2026-08-12）: ym38x6チップの開発中止、op505へ一本化

**ym38x6（YM3806(OPQ)ベースの旧チップ）は開発中止。以後の新規開発は全てop505（後継チップ、
N点Time/Level方式EG）に一本化する。**

- **ym38x6関連コード（`ym38x6-core`/`ym38x6-vst`/`ym38x6-ui`、`ym38x6/tools/`配下の変換・検証ツール群）は
  凍結した**（動作は保持するが新機能追加はしない）。当面は資産（実装済みロジック・実機忠実度
  チューニングの知見・変換テーブル）としてop505側の参考・移行元に使う運用とした。
- **共有基盤（`sound/core`・`sound/fm`・`ui/core`・`ui/layout`・`ui/codegen`）はop505が継承する。**
  Vcoトレイト・FG/質感LFO・アルゴリズム結線表等、ym38x6向けに実装された機能の大半はチップ非依存の
  共有レイヤーに実装済みのため、この方針転換による手戻りは限定的（下記フェーズ1・2・4・7を参照）。
- 以下のフェーズ一覧は、この方針転換を踏まえて**移行対象を明記する形に改訂**した
  （2026-08-12改訂。改訂前の内容はgit履歴で参照可能）。
- **2026-08-20、凍結中だったym38x6関連のコード・データを一式削除した**（`ym38x6/`ディレクトリ、
  gesture-appのデュアルエンジン構成、Cargo.tomlのワークスペースメンバー等）。op505-coreは
  元々ym38x6-coreに一切依存しない設計だったため、この削除でop505側の挙動に変更はない。
  再生成不能な原本データ（TX81Z実機吸出しsyx・実機録音等）は削除前にアーカイブへ退避済み。
  `CLAUDE.md`・`spec.md`等の他ドキュメントの棚卸しも同日中に完了した。

---

## 現在地

**op505ツール群のデフォーク（ym38x6依存ゼロ化）・gesture-app既定エンジンのOP505化・
op505-uiのXML DSL移行・共有クレートのsound/ui グループ再編・op505-vstフェーズ1（DAWで鳴らす・
編集する・プリセット選択）・op505-vstフェーズ2（NRPN・表情CC・ペダル・OP単位キーオン等の
MIDI表現系）・フェーズ5.5（検証・音作りツール群のop505移行、opzref/wavetest/patchlab/
smf2op505・op505-midi共有クレート化）が完了**（2026-08-14時点、詳細はフェーズ5.5・フェーズ8・
フェーズ12）。**2026-08-20、凍結中だったym38x6関連コード・データを一式削除し、
gesture-appも単一エンジン（OP505）構成へ単純化した**。次の主な残作業はフェーズ6（GM2テンプレートのop505向け再設計）。

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

フェーズ2: パフォーマンスLFO（現: FG／質感LFO）・マスターエフェクト実装（完了）
  → PerformanceLfo / PerformanceLfoTarget をsound-coreに実装
  → MasterEffects（Reverb/Chorus）をsound-coreに実装
  → リバーブの聴感調整完了: Freeverb方式からFDN方式（Householder行列、8ライン）へ刷新し
    プリディレイを追加（2026-07-20、`3abe090`）。詳細はmemory参照

フェーズ3: 38x6 FMエンジン導入、波形選択・デチューン拡張（完了・ym38x6固有、2026-08-20削除済み）
  → OPZ系の音色表現を取り込む（ym38x6-core）
  → op505-coreは別実装（N点Time/Level方式EG）としてフェーズ12以降に新設・デフォーク済み。
    このフェーズ自体の再実施は不要（アルゴリズム結線表・TL/KSR等のマッピングはsound-fmとして
    共有化済みのため、op505-coreはym38x6-coreの複製ではなく共有部品の再利用＋チップ固有EGの
    新規実装という形で構築されている）

フェーズ4: OP単位F-Number・独立キーオンを実装（完了）
  → OPQ由来の音楽的表現を一般化して活用

フェーズ5: 実機音色資産の取り込みと音作り基盤（op505直接変換ツール群・検証ツール群とも移行完了）
  → 変換ツール群（ym38x6/tools/、2026-08-20削除済み）: opz2x6 / vgm2x6 / psr2x6 / mucom2x6 / opm2x6
    実機・既存資産（OPZ/OPM/OPN/OPQ/PSR-70/MUCOM88）を .38x6 バンクへ変換していた
    （WAVEFORM_MEMORY_BANK+1以降。Bank 0には流用しない）
  → **2026-08-11 op505デフォーク完了**（feature/op505-defork、developへ--no-ffマージ・push済み、
    `64a1de2`）: 上記5ツール全てを .op505 直接変換版（opz2op505/vgm2op505/psr2op505/
    mucom2op505/opm2op505、`op505/tools/`）へ移行済み。ym38x6-core/xxx2x6/vgm2x6への依存はゼロ
    （fork-on-write方式で複製・再構築、詳細はmemory「op505デフォーク全フェーズ完了」・spec-fm.md「デフォーク」節）
  → 試聴・検証ツール（ym38x6/tools/、2026-08-20削除済み）: smf2wav（SMF→WAV、エンジン性能検証・
    CC/NRPN解釈の参照実装を兼ねる）/ wavetest4x6（波形試聴）/ opzref4x6（ymfm参照レンダラ、音程・配線の検算用）
    → **op505向け移行はフェーズ5.5で完了**（smf2op505 / wavetest / opzref、op505/tools/配下）
  → エンジン忠実度チューニング（feedback・KSR・EG曲線等、完了・当時のym38x6-coreで実施済み）。
    OPN/OPM忠実を保ちつつ拡張軸（8bit・非サイン波形・フィルター等）へ投資した知見は、
    op505-coreの新規EG設計（4点T/L EG検討）にも引き継がれている
  → feedback上限1.8（ハイハット系のノイズ再現）とOPNベース音色（音程安定性）のトレードオフは
    ym38x6-core上で解決済み: エンジンのフィードバック帰還を2サンプル平均へ既定化し、
    opz2x6/psr2x6のmod_tl_cap既定をNone（天井なし）化することで両方を改善
    （2026-07-17〜18、`ea2b998`）。op505-coreへの同様の適用状況は要確認

フェーズ5.5（新設・2026-08-12、完了・2026-08-14）: 検証・音作りツール群のop505移行
  → `feature/op505-tools-migration`ブランチで実施。ステップ1〜4（opzref/wavetest/patchlab/
    op505-midi+smf2op505）+ ドキュメント更新まで全て完了、developへ--no-ffマージ済み
  → **opzref のop505対応 完了**: `op505/tools/opzref`新設（opz2op505ベース、`RegSink`トレイトで
    実機/テスト二重化）。旧`opzref4x6`（`ym38x6/tools/opzref4x6`へリネームし凍結資産として保持していたが、
    2026-08-20にym38x6一式削除に伴い削除済み）とWAVハッシュ完全一致を確認済み
  → **wavetest のop505対応 完了**: `op505/tools/wavetest`新設。`op505_core::eg_convert::convert_eg_shape`
    経由で既存9音色×4opのレート方式EG数値を維持しつつTimeEg化、TimeEgネイティブデモ3種
    （多段リリース・ループGain FGゲート・非単調EG）を追加。旧`wavetest4x6`は2026-08-20削除済み
  → **patchlab のop505対応 完了**: `op505/tools/patchlab`新設（PyO3バインディング）。35次元
    パラメーター空間は維持し、`vector_to_patch()`の出口で`convert_eg_shape`を一度通す設計。
    `python/op505_patch.py`が「レート方式ノブdict→op505パッチdict」の唯一の変換入口。
    ym38x6版（`ym38x6/tools/patchlab`）は2026-08-20削除済み
  → **op505-midi クレート新設・smf2op505 新設 完了**: CC/NRPN解釈を`op505-midi`
    （`op505/midi`）へ共有クレート化し、`op505-vst`と`smf2op505`の両方が参照する
    （fork-on-writeの限定的な例外、詳細はspec-fm.md 8章）。`ControlTarget` enumで
    NRPNアドレス表を一元化し`_ =>`を禁止する全列挙で解釈のずれを構造的に防止。
    `PedalState`（CC64/66/67/120/121/123の状態機械）・`RpnTracker`・`apply_pitch_fg_expression`
    等も共有。`op505-vst`はmidi.rsを廃しop505-midi参照へ全面移行（REAPERの既存フェーズ2
    検証プロジェクトで全CC/NRPNマーカーの再生確認済み）。`smf2op505`は`ym38x6/tools/smf2wav`
    から複製し、単体テスト19件を移植・実バンク+実SMFでの動作確認済み
  → `op505-tools`に`fx`（マスターリバーブ後段適用）を追加、`/perf-bench`スキルをsmf2op505版へ切替
  → 次はフェーズ6（GM2テンプレートのop505向け再設計）

フェーズ6: 音色設計（patchlab）でGM2準拠Bank0を生成（op505向けに仕切り直し）
  → 当初は目標音声からのFMパラメーター逆算（ML/A-by-S インバース合成）を計画していたが、
    FM/PCMの音響空間が重ならず行き止まりと判明（詳細はdocs/session_history.txt参照）
  → 知覚記述子（明るさ・金属度・歪み等7軸）の自動分析＋probe探索による近傍パラメーターの
    ルックアップを叩き台に、人手で族ごとにテンプレート設計する方式へ転換（設計方針自体は継続）
  → **ym38x6版（.38x6形式）でPiano(0-7)/Organ(16-23)/Brass(56-63)の3族が完了**していたが
    （piano_template.py/organ_template.py/brass_template.py）、ym38x6凍結に伴い参考資産として
    保存していた（2026-08-20、ym38x6一式削除前にアーカイブへ退避済み）。
    op505向け（.op505形式）に再設計が必要
  → 残り13族（Chromatic Percussion(8-15)/Guitar(24-31)/Bass(32-39)/Strings(40-47)/
    Ensemble(48-55)/Reed(64-71)/Pipe(72-79)/Synth Lead(80-87)/Synth Pad(88-95)/
    Synth Effects(96-103)/Ethnic(104-111)/Percussive(112-119)/Sound Effects(120-127)）は
    未着手のまま。**op505向けpatchlab移行（フェーズ5.5）が完了したため着手可能**
  → `op505/tools/patchlab/` に収録（フェーズ5.5でym38x6/tools/patchlab/から複製・移行済み）

フェーズ7: MC-505風モジュレーション拡張（完了・共有基盤としてop505に継承）
  → 実装層は sound-core。FMをVCOと見なし後段にアナログシンセ的なVCF/VCA/VCO変調層を被せる
    （決め台詞「EG＝一発の形 / LFO＝持続する揺れ」。実機への忠実ではなく拡張軸）
  → アーキテクチャ方針: モジュレーション層は sound-core に置き、発振源（VCO）を差し替え可能にする。
    Vcoトレイトは当時`Ym38x6Engine`と`Op505Engine`の2実装を持っていた（ym38x6削除〈2026-08-20〉後は
    `Op505Engine`のみ。詳細は spec.md「クレート構成／VCO抽象」）。
    **このフェーズの成果物（Eg/Vcf/Vca・FG・質感LFO・アルゴリズム結線表等）はチップ非依存の
    sound-core/sound-fmに実装されているため、ym38x6凍結の影響を受けずop505でもそのまま有効。**
  → ステップ1「VCO抽象境界の確立」完了（Vco/AudioProcessorトレイトをsound-coreに実装）
  → ステップ2「EG形式の確定とVcf/Vca定義」完了: 5段OPM形式（AR/D1R/D1L/D2R/RR）を採用。
    状態機械`Eg`・トレイト`Vcf`/`Vca`をsound-coreに実装（具象実装`VoiceFilter`/`VoiceAmp`）。
    旧フィルターEG（4段ADSR、ym38x6/core/src/filter.rs）は撤去しVcfへ統合済み。
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
        質感LFOの完全パッチ所有化により廃止。`ym38x6-vst`/`gesture-app`/`ym38x6/tools/wavetest4x6`は新スキーマへ
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
  → ステップ9「smf2wav・変換ツール」完了（`ym38x6/tools/smf2wav`、2026-07-16）: smf2wav を VST と同等の
    CC/NRPN 解釈器にし、マルチティンバーでパッチ外の揺れを再現する。
    (1) `smf.rs` に Channel Pressure(0xD0)/Poly Key Pressure(0xA0) の抽出を追加（AT Destination用、
        従来は読み飛ばしていた）。
    (2) `render.rs` を全面刷新。VSTのプラグイングローバル・シャドウを **MIDIチャンネル別** の
        `ChannelState`×16 に持ち直し、`build_effective_patch`（ベースパッチ＋CC/NRPN上書き＋Pitch FG
        演奏補正）・`apply_live`（発音中ボイスへのライブ伝播、VST毎ブロック伝播ループの1ch版）・
        `note_on_voice`・`handle_data_entry`（RPN/NRPN全ケース）・`update_rpn_selection` を実装。
        NRPN離散/焼き込みフィールドは `Option`（None=パッチ値のまま）で「NRPNは現在のパッチの当該
        フィールドのみ書き換え」を表現。CC1/7/11/76/77/78・NRPN(0,0)〜(0,33)・AT・OPキーオン(CC103〜106)
        まで対応。
    (3) エフェクトは `sound-core::MasterEffects` を `render_smf` へ組み込み（CC91/93・NRPN(0,2)〜(0,8)、
        master単位）、チャンク単位で適用してオートメーションをサンプル正確に反映。既存の `--reverb-*`
        CLI（fx.rs後段リバーブ、既定OFF）は独立温存。
    (4) `RpnSelection`/`AtDestination`/`apply_at_modulation`/`cc_to_u8`/`cc_to_u7`/`channel_gain` は
        VST(`ym38x6-vst`はcdylibで参照不可)から `ym38x6/tools/smf2wav/src/midi.rs` へ最小複製（コメントで明記、
        VST側変更時は追従が必要）。
    → CC/NRPNの無いSMFは出力バイト同一（後方互換）。`cargo test --workspace` 0 failed（ライブ伝播・
      混在CC/NRPNのE2Eスモークで発音中CC1の反映と無クラッシュを検証）＋実バンク/実SMFでのバイナリ実行を確認済み。
  → ステップ10「手動ワウ・表情ルーティング（CC2/CC4）」完了（`ym38x6-vst`/`ym38x6/tools/smf2wav`、2026-07-16）:
    既存のAT（Channel/Poly Key Pressure）destination加算モデル（`AtDestination`→
    `ExpressionDestination`へ改称、6行先: LFO PMD/AMD・Filter Cutoff/Resonance・TL全OP/キャリア）を
    CC2（ブレス）・CC4（フット）へ一般化（`apply_at_modulation`→`apply_expression_modulation`、
    複数ソースの`(値,行先)`を受け取り同一行先なら加算）。`ym38x6-core`は無改造（既存のパッチ
    フィールド加算モデルへ載るのみ）。CC2既定行先=TLキャリア一括、CC4既定行先=Filter Cutoff
    （＝**手動ワウ**、LFOのオートワウと独立に積み重なる）。CC1はGM2慣例のPitch FG固定を維持。
    NRPN(0,34)=CC2 Destination／(0,35)=CC4 Destination（AT Destinationと同じNRPN専用シャドウ、
    DAWパラメーター非公開）。VST/smf2wav両方に配線済み、`cargo test --workspace`で0 failed。
  → CC64サステインペダル（ホールドフラグ方式）完了: `ym38x6-vst`/`ym38x6/tools/smf2wav`両方に
    MIDIチャンネルごとの`pedal_down: [bool; 16]`/`pending_release: [u128; 16]`で実装。
    smf2wav側（commit e828515）に弾き直し時の保留ビットクリア漏れ（stale pending_release）が
    見つかったため同時修正。`cargo test -p smf2wav`にホールド挙動・弾き直し回帰の2テスト追加。
  → velocity→音量「量」完了: 旧来チャンネル一括で掛けていた`velocity/127`
    （`mapping::velocity_to_volume_gain`）を、`OperatorParams.velocity_gain`
    （OP単位・0〜255・既定255＝フル）へ移設した。全キャリアが既定255の既存パッチは
    従来と数式同一の高速経路（`Channel::tick`）を通り後方互換。0にするとそのキャリアは
    ベロシティに関わらず常時フル音量になる（オルガン的運用）。モジュレーター専用の
    `velocity_sensitivity`（明るさ）とは独立・別軸。UIパネルはALG連動でVEL(明るさ)/
    V.GAIN(音量)を排他的にグレーアウトする。opz2x6はキャリアKVS(0-7)をこのフィールドへ
    写像するようになった（従来は捨てていた）
  → VST/NRPN配線（差分検知方式に追加、各ステップ内で随時）
  → スコープ外: SSG-EGループ・汎用モッドマトリクス・テンポ同期・ポリAT/MPE。
    質感LFOは固定1基であり、モッドマトリクスではない（配線先はDestination enumの4種に固定）
  → **ステップ7〜10・smf2wav対応（ステップ9）の実装対象はym38x6-vst/ym38x6/tools/smf2wavだった。
    op505-vst新設時（フェーズ8）・smf2op505新設時（フェーズ5.5）に同等の配線を行う必要がある**
    → **両方完了**（op505-vstフェーズ2、フェーズ5.5、2026-08-14時点）

フェーズ8: パラメーターUI・音色運用（op505向けに再定義）
  → **op505用VST3/CLAPプラグイン（`op505-vst`）フェーズ1完了**（2026-08-12、`op505/vst`新設）。
    `Op505Patch`全269値のうち、EG以外のスカラー75個（TL/ALG/LFO/FG Depth等）をDAWパラメーター
    （オートメーション可）、TimeEg 7本＝203値（OP1〜4 EG・Pitch/Cutoff/Gain FG）をnice-plugの
    `#[persist]`状態（プロジェクト保存、オートメーション不可）とするハイブリッド構成を採用した。
    理由: `TimeEgHandle`は「EG1本を丸ごと読み書き」するAPIのため、全269値をDAWパラメーター化すると
    グラフの点を1つ動かすたび29パラメーターへの書き込みが走りオートメーション記録単位が壊れる。
    オーディオスレッドは`RwLock::try_read()`でpersist状態を取り込み、ロック待ちで詰まらない
    （取れなければ前ブロックの値を使い1〜2ブロック遅れで収束）。既定パッチは`Op505Patch::default()`
    （tl=0・空EG＝無音）のままだと挿入直後に無音になるため、TL=200＋瞬時サステインEG
    （instant_sustain_eg、`default_gain_fg()`と同形）で明示的に「鳴る」既定値にした。
    `op505-ui`の`draw_op505_panel`をそのまま再利用（エディタ側の改修なし）。
    REAPER実機確認済み（発音・ライブ伝播・EGドラッグ・`.op505`プリセット選択・
    プロジェクト保存→再起動でのpersistラウンドトリップ）
  → **op505-vstフェーズ2完了**（2026-08-12、feature/op505-vst-phase2）: MIDI表現系
    （NRPN・表情CC・ペダル・OP単位キーオン・RPN）を実装した。
    NRPNテーブル30エントリ（op505のTimeEg 7本はpersist状態でNRPNから直接書けないため
    Pitch/Cutoff/Gain FG Loop/Curveは欠番）、表情ルーティング
    CC1/CC2/CC4/CC76/CC77/CC78（`ExpressionDestination`モデル移植、CC78はTimeEgにDelay
    フィールドが無いためPitch FG第0段の`time`への相対補正で代替）、CC66/CC67/CC120/CC121/CC123、
    OP単位キーオンCC103〜106・OP単位F-Number(NRPN 0,18〜21)・RPN(0,0)ピッチベンド感度・RPN(0,5)、
    MASTER EFFECTSのGUIノブ（`op505-ui`の`panel.xml`へ共有パネルとして追加、`Op505PanelParams`に
    9フィールド。VST独自ストリップにはせず、gesture-app editor-wasmとも共有）を実装。
    表情CC/Channel Pressure/Poly Key Pressureはym38x6のグローバル単一値から全16 MIDIチャンネル
    独立へ拡張（`Op505Engine::collect_active_channels`を新設し、発音中ボイスのみ列挙して
    MIDIチャンネル別の実効パッチを適用）。NRPN/RPN状態自体はグローバル単一のまま（パッチ全体設定の
    ため）。詳細はop505-vstフェーズ2実装メモ・spec-sound.md「op505でのNRPNテーブル差分」参照
  → パラメーターUI・音色保存・プリセットライブラリ（.op505 の書き出しUI、op505-vstのGUIからの
    保存導線は未着手）
  → Bank Select / Program Change（gesture-app側・op505-vst側とも受信・切替は実装済み。
    運用UI・ドキュメントは今後整備）
  → MUL表示のOPZ比率化（ym38x6-ui向けは完了済み、2026-07-20、`15e3df1`）。op505-ui側の
    対応要否・EGパラメーター体系の違い（Time/Level方式）を踏まえた表示設計は別途判断

フェーズ9: スケール判定・アボイド挙動の検証
  → gesture-appの既定音源エンジンは既にOP505（2026-08-11、feature/gesture-app-op505-default）
    のため、このフェーズはop505エンジンを前提に実施する

フェーズ10: タブレット対応（Tauri v2 iOS/Android）
  → マルチタッチ入力の実装（UIロジックは共通、エンジン非依存のため変更なし）

フェーズ11: アルゴリズム拡張モード（オプション）
  → SY77スタイルのルーティングレジスタ公開
  → アルゴリズム結線表は`sound-fm::algorithm`として共有化済みのため、op505-core向けに実施する

フェーズ12: OP505（新チップ、N点Time/Level方式EG。**2026-08-12以降はop505が唯一の主力チップ**）
  → 発端は「MC-505のEGは直感的でアナログを触る感覚だった。38x6のEGを4点T/Lにしては？」という設計相談。
    レート方式EGの全面置換・トレイト化は既存プリセット互換性等の理由で却下し、
    兄弟クレート（新チップ）として実装する方針で決着（当時はym38x6と並行する後継チップという位置づけ
    だったが、2026-08-12の方針転換によりym38x6が凍結され、op505が唯一の主力チップとなった）
  → `sound-core`に`TimeEg`（N点Time/Level方式・ループ範囲指定・多段リリース対応のEG、
    既存の`Eg`は無改造）を追加し、試聴実験（A〜E群）で触り心地を検証。
    「静止を挟んだ2値スイッチ」（d5）をカットオフに、さらに音量に適用した形（e3）が最も刺さるという
    結果を得た
  → `op505/core`（クレート名op505-core）を新設し、EG系統（オペレーターEG・Pitch/Cutoff/Gain
    ファンクションジェネレーター）を全面的に`TimeEg`化。EG非依存の共通部分（アルゴリズム結線・波形・
    チップ内LFO・質感LFO）は`sound-fm`をそのまま共有する
  → e3知見をそのまま実装した組み込みデモパッチ「Gain Switch (e3)」を含む3種のデモパッチを追加し、
    gesture-appのエンジン切替UIから選択・試聴できるようにした（2026-08-10、gesture-appでの
    実聴取確認まで完了）。**当初`op505/core/src/demo.rs`に実装していたが、op505デフォーク後に廃止済み**
    （デモパッチ相当のヘルパーは`gesture-app/src-tauri/src/engines.rs`のテストコードに残っていたが、
    2026-08-20のym38x6削除に伴うgesture-app単一エンジン化でこのファイル自体も削除。
    `op505/core/src/`の実体は`lib.rs`/`operator.rs`/`preset.rs`/`eg_convert.rs`の4ファイル）
  → `op505/ui`（クレート名op505-ui）を新設し、panel.xmlからのコード生成方式でエディタパネルを実装。
    `Op505Patch`はEGだけで203値（TimeEg 7本×29値、全269値中の大半）を占めるため、TimeEgエディタ
    （ノブ列のKNOBSモード／折れ線ドラッグ編集のGRAPHモードをタブ切替するハイブリッド方式）を
    `ui-core`に実装、gesture-app（editor-wasm）へ配線済み
  → **op505デフォーク完了**（2026-08-11、feature/op505-defork、developへ--no-ffマージ・push済み
    `64a1de2`）: `op505-core`の`ym38x6-core`依存を`[dev-dependencies]`のみへ降格（旧`adapter.rs`は廃止）、
    EG変換ロジックをym38x6非依存の`eg_convert.rs`へ移設、`.op505`バンクローダーを新設し
    gesture-appのBank/ProgramをAdapterその場変換から`.op505`直接解決へ切替え。
    opz2op505/mucom2op505/psr2op505/opm2op505/vgm2op505の5変換ツール（`op505/tools/`）を
    ym38x6依存ゼロで複製・再構築し、専用共有クレート`op505-tools`も新設した
    （**`.op505`ファイルI/Oはこの時点で完了済み**）
  → **gesture-app既定エンジンのOP505化完了**（2026-08-11、feature/gesture-app-op505-default、
    developへ--no-ffマージ・push済み`248cb06`）: 既定音源エンジンをOP505にし、PRESETSサイドバーも
    OP505対応済み
  → **op505-uiのXML DSL移行完了**（2026-08-10、`152c52e`）: panel.xmlベースのコード生成方式へ
    Step1〜6全て移行済み
  → **共有クレートのsound/uiグループ再編完了**（2026-08-09、`3a95c79`）: `sound-fm`/`ui-core`/
    `ui-layout`/`ui-codegen`を製品非依存の共有クレートとして整理
  → **op505-vstフェーズ1完了**（2026-08-12、`op505/vst`新設）: 詳細はフェーズ8参照
  → **op505-vstフェーズ2完了**（2026-08-12、feature/op505-vst-phase2）: MIDI表現系。詳細はフェーズ8参照
  → **フェーズ5.5完了**（2026-08-14、smf2wav/wavetest4x6/opzref4x6/patchlabのop505移行、
    op505-midi共有クレート化含む）。残タスクはフェーズ6（GM2テンプレートのop505向け再設計）
```
