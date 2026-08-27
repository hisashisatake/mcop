//! Mono Mode（CC126 Mono On / CC127 Poly On）用の状態機械。1MIDIチャンネル分。
//!
//! 「常に再アタック」方針（op505-core側のボイス管理・EGは一切変更しない）を採る：Mono ON中は
//! 常に1音だけが発音中になるよう、呼び出し側が取るべき行動を返り値で伝える（`PedalState`と
//! 同じ設計。ここは状態を持つだけでエンジンを直接呼ばない）。
//!
//! Last-note priority: 新しいノートを離したとき、まだ物理的に押されている鍵盤があれば
//! 直前に押した方（押鍵順スタックの末尾）へ再アタックで戻る。
//!
//! ヒープ確保は禁止（`ChannelState`の`reset()`がリアルタイムコンテキストから呼ばれるため、
//! モジュールdocコメント参照）のため、押鍵順スタックは固定長配列＋長さで持つ。

/// 同時に保持できる鍵盤数の上限。MIDIは同時最大128ノートだが、Mono Modeで現実的に必要な
/// 深さは指の本数程度でごく浅い。上限到達時は最古の押鍵を捨てる（`stack_push`参照）。
const MONO_STACK_LEN: usize = 16;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MonoState {
    pub enabled: bool,
    /// 現在エンジンで発音中の単一ノート（リリース中のテールは含まない）。
    pub sounding: Option<u8>,
    /// 押鍵順スタック（末尾＝最新）。物理的に押されている鍵盤の記録。
    stack: [u8; MONO_STACK_LEN],
    stack_len: u8,
    /// フォールバック再発音（last-note priorityでの復帰）に使うベロシティ。ノート番号で直接引く。
    velocity: [u8; 128],
}

impl Default for MonoState {
    fn default() -> Self {
        Self {
            enabled: false,
            sounding: None,
            stack: [0; MONO_STACK_LEN],
            stack_len: 0,
            velocity: [0; 128],
        }
    }
}

/// [`MonoState::note_off`]の結果、呼び出し側が取るべき行動。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MonoNoteOff {
    /// 何もしない（離したのは現在発音中ではないノート＝既にフォールバック済みで
    /// エンジン上は既にリリース済み）。
    Nothing,
    /// このノートをエンジンでリリースする（フォールバック先の保持鍵盤がない）。
    Release(u8),
    /// `release`をエンジンでリリースし、`sound`を`velocity`で再発音する
    /// （last-note priorityでの保持鍵盤への復帰）。
    Fallback { release: u8, sound: u8, velocity: u8 },
}

impl MonoState {
    fn stack_remove(&mut self, note: u8) {
        let len = self.stack_len as usize;
        if let Some(pos) = self.stack[..len].iter().position(|&n| n == note) {
            for i in pos..len - 1 {
                self.stack[i] = self.stack[i + 1];
            }
            self.stack_len -= 1;
        }
    }

    fn stack_push(&mut self, note: u8) {
        // 再押下（同ノート二重ノートオン）はまず既存位置を除去してから末尾へ積み直す。
        self.stack_remove(note);
        if self.stack_len as usize == MONO_STACK_LEN {
            // 上限到達時は最古の押鍵を捨てる（通常の演奏で達することはまず無い保険的処理）。
            for i in 0..MONO_STACK_LEN - 1 {
                self.stack[i] = self.stack[i + 1];
            }
            self.stack_len -= 1;
        }
        self.stack[self.stack_len as usize] = note;
        self.stack_len += 1;
    }

    /// 押鍵。スタックへ積み、直前に発音中だったノート（Some時は呼び出し側がエンジンで
    /// リリースすべき）を返す。同ノートの再押下（弾き直し）はNoneを返す
    /// （既存ボイスIDへのretriggerに任せ、余計なnote_off/note_onを発行させないため）。
    pub fn note_on(&mut self, note: u8, velocity: u8) -> Option<u8> {
        self.velocity[note as usize] = velocity;
        self.stack_push(note);
        let previous = self.sounding;
        self.sounding = Some(note);
        previous.filter(|&p| p != note)
    }

    /// 離鍵。
    pub fn note_off(&mut self, note: u8) -> MonoNoteOff {
        self.stack_remove(note);
        if self.sounding != Some(note) {
            return MonoNoteOff::Nothing;
        }
        if self.stack_len == 0 {
            self.sounding = None;
            MonoNoteOff::Release(note)
        } else {
            let fallback = self.stack[self.stack_len as usize - 1];
            self.sounding = Some(fallback);
            MonoNoteOff::Fallback { release: note, sound: fallback, velocity: self.velocity[fallback as usize] }
        }
    }

