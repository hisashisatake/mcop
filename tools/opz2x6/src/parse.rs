//! TX81Z sysex (.syx) バイナリ解析 → OpzVoice 構造体。
//!
//! 対応フォーマット:
//! - VCED: 単音 (F0 43 0n 03 00 5D [93 bytes] cksum F7)
//! - VMEM: 32音色バンク (F0 43 0n 04 20 00 [4096 bytes] cksum F7)
//!         128バイト/音色: OP4(0-9)/OP2(10-19)/OP3(20-29)/OP1(30-39) + グローバル(40-56)
//!         + 音色名(57-66) + PEG(67-72) + ACED拡張(73-83)
//!
//! バイトオフセットは TX81Z MIDI Data Format (Voice Bulk Data Format) に基づく。
//! 実ファイルで検証済み（Yamaha Factory Banks.zip / Bank A、GrandPiano 等）。

// ---------------------------------------------------------------------------
// VCED バイトレイアウト定数（単音ダンプ用）
// ---------------------------------------------------------------------------

const OP_STRIDE: usize = 12; // VCEDの1OP当たりバイト数（OP4@0, OP3@12, OP2@24, OP1@36）

const F_AR: usize = 0;
const F_D1R: usize = 1;
const F_D2R: usize = 2;
const F_RR: usize = 3;
const F_D1L: usize = 4;
const F_OUT: usize = 5;  // Output Level (0-99)
const F_RS: usize = 6;   // Rate Scale (0-3)
const F_EGT: usize = 7;  // EG Type (0=sustain, 1=decay)
const F_AME: usize = 8;  // AMS Enable (0-1)
const F_KVS: usize = 9;  // Key Velocity Sensitivity (0-7)
const F_FREQ: usize = 10; // Frequency Coarse (0-63)
const F_DET: usize = 11;  // Detune (0-6, 3=中心)

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
// bytes 58-82: TRANSPOSE 等パフォーマンス系（変換不使用）
const G_NAME: usize = 83; // VOICE NAME: 10バイトASCII

const VCED_DATA_LEN: usize = 93;

// ACED 単音ダンプ埋め込み（VCED直後、VCEDと合わせて単音+ACEDファイル用）
const ACED_BASE: usize = VCED_DATA_LEN; // = 93
const ACED_OP_STRIDE: usize = 5;
const A_OW: usize = 0; // Operator Waveform (0-7)

// ---------------------------------------------------------------------------
// VMEM バイトレイアウト定数（32音色バンクダンプ用）
// ---------------------------------------------------------------------------
// TX81Z MIDI Data Format "Voice Bulk Data Format (VMEM)"
// OP順序: OP4(addr 0-9), OP2(addr 10-19), OP3(addr 20-29), OP1(addr 30-39)

const VMEM_OP4_BASE: usize = 0;
const VMEM_OP2_BASE: usize = 10;
const VMEM_OP3_BASE: usize = 20;
const VMEM_OP1_BASE: usize = 30;

// VMEM OP内フィールドオフセット（10バイト/OP）
const VM_AR: usize = 0;   // 0-31 (bits 4-0)
const VM_D1R: usize = 1;  // 0-31 (bits 4-0)
const VM_D2R: usize = 2;  // 0-31 (bits 4-0)
const VM_RR: usize = 3;   // 0-15 (bits 3-0)
const VM_D1L: usize = 4;  // 0-15 (bits 3-0)
// addr 5: LS (Level Sensitivity 0-99)、変換未使用
// addr 6: b6=AME, b5-3=EBS, b2-0=KVS
const VM_AME_KVS: usize = 6;
const VM_OUT: usize = 7;  // Output Level 0-99 (bits 6-0)
const VM_FREQ: usize = 8; // Frequency 0-63 (bits 5-0)
// addr 9: b4-3=RS(0-3), b2-0=DBT/Detune(0-6)
const VM_RS_DBT: usize = 9;

