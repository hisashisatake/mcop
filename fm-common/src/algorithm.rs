// ---------------------------------------------------------------------------
// アルゴリズム結線テーブル
//
// 8アルゴリズムの結線（モジュレーター→ターゲット、キャリア、フィードバック対象）の
// 具体値は、ymfm（BSD-3-Clause、https://github.com/aaronsgiles/ymfm）の
// `src/ymfm_fm.ipp` 内 `s_algorithm_ops` テーブル（`fm_channel<RegisterType>::output_4op`
// が参照するOPN系4opアルゴリズム定義）を基に移植した。
// ライセンス全文・出典の詳細は THIRD_PARTY_NOTICES.md を参照。
// ---------------------------------------------------------------------------

/// (モジュレーター op index → ターゲット op index)。op indexは0〜3。
pub type ModRoute = (usize, usize);

pub struct AlgorithmDef {
    /// モジュレーター→ターゲットの変調経路一覧。同一ターゲットへの複数経路は加算する。
    pub routes: &'static [ModRoute],
    /// フィードバック（自己変調）を持つオペレーターのインデックス。
    pub feedback_op: usize,
    /// チャンネル出力に直接合算されるオペレーター（キャリア）。
    pub carriers: &'static [usize],
    /// 評価順（モジュレーター→キャリアの順）。
    pub eval_order: [usize; 4],
}

/// 8アルゴリズムの結線テーブル（ymfm OPN系4opアルゴリズム準拠、operator 1〜4を0〜3に対応）。
pub const ALGORITHMS: [AlgorithmDef; 8] = [
    // 0: O1 -> O2 -> O3 -> O4
    AlgorithmDef {
        routes: &[(0, 1), (1, 2), (2, 3)],
        feedback_op: 0,
        carriers: &[3],
        eval_order: [0, 1, 2, 3],
    },
    // 1: (O1 + O2) -> O3 -> O4
    AlgorithmDef {
        routes: &[(0, 2), (1, 2), (2, 3)],
        feedback_op: 0,
        carriers: &[3],
        eval_order: [0, 1, 2, 3],
    },
    // 2: (O1 + (O2 -> O3)) -> O4
    AlgorithmDef {
        routes: &[(0, 3), (1, 2), (2, 3)],
        feedback_op: 0,
        carriers: &[3],
        eval_order: [0, 1, 2, 3],
    },
    // 3: ((O1 -> O2) + O3) -> O4
    AlgorithmDef {
        routes: &[(0, 1), (1, 3), (2, 3)],
        feedback_op: 0,
        carriers: &[3],
        eval_order: [0, 1, 2, 3],
    },
    // 4: (O1 -> O2) + (O3 -> O4)
    AlgorithmDef {
        routes: &[(0, 1), (2, 3)],
        feedback_op: 0,
        carriers: &[1, 3],
        eval_order: [0, 1, 2, 3],
    },
    // 5: (O1 -> O2) + (O1 -> O3) + (O1 -> O4)
    AlgorithmDef {
        routes: &[(0, 1), (0, 2), (0, 3)],
        feedback_op: 0,
        carriers: &[1, 2, 3],
        eval_order: [0, 1, 2, 3],
    },
    // 6: (O1 -> O2) + O3 + O4
    AlgorithmDef {
        routes: &[(0, 1)],
        feedback_op: 0,
        carriers: &[1, 2, 3],
        eval_order: [0, 1, 2, 3],
    },
    // 7: O1 + O2 + O3 + O4（全並列）
    AlgorithmDef {
        routes: &[],
        feedback_op: 0,
        carriers: &[0, 1, 2, 3],
        eval_order: [0, 1, 2, 3],
    },
];

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_algorithms_have_non_empty_carriers() {
        for (i, algo) in ALGORITHMS.iter().enumerate() {
            assert!(!algo.carriers.is_empty(), "algorithm {i} has no carriers");
        }
    }

    #[test]
    fn all_algorithms_have_valid_eval_order() {
        for (i, algo) in ALGORITHMS.iter().enumerate() {
            let mut sorted = algo.eval_order;
            sorted.sort();
            assert_eq!(sorted, [0, 1, 2, 3], "algorithm {i} eval_order is not a permutation of 0..4");
        }
    }

    #[test]
    fn all_indices_within_range_and_routes_forward() {
        for (i, algo) in ALGORITHMS.iter().enumerate() {
            assert!(algo.feedback_op < 4, "algorithm {i} feedback_op out of range");
            for &c in algo.carriers {
                assert!(c < 4, "algorithm {i} carrier index out of range");
            }
            for &(from, to) in algo.routes {
                assert!(from < 4 && to < 4, "algorithm {i} route index out of range");
                assert!(from < to, "algorithm {i} route {from}->{to} should go from lower to higher index");
            }
        }
    }

    #[test]
    fn eval_order_respects_route_dependencies() {
        for (i, algo) in ALGORITHMS.iter().enumerate() {
            let mut position = [0usize; 4];
            for (pos, &op) in algo.eval_order.iter().enumerate() {
                position[op] = pos;
            }
            for &(from, to) in algo.routes {
                assert!(position[from] < position[to], "algorithm {i}: route {from}->{to} violates eval order");
            }
        }
    }

    #[test]
    fn algorithm_7_is_all_parallel() {
        let algo = &ALGORITHMS[7];
        assert!(algo.routes.is_empty());
        assert_eq!(algo.carriers, &[0, 1, 2, 3]);
    }
}
