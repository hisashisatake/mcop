// ---------------------------------------------------------------------------
// 質感LFO適用先（FmLfoDestination）
//
// `TextureLfo`本体はVCO実装に依存しないためsound-coreへ移設済み。ここに残るのは
// FM合成チップ固有の適用先解釈のみ（`ym38x6-core`/`op505-core`共通で使う）。
//
// **discriminantの数値はプロトコル値である**（2026-08-18並べ替え）。この値は
// Rust内部表現であると同時に、NRPN(0,0)の生値・`.op505`/`.38x6`ファイルの
// `destination`フィールドとしてシリアライズされる値でもある。並べ替えると
// 既存データ・DAWオートメーション・SMF内のNRPNデータエントリの意味が変わる。
//
// 旧: Pitch=0, Volume=1, TlCarrier=2, Cutoff=3, Unplugged=4
// 新: Unplugged=0, Pitch=1, Volume=2, TlCarrier=3, Cutoff=4
//
// 並べ替えの動機: 新規パッチの初期状態を「どこにも接続されていない」にしたい、
// という要望から、Unplugged自体をdiscriminant 0（＝新規パッチの素の状態）にした。
// 移行前調査（2026-08-18）で、リポジトリ内の`.op505`/`.38x6`全293ファイル・2034ブロックの
// `destination`は0（旧Pitch）か4（旧Unplugged）のみで、Volume/TlCarrier/Cutoffは1件も
// 使われておらず、`depth`も全て0（質感LFOは既存データのどこでも鳴っていない）と確認済み。
// そのため既存データの移行スクリプトは不要（旧0→新解釈でUnplugged、そのまま意図通り）。
// ---------------------------------------------------------------------------

/// パフォーマンスLFOの適用先。共通Destination（Pitch/Volume）に加え、
/// FM合成チップ共通の拡張Destination（TLキャリア一括、Cutoff）を持つ。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum FmLfoDestination {
    /// どこにも接続されていない（質感LFOパッチベイでケーブルをTEXTURE LFOパネル自身へ
    /// ドロップした状態）。LFOは`tick`し続けるが、いずれの変調ターゲットへも出力しない。
    /// 新規パッチの既定状態（discriminant=0）。
    #[default]
    Unplugged,
    Pitch,
    Volume,
    TlCarrier,
    /// フィルターCutoffへの持続的な変調（オートワウ）。Filter EG Depth（キーオン一発の変調）
    /// とは独立に積み重なる（`Channel::tick`でLFOがシフトした基準Cutoffを、Filter EGがさらに変調する）。
    Cutoff,
}

impl FmLfoDestination {
    /// 0〜255からの変換（質感LFOの`Destination`フィールド用、`FilterType::from_u8`と同じ慣習）。
    /// 0=未接続/1=Pitch/2=Volume/3=TL（キャリア一括）/4=Cutoff/5以上=Unplugged（未接続）へフォールバック
    /// （範囲外の値が勝手にどこかへ繋がるより、繋がらない方が安全側）。
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Pitch,
            2 => Self::Volume,
            3 => Self::TlCarrier,
            4 => Self::Cutoff,
            _ => Self::Unplugged,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// discriminantの数値がプロトコル値（NRPN(0,0)・ファイル形式）であることを固定する。
    /// ここが変わると、既存の`.op505`/`.38x6`ファイルとDAWオートメーションの意味が
    /// エラーも出ずに変わってしまう（2026-08-18の並べ替えで実際に踏んだ設計判断）。
    #[test]
    fn discriminant_values_match_protocol() {
        assert_eq!(FmLfoDestination::Unplugged as u8, 0);
        assert_eq!(FmLfoDestination::Pitch as u8, 1);
        assert_eq!(FmLfoDestination::Volume as u8, 2);
        assert_eq!(FmLfoDestination::TlCarrier as u8, 3);
        assert_eq!(FmLfoDestination::Cutoff as u8, 4);
    }

    #[test]
    fn default_is_unplugged() {
        assert_eq!(FmLfoDestination::default(), FmLfoDestination::Unplugged);
    }

    #[test]
    fn from_u8_round_trips_all_variants() {
        assert_eq!(FmLfoDestination::from_u8(0), FmLfoDestination::Unplugged);
        assert_eq!(FmLfoDestination::from_u8(1), FmLfoDestination::Pitch);
        assert_eq!(FmLfoDestination::from_u8(2), FmLfoDestination::Volume);
        assert_eq!(FmLfoDestination::from_u8(3), FmLfoDestination::TlCarrier);
        assert_eq!(FmLfoDestination::from_u8(4), FmLfoDestination::Cutoff);
    }

    /// 範囲外の値は「勝手にどこかへ繋がる」より「繋がらない」方が安全側、という設計判断
    /// （旧仕様はフォールバック先がPitchだった）。
    #[test]
    fn from_u8_out_of_range_falls_back_to_unplugged() {
        for v in [5u8, 6, 100, 255] {
            assert_eq!(FmLfoDestination::from_u8(v), FmLfoDestination::Unplugged, "value={v}");
        }
    }
}
