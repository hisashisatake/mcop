//! TX81Z sysex (.syx) バイナリ解析 → OpzVoice 構造体。
//!
//! 対応フォーマット:
//! - VCED: 単音 (F0 43 0n 03 00 5D [93 bytes] cksum F7)
//! - VMEM: 32音色バンク (F0 43 0n 04 20 00 [4096 bytes] cksum F7)
//!         各音色128バイト = VCED相当93バイト + ACED相当34バイト + パディング1バイト
//!
//! バイトオフセットは TX81Z MIDI Data Format に基づく。
//! 実ファイルで検証済み（VCED: 12バイト/OP × 4 OPs = 48 + グローバル45バイト = 93バイト）。

// ---------------------------------------------------------------------------
// VCED バイトレイアウト定数
// ---------------------------------------------------------------------------

const OP_STRIDE: usize = 12; // VCEDの1OP当たりバイト数（非パック形式）

// OP内フィールドオフセット（OP4から逆順に格納: OP4@0, OP3@12, OP2@24, OP1@36）
const F_AR: usize = 0;
const F_D1R: usize = 1;
const F_D2R: usize = 2;
const F_RR: usize = 3;
const F_D1L: usize = 4;
const F_OUT: usize = 5; // Output Level (0-99, 99=最大)
const F_RS: usize = 6;  // Rate Scale (0-3)
const F_EGT: usize = 7; // EG Type (0=sustain/D2R使用, 1=decay/D2R強制)
const F_AME: usize = 8; // AMS Enable (0-1)
const F_KVS: usize = 9; // Key Velocity Sensitivity (0-7)
const F_FREQ: usize = 10; // Frequency Coarse (0-63)
const F_DET: usize = 11; // Detune (0-6, 3=中心=無デチューン)

// グローバルセクション (バイト48〜92)
const G_BASE: usize = 4 * OP_STRIDE; // = 48
const G_ALG: usize = G_BASE + 0;
const G_FB: usize = G_BASE + 1;
const G_LFO_SPD: usize = G_BASE + 2;
const G_LFO_DLY: usize = G_BASE + 3;
const G_PMD: usize = G_BASE + 4;
const G_AMD: usize = G_BASE + 5;
const G_LFO_SYNC: usize = G_BASE + 6;
const G_LFO_WF: usize = G_BASE + 7;
const G_PMS: usize = G_BASE + 8;
const G_AMS: usize = G_BASE + 9;
// bytes 58-82: TRANSPOSE, POLY, PORTA, PB_RANGE, MW/FC割り当て, PITCH EG等 (変換不使用)
const G_NAME: usize = 83; // VOICE NAME: 10バイトASCII (83-92)

const VCED_DATA_LEN: usize = 93;

// ACED（TX81Z拡張）埋め込みオフセット（VMEM内、VCEDの直後）
const ACED_BASE: usize = VCED_DATA_LEN; // = 93
const ACED_OP_STRIDE: usize = 5;
const A_OW: usize = 0; // Operator Waveform (0-7)
// A_FIXRNG=1, A_FIXFRQ=2, A_EGS=3 (固定周波数/EGシフト、変換では無視)

const VMEM_VOICE_LEN: usize = 128;
const VMEM_VOICE_COUNT: usize = 32;

// ---------------------------------------------------------------------------
// 公開型
// ---------------------------------------------------------------------------

/// TX81Z オペレーター1個分のデータ。
#[derive(Clone, Debug, Default)]
pub struct OpzOpData {
    pub ar: u8,    // 0-31
    pub d1r: u8,   // 0-31
    pub d2r: u8,   // 0-31 (EGT=1の場合は変換側で上書き)
    pub rr: u8,    // 0-15
    pub d1l: u8,   // 0-15
    pub out: u8,   // Output Level 0-99 (99=最大)
    pub rs: u8,    // Rate Scale 0-3
    pub egt: u8,   // EG Type: 0=sustain(D2R有効), 1=decay(D2R無視して即リリース)
    pub ame: bool, // AMS Enable
    pub kvs: u8,   // Key Velocity Sensitivity 0-7
    pub freq: u8,  // Frequency Coarse 0-63
    pub det: u8,   // Detune 0-6 (3=中心)
    pub ow: u8,    // Operator Waveform 0-7 (ACEDより取得、ACED無し=0)
}

/// TX81Z 1ボイス分のデータ。
/// `ops` は VCED格納順（OP4=ops[0], OP3=ops[1], OP2=ops[2], OP1=ops[3]）。
/// 38x6 operators[] への写像は conv.rs で逆順変換する。
#[derive(Clone, Debug)]
pub struct OpzVoice {
    pub number: u32,
    pub name: String,
    pub algorithm: u8, // 0-7 (38x6と同一番号体系)
    pub feedback: u8,  // 0-7
    pub lfo_spd: u8,   // 0-99
    pub lfo_dly: u8,   // 0-99
    pub pmd: u8,       // 0-99
    pub amd: u8,       // 0-99
    pub lfo_sync: bool,
    pub lfo_wf: u8,    // 0-3
    pub pms: u8,       // 0-7
    pub ams: u8,       // 0-3
    pub ops: [OpzOpData; 4], // [OP4, OP3, OP2, OP1]
}

