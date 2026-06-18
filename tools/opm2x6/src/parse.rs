/// OPMオペレーター1個分のレジスタ値（ファイル行の値をそのまま保持）。
/// フィールド順はVOPM形式の列順（ar d1r d2r rr d1l tl ks mul dt1 dt2 ams_en）に対応。
#[derive(Clone, Debug, Default)]
pub struct OpmOpReg {
    pub ar: u8,
    pub d1r: u8,
    pub d2r: u8,
    pub rr: u8,
    pub d1l: u8,
    pub tl: u8,
    pub ks: u8,
    pub mul: u8,
    pub dt1: u8,
    /// Detune2（OPM固有。粗いデチューン、インハーモニック音色のキモ）。
    pub dt2: u8,
    pub ams_en: bool,
}

/// OPM 1ボイス分のデータ（@: ブロック1件）。
#[derive(Clone, Debug)]
pub struct OpmVoice {
    pub number: u32,
    pub name: String,
    // LFO行: lfrq amd pmd wf nfrq
    pub lfrq: u8,
    pub amd: u8,
    pub pmd: u8,
    pub lfo_wf: u8,
    // CH行: pan fl con ams pms slot ne
    pub fl: u8,
    pub con: u8,
    pub ams: u8,
    pub pms: u8,
    pub slot: u8,
    // オペレーター（OPM名で保持）
    pub m1: OpmOpReg,
    pub c1: OpmOpReg,
    pub m2: OpmOpReg,
    pub c2: OpmOpReg,
}

impl Default for OpmVoice {
    fn default() -> Self {
        Self {
            number: 0,
            name: String::new(),
            lfrq: 0, amd: 0, pmd: 0, lfo_wf: 0,
            fl: 0, con: 0, ams: 0, pms: 0,
            slot: 120, // デフォルトは全OP有効
            m1: OpmOpReg::default(),
            c1: OpmOpReg::default(),
            m2: OpmOpReg::default(),
            c2: OpmOpReg::default(),
        }
    }
}

/// オペレーター並び順の指定。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OperatorOrder {
    /// M1,C1,M2,C2 をそのまま Op0-3 に割り当てる（デフォルト）。
    Direct,
    /// YM2151レジスタ順（M1,M2,C1,C2）で割り当てる。
    Register,
}

fn parse_u8s(s: &str) -> Vec<u8> {
    s.split_whitespace().filter_map(|t| t.parse().ok()).collect()
}

fn parse_op_reg(vals: &[u8]) -> OpmOpReg {
    OpmOpReg {
        ar:     *vals.get(0).unwrap_or(&0),
        d1r:    *vals.get(1).unwrap_or(&0),
        d2r:    *vals.get(2).unwrap_or(&0),
        rr:     *vals.get(3).unwrap_or(&0),
        d1l:    *vals.get(4).unwrap_or(&0),
        tl:     *vals.get(5).unwrap_or(&0),
        ks:     *vals.get(6).unwrap_or(&0),
        mul:    *vals.get(7).unwrap_or(&0),
        dt1:    *vals.get(8).unwrap_or(&0),
        dt2:    *vals.get(9).unwrap_or(&0),
        ams_en: *vals.get(10).unwrap_or(&0) != 0,
    }
}