// VMEM グローバルセクション先頭 (addr 40)
const VMEM_GLOBAL: usize = 40;
// addr 40: b6=SY(sync), b5-3=FBL(feedback 0-7), b2-0=ALG(0-7)
// addr 41: LFS (LFO Speed 0-99)
// addr 42: LFD (LFO Delay 0-99)
// addr 43: PMD (Pitch Mod Depth 0-99)
// addr 44: AMD (Amp Mod Depth 0-99)
// addr 45: b6-4=PMS(0-7), b3-2=AMS(0-3), b1-0=LFW(LFO Wave 0-3)
// addr 46-56: TRPS, PBR, CH/MO/SU/PO/PM, PORT, FC-VOL, MW/BC 等（変換未使用）

const VMEM_NAME: usize = 57; // VOICE NAME: 10バイトASCII (addr 57-66)

// ACED 拡張領域 (addr 73-83)
// addr 73: OP4 EGSFT/FIX/FIXRG、addr 74: OP4 OPW(bits6-4)/FINE
// addr 75: OP2 EGSFT/FIX/FIXRG、addr 76: OP2 OPW(bits6-4)/FINE
// addr 77: OP3 EGSFT/FIX/FIXRG、addr 78: OP3 OPW(bits6-4)/FINE
// addr 79: OP1 EGSFT/FIX/FIXRG、addr 80: OP1 OPW(bits6-4)/FINE
const VMEM_ACED_OP4_OPW: usize = 74;
const VMEM_ACED_OP2_OPW: usize = 76;
const VMEM_ACED_OP3_OPW: usize = 78;
const VMEM_ACED_OP1_OPW: usize = 80;

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
    pub d2r: u8,   // 0-31
    pub rr: u8,    // 0-15
    pub d1l: u8,   // 0-15
    pub out: u8,   // Output Level 0-99 (99=最大)
    pub rs: u8,    // Rate Scale 0-3
    pub egt: u8,   // EG Type: 0=sustain(D2R有効), 1=decay(D2R無視して即リリース)
    pub ame: bool, // AMS Enable
    pub kvs: u8,   // Key Velocity Sensitivity 0-7
    pub freq: u8,  // Frequency Coarse 0-63
    pub det: u8,   // Detune 0-6 (3=中心)
    pub ow: u8,    // Operator Waveform 0-7 (ACEDより取得、未設定=0)
}

/// TX81Z 1ボイス分のデータ。
/// `ops` は [OP4, OP3, OP2, OP1] の順。
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
    Vced,
    VmemBank,
    AcedSingle,
    Unknown,
}

fn detect_format(data: &[u8]) -> SyxFormat {
    if data.len() < 6 || data[0] != 0xF0 || data[1] != 0x43 {
        return SyxFormat::Unknown;
    }
    match (data[3], data[4], data[5]) {
        (0x03, 0x00, 0x5D) => SyxFormat::Vced,
        (0x04, 0x20, 0x00) => SyxFormat::VmemBank,
        (0x04, 0x00, 0x22) => SyxFormat::AcedSingle,
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
// 共通ユーティリティ
// ---------------------------------------------------------------------------

fn get(data: &[u8], offset: usize) -> u8 {
    data.get(offset).copied().unwrap_or(0)
}

fn parse_name(data: &[u8], base: usize) -> String {
    let bytes: Vec<u8> = (0..10)
        .map(|i| get(data, base + i))
        .take_while(|&b| b >= 0x20 && b < 0x7F)
        .collect();
    String::from_utf8_lossy(&bytes).trim().to_string()
}

// ---------------------------------------------------------------------------
// VCED パース（単音ダンプ用）
// ---------------------------------------------------------------------------

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
        ow:   0,
    }
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
            parse_op(data, 0),
            parse_op(data, OP_STRIDE),
            parse_op(data, 2 * OP_STRIDE),
            parse_op(data, 3 * OP_STRIDE),
        ],
        ..OpzVoice::default()
    };
    // ACED が直後に続く場合（単音VCED+ACED合体ファイル）
    if data.len() >= ACED_BASE + 4 * ACED_OP_STRIDE {
        for i in 0..4 {
            v.ops[i].ow = get(data, ACED_BASE + i * ACED_OP_STRIDE + A_OW).min(7);
        }
    }
    if v.name.is_empty() {
        v.name = format!("voice{number}");
    }
    Some(v)
}

// ---------------------------------------------------------------------------
// VMEM パース（32音色バンクダンプ用）
// ---------------------------------------------------------------------------

