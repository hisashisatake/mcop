/// CC99/98(NRPN)・CC101/100(RPN) で選択中のパラメーター番号。
/// CC6(Data Entry MSB)はこの選択状態に応じて値を適用する。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum RpnSelection {
    #[default]
    None,
    Rpn(u8, u8),
    Nrpn(u8, u8),
}

/// RPN/NRPNの選択状態（CC98/99/100/101の4値）とその結果選択されているパラメーターを追跡する。
/// MIDIチャンネル非依存のグローバル状態（Algorithm/Filter Type等のNRPN対象はパッチ全体の
/// グローバルパラメーターのため）。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct RpnTracker {
    rpn_msb: u8,
    rpn_lsb: u8,
    nrpn_msb: u8,
    nrpn_lsb: u8,
    pub selection: RpnSelection,
}

impl RpnTracker {
    pub fn set_rpn_msb(&mut self, value: u8) {
        self.rpn_msb = value;
        self.update(false);
    }

    pub fn set_rpn_lsb(&mut self, value: u8) {
        self.rpn_lsb = value;
        self.update(false);
    }

    pub fn set_nrpn_msb(&mut self, value: u8) {
        self.nrpn_msb = value;
        self.update(true);
    }

    pub fn set_nrpn_lsb(&mut self, value: u8) {
        self.nrpn_lsb = value;
        self.update(true);
    }

    /// MSB,LSB=127,127（Null）の場合は選択解除する。
    fn update(&mut self, is_nrpn: bool) {
        let (msb, lsb) = if is_nrpn { (self.nrpn_msb, self.nrpn_lsb) } else { (self.rpn_msb, self.rpn_lsb) };
        self.selection = if msb == 127 && lsb == 127 {
            RpnSelection::None
        } else if is_nrpn {
            RpnSelection::Nrpn(msb, lsb)
        } else {
            RpnSelection::Rpn(msb, lsb)
        };
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
