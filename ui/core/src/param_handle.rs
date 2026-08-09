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
}
