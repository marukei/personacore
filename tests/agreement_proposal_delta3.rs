//! **裏取り: 記録済みの δ3 の通しを、Rust の依頼の組み立て・読み・票・後段に通す**
//! （ADR-0051 §軽い形の模索の結果と、実装する形 / 作業単位 A-12c-1）。
//!
//! **`#[ignore]`。測定の置き場所が要る**（会話の写しは私的なので、リポジトリには入れない）:
//!
//! ```text
//! PERSONACORE_B60X_DIR=<measurements>\b60x-99981812 \
//!   cargo test --test agreement_proposal_delta3 -- --ignored --nocapture
//! ```
//!
//! **出すのは件数と不一致の位置（通し・往復・候補の番号）だけ。会話の中身・引用・提案の中身は出さない。**
//!
//! # 突き合わせるもの（δ3 の3通し —— 弥生 `yD3a`・翼 `tD3a`・翼 `tD3b`）
//!
//! `p1/tools/rp3.py`（`det=jd-v3 chk=c1-v1 prev=0 k=1 vote=2of3 mind=2 dedup=1 detlist=0`）の再生を、
//! **記録された応答をそのまま入れて**なぞる。
//!
//! | 何 | 記録の側 | Rust の側 |
//! |---|---|---|
//! | 検出の依頼（system と user） | `runs/<who>-<tag>/T###.json` の `det` の `messages` | `detection_request` |
//! | 確かめの依頼（引き直しも） | 同じ `requests` の `chk1` / `chk2` の `messages` | `fit_check_request`（上限 8192） |
//! | 2本目を引いたか | `chk2` があるか | `needs_another_draw` |
//! | 往復ごとの候補（引用） | `cands.tsv` の「印の往復」と「引用」 | `read_detection` → `detection_candidates` |
//! | 候補ごとの票・提案になったか | `cands.tsv` の「票」「提案」 | `votes_of` / `decide`（`DedupRule::Plain`） |
//! | **提案** | `proposals.tsv` の往復・印の往復・守る側・中身 | `decide` の `Proposed` |
//!
//! 最後に、同じ記録を **dedup2（`DedupRule::Stripped`）** で通したときに提案がどう変わるかも数える
//! （**近似** —— 提案が変わった後の往復では「# 提案済み」が記録と違うので、記録の応答は
//! 違う依頼への応答になる。依頼のバイトの突き合わせはしない）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use personacore::domain::{AgreementKeeper, ChatMessage, DisplayName};
use personacore::parse::agreement_proposal::{
    Candidate, CheckReading, DedupRule, KeeperReading, Outcome, Vote, decide, detection_candidates,
    needs_another_draw, read_check, read_detection, votes_of,
};
use personacore::prompt::agreement_proposal::{
    ListedProposal, Mark, ProposalTurn, detection_request, fit_check_request,
};
use serde_json::Value;

/// 測定の置き場所（`<measurements>\b60x-99981812`）。
const ENV_DIR: &str = "PERSONACORE_B60X_DIR";

/// 依頼の上限（`pc.PROMPT_LIMIT` —— 窓 16384 の半分）。
const PROMPT_LIMIT: u32 = 8192;

fn root() -> PathBuf {
    PathBuf::from(
        std::env::var(ENV_DIR)
            .unwrap_or_else(|_| panic!("{ENV_DIR} に測定の置き場所（b60x-99981812）を指すこと")),
    )
}

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("{} を読めること: {err}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("{} が JSON であること: {err}", path.display()))
}

struct Turn {
    t: u32,
    session: i64,
    user: String,
    assistant: String,
}

fn turns_of(root: &Path, who: &str) -> Vec<Turn> {
    read_json(&root.join("p1").join(who).join("turns.json"))
        .as_array()
        .expect("往復の配列")
        .iter()
        .map(|turn| Turn {
            t: u32::try_from(turn["t"].as_u64().expect("t")).expect("t"),
            session: turn["session"].as_i64().expect("session"),
            user: turn["user"].as_str().expect("user").to_owned(),
            assistant: turn["assistant"].as_str().expect("assistant").to_owned(),
        })
        .collect()
}

fn proposal_turn(turn: &Turn) -> ProposalTurn {
    ProposalTurn {
        number: turn.t,
        user: turn.user.clone(),
        character: turn.assistant.clone(),
    }
}