fn parse_vmem_op(data: &[u8], base: usize) -> OpzOpData {
    let b6 = get(data, base + VM_AME_KVS);
    let b9 = get(data, base + VM_RS_DBT);
    OpzOpData {
        ar:   get(data, base + VM_AR)   & 0x1F,
        d1r:  get(data, base + VM_D1R)  & 0x1F,
        d2r:  get(data, base + VM_D2R)  & 0x1F,
        rr:   get(data, base + VM_RR)   & 0x0F,
        d1l:  get(data, base + VM_D1L)  & 0x0F,
        out:  get(data, base + VM_OUT)  & 0x7F, // 0-99
        rs:   (b9 >> 3) & 0x03,
        egt:  0, // VMEMにはEGTフィールドなし、デフォルト=sustain
        ame:  (b6 >> 6) & 1 != 0,
        kvs:  b6 & 0x07,
        freq: get(data, base + VM_FREQ) & 0x3F,
        det:  b9 & 0x07, // DBT(0-6): 3=中心、0=最大負デチューン
        ow:   0, // ACED領域から後で上書き
    }
}

fn parse_vmem_voice(data: &[u8], number: u32) -> Option<OpzVoice> {
    if data.len() < VMEM_VOICE_LEN { return None; }
    let g = VMEM_GLOBAL; // = 40
    let alg_fb_sy = get(data, g);       // addr40: b6=SY, b5-3=FBL, b2-0=ALG
    let pms_ams_lfw = get(data, g + 5); // addr45: b6-4=PMS, b3-2=AMS, b1-0=LFW

    let mut v = OpzVoice {
        number,
        name:     parse_name(data, VMEM_NAME),
        algorithm: alg_fb_sy & 0x07,
        feedback:  (alg_fb_sy >> 3) & 0x07,
        lfo_spd:  get(data, g + 1), // addr41: LFS
        lfo_dly:  get(data, g + 2), // addr42: LFD
        pmd:      get(data, g + 3), // addr43: PMD
        amd:      get(data, g + 4), // addr44: AMD
        lfo_sync: (alg_fb_sy >> 6) & 1 != 0,
        lfo_wf:   pms_ams_lfw & 0x03,
        pms:      (pms_ams_lfw >> 4) & 0x07,
        ams:      (pms_ams_lfw >> 2) & 0x03,
        ops: [
            parse_vmem_op(data, VMEM_OP4_BASE), // ops[0] = OP4
            parse_vmem_op(data, VMEM_OP3_BASE), // ops[1] = OP3
            parse_vmem_op(data, VMEM_OP2_BASE), // ops[2] = OP2
            parse_vmem_op(data, VMEM_OP1_BASE), // ops[3] = OP1
        ],
    };

    // ACED OPW（オペレーター波形）: bits[6:4] of each OPW byte
    // VMEM OP順(4/2/3/1) と ACED addr の対応:
    //   OP4→addr74, OP2→addr76, OP3→addr78, OP1→addr80
    if data.len() > VMEM_ACED_OP1_OPW {
        v.ops[0].ow = (get(data, VMEM_ACED_OP4_OPW) >> 4) & 0x07;
        v.ops[1].ow = (get(data, VMEM_ACED_OP3_OPW) >> 4) & 0x07;
        v.ops[2].ow = (get(data, VMEM_ACED_OP2_OPW) >> 4) & 0x07;
        v.ops[3].ow = (get(data, VMEM_ACED_OP1_OPW) >> 4) & 0x07;
    }

    if v.name.is_empty() {
        v.name = format!("voice{number}");
    }
    Some(v)
}

// ---------------------------------------------------------------------------
// 公開 API
// ---------------------------------------------------------------------------