/// .opmテキストを OpmVoice のベクタに変換する。
pub fn parse_opm(text: &str) -> Result<Vec<OpmVoice>, String> {
    let mut voices: Vec<OpmVoice> = Vec::new();
    let mut current: Option<OpmVoice> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") { continue; }
        let Some(pos) = line.find(':') else { continue };
        let key = line[..pos].trim().to_uppercase();
        let rest = line[pos + 1..].trim();

        match key.as_str() {
            "@" => {
                if let Some(v) = current.take() { voices.push(v); }
                let (num_s, name) = rest.split_once(' ').unwrap_or((rest, ""));
                let n = num_s.trim().parse::<u32>()
                    .map_err(|_| format!("@: の番号が解析できません: {rest:?}"))?;
                let mut v = OpmVoice::default();
                v.number = n;
                v.name = name.trim().to_string();
                current = Some(v);
            }
            "LFO" if current.is_some() => {
                let vals = parse_u8s(rest);
                let v = current.as_mut().unwrap();
                v.lfrq   = *vals.get(0).unwrap_or(&0);
                v.amd    = *vals.get(1).unwrap_or(&0);
                v.pmd    = *vals.get(2).unwrap_or(&0);
                v.lfo_wf = *vals.get(3).unwrap_or(&0);
            }
            "CH" if current.is_some() => {
                // pan fl con ams pms slot ne
                let vals = parse_u8s(rest);
                let v = current.as_mut().unwrap();
                v.fl   = *vals.get(1).unwrap_or(&0);
                v.con  = *vals.get(2).unwrap_or(&0);
                v.ams  = *vals.get(3).unwrap_or(&0);
                v.pms  = *vals.get(4).unwrap_or(&0);
                v.slot = *vals.get(5).unwrap_or(&120);
            }
            "M1" | "C1" | "M2" | "C2" if current.is_some() => {
                let vals = parse_u8s(rest);
                let v = current.as_mut().unwrap();
                let op = parse_op_reg(&vals);
                match key.as_str() {
                    "M1" => v.m1 = op,
                    "C1" => v.c1 = op,
                    "M2" => v.m2 = op,
                    "C2" => v.c2 = op,
                    _ => unreachable!(),
                }
            }
            _ => {}
        }
    }
    if let Some(v) = current.take() { voices.push(v); }
    Ok(voices)
}

/// ファイル名に使えない文字を `_` に置換し、前後の `_` を除去する。
pub fn sanitize_filename(name: &str) -> String {
    let s: String = name.chars().map(|c| {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }
    }).collect();
    s.trim_matches('_').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sample_opm() {
        let text = include_str!("../sample.opm");
        let voices = parse_opm(text).expect("parse ok");
        assert_eq!(voices.len(), 2);

        let ep = &voices[0];
        assert_eq!(ep.number, 0);
        assert_eq!(ep.name, "E.PIANO");
        assert_eq!(ep.con, 4);
        assert_eq!(ep.fl, 7);
        assert_eq!(ep.slot, 120);
        assert_eq!(ep.m1.ar, 31);
        assert_eq!(ep.m1.tl, 25);
        assert_eq!(ep.m1.dt2, 0);

        let saw = &voices[1];
        assert_eq!(saw.number, 1);
        assert_eq!(saw.name, "LEAD SAW");
        assert_eq!(saw.slot, 96); // M2とC2のみ有効
        assert_eq!(saw.lfrq, 60);
        assert_eq!(saw.amd, 40);
        assert_eq!(saw.pmd, 30);
    }

    #[test]
    fn sanitize_filename_replaces_special_chars() {
        assert_eq!(sanitize_filename("E.PIANO"), "E_PIANO");
        assert_eq!(sanitize_filename("LEAD SAW"), "LEAD_SAW");
        assert_eq!(sanitize_filename("bass-1"), "bass-1");
        assert_eq!(sanitize_filename("  foo  "), "foo");
    }

    #[test]
    fn parse_opm_defaults_on_missing_lfo_ch() {
        let text = "@:0 minimal\nM1: 31 0 0 7 0 0 0 1 0 0 0\nC1: 31 0 0 7 0 0 0 1 0 0 0\nM2: 31 0 0 7 0 0 0 1 0 0 0\nC2: 31 0 0 7 0 0 0 1 0 0 0\n";
        let voices = parse_opm(text).expect("parse ok");
        assert_eq!(voices[0].slot, 120); // デフォルト全OP有効
        assert_eq!(voices[0].lfrq, 0);
    }
}