/// 記録の依頼（`messages`）と、組み直した依頼がバイト単位で同じか。
fn same_messages(recorded: &Value, rebuilt: &[ChatMessage]) -> bool {
    let recorded = recorded["messages"].as_array().expect("messages");
    recorded.len() == rebuilt.len()
        && recorded.iter().zip(rebuilt).all(|(left, right)| {
            left["content"].as_str() == Some(right.content.as_str())
                && left["role"].as_str()
                    == Some(match right.role {
                        personacore::domain::ChatRole::System => "system",
                        personacore::domain::ChatRole::User => "user",
                        personacore::domain::ChatRole::Assistant => "assistant",
                    })
        })
}

/// 票の綴り（`rp1.py` の `",".join(f"{v[0]}:{v[1]}" if v[1] else v[0] ...)`）。
fn vote_spelling(vote: &Vote) -> String {
    match vote {
        Vote::Keep(keeper) => format!("残す:{}", keeper_spelling(keeper)),
        Vote::Drop => "落とす".to_owned(),
        Vote::Existing => "既存".to_owned(),
        Vote::Unjudged => "無判定".to_owned(),
    }
}

fn keeper_spelling(keeper: &KeeperReading) -> String {
    match keeper {
        KeeperReading::User => "利用者".to_owned(),
        KeeperReading::Character => "キャラクター".to_owned(),
        KeeperReading::Both => "両方".to_owned(),
        KeeperReading::Unclear(raw) => format!("曖昧:{raw}"),
        KeeperReading::Missing => "-".to_owned(),
    }
}

/// 候補（Rust の側）。記録の `cands` と同じ欄を持つ。
struct Held {
    q: u32,
    candidate: Candidate,
    votes: Option<String>,
    checked_at: Option<u32>,
    proposed: bool,
    subject_mismatch: bool,
}

/// Rust の側で出た提案。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Proposed {
    t: u32,
    q: u32,
    keeper: String,
    content: String,
}

#[derive(Default)]
struct Tally {
    det_requests: usize,
    det_mismatch: Vec<u32>,
    chk_requests: usize,
    chk_mismatch: Vec<(u32, u32)>,
    second_draw_mismatch: Vec<(u32, u32)>,
    unlistable: usize,
}

