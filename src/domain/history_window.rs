//! **会話履歴の窓の始まり**を決める規則（ADR-0063 / 作業単位 A-17）。
//!
//! 接頭辞で会話キャッシュを再利用する上流では、前のターンの要求が次のターンの先頭に
//! そのまま残っているときだけ読み直さずに済む。窓の始まりが毎ターン進むと、
//! 先頭の system の直後が毎ターン変わり、再利用が全ターンで 0 になる（ADR-0063 §背景）。
//! だから**始まりは「区切り」の往復にだけ置く。**
//!
//! # 言葉
//!
//! * **往復の通し番号** —— そのセッションの利用者の発言を古い順に数えた番号（1 始まり）。
//!   DB の行から毎回数え直す（**状態を持たない** / §決定1）。
//! * **区切り** —— 通し番号 1, 11, 21, …（[`HISTORY_TURN_STEP`] ごと）。
//!
//! この規則を使うのは2か所で、**どちらもここの関数を呼ぶ**（写しを持たない）:
//!
//! | 場所 | 使うもの |
//! |---|---|
//! | 読む窓（`Store::recent_history`） | [`first_turn_to_read`] |
//! | 予算で削るときの切り上げ（`prompt::budget`） | [`a_boundary_lies_between`] と [`HISTORY_SNAP_MIN_FITTING_TURNS`] |
//!
//! **記憶の区切り**（ADR-0064 / 作業単位 A-15b）もここに置く。抽出で増えた記憶を本文に載せるターンを、
//! 窓が動くターンに揃えるため、位相を窓の規則から導く（[`MEMORY_BOUNDARY_PHASE`]）。
//! 使うのは本文の想起の境の時刻（`Store::recall_boundary`）で、[`memory_boundary_turn`] を呼ぶ。
//!
//! **I/O を持たない純粋関数**（CLAUDE.md 3節）。

/// **区切りの間隔**（往復の数）。利用者が選んだ（候補は 5・10・20 / ADR-0063 §決定1）。
pub const HISTORY_TURN_STEP: u32 = 10;

/// **予算で削るとき、区切りへ切り上げるのに要る「入る往復の数」の下限**（ADR-0063 §決定3 / 利用者の決定）。
///
/// 切り上げは、予算に入る最も古い往復から最大 `HISTORY_TURN_STEP − 1`（9）往復を落とす
/// （区切りは間隔ごとに1つあるので、それより先へは進まない）。入る往復が F なら、残るのは F − 9 〜 F。
/// **下限を `2 × HISTORY_TURN_STEP − 1`（19）にして、切り上げた後に必ず 10 往復以上が残るようにする。**
/// 下限が 10 だと、F = 10 のとき最新の1往復まで落ちる回が出る（レビューの指摘）。
/// これ未満なら丸めない（今までどおり毎ターン動く）。
pub const HISTORY_SNAP_MIN_FITTING_TURNS: u32 = 2 * HISTORY_TURN_STEP - 1;

/// 切り上げた後に必ず `HISTORY_TURN_STEP` 往復以上が残ること（切り上げが落とすのは最大 `STEP − 1`）。
const _: () = assert!(
    HISTORY_SNAP_MIN_FITTING_TURNS >= 2 * HISTORY_TURN_STEP - 1,
    "丸める下限は 2 × 区切りの間隔 − 1 以上（切り上げた後に 10 往復以上残る）"
);

/// **読む窓が少なくとも含む往復の数**（ADR-0063 §決定2）。
///
/// 窓は「直近 25 往復を含む区切りのうち、最も新しいもの」から読むので、
/// 往復の数は `HISTORY_MIN_TURNS` 〜 `HISTORY_MIN_TURNS + HISTORY_TURN_STEP - 1`（25〜34）を行き来する。
pub const HISTORY_MIN_TURNS: u32 = 25;

