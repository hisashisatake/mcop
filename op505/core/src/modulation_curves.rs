// ---------------------------------------------------------------------------
// モジュレーションカーブ：レガシー機種のLFO関連レジスタ値(PMS/AMS/FRQ、0〜255)を
// 実際の物理量（セント幅・振幅深度・Hz）へ写す変換テーブル集。
//
// 由来は「チップ内LFO」（旧称「音色LFO」、かつてPMS/PMD/AMS/AMD/FRQ/DLYとして
// op505-coreのChannelが持っていたLFO本体、三角波固定）。2026-08-20にop505エンジンから
// 完全退役し、ピッチ経路はPitch FGへ、AM経路はGain FGのOP単位配線（gain_fg_to_operators）へ
// 厳密変換された（`Op505ChannelParams`から該当6フィールドも削除済み、memory
// `project_chip_lfo_retirement_investigation.md`参照）。エンジンとしては消えたが、
// ここにある変換テーブル自体は今も2つの用途で現役：
//
// 1. opz2op505/opm2op505等の変換ツールが、実機のPMS/AMSレジスタ値→セント幅/振幅深度への
//    写像として使う（本クレートの`chip_lfo_pitch_to_pitch_fg`/`chip_lfo_am_to_gain_fg`
//    ラッパー経由の間接利用。変換ツール自身は本モジュールへ直接依存しない）。
// 2. `op505-midi`のPitch/Gain/Cutoff FGフォールバック（stage_count=0のプリセットへ
//    CC/NRPN経由で標準形状を書き込む機構、spec-sound.md「演奏用FGフォールバック」節）が、
//    CC76由来のレート値をHzへ変換する入力として`lfo_rate_to_hz`を使う（LFOという概念こそ
//    共通するが、対象はMIDI標準コントローラーでありチップ内LFOレジスタとは無関係）。
//
// かつては`sound-fm`（ym38x6/op505間で共有する製品非依存レイヤー）に`chip_lfo`という
// 名前で置かれていたが、ym38x6削除後は実際の直接利用者がop505グループ（本クレートと
// `op505-midi`）に閉じたため、2026-09-01にこちらへ移設した。移設に合わせて
// 「chip_lfo」という退役済みエンジンの名残りだった名前も、実態（値変換カーブの集まり）に
// 合わせて`modulation_curves`／`lfo_rate_to_hz`／`ReferenceLfo`へ改めた
// （`pms_to_cents_range`/`ams_to_depth`は実機レジスタ名PMS/AMSそのものなので据え置き）。
// `ReferenceLfo`（旧`ChipLfo`、三角波オシレーター本体）は`chip_lfo_am_to_gain_fg`テスト
// （`chip_lfo_am_to_gain_fg_matches_chip_lfo_amplitude_extremes`）が、Gain FGへの変換が
// 実機挙動と一致することを検証するオラクルとしてのみ使う（本番コードからの参照はゼロ）。
// ---------------------------------------------------------------------------

/// LFOのレート値(0〜255)→Hz。OPN系LFOの周波数レンジ（約3〜80Hz）を指数マッピング（暫定）。
///
/// 以前は`ReferenceLfo::tick()`から毎サンプル`powf()`を呼んでいた。`tl_to_gain`等と同じ
/// 256要素テーブルパターンで初回アクセス時に1回だけ構築し（`OnceLock`、全チャンネル共有）、
/// 以降は配列参照のみで済ませる。数式は変更していないため出力は従来とビット単位で同一。
pub fn lfo_rate_to_hz(rate: u8) -> f32 {
    static TABLE: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        const F_MIN: f32 = 3.0;
        const F_MAX: f32 = 80.0;
        let mut table = [0.0f32; 256];
        for (rate, slot) in table.iter_mut().enumerate() {
            *slot = F_MIN * (F_MAX / F_MIN).powf(rate as f32 / 255.0);
        }
        table
    });
    table[rate as usize]
}