/// 1つの通しを再生する。`check_bytes` が真なら、依頼のバイトを記録と突き合わせる。
fn replay(
    root: &Path,
    who: &str,
    tag: &str,
    name: &str,
    dedup: DedupRule,
    check_bytes: bool,
) -> (Vec<Held>, Vec<Proposed>, Tally) {
    let display = DisplayName::parse(name).expect("表示名");
    let turns = turns_of(root, who);
    let by_t: BTreeMap<u32, &Turn> = turns.iter().map(|turn| (turn.t, turn)).collect();
    let dir = root.join("p1").join("runs").join(format!("{who}-{tag}"));
    let mut held: Vec<Held> = Vec::new();
    let mut proposals: Vec<Proposed> = Vec::new();
    let mut tally = Tally::default();

    for (index, turn) in turns.iter().enumerate() {
        let t = turn.t;
        let last_of_session = turns
            .get(index + 1)
            .is_none_or(|next| next.session != turn.session);
        let record = read_json(&dir.join(format!("T{t:03}.json")));
        let requests = record["requests"].as_array().expect("requests");
        let mut next_request = 0;

        // ---- 検出 ----
        assert!(
            record.get("det_skipped").is_none(),
            "{who}-{tag} T{t}: 検出を飛ばした回は無い前提"
        );
        let det = &requests[next_request];
        next_request += 1;
        assert_eq!(det["kind"], "det");
        tally.det_requests += 1;
        if check_bytes && !same_messages(det, &detection_request(&display, &proposal_turn(turn))) {
            tally.det_mismatch.push(t);
        }
        let reading = read_detection(det["content"].as_str().unwrap_or(""), name);
        let found = detection_candidates(&reading, &turn.user, &turn.assistant);
        for candidate in found.candidates {
            held.push(Held {
                q: t,
                candidate,
                votes: None,
                checked_at: None,
                proposed: false,
                subject_mismatch: false,
            });
        }

        // ---- 確かめ（1往復前の候補。会話の最後の往復なら、その往復の候補も） ----
        let mut groups: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for (at, item) in held.iter().enumerate() {
            if item.checked_at.is_none() && (t - item.q >= 1 || last_of_session) {
                groups.entry(item.q).or_default().push(at);
            }
        }
        for (q, members) in groups {
            let window: Vec<ProposalTurn> = (q..=t.min(q + 1))
                .filter_map(|x| by_t.get(&x))
                .filter(|x| x.session == turn.session)
                .map(|x| proposal_turn(x))
                .collect();
            let marks: Vec<Mark<'_>> = members
                .iter()
                .map(|at| Mark {
                    turn: q,
                    from_user: held[*at].candidate.from_user,
                    quote: &held[*at].candidate.quote,
                })
                .collect();
            let listed: Vec<ListedProposal<'_>> = proposals
                .iter()
                .rev()
                .filter_map(|p| {
                    let keeper = match p.keeper.as_str() {
                        "利用者" => AgreementKeeper::User,
                        "キャラクター" => AgreementKeeper::Character,
                        "両方" => AgreementKeeper::Both,
                        _ => return None,
                    };
                    Some(ListedProposal {
                        keeper,
                        content: &p.content,
                    })
                })
                .collect();
            if listed.len() != proposals.len() {
                tally.unlistable += 1;
            }
            let fitted = fit_check_request(&display, &window, &marks, &listed, PROMPT_LIMIT);
            let Some(fitted) = fitted else {
                // 記録の側も飛ばしているはず（依頼が無い）。
                for at in &members {
                    held[*at].checked_at = Some(t);
                }
                continue;
            };
            let first = &requests[next_request];
            next_request += 1;
            assert_eq!(first["kind"], "chk1", "{who}-{tag} T{t}");
            tally.chk_requests += 1;
            if check_bytes && !same_messages(first, &fitted.messages) {
                tally.chk_mismatch.push((t, q));
            }
            let candidates: Vec<Candidate> = members
                .iter()
                .map(|at| held[*at].candidate.clone())
                .collect();
            let mut draws: Vec<CheckReading> = vec![read_check(
                first["content"].as_str().unwrap_or(""),
                name,
                candidates.len(),
            )];
            let recorded_second = requests
                .get(next_request)
                .is_some_and(|request| request["kind"] == "chk2");
            let wanted_second = needs_another_draw(&candidates, &draws[0]);
            if recorded_second != wanted_second {
                tally.second_draw_mismatch.push((t, q));
            }
            if recorded_second {
                let second = &requests[next_request];
                next_request += 1;
                tally.chk_requests += 1;
                if check_bytes && !same_messages(second, &fitted.messages) {
                    tally.chk_mismatch.push((t, q));
                }
                draws.push(read_check(
                    second["content"].as_str().unwrap_or(""),
                    name,
                    candidates.len(),
                ));
            }
            let existing: Vec<&str> = proposals.iter().map(|p| p.content.as_str()).collect();
            let decided = decide(&candidates, &draws, name, &existing, dedup);
            let mut fresh = Vec::new();
            for (offset, (at, outcome)) in members.iter().zip(&decided.outcomes).enumerate() {
                let number = offset + 1;
                let verdicts: Vec<_> = draws
                    .iter()
                    .map(|draw| draw.by_number.get(&number))
                    .collect();
                let (votes, _) = votes_of(&held[*at].candidate, &verdicts);
                held[*at].votes = Some(
                    votes
                        .iter()
                        .map(vote_spelling)
                        .collect::<Vec<_>>()
                        .join(","),
                );
                held[*at].checked_at = Some(t);
                match outcome {
                    Outcome::Proposed { keeper, content } => {
                        held[*at].proposed = true;
                        fresh.push(Proposed {
                            t,
                            q,
                            keeper: keeper_spelling(keeper),
                            content: content.clone(),
                        });
                    }
                    Outcome::SubjectMismatch => held[*at].subject_mismatch = true,
                    Outcome::Deduped | Outcome::NotKept => {}
                }
            }
            proposals.extend(fresh);
        }
        assert_eq!(
            next_request,
            requests.len(),
            "{who}-{tag} T{t}: 記録の依頼を全部なぞったこと"
        );
    }
    (held, proposals, tally)
}

fn recorded_rows(path: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("{} を読めること: {err}", path.display()))
        .lines()
        .skip(1)
        .map(|line| line.split('\t').map(str::to_owned).collect())
        .collect()
}

