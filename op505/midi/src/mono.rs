//! Mono Mode（CC126 Mono On / CC127 Poly On）用の状態機械。1MIDIチャンネル分。
//!
//! 基本方針は「常に再アタック」（op505-core側のボイス管理・EGは一切変更しない）：Mono ON中は
//! 常に1音だけが発音中になるよう、呼び出し側が取るべき行動を返り値で伝える（`PedalState`と
//! 同じ設計。ここは状態を持つだけでエンジンを直接呼ばない）。
//!
//! **例外（レガート、2026-08-27）**: Mono ON かつ Portamento ON（CC65）かつレガート
//! （前の鍵盤を押したまま次の鍵盤を押した状態）のときだけ、エンジンのボイスを作り直さず
//! ピッチだけ滑らせる（`Op505Engine::glide_to`）。この状態機械はそのために`sounding`
//! （今どのノートの音程を鳴らしているか＝押鍵順位の頂点）と`voice`（エンジン側で実際に
//! ボイス（EG含む）を保持しているノート番号）を分けて追跡する。非レガート時はこの2つは
//! 常に一致する。レガート継続中はエンジンのボイスIDが最初のノートのまま変わらないため、
//! `voice`だけがそのIDを指し続け、`sounding`は新しい鍵盤ごとに移っていく。
//!
//! Last-note priority: 新しいノートを離したとき、まだ物理的に押されている鍵盤があれば
//! 直前に押した方（押鍵順スタックの末尾）へ戻る（レガート条件が揃えばレガートで、
//! 揃わなければ再アタックで）。
//!
//! ヒープ確保は禁止（`ChannelState`の`reset()`がリアルタイムコンテキストから呼ばれるため、
//! モジュールdocコメント参照）のため、押鍵順スタックは固定長配列＋長さで持つ。

/// 同時に保持できる鍵盤数の上限。MIDIは同時最大128ノートだが、Mono Modeで現実的に必要な
/// 深さは指の本数程度でごく浅い。上限到達時は最古の押鍵を捨てる（`stack_push`参照）。
const MONO_STACK_LEN: usize = 16;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MonoState {
    pub enabled: bool,
    /// 現在エンジンで発音中の単一ノート（リリース中のテールは含まない）。押鍵順位の頂点
    /// （＝今聞こえているべき音程）を表す。レガート中は`voice`と食い違いうる。
    pub sounding: Option<u8>,
    /// エンジン側で実際にボイス（EG含む）を保持しているノート番号。非レガート時は常に
    /// `sounding`と一致する。レガート継続中はエンジンのボイスIDを作り直さないため、
    /// フレーズの起点ノートのまま変わらない。呼び出し側がエンジンへ`note_off`/`glide_to`
    /// する対象は常にこちら（`sounding`ではない）。
    pub voice: Option<u8>,
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
            voice: None,
            stack: [0; MONO_STACK_LEN],
            stack_len: 0,
            velocity: [0; 128],
        }
    }
}

/// [`MonoState::note_on`]の結果、呼び出し側が取るべき行動。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MonoNoteOn {
    /// 通常発音。`release`があればそのボイスをエンジンでリリースしてから、新ノートで
    /// 通常の（Mono非依存の）Note On処理を行う。
    Retrigger { release: Option<u8> },
    /// レガート継続。`voice`のボイスをリリースせず、`Op505Engine::glide_to`でピッチだけ
    /// 新ノートへ滑らせる。`glide_to`が対象ボイスの消失（既にIdle等）で失敗したときは、
    /// 呼び出し側が[`MonoState::demote_legato`]を呼んでから通常のNote On処理へフォールバックする。
    Legato { voice: u8 },
}