/// PMS(0〜255)→ピッチ変調の最大幅（セント）。
/// OPM PMS(3bit、0=オフ・1〜7は+/-5〜+/-700セント、約7.13oct)を踏まえ、pms=0は
/// 実機PMS=0と同じ「ピッチ変調なし」の特殊値。pms=1〜255は実機PMS=1(+/-5セント)〜
/// PMS=7(+/-700セント)の理論値を両端アンカーとした指数カーブにマッピングする。
pub fn pms_to_cents_range(pms: u8) -> f32 {
    // pms=0（オフ特殊値）はテーブル構築時に0.0として焼き込む。数式は不変
    // （`lfo_rate_to_hz`と同じOnceLockテーブル化、毎サンプルpowf()の排除）。
    static TABLE: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        const MIN_CENTS: f32 = 5.0;
        const MAX_CENTS: f32 = 700.0;
        let mut table = [0.0f32; 256];
        for (pms, slot) in table.iter_mut().enumerate().skip(1) {
            *slot = MIN_CENTS * (MAX_CENTS / MIN_CENTS).powf((pms as f32 - 1.0) / 254.0);
        }
        table
    });
    table[pms as usize]
}

/// AMS(0〜255)→振幅変調の最大深さ(0.0〜1.0)。
/// OPM AMS(2bit、0=オフ・1〜3は23.9dB〜95.6dB、1段ごとに2倍=2oct)を踏まえ、ams=0は
/// 実機AMS=0と同じ「振幅変調なし」の特殊値。ams=1〜255は実機AMS=1(23.9dB)〜
/// AMS=3(95.6dB)の理論値を両端アンカーとした指数カーブでdB値を求め、
/// depth = 1 - 10^(-dB/20) で線形振幅深度に変換する
/// (operator.rsのamp_factor = (1 - chip_lfo_amp_mod).clamp(0,1)と整合)。
pub fn ams_to_depth(ams: u8) -> f32 {
    // ams=0（オフ特殊値）はテーブル構築時に0.0として焼き込む。数式は不変
    // （`lfo_rate_to_hz`と同じOnceLockテーブル化、毎サンプルpowf()×2の排除）。
    static TABLE: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        const MIN_DB: f32 = 23.9;
        const MAX_DB: f32 = 95.6;
        let mut table = [0.0f32; 256];
        for (ams, slot) in table.iter_mut().enumerate().skip(1) {
            let db = MIN_DB * (MAX_DB / MIN_DB).powf((ams as f32 - 1.0) / 254.0);
            *slot = 1.0 - 10f32.powf(-db / 20.0);
        }
        table
    });
    table[ams as usize]
}

/// 旧チップ内LFO本体の参照実装：三角波固定（spec.md準拠）+ Delay。
/// op505エンジンからは退役済みで、現在は本クレートのテストオラクル専用（本ファイル冒頭コメント参照）。
pub struct ReferenceLfo {
    phase: f32,
    elapsed: f32,
}

impl ReferenceLfo {
    pub fn new() -> Self {
        Self { phase: 0.0, elapsed: 0.0 }
    }

    /// キーオン時に呼び出し、位相とディレイ経過時間をリセットする。
    pub fn note_on(&mut self) {
        self.phase = 0.0;
        self.elapsed = 0.0;
    }

    /// 戻り値: -1.0〜1.0の三角波。Delay中は0.0。
    pub fn tick(&mut self, sample_rate: f32, rate: u8, delay: u8) -> f32 {
        self.elapsed += 1.0 / sample_rate;
        // sound_core::lfo::delay_to_secondsと同型（0〜10秒、線形）。
        let delay_seconds = delay as f32 / 255.0 * 10.0;
        if self.elapsed < delay_seconds {
            return 0.0;
        }

        let hz = lfo_rate_to_hz(rate);
        self.phase = (self.phase + hz / sample_rate).fract();
        if self.phase < 0.5 {
            4.0 * self.phase - 1.0
        } else {
            3.0 - 4.0 * self.phase
        }
    }
}