/// 1つの通しを突き合わせ、不一致の数を返す（**件数と位置だけを出す**）。
fn compare(root: &Path, who: &str, tag: &str, name: &str) -> usize {
    let (held, proposals, tally) = replay(root, who, tag, name, DedupRule::Plain, true);
    let dir = root.join("p1").join("runs").join(format!("{who}-{tag}"));
    let mut mismatches = 0;

    // 依頼のバイト。
    println!(
        "{who}-{tag}: 検出の依頼 {} 本（バイト不一致 {}）・確かめの依頼 {} 本（バイト不一致 {}）・2本目の判定の不一致 {}・載せられない提案 {}",
        tally.det_requests,
        tally.det_mismatch.len(),
        tally.chk_requests,
        tally.chk_mismatch.len(),
        tally.second_draw_mismatch.len(),
        tally.unlistable
    );
    if !tally.det_mismatch.is_empty() {
        println!("  検出の依頼が違う往復: {:?}", tally.det_mismatch);
    }
    if !tally.chk_mismatch.is_empty() {
        println!(
            "  確かめの依頼が違う (往復, 印の往復): {:?}",
            tally.chk_mismatch
        );
    }
    if !tally.second_draw_mismatch.is_empty() {
        println!(
            "  2本目の判定が違う (往復, 印の往復): {:?}",
            tally.second_draw_mismatch
        );
    }
    mismatches += tally.det_mismatch.len()
        + tally.chk_mismatch.len()
        + tally.second_draw_mismatch.len()
        + tally.unlistable;

    // 候補（cands.tsv）。
    let cands = recorded_rows(&dir.join("cands.tsv"));
    let mut candidate_mismatch = Vec::new();
    if cands.len() != held.len() {
        println!(
            "  候補の数が違う: 記録 {} / Rust {}",
            cands.len(),
            held.len()
        );
        mismatches += 1;
    }
    for (index, (row, item)) in cands.iter().zip(&held).enumerate() {
        let q: u32 = row[1].parse().expect("印の往復");
        let field = if item.candidate.from_user {
            "利用者"
        } else {
            "キャラクター"
        };
        let flag = if item.proposed {
            "提案"
        } else if item.subject_mismatch {
            "主語不一致"
        } else {
            "-"
        };
        let same = q == item.q
            && row[2] == field
            && row[9] == item.candidate.quote
            && row[6]
                == item
                    .checked_at
                    .map_or_else(|| "-".to_owned(), |t| t.to_string())
            && row[7] == item.votes.clone().unwrap_or_default()
            && row[8] == flag;
        if !same {
            candidate_mismatch.push(index + 1);
        }
    }
    println!(
        "  候補 {} 件（引用・欄・確かめた往復・票・提案になったかの不一致 {}）",
        held.len(),
        candidate_mismatch.len()
    );
    if !candidate_mismatch.is_empty() {
        println!("  不一致の候補の番号: {candidate_mismatch:?}");
    }
    mismatches += candidate_mismatch.len();

    // 提案（proposals.tsv）。
    let recorded: Vec<Proposed> = recorded_rows(&dir.join("proposals.tsv"))
        .iter()
        .map(|row| Proposed {
            t: row[0].parse().expect("提案の往復"),
            q: row[1].parse().expect("印の往復"),
            keeper: row[3].clone(),
            content: row[4].clone(),
        })
        .collect();
    let differing = recorded.len().abs_diff(proposals.len())
        + recorded
            .iter()
            .zip(&proposals)
            .filter(|(left, right)| left != right)
            .count();
    println!(
        "  提案: 記録 {} 件 / Rust {} 件（不一致 {}）",
        recorded.len(),
        proposals.len(),
        differing
    );
    mismatches += differing;

    // dedup2 で通したら（近似）。
    let (_, stripped, _) = replay(root, who, tag, name, DedupRule::Stripped, false);
    let added: Vec<(u32, u32)> = stripped
        .iter()
        .filter(|p| !proposals.contains(p))
        .map(|p| (p.t, p.q))
        .collect();
    let removed: Vec<(u32, u32)> = proposals
        .iter()
        .filter(|p| !stripped.contains(p))
        .map(|p| (p.t, p.q))
        .collect();
    println!(
        "  dedup2 で通すと（近似）: 提案 {} 件・増えた (往復, 印の往復) {added:?}・消えた {removed:?}",
        stripped.len()
    );
    mismatches
}

/// **記録済みの δ3 の3通しが、Rust の組み立て・読み・票・後段と一致する**（`DedupRule::Plain` で）。
#[test]
#[ignore = "測定の置き場所（PERSONACORE_B60X_DIR）が要る。会話の写しはリポジトリに入れない"]
fn the_recorded_delta3_runs_agree_with_rp3() {
    let root = root();
    let mut total = 0;
    for (who, tag, name) in [
        ("yayoi", "yD3a", "弥生"),
        ("tsubasa", "tD3a", "翼"),
        ("tsubasa", "tD3b", "翼"),
    ] {
        total += compare(&root, who, tag, name);
    }
    assert_eq!(total, 0, "不一致の合計");
}