/// [`MonoState::note_off`]の結果、呼び出し側が取るべき行動。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MonoNoteOff {
    /// 何もしない（離したのは現在発音中ではないノート＝既にフォールバック済みで
    /// エンジン上は既にリリース済み）。
    Nothing,
    /// このノート（`voice`のID）をエンジンでリリースする（フォールバック先の保持鍵盤がない）。
    Release(u8),
    /// `release`（`voice`のID）をエンジンでリリースし、`sound`を`velocity`で再発音する
    /// （last-note priorityでの保持鍵盤への通常の再アタック復帰）。
    Fallback { release: u8, sound: u8, velocity: u8 },
    /// `voice`のボイスをリリースせず、`Op505Engine::glide_to`で`sound`のピッチへレガートで
    /// 戻す（last-note priorityでのレガート復帰）。`glide_to`が成功する経路では`velocity`は
    /// 使わない（既存ボイスの音量・音色のまま）。`glide_to`が失敗したとき（対象ボイスが
    /// 既にIdle等）だけ、[`MonoState::demote_legato`]を呼んでから`velocity`で通常の
    /// フォールバック（新規note_on）へ切り替える。
    LegatoFallback { voice: u8, sound: u8, velocity: u8 },
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

    /// 押鍵。スタックへ積み、呼び出し側が取るべき行動を返す。
    ///
    /// `portamento`（CC65 ON）かつ既に別のノートのボイスを保持中（`voice`がSomeで`note`と
    /// 異なる）なら`Legato`を返す（＝レガート、ボイス継続）。それ以外（`portamento`が
    /// falseか、これがこのMono Modeランで最初の発音）は`Retrigger`を返す。
    ///
    /// 同ノートの再押下（弾き直し、`voice == Some(note)`）は`portamento`の値に関わらず
    /// `Retrigger { release: None }`を返す（既存ボイスIDへのretriggerに任せ、余計な
    /// note_off/note_onを発行させないため）。
    pub fn note_on(&mut self, note: u8, velocity: u8, portamento: bool) -> MonoNoteOn {
        self.velocity[note as usize] = velocity;
        self.stack_push(note);
        self.sounding = Some(note);
        if portamento {
            if let Some(voice) = self.voice {
                if voice != note {
                    return MonoNoteOn::Legato { voice };
                }
            }
        }
        let release = self.voice.filter(|&v| v != note);
        self.voice = Some(note);
        MonoNoteOn::Retrigger { release }
    }

    /// レガート継続（`Legato`/`LegatoFallback`）がエンジン側で失敗した（対象ボイスが
    /// 既にIdle等で`glide_to`がfalseを返した）ときの後始末。呼び出し側はこの直後に
    /// `note`を新規ボイスとして通常発音するため、`voice`をそのIDへ揃える。
    pub fn demote_legato(&mut self, note: u8) {
        self.voice = Some(note);
    }

    /// 離鍵。`portamento`（CC65 ON）かつフォールバック先があるならレガートで戻す
    /// （`LegatoFallback`）。エンジンへの解放対象は常に`voice`（レガート中は`sounding`と
    /// 食い違いうるため、`note`をそのまま使ってはいけない）。
    pub fn note_off(&mut self, note: u8, portamento: bool) -> MonoNoteOff {
        self.stack_remove(note);
        if self.sounding != Some(note) {
            return MonoNoteOff::Nothing;
        }
        let held = self.voice.unwrap_or(note);
        if self.stack_len == 0 {
            self.sounding = None;
            self.voice = None;
            MonoNoteOff::Release(held)
        } else {
            let fallback = self.stack[self.stack_len as usize - 1];
            self.sounding = Some(fallback);
            if portamento {
                MonoNoteOff::LegatoFallback { voice: held, sound: fallback, velocity: self.velocity[fallback as usize] }
            } else {
                self.voice = Some(fallback);
                MonoNoteOff::Fallback { release: held, sound: fallback, velocity: self.velocity[fallback as usize] }
            }
        }
    }

    /// CC126(true)/CC127(false)によるモード切替。切り替え時に本状態機械が把握している
    /// 発音中ボイス（Some時は呼び出し側がエンジンでリリースすべき）を返し、内部状態を
    /// リセットする。
    ///
    /// Poly→Monoの遷移時、Poly中に複数ノートが同時発音していた場合はこの返り値だけでは
    /// 足りない（本状態機械はPoly中ノートを追跡していないため）。呼び出し側はCC123
    /// （All Notes Off）と同じ「そのチャンネルの全ノートを一括note_off」もあわせて行うこと。
    pub fn set_enabled(&mut self, enabled: bool) -> Option<u8> {
        self.enabled = enabled;
        let released = self.voice;
        self.sounding = None;
        self.voice = None;
        self.stack_len = 0;
        released
    }

    /// CC120(All Sound Off)/CC123(All Notes Off)用の状態クリア。`enabled`は変えない
    /// （GM2のチャンネルモードはAll Sound Off/All Notes Off/Reset All Controllersの対象外）。
    pub fn reset(&mut self) {
        self.sounding = None;
        self.voice = None;
        self.stack_len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_on_returns_previous_sounding_note() {
        let mut mono = MonoState::default();
        assert_eq!(mono.note_on(60, 100, false), MonoNoteOn::Retrigger { release: None }, "最初の1音は解放対象なし");
        assert_eq!(
            mono.note_on(64, 100, false),
            MonoNoteOn::Retrigger { release: Some(60) },
            "次の音で前のノートが解放対象になる"
        );
        assert_eq!(mono.sounding, Some(64));
        assert_eq!(mono.voice, Some(64));
    }

    #[test]
    fn retriggering_the_same_note_returns_no_release() {
        let mut mono = MonoState::default();
        mono.note_on(60, 100, false);
        assert_eq!(
            mono.note_on(60, 110, false),
            MonoNoteOn::Retrigger { release: None },
            "同ノートの弾き直しは解放対象なし"
        );
        assert_eq!(mono.sounding, Some(60));
    }

    #[test]
    fn releasing_middle_note_does_not_change_sounding() {
        let mut mono = MonoState::default();
        mono.note_on(60, 100, false);
        mono.note_on(64, 100, false);
        mono.note_on(67, 100, false); // 60,64,67の順で押鍵、67が発音中
        assert_eq!(mono.note_off(64, false), MonoNoteOff::Nothing, "発音中でないノートを離しても何もしない");
        assert_eq!(mono.sounding, Some(67));
    }

    #[test]
    fn releasing_sounding_note_falls_back_to_last_held_note() {
        let mut mono = MonoState::default();
        mono.note_on(60, 100, false);
        mono.note_on(64, 105, false);
        mono.note_on(67, 110, false); // 60,64,67の順、67が発音中
        assert_eq!(
            mono.note_off(67, false),
            MonoNoteOff::Fallback { release: 67, sound: 64, velocity: 105 },
            "直前に押していた64へフォールバックするはず"
        );
        assert_eq!(mono.sounding, Some(64));

        assert_eq!(
            mono.note_off(64, false),
            MonoNoteOff::Fallback { release: 64, sound: 60, velocity: 100 },
            "さらに戻ると最初に押した60へフォールバックするはず"
        );
        assert_eq!(mono.sounding, Some(60));
    }

    #[test]
    fn releasing_last_held_note_releases_without_fallback() {
        let mut mono = MonoState::default();
        mono.note_on(60, 100, false);
        assert_eq!(mono.note_off(60, false), MonoNoteOff::Release(60));
        assert_eq!(mono.sounding, None);
        assert_eq!(mono.voice, None);
    }

    #[test]
    fn stack_does_not_overflow_when_many_keys_held() {
        let mut mono = MonoState::default();
        for note in 0..40u8 {
            mono.note_on(note, 100, false);
        }
        // 破綻せず、直近の押鍵が発音中であること。
        assert_eq!(mono.sounding, Some(39));
        // 上限を超えて古い押鍵は捨てられているはずだが、直近の押鍵を離せば
        // その1つ前（上限内に残っている）へフォールバックできる。
        let result = mono.note_off(39, false);
        assert!(matches!(result, MonoNoteOff::Fallback { release: 39, sound: 38, .. }));
    }

    #[test]
    fn set_enabled_returns_sounding_note_and_resets() {
        let mut mono = MonoState::default();
        mono.note_on(60, 100, false);
        mono.note_on(64, 100, false);
        let released = mono.set_enabled(false);
        assert_eq!(released, Some(64));
        assert!(!mono.enabled);
        assert_eq!(mono.sounding, None);
        assert_eq!(mono.voice, None);
        // リセット後は新規押鍵として扱われる。
        assert_eq!(mono.note_on(67, 100, false), MonoNoteOn::Retrigger { release: None });
    }

    #[test]
    fn reset_clears_state_but_keeps_enabled() {
        let mut mono = MonoState::default();
        mono.enabled = true;
        mono.note_on(60, 100, false);
        mono.reset();
        assert!(mono.enabled, "reset()はenabledを変えない");
        assert_eq!(mono.sounding, None);
        assert_eq!(mono.voice, None);
        assert_eq!(mono.note_on(64, 100, false), MonoNoteOn::Retrigger { release: None }, "リセット後は新規押鍵扱い");
    }

    // --- レガート（方式B）関連 ---

    #[test]
    fn legato_note_on_keeps_the_original_voice() {
        let mut mono = MonoState::default();
        mono.note_on(60, 100, true); // 最初の1音はportamento onでも通常発音
        assert_eq!(
            mono.note_on(64, 100, true),
            MonoNoteOn::Legato { voice: 60 },
            "Mono+CC65 ON+レガートで前のボイスを継続するはず"
        );
        assert_eq!(mono.sounding, Some(64), "聞こえているべき音程は新ノートへ移る");
        assert_eq!(mono.voice, Some(60), "エンジンボイスのIDは最初のノートのまま");

        // さらにレガート連結。voiceは60のまま。
        assert_eq!(mono.note_on(67, 100, true), MonoNoteOn::Legato { voice: 60 });
        assert_eq!(mono.sounding, Some(67));
        assert_eq!(mono.voice, Some(60));
    }

    #[test]
    fn non_legato_note_on_still_retriggers_even_with_portamento_on() {
        // 前の鍵盤を離してから次を押す（非レガート）は、CC65 ONでも通常のRetrigger。
        let mut mono = MonoState::default();
        mono.note_on(60, 100, true);
        mono.note_off(60, true); // 全鍵を離す
        assert_eq!(
            mono.note_on(64, 100, true),
            MonoNoteOn::Retrigger { release: None },
            "非レガートはCC65 ONでも再アタックのはず"
        );
        assert_eq!(mono.voice, Some(64));
    }

    #[test]
    fn note_on_without_portamento_ignores_legato_even_mid_run() {
        let mut mono = MonoState::default();
        mono.note_on(60, 100, false);
        assert_eq!(
            mono.note_on(64, 100, false),
            MonoNoteOn::Retrigger { release: Some(60) },
            "CC65 OFFなら常に通常のRetrigger"
        );
        assert_eq!(mono.voice, Some(64));
    }

    #[test]
    fn demote_legato_reassigns_voice_after_a_failed_glide() {
        let mut mono = MonoState::default();
        mono.note_on(60, 100, true);
        assert_eq!(mono.note_on(64, 100, true), MonoNoteOn::Legato { voice: 60 });
        // glide_toがエンジン側で失敗した（ボイス消失）と想定し、通常発音へフォールバックする。
        mono.demote_legato(64);
        assert_eq!(mono.voice, Some(64), "以後のフォールバック先ボイスは新ノートのIDになる");
        assert_eq!(mono.sounding, Some(64));
    }

    #[test]
    fn legato_fallback_returns_to_last_held_note_without_retriggering() {
        let mut mono = MonoState::default();
        mono.note_on(60, 100, true);
        mono.note_on(64, 105, true); // Legato: voice=60のまま
        mono.note_on(67, 110, true); // Legato: voice=60のまま、60,64,67の順で押鍵
        assert_eq!(
            mono.note_off(67, true),
            MonoNoteOff::LegatoFallback { voice: 60, sound: 64, velocity: 105 },
            "レガート中に上の鍵盤を離すと、voiceを保ったままsound(64)へ戻るはず"
        );
        assert_eq!(mono.sounding, Some(64));
        assert_eq!(mono.voice, Some(60), "voiceはフレーズ起点の60のまま変わらない");

        assert_eq!(
            mono.note_off(64, true),
            MonoNoteOff::LegatoFallback { voice: 60, sound: 60, velocity: 100 },
            "さらに戻ると起点の60自身へ戻る（voice==soundの自明なグライドになる）"
        );
        assert_eq!(mono.sounding, Some(60));
        assert_eq!(mono.voice, Some(60));
    }

    #[test]
    fn note_off_uses_voice_not_note_when_portamento_turned_off_mid_run() {
        // レガート中にCC65がOFFへ変わった後の離鍵は、noteそのものではなく
        // 実際にエンジンで保持しているvoiceをリリース対象にしなければならない。
        let mut mono = MonoState::default();
        mono.note_on(60, 100, true);
        mono.note_on(64, 105, true); // Legato: voice=60のまま、sounding=64
        assert_eq!(
            mono.note_off(64, false), // ここでCC65がOFFになっていたとする
            MonoNoteOff::Fallback { release: 60, sound: 60, velocity: 100 },
            "releaseは離したnote(64)ではなく、実際にエンジンで鳴っているvoice(60)であるべき"
        );
        assert_eq!(mono.sounding, Some(60));
        assert_eq!(mono.voice, Some(60));
    }

    #[test]
    fn re_pressing_the_anchor_note_mid_legato_run_is_a_plain_retrigger() {
        // レガート連結中に、ボイスを保持しているノート自身を再度押した場合
        // （同ノートの弾き直し）は、既存ボイスIDへのretriggerに任せる。
        let mut mono = MonoState::default();
        mono.note_on(60, 100, true);
        mono.note_on(64, 100, true); // Legato: voice=60
        assert_eq!(mono.note_on(60, 120, true), MonoNoteOn::Retrigger { release: None });
        assert_eq!(mono.voice, Some(60));
    }
}