impl Default for ReferenceLfo {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lfo_rate_to_hz_bounds() {
        assert!((lfo_rate_to_hz(0) - 3.0).abs() < 1e-3);
        assert!((lfo_rate_to_hz(255) - 80.0).abs() < 1e-2);
        assert!(lfo_rate_to_hz(255) > lfo_rate_to_hz(0));
    }

    #[test]
    fn pms_to_cents_range_bounds() {
        // pms=0はオフ（実機PMS=0=0cents）
        assert_eq!(pms_to_cents_range(0), 0.0);
        // pms=1は実機PMS=1(+/-5cents)、pms=255は実機PMS=7(+/-700cents)
        assert!((pms_to_cents_range(1) - 5.0).abs() < 1e-3);
        assert!((pms_to_cents_range(255) - 700.0).abs() < 1e-2);
        // 指数カーブ：pms=0(オフ)以外は全域で滑らかに増加する
        assert!(pms_to_cents_range(1) > 0.0);
        assert!(pms_to_cents_range(64) < pms_to_cents_range(128));
        assert!(pms_to_cents_range(128) < pms_to_cents_range(192));
        assert!(pms_to_cents_range(192) < pms_to_cents_range(255));
    }

    #[test]
    fn ams_to_depth_bounds() {
        // ams=0はオフ（実機AMS=0=0dB）
        assert_eq!(ams_to_depth(0), 0.0);
        // ams=1は実機AMS=1(23.9dB)相当の深度、ams=255は実機AMS=3(95.6dB)相当でほぼ1.0
        assert!(ams_to_depth(1) > 0.9 && ams_to_depth(1) < 1.0);
        assert!((ams_to_depth(255) - 1.0).abs() < 1e-3);
        // 指数カーブ：ams=0(オフ)以外は全域で滑らかに増加する
        assert!(ams_to_depth(1) > 0.0);
        assert!(ams_to_depth(64) < ams_to_depth(128));
        assert!(ams_to_depth(128) < ams_to_depth(192));
        assert!(ams_to_depth(192) < ams_to_depth(255));
        // depthは常に0.0〜1.0の範囲内
        for ams in [0u8, 1, 64, 128, 192, 255] {
            let d = ams_to_depth(ams);
            assert!((0.0..=1.0).contains(&d), "ams={ams} depth={d}");
        }
    }

    #[test]
    fn delay_holds_output_at_zero() {
        let sr = 44100.0;
        let mut lfo = ReferenceLfo::new();
        // delay=255 → 10秒。1秒分ティックしても出力0のはず
        for _ in 0..44100 {
            assert_eq!(lfo.tick(sr, 128, 255), 0.0);
        }
    }

    #[test]
    fn triangle_wave_is_periodic_and_bounded() {
        let sr = 44100.0;
        let mut lfo = ReferenceLfo::new();
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for _ in 0..(sr as usize) {
            let v = lfo.tick(sr, 255, 0); // rate=255 → 約80Hz, delay=0
            assert!((-1.0..=1.0).contains(&v), "out of range: {v}");
            min = min.min(v);
            max = max.max(v);
        }
        assert!(min < -0.9, "min={min}");
        assert!(max > 0.9, "max={max}");
    }

    #[test]
    fn note_on_resets_phase_and_elapsed() {
        let sr = 44100.0;
        let mut lfo = ReferenceLfo::new();
        for _ in 0..1000 {
            lfo.tick(sr, 200, 0);
        }
        lfo.note_on();
        // リセット直後はphase≈0付近 → 三角波の谷(-1.0)からスタート
        let v = lfo.tick(sr, 200, 0);
        assert!(v < -0.9, "expected near -1.0 right after note_on, got {v}");
    }
}
