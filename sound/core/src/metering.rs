// ---------------------------------------------------------------------------
// 計測データの受け渡し基盤（オーディオスレッド ⇄ GUIスレッド）
// ---------------------------------------------------------------------------
//
// `MasterOutput::take_measurement()`が計測した`Measurement`をGUIへ届けるための
// 一方向の橋渡し。オーディオスレッドは`try_lock`のみを使い、ロックが取れなければ
// その回のpublishを諦める（既存の`SharedEditState`が`try_read()`を使う設計と同じ
// 「オーディオ側は絶対に待たない」パターン）。
//
// `AtomicU32`ではなく`Mutex`を使うのは、将来`Measurement`へオシロスコープ用の
// 波形（`Vec<f32>`）を追加したときも同じ型のまま拡張できるようにするため。

use std::sync::Mutex;

use crate::master_output::Measurement;

/// オーディオスレッドが書き、GUIスレッドが読む計測値の橋渡し。
pub struct MeterBridge {
    measurement: Mutex<Measurement>,
}

impl MeterBridge {
    pub fn new() -> Self {
        Self { measurement: Mutex::new(Measurement::default()) }
    }

    /// オーディオスレッド側。ロックが取れなければ何もせず戻る
    /// （GUIスレッドが読み取り中の稀なケース。次のブロックで再度publishされる）。
    pub fn publish(&self, m: &Measurement) {
        if let Ok(mut guard) = self.measurement.try_lock() {
            *guard = m.clone();
        }
    }

    /// GUIスレッド側。ブロックしてよい（GUI操作にリアルタイム制約は無い）。
    pub fn read(&self) -> Measurement {
        self.measurement.lock().unwrap().clone()
    }

    /// GUIスレッド側。クリップランプのクリックによる手動リセット。
    pub fn reset_clip(&self) {
        self.measurement.lock().unwrap().clipped = false;
    }
}

impl Default for MeterBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_measurement_is_zero() {
        let bridge = MeterBridge::new();
        assert_eq!(bridge.read(), Measurement::default());
    }

    #[test]
    fn publish_then_read_round_trips() {
        let bridge = MeterBridge::new();
        let m = Measurement { peak_l: 0.5, peak_r: 0.7, clipped: true };
        bridge.publish(&m);
        assert_eq!(bridge.read(), m);
    }

    #[test]
    fn publish_is_skipped_when_lock_is_held() {
        let bridge = MeterBridge::new();
        let guard = bridge.measurement.lock().unwrap();

        // GUIスレッドがロックを保持している間にオーディオスレッドがpublishしても、
        // try_lockに失敗して何も起きない（パニックしない、ブロックしない）。
        let m = Measurement { peak_l: 0.9, peak_r: 0.9, clipped: true };
        bridge.publish(&m);

        assert_eq!(*guard, Measurement::default(), "ロック保持中のpublishは反映されないはず");
        drop(guard);
    }

    #[test]
    fn reset_clip_clears_only_clipped_flag() {
        let bridge = MeterBridge::new();
        bridge.publish(&Measurement { peak_l: 0.3, peak_r: 0.4, clipped: true });

        bridge.reset_clip();

        let m = bridge.read();
        assert!(!m.clipped);
        assert_eq!(m.peak_l, 0.3, "クリップフラグ以外は保持されるはず");
        assert_eq!(m.peak_r, 0.4);
    }

    #[test]
    fn later_publish_overwrites_previous_value() {
        let bridge = MeterBridge::new();
        bridge.publish(&Measurement { peak_l: 0.1, peak_r: 0.1, clipped: false });
        bridge.publish(&Measurement { peak_l: 0.8, peak_r: 0.6, clipped: true });

        let m = bridge.read();
        assert_eq!(m.peak_l, 0.8);
        assert_eq!(m.peak_r, 0.6);
        assert!(m.clipped);
    }
}