/// sysex バイナリから全ボイスを解析する。
pub fn parse_syx(data: &[u8]) -> Result<Vec<OpzVoice>, String> {
    let messages = split_sysex_messages(data);
    if messages.is_empty() {
        return Err("有効な sysex メッセージが見つかりませんでした".to_string());
    }

    let mut voices = Vec::new();
    let mut aced_overlay: Option<Vec<u8>> = None;

    for msg in &messages {
        match detect_format(msg) {
            SyxFormat::VmemBank => {
                let payload = &msg[6..msg.len().saturating_sub(2)];
                if payload.len() < VMEM_VOICE_COUNT * VMEM_VOICE_LEN {
                    return Err(format!(
                        "VMEM データが短すぎます: {} バイト（期待: {}）",
                        payload.len(), VMEM_VOICE_COUNT * VMEM_VOICE_LEN
                    ));
                }
                for i in 0..VMEM_VOICE_COUNT {
                    let chunk = &payload[i * VMEM_VOICE_LEN..(i + 1) * VMEM_VOICE_LEN];
                    if let Some(v) = parse_vmem_voice(chunk, i as u32) {
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
                aced_overlay = Some(msg[6..msg.len().saturating_sub(2)].to_vec());
            }
            SyxFormat::Unknown => {
                eprintln!("warning: 不明な sysex フォーマットをスキップ: {:02X?}", &msg[..msg.len().min(8)]);
            }
        }
    }

    // 単独ACEDが後続している場合、最後のVCEDボイスに適用
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

/// ファイル名に使えない文字を `_` に置換する。
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
        data[G_ALG] = 4;
        for (i, &b) in b"TEST      ".iter().enumerate() {
            data[G_NAME + i] = b;
        }
        let v = parse_vced_data(&data, 0).unwrap();
        assert_eq!(v.algorithm, 4);
        assert_eq!(v.name, "TEST");
        assert_eq!(v.ops[0].ow, 0);
    }

    #[test]
    fn vced_aced_ow_applied() {
        let mut data = vec![0u8; VCED_DATA_LEN + 4 * ACED_OP_STRIDE];
        data[G_ALG] = 0;
        for (i, &b) in b"BRASS     ".iter().enumerate() {
            data[G_NAME + i] = b;
        }
        data[ACED_BASE + 0 * ACED_OP_STRIDE + A_OW] = 3;
        let v = parse_vced_data(&data, 0).unwrap();
        assert_eq!(v.ops[0].ow, 3);
    }

    // ---- VMEM テスト ----

    #[test]
    fn vmem_voice_short_returns_none() {
        let data = vec![0u8; 64];
        assert!(parse_vmem_voice(&data, 0).is_none());
    }

    #[test]
    fn vmem_voice_name_at_57() {
        let mut data = vec![0u8; VMEM_VOICE_LEN];
        for (i, &b) in b"GrandPiano".iter().enumerate() {
            data[VMEM_NAME + i] = b;
        }
        let v = parse_vmem_voice(&data, 0).unwrap();
        assert_eq!(v.name, "GrandPiano");
    }

    #[test]
    fn vmem_global_alg_fb_sy() {
        let mut data = vec![0u8; VMEM_VOICE_LEN];
        // addr40: SY=0, FB=7(=0b111), ALG=2(=0b010) → 0b0_111_010 = 0x3A
        data[VMEM_GLOBAL] = 0x3A;
        let v = parse_vmem_voice(&data, 0).unwrap();
        assert_eq!(v.algorithm, 2);
        assert_eq!(v.feedback, 7);
        assert!(!v.lfo_sync);
    }

    #[test]
    fn vmem_global_pms_ams_lfw() {
        let mut data = vec![0u8; VMEM_VOICE_LEN];
        // addr45: PMS=3(bits6-4), AMS=1(bits3-2), LFW=2(bits1-0) → 0b0_011_01_10 = 0x36
        data[VMEM_GLOBAL + 5] = 0x36;
        let v = parse_vmem_voice(&data, 0).unwrap();
        assert_eq!(v.pms, 3);
        assert_eq!(v.ams, 1);
        assert_eq!(v.lfo_wf, 2);
    }

    #[test]
    fn vmem_op4_fields() {
        let mut data = vec![0u8; VMEM_VOICE_LEN];
        // OP4 at addr 0-9
        data[0] = 29;  // AR=29
        data[1] = 4;   // D1R=4
        data[2] = 12;  // D2R=12
        data[3] = 6;   // RR=6
        data[4] = 0;   // D1L=0
        // addr5: LS=52 (unused in conversion)
        data[5] = 52;
        // addr6: AME=0, EBS=0, KVS=1 → 0b0_0_000_001 = 0x01
        data[6] = 0x01;
        data[7] = 77;  // OUT=77
        data[8] = 4;   // FREQ=4
        // addr9: RS=1(bits4-3), DBT=5(bits2-0) → 0b000_01_101 = 0x0D
        data[9] = 0x0D;
        let v = parse_vmem_voice(&data, 0).unwrap();
        let op4 = &v.ops[0]; // ops[0] = OP4
        assert_eq!(op4.ar, 29);
        assert_eq!(op4.d1r, 4);
        assert_eq!(op4.rr, 6);
        assert_eq!(op4.d1l, 0);
        assert_eq!(op4.out, 77);
        assert_eq!(op4.rs, 1);
        assert_eq!(op4.det, 5);
        assert_eq!(op4.kvs, 1);
        assert!(!op4.ame);
        assert_eq!(op4.egt, 0); // VMEMはEGT常に0
    }

    #[test]
    fn vmem_op_order_op3_at_20() {
        let mut data = vec![0u8; VMEM_VOICE_LEN];
        // OP3 at addr 20: AR=24
        data[20] = 24;
        let v = parse_vmem_voice(&data, 0).unwrap();
        assert_eq!(v.ops[1].ar, 24); // ops[1] = OP3
        assert_eq!(v.ops[0].ar, 0);  // ops[0] = OP4 (AR=0)
    }

    #[test]
    fn vmem_op_order_op2_at_10() {
        let mut data = vec![0u8; VMEM_VOICE_LEN];
        // OP2 at addr 10: AR=19
        data[10] = 19;
        let v = parse_vmem_voice(&data, 0).unwrap();
        assert_eq!(v.ops[2].ar, 19); // ops[2] = OP2
    }

    #[test]
    fn vmem_op_order_op1_at_30() {
        let mut data = vec![0u8; VMEM_VOICE_LEN];
        // OP1 at addr 30: AR=31, OUT=99
        data[30] = 31;
        data[37] = 99; // addr30+7 = OUT
        let v = parse_vmem_voice(&data, 0).unwrap();
        assert_eq!(v.ops[3].ar, 31); // ops[3] = OP1
        assert_eq!(v.ops[3].out, 99);
    }

    #[test]
    fn vmem_aced_opw_applied() {
        let mut data = vec![0u8; VMEM_VOICE_LEN];
        // OP4 OPW=1(half sine) at addr74 bits[6:4]: 0b0_001_0000 = 0x10
        data[VMEM_ACED_OP4_OPW] = 0x10;
        // OP3 OPW=2 at addr78 bits[6:4]: 0b0_010_0000 = 0x20
        data[VMEM_ACED_OP3_OPW] = 0x20;
        let v = parse_vmem_voice(&data, 0).unwrap();
        assert_eq!(v.ops[0].ow, 1); // ops[0]=OP4 OPW=1
        assert_eq!(v.ops[1].ow, 2); // ops[1]=OP3 OPW=2
        assert_eq!(v.ops[2].ow, 0); // ops[2]=OP2 OPW=0
        assert_eq!(v.ops[3].ow, 0); // ops[3]=OP1 OPW=0
    }

    #[test]
    fn vmem_grandpiano_voice1_spot_check() {
        // Yamaha Factory Bank A, voice 1 "GrandPiano" の実ファイル値検証
        // （tools/opz2x6/private/extracted/Yamaha Factory Bank A.syx から抽出）
        let syx = std::fs::read(
            "private/extracted/Yamaha Factory Banks.zip".replace("Banks.zip", "Bank A.syx")
        );
        // ファイルが存在しない環境ではスキップ
        if syx.is_err() { return; }
        let voices = super::parse_syx(&syx.unwrap()).unwrap();
        let v = &voices[0];
        assert_eq!(v.name, "GrandPiano");
        assert_eq!(v.algorithm, 2);
        assert_eq!(v.feedback, 7);
        assert_eq!(v.ops[0].ar, 29);   // OP4 AR
        assert_eq!(v.ops[0].out, 77);  // OP4 OUT
        assert_eq!(v.ops[3].out, 99);  // OP1 OUT（モジュレーター最大）
        assert_eq!(v.ops[1].ow, 1);    // OP3 OPW=1(half sine)
    }
}
