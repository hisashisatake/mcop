// ---------------------------------------------------------------------------
// 質感LFO適用先（FmLfoDestination）
//
// `TextureLfo`本体はVCO実装に依存しないためsound-coreへ移設済み。ここに残るのは
// FM合成チップ固有の適用先解釈のみ（`ym38x6-core`/`op505-core`共通で使う）。
// ---------------------------------------------------------------------------

/// パフォーマンスLFOの適用先。共通Destination（Pitch/Volume）に加え、
/// FM合成チップ共通の拡張Destination（TLキャリア一括、Cutoff）を持つ。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum FmLfoDestination {
    #[default]
    Pitch,
    Volume,
    TlCarrier,
    /// フィルターCutoffへの持続的な変調（オートワウ）。Filter EG Depth（キーオン一発の変調）
    /// とは独立に積み重なる（`Channel::tick`でLFOがシフトした基準Cutoffを、Filter EGがさらに変調する）。
    Cutoff,
    /// どこにも接続されていない（質感LFOパッチベイでケーブルをTEXTURE LFOパネル自身へ
    /// ドロップした状態）。LFOは`tick`し続けるが、いずれの変調ターゲットへも出力しない。
    Unplugged,
}

impl FmLfoDestination {
    /// 0〜255からの変換（質感LFOの`Destination`フィールド用、`FilterType::from_u8`と同じ慣習）。
    /// 0=Pitch/1=Volume/2=TL（キャリア一括）/3=Cutoff/4=未接続/5以上=Pitchへフォールバック。
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Volume,
            2 => Self::TlCarrier,
            3 => Self::Cutoff,
            4 => Self::Unplugged,
            _ => Self::Pitch,
        }
    }
}
