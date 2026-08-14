/// ペダル（CC64 Sustain / CC66 Sostenuto / CC67 Soft）の状態機械。1MIDIチャンネル分。
/// エンジン無改造・1ノート=1チャンネルのまま、呼び出し側が`[PedalState; 16]`等で
/// チャンネルごとに保持する（spec-sound.md「サステインペダル（CC64）の実装方針」参照）。
///
/// エンジンへの`note_off`呼び出しはこの構造体からは行わない（エンジンを持たないため）。
/// `cc64`/`cc66`/`cc121`は「今リリースすべきノート」をビットマスクで返すので、
/// 呼び出し側は[`released_notes`]で走査して`engine.note_off(...)`を呼ぶ。
#[derive(Clone, Copy, Default)]
pub struct PedalState {
    pub keys_down: u128,
    pub pedal_down: bool,
    pub pending_release: u128,
    /// CC66 ON時点でkeys_downをスナップショットしたノート（bit N = ノート番号N）。CC66 OFFで解除。
    pub sostenuto: u128,
    /// Soft Pedal（CC67）の深さ（0〜127、0=無効）。
    pub cc67: u8,
    /// cc67>0の間にNote-Onしたノート（bit N = ノート番号N）。実効TL/Cutoff減算の対象。
    pub soft_notes: u128,
}

impl PedalState {
    /// キーオン時の状態更新。弾き直したらペダル保留を解除する。Soft Pedal（CC67）が
    /// ON中の新規キーオンのみ`soft_notes`へ入れる。
    pub fn note_on(&mut self, note: u8) {
        let bit = 1u128 << note;
        self.pending_release &= !bit;
        self.keys_down |= bit;
        if self.cc67 > 0 {
            self.soft_notes |= bit;
        } else {
            self.soft_notes &= !bit;
        }
    }

    /// キーオフ時の状態更新。戻り値`true`なら呼び出し側が即座に`engine.note_off`してよい
    /// （ペダル保持なし）。`false`なら`pending_release`へ回されたので呼び出し側は何もしない。
    pub fn note_off(&mut self, note: u8) -> bool {
        let bit = 1u128 << note;
        self.keys_down &= !bit;
        let held = self.pedal_down || self.sostenuto & bit != 0;
        if held {
            self.pending_release |= bit;
            false
        } else {
            self.soft_notes &= !bit;
            true
        }
    }

    /// `candidates & pending_release`のうち、`other_held`にも`keys_down`にも該当しないものを
    /// リリースし、リリースしたノートのビットマスクを返す（CC64/CC66 OFF・CC121で共有する内部処理）。
    fn release_unheld(&mut self, candidates: u128, other_held: u128) -> u128 {
        let mut mask = candidates & self.pending_release;
        let mut released = 0u128;
        while mask != 0 {
            let note = mask.trailing_zeros();
            let bit = 1u128 << note;
            mask &= mask - 1;
            if other_held & bit == 0 && self.keys_down & bit == 0 {
                self.pending_release &= !bit;
                self.soft_notes &= !bit;
                released |= bit;
            }
        }
        released
    }

    /// CC64(Sustain)。`value_u7`は0〜127（cc_to_u7変換後）。OFF遷移時のみリリース対象ビットマスクを返す。
    pub fn cc64(&mut self, value_u7: u8) -> u128 {
        if value_u7 >= 64 {
            self.pedal_down = true;
            0
        } else {
            self.pedal_down = false;
            let candidates = self.pending_release;
            let other_held = self.sostenuto;
            self.release_unheld(candidates, other_held)
        }
    }

    /// CC66(Sostenuto)。ON時点でkeys_down中のノートのみをlatchし、CC66 OFF
    /// （かつCC64も踏まれていない）までReleaseに入らせない。
    pub fn cc66(&mut self, value_u7: u8) -> u128 {
        if value_u7 >= 64 {
            self.sostenuto = self.keys_down;
            0
        } else {
            let candidates = self.sostenuto;
            let other_held = if self.pedal_down { u128::MAX } else { 0 };
            let released = self.release_unheld(candidates, other_held);
            self.sostenuto = 0;
            released
        }
    }

