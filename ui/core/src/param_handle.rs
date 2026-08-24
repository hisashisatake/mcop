/// 整数パラメーター(min〜maxのレンジを持つ)へのハンドル。
/// ホスト環境（VST/nice-plug、gesture-app/Tauri等）ごとに実装し、本クレートの描画ロジックを
/// 特定のパラメーターシステムから切り離す。
pub trait IntParamHandle {
    /// 現在の値（plain値、min〜maxの範囲）。
    fn value(&self) -> i32;
    fn min(&self) -> i32;
    fn max(&self) -> i32;
    fn default(&self) -> i32;
    /// ツールチップ等に表示するパラメーター名。
    fn name(&self) -> String;
    /// 現在値の文字列表現。既定実装は数値そのまま。
    fn display(&self) -> String {
        self.value().to_string()
    }
    /// 操作開始（ドラッグ開始・スピン開始時など、ホスト側のオートメーション開始通知に対応）。
    fn begin_edit(&self);
    /// 値を設定する。呼び出し側で`[min, max]`にクランプ済みの値を渡すこと。
    fn set(&self, value: i32);
    /// 操作終了。
    fn end_edit(&self);
}

/// 中央値（バイポーラパラメーターで「変調なし」を表す生値）。
///
/// このプロジェクトのバイポーラパラメーターは0〜255の**オフセットバイナリ**（下駄履き表現）で、
/// `(生値 - 128) / 128` を係数として使う（`dt1_to_cents`・`op_fine_tune_to_cents`・
/// `lfo_offset_from_param`・`effective_cutoff`・Pitch FGのセント換算がすべてこの形）。
/// 2の補数ではないので生値は0〜255で単調増加し、途中で符号が折り返さない。
pub const BIPOLAR_CENTER: i32 = 128;

/// 中央128のバイポーラパラメーターを、0〜255の生値ではなく**-128〜+127の符号付き整数**として
/// 見せるアダプタ。`P.DEP±`/`F.DEP±`/`DT1`/`FINE`/`TX.OFS`のように「0が変調なし」の
/// パラメーターで、ノブの数値欄・ツールチップ・直接入力をすべて符号付きに揃えるために挟む。
///
/// `display()`だけを書き換える方法は取れない。`spin_control`は表示文字列とは独立に入力を
/// 生値としてパースし`min()`/`max()`でクランプするため、表示が`-40`なのにmin=0だと
/// ユーザーが`-40`と打った瞬間に0へ丸められてしまう。`value()`/`min()`/`max()`/`default()`/
/// `set()`をまとめてオフセットするこの方式なら、ノブの指針位置・±ボタン・直接入力が
/// **すべて無改造で正しくなる**（`Knob::normalized()`は`(value-min)/(max-min)`なので
/// `(生値-128+128)/255 = 生値/255`となり、オフセット前と完全に同一の値になる）。
///
/// 負側が1目盛り広い非対称レンジ（-128〜+127）になるのは意図的。`(生値-128)/128`は生値0で
/// ちょうど-1.0（フルスケール）に届くが、生値255では+127/128=0.9922止まりで、
/// どちらかが必ず1目盛り損をする。「全開の逆方向」を正確に出せる側を負に割り当ててある
/// （`sound_core::lfo_offset_from_param`のテストが同じ性質を「このプロジェクト共通の性質」として固定済み）。
pub struct BipolarHandle<'a> {
    inner: &'a dyn IntParamHandle,
}

impl<'a> BipolarHandle<'a> {
    pub fn new(inner: &'a dyn IntParamHandle) -> Self {
        Self { inner }
    }
}