/// **DB から読む件数の天井**（メッセージの件数 / ADR-0063 §決定2「読む件数に天井を残す」）。
///
/// 往復の形に揃った窓は最大 `2 × 34 = 68` 件。そこに、**数えてから読むまでの間に
/// 同じセッションへ書き足される往復**（並行するターン）と、往復の形になっていない古い行の分の
/// 余裕を足して 100 にした。
///
/// **外したとき壊れるか**（ADR-0036）: 天井に当たると、窓の頭が区切りより新しい側で切れる
/// （そのターンは区切りから始まらず、会話キャッシュが効かないだけ）。プロンプトは壊れない ——
/// 往復の形は `prompt::history_turns` が揃え、予算は `prompt::budget` が守る。
pub const HISTORY_READ_CAP: u32 = 100;

/// 天井は、往復の形に揃った窓を必ず丸ごと読めること。
const _: () = assert!(
    HISTORY_READ_CAP >= 2 * (HISTORY_MIN_TURNS + HISTORY_TURN_STEP - 1),
    "読む件数の天井は、最も長い窓（34 往復）を丸ごと読めること"
);

/// 間隔は 1 以上（0 で割らない）、窓の下限は間隔以上（窓の中に区切りが少なくとも1つ入る）。
const _: () = assert!(
    HISTORY_TURN_STEP > 0 && HISTORY_MIN_TURNS >= HISTORY_TURN_STEP,
    "区切りの間隔は 1 以上で、窓の下限より大きくないこと"
);

/// **往復が `turns` 個あるセッションで、読み始める往復の通し番号**（1 始まり）。
///
/// 「`turns − (HISTORY_MIN_TURNS − 1)` 以下で最大の区切り」。`turns` が
/// [`HISTORY_MIN_TURNS`] 未満なら 1（全部読む）。`turns` が 0 でも 1 を返す（読む往復が無いだけ）。
///
/// 返る番号から `turns` までの往復の数は、`turns ≥ HISTORY_MIN_TURNS` なら 25〜34。
#[must_use]
pub const fn first_turn_to_read(turns: u32) -> u32 {
    if turns < HISTORY_MIN_TURNS {
        return 1;
    }
    // 直近 25 往復を含む始まりのうち最も新しいもの（≥ 1）。
    let latest = turns - (HISTORY_MIN_TURNS - 1);
    // それ以下で最大の区切り（1, 11, 21, …）。
    (latest - 1) / HISTORY_TURN_STEP * HISTORY_TURN_STEP + 1
}

/// **通し番号 `previous` の往復の直後に、通し番号 `current` の往復が続くとき、
/// `current` の往復が区切りの往復か**（ADR-0063 §決定1・§決定3）。
///
/// `previous < a ≤ current` となる区切り `a`（1, 11, 21, …）があれば真。
/// 区切りの往復が空白で落ちていても（`prompt::history_turns` は空白の往復を相方ごと落とす）、
/// その後ろで最初に残った往復が真になる —— `current − previous` が 1 より大きくてよい。
///
/// `current ≤ previous`（番号が進まない並び）なら偽。
#[must_use]
pub const fn a_boundary_lies_between(previous: u32, current: u32) -> bool {
    boundaries_through(current) > boundaries_through(previous)
}

/// 通し番号 `turn` 以下にある区切りの数（`turn` が 0 なら 0）。
const fn boundaries_through(turn: u32) -> u32 {
    match turn {
        0 => 0,
        _ => (turn - 1) / HISTORY_TURN_STEP + 1,
    }
}

