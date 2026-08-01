// ---------------------------------------------------------------------------
// チップ内LFO（spec.md「チップ内LFO」セクション参照）
//
// プリセット・NRPNで設定する「音作り」用のLFO。波形は三角波固定。
// PMS/AMSはチャンネルごとの変調感度、PMD/AMDはLFOそのものの深さで、
// 両者の積が実際の変調量になる。AMはオペレーターごとのAME（OperatorParams::am_enable）
// でON/OFFする。演奏系モジュレーション（FG/質感LFO）とは完全に独立した別系統
// （FMチップ内蔵・パッチ内で完結・VCO差し替えで消えるレイヤー、旧称「音色LFO」）。
//
// 数式はすべて初期案（暫定）。CLAUDE.mdのテスト方針に従い、
// 実装後に音を聴いて係数を調整する。
// ---------------------------------------------------------------------------

/// チップ内LFOの周波数(0〜255)→Hz。OPN系LFOの周波数レンジ（約3〜80Hz）を指数マッピング（暫定）。
///
/// 以前は`ChipLfo::tick()`から毎サンプル`powf()`を呼んでいた。`tl_to_gain`等と同じ
/// 256要素テーブルパターンで初回アクセス時に1回だけ構築し（`OnceLock`、全チャンネル共有）、
/// 以降は配列参照のみで済ませる。数式は変更していないため出力は従来とビット単位で同一。
pub fn chip_lfo_freq_to_hz(freq: u8) -> f32 {
    static TABLE: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        const F_MIN: f32 = 3.0;
        const F_MAX: f32 = 80.0;
        let mut table = [0.0f32; 256];
        for (freq, slot) in table.iter_mut().enumerate() {
            *slot = F_MIN * (F_MAX / F_MIN).powf(freq as f32 / 255.0);
        }
        table
    });
    table[freq as usize]
}

/// PMS(0〜255)→ピッチ変調の最大幅（セント）。
/// OPM PMS(3bit、0=オフ・1〜7は+/-5〜+/-700セント、約7.13oct)を踏まえ、pms=0は
/// 実機PMS=0と同じ「ピッチ変調なし」の特殊値。pms=1〜255は実機PMS=1(+/-5セント)〜
/// PMS=7(+/-700セント)の理論値を両端アンカーとした指数カーブにマッピングする。
pub fn pms_to_cents_range(pms: u8) -> f32 {
    // pms=0（オフ特殊値）はテーブル構築時に0.0として焼き込む。数式は不変
    // （`chip_lfo_freq_to_hz`と同じOnceLockテーブル化、毎サンプルpowf()の排除）。
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
    // （`chip_lfo_freq_to_hz`と同じOnceLockテーブル化、毎サンプルpowf()×2の排除）。
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

/// チップ内LFO本体：三角波固定（spec.md準拠）+ Delay。
pub struct ChipLfo {
    phase: f32,
    elapsed: f32,
}

impl ChipLfo {
    pub fn new() -> Self {
        Self { phase: 0.0, elapsed: 0.0 }
    }

    /// キーオン時に呼び出し、位相とディレイ経過時間をリセットする。
    pub fn note_on(&mut self) {
        self.phase = 0.0;
        self.elapsed = 0.0;
    }

    /// 戻り値: -1.0〜1.0の三角波。Delay中は0.0。
    pub fn tick(&mut self, sample_rate: f32, freq: u8, delay: u8) -> f32 {
        self.elapsed += 1.0 / sample_rate;
        // sound_core::lfo::delay_to_secondsと同型（0〜10秒、線形）。
        let delay_seconds = delay as f32 / 255.0 * 10.0;
        if self.elapsed < delay_seconds {
            return 0.0;
        }

        let hz = chip_lfo_freq_to_hz(freq);
        self.phase = (self.phase + hz / sample_rate).fract();
        if self.phase < 0.5 {
            4.0 * self.phase - 1.0
        } else {
            3.0 - 4.0 * self.phase
        }
    }
}

impl Default for ChipLfo {
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
    fn chip_lfo_freq_to_hz_bounds() {
        assert!((chip_lfo_freq_to_hz(0) - 3.0).abs() < 1e-3);
        assert!((chip_lfo_freq_to_hz(255) - 80.0).abs() < 1e-2);
        assert!(chip_lfo_freq_to_hz(255) > chip_lfo_freq_to_hz(0));
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
        let mut lfo = ChipLfo::new();
        // delay=255 → 10秒。1秒分ティックしても出力0のはず
        for _ in 0..44100 {
            assert_eq!(lfo.tick(sr, 128, 255), 0.0);
        }
    }

    #[test]
    fn triangle_wave_is_periodic_and_bounded() {
        let sr = 44100.0;
        let mut lfo = ChipLfo::new();
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for _ in 0..(sr as usize) {
            let v = lfo.tick(sr, 255, 0); // freq=255 → 約80Hz, delay=0
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
        let mut lfo = ChipLfo::new();
        for _ in 0..1000 {
            lfo.tick(sr, 200, 0);
        }
        lfo.note_on();
        // リセット直後はphase≈0付近 → 三角波の谷(-1.0)からスタート
        let v = lfo.tick(sr, 200, 0);
        assert!(v < -0.9, "expected near -1.0 right after note_on, got {v}");
    }
}
