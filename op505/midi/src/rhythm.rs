//! GM2方式のリズム（ドラム）チャンネル判定・アドレス解決。
//!
//! Bank Select MSB(CC0)=120 の後の Program Change でそのMIDIチャンネルがリズムチャンネルに
//! なる（**切替を確定させるのはProgram Changeであって Bank Select 単体ではない**、実機GM2準拠）。
//! MSB=121（旋律バンク）+PCで旋律チャンネルへ戻る。
//!
//! ドラムキットは新しいデータ構造を作らず、既存の`.op505`バンク（`op505_core::Op505PresetBank`）
//! をそのまま流用する。`bank = 15360 + キット番号`（Bank Select MSB=120 相当）、
//! `program = ノート番号`として引くことで「ノートごとに音色が変わる」を表現する。
//!
//! 1MIDIチャンネル分の状態を[`ChannelProgramState`]が持ち、呼び出し側が`[ChannelProgramState; 16]`
//! 等でチャンネルごとに保持する（[`crate::rpn::RpnTracker`]・[`crate::pedal::PedalState`]と
//! 同じ責務分担）。`op505-vst`/`op505/tools/smf2op505`の両方から参照する（fork-on-writeの
//! 限定的な例外、詳細は本クレートのlib.rsおよびspec-fm.md 8章参照）。

/// GM2リズムバンクのBank Select MSB。
pub const RHYTHM_BANK_MSB: u8 = 120;
/// GM2旋律バンクのBank Select MSB（明示的にリズムから旋律へ戻すときに送る値）。
pub const MELODIC_BANK_MSB: u8 = 121;
/// リズムキットの`.op505`バンク番号のベース（`RHYTHM_BANK_MSB as u16 * 128`）。
/// キット番号を足してbank番号にする（[`rhythm_bank`]参照）。
pub const RHYTHM_BANK_BASE: u16 = RHYTHM_BANK_MSB as u16 * 128;
/// リズムキットが占める`.op505`バンク番号の範囲（15360..=15487、キット0〜127分）。
/// `Op505PresetBank::has_bank_in`でリズムキットの有無を判定するのに使う。
pub const RHYTHM_BANK_RANGE: std::ops::RangeInclusive<u16> = RHYTHM_BANK_BASE..=(RHYTHM_BANK_BASE + 127);
/// リセット時にドラムモードONで始まるMIDIチャンネルindex（MIDI ch10、0始まりで9）。
pub const RHYTHM_DEFAULT_CHANNEL: usize = 9;

/// キット番号(Program Change値)→`.op505`バンク番号。
#[inline]
pub fn rhythm_bank(kit: u8) -> u16 {
    RHYTHM_BANK_BASE + (kit & 0x7f) as u16
}

/// このチャンネルが今どう音色を引くか。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProgramSelection {
    /// 旋律：`(bank, program)`で1音色を引く（従来どおり`Op505PresetBank::get(bank, program)`）。
    Melodic { bank: u16, program: u8 },
    /// リズム：ノート番号ごとに`(rhythm_bank(kit), ノート番号)`で引く。
    Rhythm { kit: u8 },
}

/// Bank Select(CC0/CC32) + Program Change の状態機械。1MIDIチャンネル分。
///
/// **切替を確定させるのはProgram Changeであって Bank Select 単体ではない**（GM2実機準拠、
/// [`ChannelProgramState::program_change`]参照）。CC0=120を送っただけでは現在の旋律/リズムは
/// 変わらず、次のPCで初めて確定する。
///
/// GM1互換の粘りルール：一度リズムになったチャンネルは、CC0=121（旋律バンク）が明示的に
/// 来ない限り、CC0を伴わない裸のProgram Changeが来てもリズムのまま（kit番号だけ更新される）。
/// GM1時代のSMFはch10へBank Selectを送らずProgram Changeだけでキットを切り替えることが多く、
/// この粘りが無いと厳密なGM2解釈ではドラムが旋律楽器に化けてしまう。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ChannelProgramState {
    bank_msb: u8,
    bank_lsb: u8,
    is_rhythm: bool,
    program: u8,
}

impl ChannelProgramState {
    /// `midi_channel == RHYTHM_DEFAULT_CHANNEL`（MIDI ch10）かつ`rhythm_kits_available`のときだけ、
    /// ドラムモードONで始まる。`rhythm_kits_available`は「リズムキットが1つでもロードされているか」
    /// （`Op505PresetBank::has_bank_in(RHYTHM_BANK_RANGE)`）。
    ///
    /// falseのときch10初期ONを立てないのは、キットを持たない環境（既存の`.op505`バンクだけを
    /// 使う既存SMF・既存DAWプロジェクト）でch10が突然無音になる回帰を防ぐため。
    /// 明示的なCC0=120+PCは`rhythm_kits_available`に関わらず常に効く（このデフォルトはあくまで
    /// 「PCが一度も来ていない初期状態」の既定値）。
    pub fn new(midi_channel: usize, rhythm_kits_available: bool) -> Self {
        let is_rhythm = midi_channel == RHYTHM_DEFAULT_CHANNEL && rhythm_kits_available;
        Self { bank_msb: if is_rhythm { RHYTHM_BANK_MSB } else { 0 }, bank_lsb: 0, is_rhythm, program: 0 }
    }