    /// CC67(Soft Pedal)：深さを保持するのみ。ON中に新規キーオンしたノートのみへの適用は
    /// `note_on`側（soft_notesビット）で行う。
    pub fn cc67(&mut self, value_u7: u8) {
        self.cc67 = value_u7;
    }

    /// CC121(Reset All Controllers)：③ジェスチャー層のみリセットする
    /// （②パート状態・①音色は保持、spec-sound.md「補強規則」）。リリース対象ビットマスクを返す。
    pub fn cc121(&mut self) -> u128 {
        let candidates = self.pending_release;
        let released = self.release_unheld(candidates, 0);
        self.pedal_down = false;
        self.sostenuto = 0;
        self.cc67 = 0;
        self.soft_notes = 0;
        released
    }

    /// CC120(All Sound Off)：呼び出し側が`silence_group`でReleaseを経ず即座に消音するため、
    /// ここでは状態リセットのみ行う（`cc67`の深さは維持、CC123と対になる非対称に注意）。
    pub fn cc120_reset(&mut self) {
        self.keys_down = 0;
        self.pending_release = 0;
        self.pedal_down = false;
        self.sostenuto = 0;
        self.soft_notes = 0;
    }

    /// CC123(All Notes Off)：呼び出し側が全ノートをReleaseして自然減衰させる（`pedal_down`と
    /// `cc67`は維持、CC120と対になる非対称に注意）。
    pub fn cc123_reset(&mut self) {
        self.keys_down = 0;
        self.pending_release = 0;
        self.sostenuto = 0;
        self.soft_notes = 0;
    }

    /// プラグイン`reset()`用の完全初期化（`cc67`含め全フィールドを既定値へ戻す）。
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// `PedalState::cc64`/`cc66`/`cc121`が返すビットマスクを走査するイテレータ。
pub fn released_notes(mut mask: u128) -> impl Iterator<Item = u8> {
    std::iter::from_fn(move || {
        if mask == 0 {
            None
        } else {
            let note = mask.trailing_zeros() as u8;
            mask &= mask - 1;
            Some(note)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sustain_holds_note_off_until_pedal_up() {
        let mut pedal = PedalState::default();
        pedal.note_on(60);
        assert_eq!(pedal.cc64(127), 0); // CC64 ON
        assert!(!pedal.note_off(60)); // ペダル保持中はheldでfalse
        assert_eq!(pedal.pending_release, 1u128 << 60);
        let released: Vec<u8> = released_notes(pedal.cc64(0)).collect(); // CC64 OFF
        assert_eq!(released, vec![60]);
        assert_eq!(pedal.pending_release, 0);
    }

    #[test]
    fn retriggered_note_cancels_pending_release() {
        let mut pedal = PedalState::default();
        pedal.note_on(60);
        pedal.cc64(127);
        assert!(!pedal.note_off(60));
        pedal.note_on(60); // 弾き直し
        assert_eq!(pedal.pending_release, 0);
        let released: Vec<u8> = released_notes(pedal.cc64(0)).collect();
        assert!(released.is_empty()); // 再押下中なのでリリース対象にならない
    }

    #[test]
    fn sostenuto_only_latches_notes_already_down_at_on() {
        let mut pedal = PedalState::default();
        pedal.note_on(60);
        pedal.cc66(127); // CC66 ON、60をlatch
        pedal.note_on(64); // CC66 ON後に押した64はlatch対象外
        assert!(!pedal.note_off(60)); // sostenutoでheld
        assert!(pedal.note_off(64)); // latch対象外なので即off
        let released: Vec<u8> = released_notes(pedal.cc66(0)).collect(); // CC66 OFF
        assert_eq!(released, vec![60]);
    }

    #[test]
    fn cc120_keeps_soft_pedal_depth_cc123_keeps_pedal_down() {
        let mut pedal = PedalState::default();
        pedal.cc64(127);
        pedal.cc67(100);
        pedal.cc120_reset();
        assert!(!pedal.pedal_down);
        assert_eq!(pedal.cc67, 100); // CC120はcc67(Soft Pedal深さ)を保持する

        let mut pedal = PedalState::default();
        pedal.cc64(127);
        pedal.cc67(100);
        pedal.cc123_reset();
        assert!(pedal.pedal_down); // CC123はpedal_downを保持する
        assert_eq!(pedal.cc67, 100);
    }
}