impl Default for OpzVoice {
    fn default() -> Self {
        Self {
            number: 0, name: String::new(),
            algorithm: 0, feedback: 0,
            lfo_spd: 0, lfo_dly: 0, pmd: 0, amd: 0,
            lfo_sync: false, lfo_wf: 0, pms: 0, ams: 0,
            ops: Default::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// sysex フォーマット検出
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub enum SyxFormat {
    Vced,      // 単音
    VmemBank,  // 32音色バンク
    AcedSingle, // 単音ACED（単独ファイル、VMEM内には含まれない）
    Unknown,
}

fn detect_format(data: &[u8]) -> SyxFormat {
    if data.len() < 6 || data[0] != 0xF0 || data[1] != 0x43 {
        return SyxFormat::Unknown;
    }
    // data[2] = 0x0n (デバイスID下位4bit可変), data[3]=フォーマット種別
    match (data[3], data[4], data[5]) {
        (0x03, 0x00, 0x5D) => SyxFormat::Vced,         // single voice (5D=93)
        (0x04, 0x20, 0x00) => SyxFormat::VmemBank,     // 32-voice bank (2000h=4096)
        (0x04, 0x00, 0x22) => SyxFormat::AcedSingle,   // ACED single (22h=34)
        _ => SyxFormat::Unknown,
    }
}

/// ファイル全体から F0...F7 のsysexメッセージを切り出す。
pub fn split_sysex_messages(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if data[i] == 0xF0 {
            if let Some(end) = data[i..].iter().position(|&b| b == 0xF7) {
                out.push(&data[i..=i + end]);
                i += end + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// VCED パース
// ---------------------------------------------------------------------------

fn get(data: &[u8], offset: usize) -> u8 {
    data.get(offset).copied().unwrap_or(0)
}

fn parse_op(data: &[u8], base: usize) -> OpzOpData {
    OpzOpData {
        ar:   get(data, base + F_AR),
        d1r:  get(data, base + F_D1R),
        d2r:  get(data, base + F_D2R),
        rr:   get(data, base + F_RR),
        d1l:  get(data, base + F_D1L),
        out:  get(data, base + F_OUT),
        rs:   get(data, base + F_RS),
        egt:  get(data, base + F_EGT),
        ame:  get(data, base + F_AME) != 0,
        kvs:  get(data, base + F_KVS),
        freq: get(data, base + F_FREQ),
        det:  get(data, base + F_DET),
        ow:   0, // ACED から後で上書き
    }
}

fn parse_name(data: &[u8], base: usize) -> String {
    let bytes: Vec<u8> = (0..10)
        .map(|i| get(data, base + i))
        .take_while(|&b| b >= 0x20 && b < 0x7F)
        .collect();
    String::from_utf8_lossy(&bytes).trim().to_string()
}

fn parse_vced_data(data: &[u8], number: u32) -> Option<OpzVoice> {
    if data.len() < VCED_DATA_LEN { return None; }
    let mut v = OpzVoice {
        number,
        name: parse_name(data, G_NAME),
        algorithm: get(data, G_ALG).min(7),
        feedback:  get(data, G_FB).min(7),
        lfo_spd:   get(data, G_LFO_SPD),
        lfo_dly:   get(data, G_LFO_DLY),
        pmd:       get(data, G_PMD),
        amd:       get(data, G_AMD),
        lfo_sync:  get(data, G_LFO_SYNC) != 0,
        lfo_wf:    get(data, G_LFO_WF).min(3),
        pms:       get(data, G_PMS).min(7),
        ams:       get(data, G_AMS).min(3),
        ops: [
            parse_op(data, 0),          // OP4
            parse_op(data, OP_STRIDE),  // OP3
            parse_op(data, 2 * OP_STRIDE), // OP2
            parse_op(data, 3 * OP_STRIDE), // OP1
        ],
        ..OpzVoice::default()
    };
    // ACED が VMEM 内に埋め込まれている場合（バイト93〜）
    if data.len() >= ACED_BASE + 4 * ACED_OP_STRIDE {
        for i in 0..4 {
            let aced_off = ACED_BASE + i * ACED_OP_STRIDE;
            v.ops[i].ow = get(data, aced_off + A_OW).min(7);
        }
    }
    // 名前が空の場合はプレースホルダー
    if v.name.is_empty() {
        v.name = format!("voice{number}");
    }
    Some(v)
}

// ---------------------------------------------------------------------------
// 公開 API
// ---------------------------------------------------------------------------

/// sysex バイナリから全ボイスを解析する。
/// 単音VCED・32音色VMEMを自動判定する。
pub fn parse_syx(data: &[u8]) -> Result<Vec<OpzVoice>, String> {
    let messages = split_sysex_messages(data);
    if messages.is_empty() {
        return Err("有効な sysex メッセージが見つかりませんでした".to_string());
    }

    // 最初に VMEM or VCED を探す
    let mut voices = Vec::new();
    let mut aced_overlay: Option<Vec<u8>> = None;

    for msg in &messages {
        match detect_format(msg) {
            SyxFormat::VmemBank => {
                // データ部: バイト6〜(末尾-2)
                let payload = &msg[6..msg.len().saturating_sub(2)];
                if payload.len() < VMEM_VOICE_COUNT * VMEM_VOICE_LEN {
                    return Err(format!(
                        "VMEM データが短すぎます: {} バイト（期待: {}）",
                        payload.len(), VMEM_VOICE_COUNT * VMEM_VOICE_LEN
                    ));
                }
                for i in 0..VMEM_VOICE_COUNT {
                    let chunk = &payload[i * VMEM_VOICE_LEN..(i + 1) * VMEM_VOICE_LEN];
                    if let Some(v) = parse_vced_data(chunk, i as u32) {
                        voices.push(v);
                    }
                }
            }
            SyxFormat::Vced => {
                let payload = &msg[6..msg.len().saturating_sub(2)];
                if let Some(v) = parse_vced_data(payload, voices.len() as u32) {
                    voices.push(v);
                }
            }
            SyxFormat::AcedSingle => {
                // 単独 ACED: VMEM の OW を上書き（後続で適用）
                aced_overlay = Some(msg[6..msg.len().saturating_sub(2)].to_vec());
            }
            SyxFormat::Unknown => {
                eprintln!("warning: 不明な sysex フォーマットをスキップ: {:02X?}", &msg[..msg.len().min(8)]);
            }
        }
    }

    // 単独ACED が後続している場合、最後のボイスに適用
    if let Some(aced) = aced_overlay {
        if let Some(v) = voices.last_mut() {
            for i in 0..4 {
                let off = i * ACED_OP_STRIDE;
                if off + A_OW < aced.len() {
                    v.ops[i].ow = aced[off + A_OW].min(7);
                }
            }
        }
    }

    if voices.is_empty() {
        return Err("パース可能なボイスが見つかりませんでした".to_string());
    }
    Ok(voices)
}

/// ファイル名に使えない文字を `_` に置換する（opm2x6 と同実装）。
pub fn sanitize_filename(name: &str) -> String {
    let s: String = name.chars().map(|c| {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }
    }).collect();
    s.trim_matches('_').to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_vced_header() {
        let hdr = [0xF0, 0x43, 0x00, 0x03, 0x00, 0x5D];
        assert_eq!(detect_format(&hdr), SyxFormat::Vced);
    }

    #[test]
    fn detect_vmem_header() {
        let hdr = [0xF0, 0x43, 0x00, 0x04, 0x20, 0x00];
        assert_eq!(detect_format(&hdr), SyxFormat::VmemBank);
    }

    #[test]
    fn detect_unknown() {
        let hdr = [0xF0, 0x43, 0x00, 0x09, 0x00, 0x00];
        assert_eq!(detect_format(&hdr), SyxFormat::Unknown);
    }

    #[test]
    fn split_two_messages() {
        let data = [0xF0, 0x01, 0x02, 0xF7, 0xF0, 0x03, 0xF7];
        let msgs = split_sysex_messages(&data);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0], &[0xF0, 0x01, 0x02, 0xF7]);
        assert_eq!(msgs[1], &[0xF0, 0x03, 0xF7]);
    }

    #[test]
    fn sanitize_names() {
        assert_eq!(sanitize_filename("E.PIANO"), "E_PIANO");
        assert_eq!(sanitize_filename("BASS 1"), "BASS_1");
        assert_eq!(sanitize_filename("lead-synth"), "lead-synth");
    }

    #[test]
    fn vced_data_short_returns_none() {
        let data = [0u8; 10];
        assert!(parse_vced_data(&data, 0).is_none());
    }

    #[test]
    fn vced_default_voice_parses() {
        let mut data = vec![0u8; VCED_DATA_LEN];
        // ALG=4 at G_ALG
        data[G_ALG] = 4;
        // NAME = "TEST      "
        for (i, &b) in b"TEST      ".iter().enumerate() {
            data[G_NAME + i] = b;
        }
        let v = parse_vced_data(&data, 0).unwrap();
        assert_eq!(v.algorithm, 4);
        assert_eq!(v.name, "TEST");
        assert_eq!(v.ops[0].ow, 0); // ACED未設定 → サイン波
    }

    #[test]
    fn vmem_aced_ow_applied() {
        let mut data = vec![0u8; VMEM_VOICE_LEN];
        // ALG=0, NAME="BRASS     "
        data[G_ALG] = 0;
        for (i, &b) in b"BRASS     ".iter().enumerate() {
            data[G_NAME + i] = b;
        }
        // ACED OP4 OW = 3 (at byte 93)
        data[ACED_BASE + 0 * ACED_OP_STRIDE + A_OW] = 3;
        let v = parse_vced_data(&data, 0).unwrap();
        assert_eq!(v.ops[0].ow, 3); // OP4 OW = 3
    }
}