impl IntParamHandle for BipolarHandle<'_> {
    fn value(&self) -> i32 {
        self.inner.value() - BIPOLAR_CENTER
    }
    fn min(&self) -> i32 {
        self.inner.min() - BIPOLAR_CENTER
    }
    fn max(&self) -> i32 {
        self.inner.max() - BIPOLAR_CENTER
    }
    fn default(&self) -> i32 {
        self.inner.default() - BIPOLAR_CENTER
    }
    fn name(&self) -> String {
        self.inner.name()
    }
    /// 0以外は符号を明示する（`+40`／`-40`）。0だけは`+0`が不自然なので符号を付けない。
    fn display(&self) -> String {
        let v = self.value();
        if v == 0 {
            "0".to_string()
        } else {
            format!("{v:+}")
        }
    }
    fn begin_edit(&self) {
        self.inner.begin_edit();
    }
    fn set(&self, value: i32) {
        self.inner.set(value + BIPOLAR_CENTER);
    }
    fn end_edit(&self) {
        self.inner.end_edit();
    }
}

/// 真偽パラメーターへのハンドル。
pub trait BoolParamHandle {
    fn value(&self) -> bool;
    fn begin_edit(&self);
    fn set(&self, value: bool);
    fn end_edit(&self);
}

/// `TimeEgParams`（N点Time/Level方式EG、OP505用）1本ぶんへのハンドル。
/// 段×フィールドごとに`IntParamHandle`を280個(40値×7本)構築する代わりに、EG単位で1個
/// 用意すれば済む（`time_eg_editor`が内部でこのハンドルから`IntParamHandle`を都度導出する）。
pub trait TimeEgHandle {
    /// 現在値のスナップショット（`TimeEgParams`はCopyなので値返し）。
    fn params(&self) -> sound_core::TimeEgParams;
    /// 全体を書き戻す。
    fn set_params(&self, params: sound_core::TimeEgParams);
    /// ツールチップ・egui memoryのId salt（EGごとに一意であること。例:"OP1 EG"/"GAIN FG"）に使う名前。
    fn name(&self) -> String;
    /// 操作開始（ドラッグ開始時など）。
    fn begin_edit(&self);
    /// 操作終了。
    fn end_edit(&self);

    /// テンポ同期の有効/無効（`sync_enabled`）へのハンドル。`params()`/`set_params()`の
    /// 往復で1フィールドだけ読み書きするアダプタで、ホスト側（VST/gesture-app）の実装追加は不要
    /// （段×フィールドの個別ハンドルを`time_eg_editor`が内部導出するのと同じ設計）。
    /// ラッパー構造体を`T: TimeEgHandle + ?Sized`のジェネリックにすることで、`dyn TimeEgHandle`
    /// 越しの呼び出し（`Self`が非Sized）でもコンパイル時のアンサイズ変換なしに素通しできる。
    fn sync_enabled_handle(&self) -> Box<dyn BoolParamHandle + '_> {
        Box::new(TimeEgSyncEnabledHandle { eg: self })
    }

    /// 同期先の連続レート（`sync_rate`、0〜255）へのハンドル。20音価は
    /// `sound_core::sync_note_anchor()`のアンカー値へ厳密に乗る。
    fn sync_rate_handle(&self) -> Box<dyn IntParamHandle + '_> {
        Box::new(TimeEgIntFieldHandle { eg: self, field: TimeEgIntField::SyncRate })
    }

    /// retrigger()時のFGレベル継承モード（`retrigger_mode`、0=Continue/1=Reset）へのハンドル。
    fn retrigger_mode_handle(&self) -> Box<dyn IntParamHandle + '_> {
        Box::new(TimeEgIntFieldHandle { eg: self, field: TimeEgIntField::RetriggerMode })
    }

    /// 質感（`texture`、0=OFF/1=S&H/2=Random/3=Chaos）へのハンドル（旧質感LFOのS&H/Random/
    /// Chaos波形の後継、memory `project_texture_lfo_retirement.md`参照）。
    fn texture_handle(&self) -> Box<dyn IntParamHandle + '_> {
        Box::new(TimeEgIntFieldHandle { eg: self, field: TimeEgIntField::Texture })
    }

    /// ワンショット化（`auto_release`、0=OFF/N≥1=保持区間をN回通過したら自動リリース）へのハンドル。
    /// 非ゼロの間、外部note_offは無視される（GM2リズムチャンネル用、memory
    /// `project_gm2_rhythm_channel_implementation.md`参照）。
    fn auto_release_handle(&self) -> Box<dyn IntParamHandle + '_> {
        Box::new(TimeEgIntFieldHandle { eg: self, field: TimeEgIntField::AutoRelease })
    }
}