    /// GM2 System Reset相当（プラグインの`reset()`/レンダリング開始時に呼ぶ）。
    ///
    /// **CC121(Reset All Controllers)からは呼ばないこと**：GM2ではRACはbank/programを
    /// リセットしない（`crate::pedal::PedalState::cc121`の「③ジェスチャー層のみリセット」の
    /// 設計思想と同じ）。ドラムモードはRACを越えて保持される。
    pub fn reset(&mut self, midi_channel: usize, rhythm_kits_available: bool) {
        *self = Self::new(midi_channel, rhythm_kits_available);
    }

    /// Bank Select MSB（CC0、0〜127）。ここだけでは旋律/リズムは切り替わらない
    /// （次のProgram Changeで確定する）。
    pub fn bank_select_msb(&mut self, value_u7: u8) {
        self.bank_msb = value_u7;
    }

    /// Bank Select LSB（CC32、0〜127）。GM2のリズムバンクは常にLSB=0のためリズム解決では
    /// 使わないが、旋律バンク番号の計算（`bank = msb*128+lsb`）には使うので保持だけする。
    pub fn bank_select_lsb(&mut self, value_u7: u8) {
        self.bank_lsb = value_u7;
    }

    /// Program Change（VST3ではCC102代替も同じくここへ通す）。ここで初めて旋律/リズムが確定する。
    ///
    /// 判定式（GM1互換の粘りルールを含む）：
    /// - `bank_msb == RHYTHM_BANK_MSB(120)` なら常にリズムへ入る
    /// - それ以外は、**既にリズムで**かつ `bank_msb != MELODIC_BANK_MSB(121)`（＝旋律への
    ///   明示切替でない）なら、リズムのまま維持する（kit番号だけ更新）
    /// - それ以外（旋律だった、または明示的にMSB=121が来た）は旋律になる
    pub fn program_change(&mut self, program_u7: u8) -> ProgramSelection {
        self.program = program_u7 & 0x7f;
        self.is_rhythm =
            self.bank_msb == RHYTHM_BANK_MSB || (self.is_rhythm && self.bank_msb != MELODIC_BANK_MSB);
        self.selection()
    }

    /// 現在の選択状態。
    pub fn selection(&self) -> ProgramSelection {
        if self.is_rhythm {
            ProgramSelection::Rhythm { kit: self.program }
        } else {
            ProgramSelection::Melodic { bank: (self.bank_msb as u16) * 128 + self.bank_lsb as u16, program: self.program }
        }
    }

    #[inline]
    pub fn is_rhythm(&self) -> bool {
        self.is_rhythm
    }

    /// 1ノートを鳴らすときに引くべき`(bank, program)`。
    /// - 旋律: `(bank_msb*128 + bank_lsb, PC番号)`（現行`op505-vst`と同じ式）
    /// - リズム: `(rhythm_bank(kit), note)`（ノート番号をそのままプログラム番号として使う）
    pub fn lookup_address(&self, note: u8) -> (u16, u8) {
        match self.selection() {
            ProgramSelection::Melodic { bank, program } => (bank, program),
            ProgramSelection::Rhythm { kit } => (rhythm_bank(kit), note),
        }
    }

