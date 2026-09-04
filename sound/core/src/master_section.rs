// ---------------------------------------------------------------------------
// マスターセクション（エフェクトスロット配列 + 合算 + マスター出力の統合）
// ---------------------------------------------------------------------------
//
// standalone/vst/smf2op505の3ホストに散らばっていた「スロット別スクラッチ確保→
// Vco::render_routed()→スロットごとにMasterEffects::process()→合算」という
// 合算ループを一本化する。各ホストの差分（出力バッファへの書き込み方）だけが残る。

use crate::{AudioProcessor, MasterEffects, MasterOutput};

/// エフェクトスロット配列・スクラッチバッファ・マスター出力をまとめて持つ。
pub struct MasterSection {
    slots: Vec<MasterEffects>,
    output: MasterOutput,
    /// `render_routed`のスロット別出力スクラッチ（`slot_count`本分を1本のVecへ連結、grow-only）。
    slot_scratch: Vec<f32>,
    /// スロット合算後のミックス結果スクラッチ（grow-only）。
    mix_scratch: Vec<f32>,
}

impl MasterSection {
    /// `slot_count`はホスト側が持つエフェクトスロット数（`op505_midi::EFFECT_SLOT_COUNT`等）。
    /// `sound-core`はop505に依存しないため、スロット数は引数で受け取る。
    pub fn new(sample_rate: f32, slot_count: usize) -> Self {
        Self {
            slots: (0..slot_count).map(|_| MasterEffects::new(sample_rate)).collect(),
            output: MasterOutput::new(),
            slot_scratch: Vec::new(),
            mix_scratch: Vec::new(),
        }
    }

    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// 個別スロットへのアクセス（NRPN/CC91/93等、スロット単位のエフェクト設定用）。
    /// `slot`が範囲外の場合はパニックする（呼び出し側が既に`EFFECT_SLOT_COUNT - 1`へ
    /// クランプ済みであることが前提。既存3ホストの`effects[slot]`と同じ契約）。
    pub fn slot_mut(&mut self, slot: usize) -> &mut MasterEffects {
        &mut self.slots[slot]
    }

    pub fn output_mut(&mut self) -> &mut MasterOutput {
        &mut self.output
    }

    /// テンポ（BPM）を全スロットへ配る。Delay/Panning Delayのテンポ同期用
    /// （`MasterEffects::set_tempo`参照）。どのスロットが同期を使っているか
    /// ホスト側は知らないため、常に全スロットへ通知する。
    pub fn set_tempo(&mut self, bpm: f32) {
        for fx in self.slots.iter_mut() {
            fx.set_tempo(bpm);
        }
    }

    /// スロット別レンダリング→合算→マスター出力適用までを行い、結果のスライスを返す。
    ///
    /// `fill_slots`は`slot_buffer`（`slot_count * interleaved_len`長、ゼロ埋め済み）へ
    /// `Vco::render_routed(slot_buffer, interleaved_len, ..)`を呼ぶためのクロージャ。
    /// ここをクロージャで受ける理由: `&mut dyn Vco`を引数として直接受け取る形にすると、
    /// ホスト側で`engine`と`master`が同じ構造体のフィールドの場合
    /// （例: `self.master.render(&mut self.engine, ..)`）、コンパイラには「同じ`self`の
    /// 二重借用」に見えてしまいコンパイルエラーになる。クロージャにすれば`engine`を
    /// 借りる範囲がクロージャの内側だけに閉じるため、`master`（`self.master`）への
    /// `&mut`借用と衝突しない（呼び出し側で`let engine = &mut self.engine;`と事前に
    /// フィールドを取り出し、クロージャにその変数をキャプチャさせる形で使う）。
    pub fn render(
        &mut self,
        interleaved_len: usize,
        num_channels: usize,
        fill_slots: impl FnOnce(&mut [f32], usize),
    ) -> &[f32] {
        let slot_count = self.slots.len();
        let slot_total_len = interleaved_len * slot_count;
        if slot_total_len > self.slot_scratch.len() {
            self.slot_scratch.resize(slot_total_len, 0.0);
        }
        let slot_buf = &mut self.slot_scratch[..slot_total_len];
        slot_buf.fill(0.0);

        fill_slots(slot_buf, interleaved_len);

        if interleaved_len > self.mix_scratch.len() {
            self.mix_scratch.resize(interleaved_len, 0.0);
        }
        let mix_buf = &mut self.mix_scratch[..interleaved_len];
        mix_buf.fill(0.0);

        for (slot, fx) in self.slots.iter_mut().enumerate() {
            let s = &mut slot_buf[slot * interleaved_len..(slot + 1) * interleaved_len];
            fx.process(s, num_channels);
            for (m, v) in mix_buf.iter_mut().zip(s.iter()) {
                *m += v;
            }
        }

        self.output.process(mix_buf, num_channels);

        &self.mix_scratch[..interleaved_len]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_count_matches_constructor_argument() {
        let section = MasterSection::new(44100.0, 4);
        assert_eq!(section.slot_count(), 4);
    }

    /// 全スロットが既定（エフェクト未使用）・マスターボリューム既定(255)なら、
    /// `fill_slots`が書き込んだ値がそのまま出てくること（ビット一致維持の要）。
    #[test]
    fn default_state_is_bit_identical_passthrough() {
        let mut section = MasterSection::new(44100.0, 2);
        let interleaved_len = 8;

        let result = section
            .render(interleaved_len, 2, |slot_buf, stride| {
                // スロット0へ値を書き込む（render_routedを模した最小実装）。
                for (i, s) in slot_buf[..stride].iter_mut().enumerate() {
                    *s = i as f32 * 0.1;
                }
            })
            .to_vec();

        let expected: Vec<f32> = (0..interleaved_len).map(|i| i as f32 * 0.1).collect();
        assert_eq!(result, expected);
    }

    /// 複数スロットへ書き込んだ値が合算されること。
    #[test]
    fn slots_are_summed() {
        let mut section = MasterSection::new(44100.0, 3);
        let interleaved_len = 4;

        let result = section
            .render(interleaved_len, 2, |slot_buf, stride| {
                slot_buf[0 * stride] = 1.0;
                slot_buf[1 * stride] = 2.0;
                slot_buf[2 * stride] = 3.0;
            })
            .to_vec();

        assert_eq!(result[0], 6.0, "3スロット分の1.0+2.0+3.0が合算されるはず");
    }

    /// スクラッチバッファがgrow-onlyで再利用され、小さいバッファへ戻したときも
    /// 前回の残骸が混ざらないこと（毎回ゼロ埋めされること）。
    #[test]
    fn scratch_reuse_does_not_leak_previous_data() {
        let mut section = MasterSection::new(44100.0, 1);

        let _ = section.render(16, 2, |slot_buf, stride| {
            slot_buf[..stride].fill(9.0);
        });

        let result = section.render(4, 2, |slot_buf, stride| {
            slot_buf[..stride].fill(1.0);
        });

        assert_eq!(result, &[1.0, 1.0, 1.0, 1.0], "縮小後は前回の9.0が混入しないはず");
    }

    /// slot_mut/output_mutで個別スロット・マスター出力の設定を変更できること。
    #[test]
    fn slot_mut_and_output_mut_allow_individual_configuration() {
        let mut section = MasterSection::new(44100.0, 2);
        section.output_mut().set_volume(0);

        let result = section
            .render(2, 2, |slot_buf, stride| {
                slot_buf[..stride].fill(1.0);
            })
            .to_vec();

        assert_eq!(result, vec![0.0, 0.0], "マスターボリューム0なので無音になるはず");
    }
}