struct TimeEgSyncEnabledHandle<'a, T: TimeEgHandle + ?Sized> {
    eg: &'a T,
}

impl<'a, T: TimeEgHandle + ?Sized> BoolParamHandle for TimeEgSyncEnabledHandle<'a, T> {
    fn value(&self) -> bool {
        self.eg.params().sync_enabled != 0
    }
    fn begin_edit(&self) {
        self.eg.begin_edit();
    }
    fn set(&self, value: bool) {
        let mut params = self.eg.params();
        params.sync_enabled = if value { 1 } else { 0 };
        self.eg.set_params(params);
    }
    fn end_edit(&self) {
        self.eg.end_edit();
    }
}

enum TimeEgIntField {
    SyncRate,
    RetriggerMode,
    Texture,
    AutoRelease,
}

struct TimeEgIntFieldHandle<'a, T: TimeEgHandle + ?Sized> {
    eg: &'a T,
    field: TimeEgIntField,
}

impl<'a, T: TimeEgHandle + ?Sized> IntParamHandle for TimeEgIntFieldHandle<'a, T> {
    fn value(&self) -> i32 {
        let p = self.eg.params();
        match self.field {
            TimeEgIntField::SyncRate => p.sync_rate as i32,
            TimeEgIntField::RetriggerMode => p.retrigger_mode as i32,
            TimeEgIntField::Texture => p.texture as i32,
            TimeEgIntField::AutoRelease => p.auto_release as i32,
        }
    }
    fn min(&self) -> i32 {
        0
    }
    fn max(&self) -> i32 {
        match self.field {
            TimeEgIntField::SyncRate => 255,
            TimeEgIntField::RetriggerMode => 1,
            TimeEgIntField::Texture => 3,
            TimeEgIntField::AutoRelease => 255,
        }
    }
    fn default(&self) -> i32 {
        match self.field {
            TimeEgIntField::SyncRate => sound_core::sync_note_anchor(10) as i32,
            TimeEgIntField::RetriggerMode => sound_core::RETRIGGER_MODE_CONTINUE as i32,
            TimeEgIntField::Texture => sound_core::TEXTURE_OFF as i32,
            TimeEgIntField::AutoRelease => 0,
        }
    }
    fn name(&self) -> String {
        let suffix = match self.field {
            TimeEgIntField::SyncRate => "Sync Rate",
            TimeEgIntField::RetriggerMode => "Retrigger",
            TimeEgIntField::Texture => "Texture",
            TimeEgIntField::AutoRelease => "Auto Release",
        };
        format!("{} {}", self.eg.name(), suffix)
    }
    /// `sync_rate`は生の0〜255ではなく音価名で見せる（ノブのツールチップ／数値欄用）。
    /// アンカーから外れているときは`~1/8`のようにチルダを付けて近似であることを示す。
    fn display(&self) -> String {
        match self.field {
            TimeEgIntField::SyncRate => crate::selector::sync_rate_display(self.value() as u8),
            TimeEgIntField::RetriggerMode => self.value().to_string(),
            TimeEgIntField::Texture => {
                crate::selector::TEXTURE_NAMES[self.value().clamp(0, 3) as usize].to_string()
            }
            TimeEgIntField::AutoRelease => {
                if self.value() == 0 {
                    "OFF".to_string()
                } else {
                    self.value().to_string()
                }
            }
        }
    }
    fn begin_edit(&self) {
        self.eg.begin_edit();
    }
    fn set(&self, value: i32) {
        let mut params = self.eg.params();
        match self.field {
            TimeEgIntField::SyncRate => {
                params.sync_rate = value.clamp(0, 255) as u8;
            }
            TimeEgIntField::RetriggerMode => {
                params.retrigger_mode = value.clamp(0, 1) as u8;
            }
            TimeEgIntField::Texture => {
                params.texture = value.clamp(0, 3) as u8;
            }
            TimeEgIntField::AutoRelease => {
                params.auto_release = value.clamp(0, 255) as u8;
            }
        }
        self.eg.set_params(params);
    }
    fn end_edit(&self) {
        self.eg.end_edit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// 0〜255・中央128の生パラメーター（P.DEP±等のホスト実装に相当）。
    struct RawParam {
        value: Cell<i32>,
    }

    impl IntParamHandle for RawParam {
        fn value(&self) -> i32 {
            self.value.get()
        }
        fn min(&self) -> i32 {
            0
        }
        fn max(&self) -> i32 {
            255
        }
        fn default(&self) -> i32 {
            128
        }
        fn name(&self) -> String {
            "F.DEP±".to_string()
        }
        fn begin_edit(&self) {}
        fn set(&self, value: i32) {
            self.value.set(value.clamp(0, 255));
        }
        fn end_edit(&self) {}
    }

    fn raw(value: i32) -> RawParam {
        RawParam { value: Cell::new(value) }
    }

    #[test]
    fn bipolar_shifts_range_to_signed() {
        let inner = raw(128);
        let b = BipolarHandle::new(&inner);
        assert_eq!(b.value(), 0, "生値128は0");
        assert_eq!(b.min(), -128);
        assert_eq!(b.max(), 127, "負側が1目盛り広い非対称レンジ（オフセットバイナリの性質）");
        assert_eq!(b.default(), 0);
        assert_eq!(b.name(), "F.DEP±", "名前は素通し");
    }

    #[test]
    fn bipolar_set_writes_offset_raw_value() {
        let inner = raw(128);
        for (input, expected_raw) in [(-128, 0), (0, 128), (127, 255)] {
            BipolarHandle::new(&inner).set(input);
            assert_eq!(inner.value(), expected_raw, "set({input})");
        }
    }

    #[test]
    fn bipolar_display_shows_sign_except_zero() {
        for (raw_value, expected) in [(128, "0"), (168, "+40"), (88, "-40"), (0, "-128"), (255, "+127")] {
            let inner = raw(raw_value);
            assert_eq!(BipolarHandle::new(&inner).display(), expected, "生値{raw_value}");
        }
    }

    /// バイポーラ化してもノブの指針位置が一切動かないこと。`Knob::normalized()`は
    /// `(value - min) / (max - min)`なので、オフセットが分子と分母で打ち消えて`生値/255`に戻る。
    /// ここが崩れると「表示を変えただけでノブの位置がずれる」という最悪の回帰になる。
    #[test]
    fn bipolar_preserves_knob_needle_position() {
        for raw_value in 0..=255 {
            let inner = raw(raw_value);
            let b = BipolarHandle::new(&inner);
            let plain = (inner.value() - inner.min()) as f32 / (inner.max() - inner.min()) as f32;
            let shifted = (b.value() - b.min()) as f32 / (b.max() - b.min()) as f32;
            assert_eq!(shifted, plain, "生値{raw_value}で指針位置がずれた");
        }
    }

    /// `spin_control`の直接入力経路（表示文字列→パース→set）が往復すること。
    /// 表示だけをバイポーラにする実装だと、ここで`-40`がmin(0)へクランプされて0になり壊れる。
    #[test]
    fn bipolar_display_round_trips_through_set() {
        for raw_value in 0..=255 {
            let shown = {
                let probe = raw(raw_value);
                BipolarHandle::new(&probe).display()
            };
            let parsed: i32 = shown.parse().expect("符号付き整数としてパースできること");
            let inner = raw(128);
            let b = BipolarHandle::new(&inner);
            b.set(parsed.clamp(b.min(), b.max()));
            assert_eq!(inner.value(), raw_value, "表示{shown}が生値{raw_value}へ戻らない");
        }
    }
}