    /// リズムキットに未定義ノートがあったときのGM2フォールバック先（Standard Kit = kit 0）。
    /// 旋律チャンネルでは`None`（旋律に「見つからない番号のフォールバック」という概念は無い）。
    pub fn rhythm_fallback_address(&self, note: u8) -> Option<(u16, u8)> {
        self.is_rhythm.then(|| (rhythm_bank(0), note))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bank_select_alone_does_not_switch() {
        let mut st = ChannelProgramState::new(0, false); // ch1、旋律から開始
        st.bank_select_msb(RHYTHM_BANK_MSB);
        assert!(!st.is_rhythm(), "Bank Selectだけでは切り替わらないはず");
    }

    #[test]
    fn program_change_after_msb120_enters_rhythm() {
        let mut st = ChannelProgramState::new(0, false);
        st.bank_select_msb(RHYTHM_BANK_MSB);
        let sel = st.program_change(5);
        assert!(st.is_rhythm());
        assert_eq!(sel, ProgramSelection::Rhythm { kit: 5 });
    }

    #[test]
    fn msb121_plus_pc_returns_to_melodic() {
        let mut st = ChannelProgramState::new(RHYTHM_DEFAULT_CHANNEL, true); // ch10、リズムで開始
        assert!(st.is_rhythm());
        st.bank_select_msb(MELODIC_BANK_MSB);
        let sel = st.program_change(10);
        assert!(!st.is_rhythm());
        assert_eq!(sel, ProgramSelection::Melodic { bank: MELODIC_BANK_MSB as u16 * 128, program: 10 });
    }

    /// GM1のSMFはch10へBank Selectを送らずProgram Changeだけでキットを切り替えることが多い。
    /// 一度リズムになっていれば、CC0=121が明示的に来ない限りリズムを維持する（粘りルール）。
    #[test]
    fn gm1_pc_without_bank_select_keeps_rhythm() {
        let mut st = ChannelProgramState::new(RHYTHM_DEFAULT_CHANNEL, true); // ch10、リズムで開始
        // Bank Selectを送らず、いきなりProgram Changeだけ送る（GM1 SMFの典型パターン）。
        let sel = st.program_change(25); // 例: TR-808 Kit相当
        assert!(st.is_rhythm(), "CC0が来なくてもリズムのまま維持されるはず");
        assert_eq!(sel, ProgramSelection::Rhythm { kit: 25 });
    }

    #[test]
    fn channel9_starts_in_rhythm_when_kits_available() {
        let st = ChannelProgramState::new(RHYTHM_DEFAULT_CHANNEL, true);
        assert!(st.is_rhythm());
        assert_eq!(st.selection(), ProgramSelection::Rhythm { kit: 0 });
    }

    #[test]
    fn channel9_stays_melodic_without_kits() {
        let st = ChannelProgramState::new(RHYTHM_DEFAULT_CHANNEL, false);
        assert!(!st.is_rhythm(), "リズムキット未ロード時はch10でも旋律のまま始まるはず");
    }

    #[test]
    fn other_channels_start_melodic_even_with_kits_available() {
        let st = ChannelProgramState::new(0, true); // ch1
        assert!(!st.is_rhythm());
    }

    #[test]
    fn explicit_rhythm_works_even_without_kits() {
        // rhythm_kits_available=falseでも、明示的なCC0=120+PCは常に効く
        // （既定値はあくまで「PCが一度も来ていない初期状態」の話）。
        let mut st = ChannelProgramState::new(0, false);
        st.bank_select_msb(RHYTHM_BANK_MSB);
        st.program_change(0);
        assert!(st.is_rhythm());
    }

    #[test]
    fn rhythm_lookup_address_uses_note_as_program() {
        let mut st = ChannelProgramState::new(0, false);
        st.bank_select_msb(RHYTHM_BANK_MSB);
        st.program_change(3); // kit 3
        assert_eq!(st.lookup_address(36), (RHYTHM_BANK_BASE + 3, 36));
        assert_eq!(st.lookup_address(42), (RHYTHM_BANK_BASE + 3, 42));
    }

    #[test]
    fn rhythm_ignores_bank_lsb() {
        let mut st = ChannelProgramState::new(0, false);
        st.bank_select_msb(RHYTHM_BANK_MSB);
        st.bank_select_lsb(99); // GM2上リズムのLSBは常に0扱い、何を送っても無視されるはず
        st.program_change(3);
        assert_eq!(st.lookup_address(36), (RHYTHM_BANK_BASE + 3, 36));
    }

    #[test]
    fn melodic_lookup_uses_bank_select_formula() {
        let mut st = ChannelProgramState::new(0, false);
        st.bank_select_msb(2);
        st.bank_select_lsb(10);
        st.program_change(5);
        assert_eq!(st.lookup_address(60), (2 * 128 + 10, 5), "旋律は現行op505-vstと同じ式のはず");
    }

    #[test]
    fn rhythm_bank_numbers_fit_in_u16() {
        assert_eq!(rhythm_bank(0), 15360);
        assert_eq!(rhythm_bank(127), 15487);
        assert!(RHYTHM_BANK_RANGE.contains(&15360));
        assert!(RHYTHM_BANK_RANGE.contains(&15487));
        assert!(!RHYTHM_BANK_RANGE.contains(&15488));
        assert!(15487u16 < 16383, "既存のbank空間(0〜16383)に収まるはず");
    }

    #[test]
    fn rhythm_fallback_targets_kit_zero() {
        let mut st = ChannelProgramState::new(0, false);
        st.bank_select_msb(RHYTHM_BANK_MSB);
        st.program_change(9); // kit 9（未定義ノートを想定）
        assert_eq!(st.rhythm_fallback_address(36), Some((RHYTHM_BANK_BASE, 36)));
    }

    #[test]
    fn melodic_has_no_rhythm_fallback() {
        let st = ChannelProgramState::new(0, false);
        assert_eq!(st.rhythm_fallback_address(36), None);
    }
}