    /// CC126(true)/CC127(false)によるモード切替。切り替え時に本状態機械が把握している
    /// 発音中ノート（Some時は呼び出し側がエンジンでリリースすべき）を返し、内部状態を
    /// リセットする。
    ///
    /// Poly→Monoの遷移時、Poly中に複数ノートが同時発音していた場合はこの返り値だけでは
    /// 足りない（本状態機械はPoly中ノートを追跡していないため）。呼び出し側はCC123
    /// （All Notes Off）と同じ「そのチャンネルの全ノートを一括note_off」もあわせて行うこと。
    pub fn set_enabled(&mut self, enabled: bool) -> Option<u8> {
        self.enabled = enabled;
        let released = self.sounding;
        self.sounding = None;
        self.stack_len = 0;
        released
    }

    /// CC120(All Sound Off)/CC123(All Notes Off)用の状態クリア。`enabled`は変えない
    /// （GM2のチャンネルモードはAll Sound Off/All Notes Off/Reset All Controllersの対象外）。
    pub fn reset(&mut self) {
        self.sounding = None;
        self.stack_len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_on_returns_previous_sounding_note() {
        let mut mono = MonoState::default();
        assert_eq!(mono.note_on(60, 100), None, "最初の1音は解放対象なし");
        assert_eq!(mono.note_on(64, 100), Some(60), "次の音で前のノートが解放対象になる");
        assert_eq!(mono.sounding, Some(64));
    }

    #[test]
    fn retriggering_the_same_note_returns_none() {
        let mut mono = MonoState::default();
        mono.note_on(60, 100);
        assert_eq!(mono.note_on(60, 110), None, "同ノートの弾き直しは解放対象なし");
        assert_eq!(mono.sounding, Some(60));
    }

    #[test]
    fn releasing_middle_note_does_not_change_sounding() {
        let mut mono = MonoState::default();
        mono.note_on(60, 100);
        mono.note_on(64, 100);
        mono.note_on(67, 100); // 60,64,67の順で押鍵、67が発音中
        assert_eq!(mono.note_off(64), MonoNoteOff::Nothing, "発音中でないノートを離しても何もしない");
        assert_eq!(mono.sounding, Some(67));
    }

    #[test]
    fn releasing_sounding_note_falls_back_to_last_held_note() {
        let mut mono = MonoState::default();
        mono.note_on(60, 100);
        mono.note_on(64, 105);
        mono.note_on(67, 110); // 60,64,67の順、67が発音中
        assert_eq!(
            mono.note_off(67),
            MonoNoteOff::Fallback { release: 67, sound: 64, velocity: 105 },
            "直前に押していた64へフォールバックするはず"
        );
        assert_eq!(mono.sounding, Some(64));

        assert_eq!(
            mono.note_off(64),
            MonoNoteOff::Fallback { release: 64, sound: 60, velocity: 100 },
            "さらに戻ると最初に押した60へフォールバックするはず"
        );
        assert_eq!(mono.sounding, Some(60));
    }

    #[test]
    fn releasing_last_held_note_releases_without_fallback() {
        let mut mono = MonoState::default();
        mono.note_on(60, 100);
        assert_eq!(mono.note_off(60), MonoNoteOff::Release(60));
        assert_eq!(mono.sounding, None);
    }

    #[test]
    fn stack_does_not_overflow_when_many_keys_held() {
        let mut mono = MonoState::default();
        for note in 0..40u8 {
            mono.note_on(note, 100);
        }
        // 破綻せず、直近の押鍵が発音中であること。
        assert_eq!(mono.sounding, Some(39));
        // 上限を超えて古い押鍵は捨てられているはずだが、直近の押鍵を離せば
        // その1つ前（上限内に残っている）へフォールバックできる。
        let result = mono.note_off(39);
        assert!(matches!(result, MonoNoteOff::Fallback { release: 39, sound: 38, .. }));
    }

    #[test]
    fn set_enabled_returns_sounding_note_and_resets() {
        let mut mono = MonoState::default();
        mono.note_on(60, 100);
        mono.note_on(64, 100);
        let released = mono.set_enabled(false);
        assert_eq!(released, Some(64));
        assert!(!mono.enabled);
        assert_eq!(mono.sounding, None);
        // リセット後は新規押鍵として扱われる。
        assert_eq!(mono.note_on(67, 100), None);
    }

    #[test]
    fn reset_clears_state_but_keeps_enabled() {
        let mut mono = MonoState::default();
        mono.enabled = true;
        mono.note_on(60, 100);
        mono.reset();
        assert!(mono.enabled, "reset()はenabledを変えない");
        assert_eq!(mono.sounding, None);
        assert_eq!(mono.note_on(64, 100), None, "リセット後は新規押鍵扱い");
    }
}
