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

/// 真偽パラメーターへのハンドル。
pub trait BoolParamHandle {
    fn value(&self) -> bool;
    fn begin_edit(&self);
    fn set(&self, value: bool);
    fn end_edit(&self);
}

/// `TimeEgParams`（N点Time/Level方式EG、OP505用）1本ぶんへのハンドル。
/// 段×フィールドごとに`IntParamHandle`を196個(28値×7本)構築する代わりに、EG単位で1個
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
        }
    }
    fn min(&self) -> i32 {
        0
    }
    fn max(&self) -> i32 {
        match self.field {
            TimeEgIntField::SyncRate => 255,
            TimeEgIntField::RetriggerMode => 1,
        }
    }
    fn default(&self) -> i32 {
        match self.field {
            TimeEgIntField::SyncRate => sound_core::sync_note_anchor(10) as i32,
            TimeEgIntField::RetriggerMode => sound_core::RETRIGGER_MODE_CONTINUE as i32,
        }
    }
    fn name(&self) -> String {
        let suffix = match self.field {
            TimeEgIntField::SyncRate => "Sync Rate",
            TimeEgIntField::RetriggerMode => "Retrigger",
        };
        format!("{} {}", self.eg.name(), suffix)
    }
    /// `sync_rate`は生の0〜255ではなく音価名で見せる（ノブのツールチップ／数値欄用）。
    /// アンカーから外れているときは`~1/8`のようにチルダを付けて近似であることを示す。
    fn display(&self) -> String {
        match self.field {
            TimeEgIntField::SyncRate => crate::selector::sync_rate_display(self.value() as u8),
            TimeEgIntField::RetriggerMode => self.value().to_string(),
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
        }
        self.eg.set_params(params);
    }
    fn end_edit(&self) {
        self.eg.end_edit();
    }
}
