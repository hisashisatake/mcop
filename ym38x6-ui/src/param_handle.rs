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