/// **記憶の区切りの位相**（ADR-0064 §決定1 / 作業単位 A-15b）。
///
/// 抽出で増えた記憶を本文に載せるのは、往復の数 N が `MEMORY_BOUNDARY_PHASE`
/// （mod [`HISTORY_TURN_STEP`]）になったターン（5, 15, 25, …）だけ。
///
/// **位相は窓の規則から導く。** 件数で決まる窓が動くのは
/// `first_turn_to_read(N)` が変わるターン ＝ N ≡ [`HISTORY_MIN_TURNS`]（mod 間隔）のターン
/// （N = 35, 45, …）なので、同じ位相にすると、窓が動くターンと記憶が載るターンが重なる
/// （どちらも、そのターンに全部を読み直す —— 読み直しの回数が増えない）。
/// 重なることの見張り番は `every_turn_that_moves_the_history_window_is_a_memory_boundary`。
pub const MEMORY_BOUNDARY_PHASE: u32 = HISTORY_MIN_TURNS % HISTORY_TURN_STEP;

/// **往復が `turns` 個あるセッションで、記憶の境になる往復の通し番号**（ADR-0064 §決定1）。
///
/// `turns` 以下で最大の r（r ≥ 1、r ≡ [`MEMORY_BOUNDARY_PHASE`]（mod [`HISTORY_TURN_STEP`])）。
/// 無ければ（`turns` < 5）`None` —— そのときの境はセッションの作成時刻（`Store::recall_boundary`）。
///
/// `turns` は今回の発言を含めない往復の数（`Store::recent_history` と同じ数え方 / ADR-0063 §決定2）。
#[must_use]
pub const fn memory_boundary_turn(turns: u32) -> Option<u32> {
    // `turns` から r まで戻る数（0〜間隔−1）。位相は間隔より小さいので、引き算は溢れない。
    let behind =
        (turns % HISTORY_TURN_STEP + HISTORY_TURN_STEP - MEMORY_BOUNDARY_PHASE) % HISTORY_TURN_STEP;
    match turns.checked_sub(behind) {
        Some(0) | None => None,
        Some(turn) => Some(turn),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **読み始めは 10 往復に1回だけ変わり、変わった先は区切り**（往復の数を 0〜120 と伸ばす）。
    ///
    /// 期待値は関数の外の式で作らず、変わり目の表をリテラルで持つ。
    #[test]
    fn the_first_turn_to_read_moves_once_every_ten_turns() {
        // (往復の数, 読み始め)。変わり目の前後を全部置く。
        let table: [(u32, u32); 14] = [
            (0, 1),
            (1, 1),
            (24, 1),
            (25, 1),
            (34, 1),
            (35, 11),
            (44, 11),
            (45, 21),
            (54, 21),
            (55, 31),
            (60, 31),
            (64, 31),
            (65, 41),
            (120, 91),
        ];
        for (turns, expected) in table {
            assert_eq!(
                first_turn_to_read(turns),
                expected,
                "往復 {turns} 個のときの読み始め"
            );
        }

        let mut changes = 0;
        let mut previous = first_turn_to_read(0);
        for turns in 1..=120 {
            let first = first_turn_to_read(turns);
            if first != previous {
                changes += 1;
                assert_eq!(
                    first % HISTORY_TURN_STEP,
                    1,
                    "変わった先は区切り（{turns}）"
                );
                assert_eq!(
                    first,
                    previous + HISTORY_TURN_STEP,
                    "1区切りずつ進む（{turns}）"
                );
            }
            if turns >= HISTORY_MIN_TURNS {
                let window = turns - first + 1;
                assert!(
                    (25..=34).contains(&window),
                    "往復 {turns} 個の窓は 25〜34 往復（いま {window}）"
                );
            } else {
                assert_eq!(first, 1, "往復 {turns} 個は全部読む");
            }
            previous = first;
        }
        // 35, 45, …, 115 で変わる。
        assert_eq!(changes, 9, "120 往復までに変わるのは 9 回");
    }

    /// **記憶の区切りの往復は、往復の数 N 以下で最大の 5, 15, 25, …**（ADR-0064 §決定1）。
    ///
    /// 期待値は関数の外の式で作らず、リテラルの表で持つ。N < 5 は区切りが無い（`None`）。
    #[test]
    fn the_memory_boundary_turn_is_the_latest_five_mod_ten_at_or_below_the_turns() {
        let table: [(u32, Option<u32>); 18] = [
            (0, None),
            (1, None),
            (4, None),
            (5, Some(5)),
            (6, Some(5)),
            (14, Some(5)),
            (15, Some(15)),
            (16, Some(15)),
            (24, Some(15)),
            (25, Some(25)),
            (34, Some(25)),
            (35, Some(35)),
            (44, Some(35)),
            (45, Some(45)),
            (104, Some(95)),
            (105, Some(105)),
            // 上端でも溢れない（4294967295 は 5 mod 10）。
            (u32::MAX - 1, Some(4_294_967_285)),
            (u32::MAX, Some(u32::MAX)),
        ];
        for (turns, expected) in table {
            assert_eq!(
                memory_boundary_turn(turns),
                expected,
                "往復 {turns} 個のときの記憶の区切り"
            );
        }
    }

    /// **件数で決まる窓が動くターンは、必ず記憶の区切りのターンでもある**（ADR-0064 §決定1）。
    ///
    /// 窓が動くターン（`first_turn_to_read` が変わる N = 35, 45, …）で、記憶の区切りが N そのもの
    /// であること。位相 5 は `HISTORY_MIN_TURNS mod HISTORY_TURN_STEP` から導いている
    /// （[`MEMORY_BOUNDARY_PHASE`]）。窓の下限か間隔を変えたとき、こちらだけ古い位相のまま
    /// 残ると、ここが赤になる。
    #[test]
    fn every_turn_that_moves_the_history_window_is_a_memory_boundary() {
        assert_eq!(
            MEMORY_BOUNDARY_PHASE,
            HISTORY_MIN_TURNS % HISTORY_TURN_STEP,
            "位相は窓の下限と間隔から導く"
        );
        let mut moves = 0;
        for turns in 1..=200 {
            if first_turn_to_read(turns) != first_turn_to_read(turns - 1) {
                moves += 1;
                assert_eq!(
                    memory_boundary_turn(turns),
                    Some(turns),
                    "窓が動く往復 {turns} 個のターンは記憶の区切り"
                );
            }
        }
        // 35, 45, …, 195 の 17 回（窓が1回も動かないと、上の主張は何も見ない）。
        assert_eq!(moves, 17, "200 往復までに窓が動くのは 17 回");
        // 区切りは窓が動くターンより多い（5・15・25 は窓が動かない区切り）。
        let boundaries = (1..=200)
            .filter(|&turns| memory_boundary_turn(turns) == Some(turns))
            .count();
        assert_eq!(
            boundaries, 20,
            "200 往復までの記憶の区切りは 5, 15, …, 195 の 20 回"
        );
    }

    /// **区切りをまたぐかどうか**（隣り合う番号・飛んだ番号・またがない飛び・進まない並び）。
    #[test]
    fn a_boundary_lies_between_two_turns_only_when_one_is_crossed() {
        // 隣り合う番号: 区切り 11 / 21 の手前と後。
        assert!(a_boundary_lies_between(10, 11));
        assert!(a_boundary_lies_between(20, 21));
        assert!(!a_boundary_lies_between(11, 12));
        assert!(!a_boundary_lies_between(9, 10));
        // 区切りの往復（11）が空白で落ちた形: 10 の次が 12。
        assert!(a_boundary_lies_between(10, 12));
        // 区切りを2つまたぐ飛び。
        assert!(a_boundary_lies_between(5, 25));
        // またがない飛び（12 → 20）。
        assert!(!a_boundary_lies_between(12, 20));
        // 最初の区切り（1）: 0 の後の 1。
        assert!(a_boundary_lies_between(0, 1));
        // 進まない並び。
        assert!(!a_boundary_lies_between(11, 11));
        assert!(!a_boundary_lies_between(21, 11));
        // 上端でも溢れない。
        assert!(!a_boundary_lies_between(u32::MAX - 1, u32::MAX));
        assert!(a_boundary_lies_between(1, u32::MAX));
    }
}
