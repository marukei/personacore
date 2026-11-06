//! リポジトリ根の `README.md` が名指しするものを、実体と突き合わせる。
//!
//! 英語の README（`README.en.md`）にも、同じ走査の本体の一部を当てている
//! （作業単位 O-10 / Issue origin#104。何を当てて何を当てていないかは、ファイルの末尾の節の頭）。
//!
//! # なぜ要るか（作業単位 F-2）
//!
//! README は**リポジトリの外側にある写しのうち、いちばん最初に読まれるもの**である。
//! 案内する手順・引数・環境変数・既定値は、どれも**片方だけ変えても何も落ちない。**
//! 現れ方は「書いてあるとおりにやったのに、そうならない」という、いちばん人を消耗させる形になる。
//!
//! この家でいちばん多く出ている失敗は「**doc が、守られていない場所に『守った』と書く**」で、
//! **README はその最大の面**である。だから README には「実在する道」しか書かず、
//! **実在するかどうかをここが機械的に見る。**
//!
//! # 何を守っていないか（**先に書く**）
//!
//! * **これは README の手順を実行していない。** 綴りの突き合わせだけである。
//!   「`pwsh ./scripts/start.ps1` で立ち上がる」が**本当かどうかは、ここは一切見ていない**
//!   （F-1 の作業者とレビュアーが実際に走らせて確かめた範囲は
//!   `scripts/README-start.md` の「見張り番」節が持つ）。
//! * **⚠ 「使っているか」を見る節も、綴りの検査でしかない。**
//!   [`the_readme_only_names_options_the_start_script_declares_and_uses`] は
//!   `$名前` が宣言と使用で2回以上現れることしか見ていない
//!   （F-1 のレビューが「宣言だけを見ていた走査は、使用箇所を差し替える変異を全部通した」と
//!   実測したので、少なくとも**宣言だけでは緑にならない**ようにしてある）。
//!   **その行が効いていること**は、依然として見ていない。
//! * **引数の走査は「同じ行の中」しか見ない。** 目印（`start.ps1`）と引数が別の行に
//!   分かれて書かれると、その引数は**案内されていない扱い**になる（見逃す方向の限界で、
//!   [`LEAST_OPTIONS`] の床が「引数の案内が丸ごと消えたこと」だけを受け持つ）。
//! * **リンクは「行き先が在ること」しか見ていない。** 見出しの錨（`#…`）は落として読むので、
//!   **節が消えても鳴らない。** 表示文字列が中身と合っているかも見ていない。
//! * **散文の主張は1つも見ていない。** 「上流は起動しない」「外部公開しない」といった文は、
//!   ここでは検査できない（**綴りに落ちないものは、この道具の外**）。
//! * **`scripts/README-start.md` は走査していない。**
//!   **他の doc の写しは、原則としてここの範囲外**である。
//! * **⚠ 例外は [`PORT_WATCHED_LINKED_DOCUMENTS`] に挙げた文書だけ**
//!   （`web/README.md` を B-30 で / 2026-09-02、`docs/09-setup.md` を F-3 で / 2026-09-03）。
//!   **根の README がそれらをリンクしている**ので、
//!   **読者がたどった先には写しがあった**（F-2 のレビューの指摘）——
//!   `cd .. && cargo run       # http://127.0.0.1:8080/ で画面が出る`。
//!   **見ているのは既定ポートを写していないことだけ**で、
//!   その文書の他の主張は1つも見ていない
//!   （`docs/09-setup.md` の側は [tests/setup_doc.rs](../tests/setup_doc.rs) が別に見る）。
//!   帳簿は [`PORT_WATCHED_LINKED_DOCUMENTS`]、見張り番は
//!   [`the_documents_the_readme_links_to_point_at_the_default_port_instead_of_copying_it`]。
//!   **`5173`（Vite の dev server）は見ていない** ——理由はその見張り番の doc。
//! * **⚠ 床と帳簿そのものは、誰も見張っていない**（**変異検査で実測** / 2026-08-29）。
//!   **無防備なのは5つで、1つも例外は無い**（**5つ目は B-30 で増やした / 2026-09-02**）。
//!   挙げていないものが
//!   「守られている」と読まれるのを避けるため、**全部書く**:
//!
//!   | 緩める手 | 実測 |
//!   |---|---|
//!   | [`LEAST_LINKS`] を `0` に | **1797 全緑** |
//!   | [`LEAST_OPTIONS`] を `0` に | 同じ形（**実数 2 対 床 2 で余裕ゼロ**） |
//!   | [`LEAST_CARGO_COMMANDS`] を `0` に | 同じ形（**実数 4 対 床 4 で余裕ゼロ**） |
//!   | [`POINTED_CONSTANTS`] から列を落とす | **`DEFAULT_ENDPOINT` 側も `DEFAULT_PORT` 側も全緑**（レビューが両列を実測） |
//!   | [`PORT_WATCHED_LINKED_DOCUMENTS`] を空にする | **1827 全緑**（B-30 の作業者が実測） |
//!
//!   **緩めた瞬間から、その面は守られていない。**
//!   （床が死んでいるわけではない —— 走査の綴りを壊して 0 件にする変異では
//!   [`every_link_in_the_readme_points_at_something_that_exists`] が実際に赤になる。
//!   **床は効くが、床を下げる手を止めるものが無い。**）
//!   **受け入れているのは、床が受け持つのが「丸ごと消える」だけで、
//!   個々の突き合わせは床と無関係に全部生きているから**（司令塔の判断 / B-23 と同じ）。
//! * **ディレクトリを歩いていない。** 読むのは名指しされたファイルだけなので、
//!   `.claude/worktrees/` の隔離ワークツリーを判定に入れることは無い
//!   （B-23 のレビューが実測した壊れ方 / `src/prompt/safe.rs` の `SKIPPED_DIRECTORIES`）。
//!
//! # 走査の作法
//!
//! **走査の本体はすべてソースを引数で受け取る。** 実ファイルと、テストが植えたソースを
//! **同じ本体に通す**ため（CLAUDE.md `15b`「走査の対照は自己成就しうる」）。
//! 実ファイルだけを見る対照は、走査の綴りを壊す変異が
//! **壊した綴りを走査対象に書き込むだけで緑になる。**
//!
//! # ⚠ `tests/start_script.rs` と同じ形の本体が4つある
//!
//! `personacore_env_names` / `defined_env_names` / `ipv4_literals` / `occurrences` は、
//! あちらと同じ規則を別のソースに当てる。
//! **片方だけ規則が変わると、もう片方は黙って古い規則で見続ける** —— そういう写しである。
//!
//! **⚠ 「共有できる場所が無いから」ではない**（着手時にそう書いたが、事実ではなかった /
//! レビューが指摘 / 2026-08-29）。**`tests/common/mod.rs` がその場所**で、
//! **26 本の統合テストが `mod common;` で取り込んでいる**
//! （そのファイル自身が `#[path = "../../src/testlog/assert.rs"]` で `src/` の1本を引いており、
//! `tests/common/calibration.rs` を使う例もある）。
//! 元の理由づけ（`src/testlog.rs` が `#[cfg(test)]` だから見えない）は
//! **`src/` を共有できない話**であって、**`tests/` 同士の話ではなかった。**
//!
//! **畳んでいないのは代償の判断である** —— `tests/common/mod.rs` は
//! `chrono` / `sqlx` / `Store` を引き込む重い器で、綴りの走査2本のために取り込ませたくない。
//! **そして `13c-3a` の実測どおり、畳んでも自動では守られない**
//! （呼び出し側が同じ関数で期待値を作るので、関数が緩めば両方が一緒に緩む）。

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::path::PathBuf;

use personacore::llm::DEFAULT_ENDPOINT;
use personacore::server::{BindAddr, DEFAULT_PORT, ENV_LLM_ENDPOINT, parse_args};

/// 走査対象。**リポジトリ根の README。**
const README: &str = "README.md";

/// README が名指しする起動スクリプト。
const START_SCRIPT: &str = "scripts/start.ps1";

/// 起動スクリプトが**サーバを起こす行そのもの**。
///
/// # ⚠ 引数まで綴りに含める（作業単位 F-4b-1）
///
/// **`-Upstream` を渡した起動は、`--upstream <名前>` を実行ファイルへ流す**
/// （ADR-0042 §決定6）。**旗を渡さない起動では `$serverArgs` が空**なので、
/// この1行が両方を賄っている。
/// **`cargo run` だけを探す形に戻さないこと** —— 戻すと、
/// **引数を渡す部分を落とす変異がここを素通りする。**
const SERVER_RUN: &str = "cargo run -- @serverArgs";

/// 環境変数の綴りを定義しているファイル。
const CONFIG_SOURCE: &str = "src/server/config.rs";

/// 開発の作法の権威。README の `cargo` の並びはここの写しである。
const GUIDE: &str = "CLAUDE.md";

/// `pub const ENV_…: &str = "PERSONACORE_…";` の頭。
const ENV_CONST_PREFIX: &str = "pub const ENV_";

/// 環境変数の綴りの接頭辞。
const ENV_NAME_PREFIX: &str = "PERSONACORE_";

/// README が引数の話をしている行を選ぶ目印。
const OPTION_CONTEXT: &str = "start.ps1";

// ----------------------------------------------------------------------
// 帳簿。**「増えたら足す」ではなく「減ったら鳴る」**（B-23 の作法）。
// ----------------------------------------------------------------------

/// **README が「値の在処」として指している定数**（指さなくなったら鳴る）。
///
/// README にこの値を**写させない**ためのもの。写すと、Rust 側で既定を変えたときに
/// **README だけが黙って古びる。**
const POINTED_CONSTANTS: [&str; 2] = ["DEFAULT_PORT", "DEFAULT_ENDPOINT"];

/// 既定の待ち受けポートの**権威の名前**。
const PORT_CONSTANT: &str = "DEFAULT_PORT";

/// **根の README がたどらせる markdown のうち、既定ポートの写しをここが見張るもの**
/// （作業単位 B-30）。
///
/// **「増えたら足す」ではなく「この1つは必ず見る」** ——
/// ここから落とすと、その文書は**誰にも見られないまま写しを持てる。**
/// 見張り番は [`the_documents_the_readme_links_to_point_at_the_default_port_instead_of_copying_it`]。
///
/// [`the_documents_the_readme_links_to_point_at_the_default_port_instead_of_copying_it`]:
///     the_documents_the_readme_links_to_point_at_the_default_port_instead_of_copying_it
const PORT_WATCHED_LINKED_DOCUMENTS: [&str; 2] = ["web/README.md", "docs/09-setup.md"];

/// README が名指しする起動スクリプトの引数の最低本数。
///
/// **1本にしないこと** —— 1本だと「たまたま在る1本」で緑になり、
/// 引数の案内が丸ごと消えても気付けない。
const LEAST_OPTIONS: usize = 2;

/// README がコードブロックに書く `cargo` の行の最低本数。
///
/// CLAUDE.md §2 が要求する4本（`build` / `test` / `clippy` / `fmt`）に合わせてある。
const LEAST_CARGO_COMMANDS: usize = 4;

/// README の相対リンクの最低本数。
///
/// **実数（23）に近い床。** 着手時（2026-08-29）に自分で数えた値である
/// （CLAUDE.md「書く数は自分が読んだ数だけにする」）。
/// **緩い床は「守っている」と数えられるだけで何も守らない**ので、実数に寄せてある。
const LEAST_LINKS: usize = 20;

// ----------------------------------------------------------------------
// 走査の本体。**すべてソースを引数で受け取る。**
// ----------------------------------------------------------------------

/// markdown の**相対リンク**を「表示文字列, 行き先」で書いてある順に返す。
///
/// `http://` `https://` `mailto:` と、同じ文書の中の錨（`#…`）は落とす。
/// 行き先に錨が付いていたら**ファイルの部分だけ**を返す。
fn relative_links(source: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for (at, _) in source.match_indices("](") {
        let Some(open) = source[..at].rfind('[') else {
            continue;
        };
        let rest = &source[at + 2..];
        let Some(end) = rest.find(')') else {
            continue;
        };
        let target = &rest[..end];
        let external = target.starts_with("http://")
            || target.starts_with("https://")
            || target.starts_with("mailto:")
            || target.starts_with('#');
        if external {
            continue;
        }
        let file = target.split('#').next().unwrap_or(target);
        if file.is_empty() {
            continue;
        }
        found.push((source[open + 1..at].to_owned(), file.to_owned()));
    }
    found
}

/// ``` で囲まれた中の行を返す。
fn code_block_lines(source: &str) -> Vec<&str> {
    let mut inside = false;
    let mut found = Vec::new();
    for line in source.lines() {
        if line.trim_start().starts_with("```") {
            inside = !inside;
            continue;
        }
        if inside {
            found.push(line);
        }
    }
    found
}

/// コードブロックの中で `program` から始まる行を返す（前後の空白は落とす）。
///
/// **散文の中の言及は拾わない** —— 拾うと「読者が打つ行」と「話題にしただけの綴り」が
/// 混ざり、権威との突き合わせが意味を失う。
fn commands_in(source: &str, program: &str) -> Vec<String> {
    code_block_lines(source)
        .into_iter()
        .map(str::trim)
        .filter(|line| {
            line.strip_prefix(program)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
        })
        .map(str::to_owned)
        .collect()
}

/// `command` が `required` のどれかと**同じ命令**か。
///
/// # ⚠ `contains` では駄目（**変異検査で実測** / 2026-08-29）
///
/// 権威の文字列に対して `contains` で見ていたところ、
/// **`-D warnings` を `-D warning` に縮める変異が 1795 全緑を通った**
/// ——縮めた綴りは権威の**接頭辞**なので、部分一致が成立してしまう。
///
/// ここでは**行そのもの**として突き合わせる。権威の側に注記が続くこと
/// （`cargo build   # ← 省略不可`）は許すが、**続きは空白から始まらねばならない。**
fn command_is_required(command: &str, required: &[String]) -> bool {
    required.iter().any(|line| {
        line.strip_prefix(command)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
    })
}

/// 1行の中で、バッククォートで囲まれた区間を書いてある順に返す。
fn backticked(line: &str) -> Vec<&str> {
    line.split('`').skip(1).step_by(2).collect()
}

/// `needle` を含む行の中で、バッククォートで囲まれた `-Xxx` を全部返す。
///
/// **囲まれていないものは拾わない** —— 拾うと `README-start.md` のような
/// 綴りの中の `-` が引数に化ける。
fn options_named_on_lines_with(source: &str, needle: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for line in source.lines().filter(|line| line.contains(needle)) {
        for segment in backticked(line) {
            let Some(name) = segment.strip_prefix('-') else {
                continue;
            };
            if !name.is_empty() && name.chars().all(|ch| ch.is_ascii_alphanumeric()) {
                found.insert(segment.to_owned());
            }
        }
    }
    found
}

/// PowerShell のソースから**コメントを落とした行**を返す（`<# … #>` と行頭の `#`）。
///
/// # ⚠ いま実ファイルでは効いていない（**レビューが実測** / 2026-08-29）
///
/// **これを外しても `scripts/start.ps1` に対しては 1797 全緑になる。**
/// 理由は針の形で、**着手時の doc に書いた理由づけは事実ではなかった**:
///
/// * 引数を数える針は `$名前` で、`.EXAMPLE` に在るのは `-Port 9000` という
///   **`-` 始まりの綴り**。**`start.ps1` のコメントに、針に当たる綴り
///   （`$Port` / `$SkipWebBuild`）は1件も無い**
///   （実測: `$Port` 3件・`$SkipWebBuild` 2件、**全部コード行**）。
///   **⚠ 「コメントに `$` を含む綴りは1つも無い」と書いてあったが、偽だった**
///   （F-9 のレビューが実測 / 2026-09-18 —— `$env:PERSONACORE_DB_PATH` や
///   `$LASTEXITCODE` などで**8件**あり、**7件は F-9 以前から在った**）。
///   **無いのは「針に当たる綴り」で、`$` そのものではない。**
/// * [`invokes`] のほうも、効いているのは `line.trim() == command`（完全一致）で、
///   コメントの中の `cargo run` は前後に散文が付くので**そもそも一致しない。**
///
/// **⚠ これは `B-12b`（囮が的を外していた）と同じ形。**
///
/// # ではなぜ残すか
///
/// **将来こう書かれた瞬間に効く**からで、どちらも PowerShell では普通の書き方:
///
/// * `.PARAMETER` の説明文が `$Port に入る` と**変数名で**説明する
/// * `.EXAMPLE` の中に **`cargo run` だけの行**を置く
///
/// **観測しているのは、植えたソースの対照2本だけ**
/// （[`the_powershell_code_scan_reads_the_source_it_is_given`] と
/// [`the_option_wiring_scan_reads_the_script_it_is_given`]。
/// どちらも**コメント行が針を満たす囮**を持っているので、
/// この関数を外すと赤になる）。**実ファイル側は、いまは何も見ていない。**
///
/// [`invokes`]: invokes
/// [`the_powershell_code_scan_reads_the_source_it_is_given`]:
///     the_powershell_code_scan_reads_the_source_it_is_given
/// [`the_option_wiring_scan_reads_the_script_it_is_given`]:
///     the_option_wiring_scan_reads_the_script_it_is_given
fn powershell_code(script: &str) -> Vec<&str> {
    let mut inside_block = false;
    let mut found = Vec::new();
    for line in script.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("<#") {
            inside_block = true;
        }
        let closes = trimmed.contains("#>");
        if inside_block {
            if closes {
                inside_block = false;
            }
            continue;
        }
        if trimmed.starts_with('#') {
            continue;
        }
        found.push(line);
    }
    found
}

/// そのスクリプトが、**その命令だけを1行として実行しているか。**
///
/// メッセージの中の言及（`Write-Host '… （pnpm run build）…'`）を
/// 「呼んでいる」と数えないための形。
///
/// **⚠ 実ファイルに効いているのは完全一致のほうだけ**
/// （[`powershell_code`] の「いま実ファイルでは効いていない」節）。
///
/// [`powershell_code`]: powershell_code
fn invokes(script: &str, command: &str) -> bool {
    powershell_code(script)
        .into_iter()
        .any(|line| line.trim() == command)
}

/// PowerShell の識別子として**続いている**か（`$SkipWebBuild` と `$SkipWebBuildMode` を分ける）。
fn continues_identifier(next: Option<char>) -> bool {
    next.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// `[型]$名前` の形で**宣言**されているか。
///
/// **後ろへ伸びた綴りは別物として扱う** —— `]$SkipWebBuildMode` を
/// `$SkipWebBuild` の宣言と数えない。
fn declares_parameter(source: &str, variable: &str) -> bool {
    let needle = format!("]{variable}");
    source
        .match_indices(&needle)
        .any(|(at, _)| !continues_identifier(source[at + needle.len()..].chars().next()))
}

/// その変数への**参照**を数える（宣言も1回に数える）。
///
/// # ⚠ `occurrences` そのままでは駄目（**レビューが実測** / 2026-08-29）
///
/// `$SkipWebBuild` は `$SkipWebBuildMode` の**接頭辞**なので、
/// **宣言と唯一の使用をまとめて改名する変異が 1797 全緑を通った**
/// ——README は消えた引数を案内したまま。
/// **`-D warnings` → `-D warning` で自分が直したのと同じ形**が、
/// この2行に写っていなかった（`D-6`「写しから列が1つだけ落ちる」）。
fn variable_uses(source: &str, variable: &str) -> usize {
    source
        .match_indices(variable)
        .filter(|(at, _)| !continues_identifier(source[at + variable.len()..].chars().next()))
        .count()
}

/// そのスクリプトが、その引数を**宣言し、しかも使っている**か。
///
/// # なぜ「宣言」だけでは足りないか（F-1 のレビューが実測）
///
/// **宣言はそのままで使用箇所だけを差し替える変異は、宣言を見る検査を全部通り抜けた。**
/// **死んだ引数になり、案内だけが正しく見える。**
/// なので `$名前` が**2回以上**現れることを数える（宣言 ＋ 最低1回の使用）。
///
/// # ⚠ それでも綴りの検査でしかない
///
/// **その行が効いていること**は見ていない。走らせて確かめる形は別の作業単位。
fn option_is_declared_and_used(script: &str, option: &str) -> bool {
    let Some(name) = option.strip_prefix('-') else {
        return false;
    };
    let variable = format!("${name}");
    // **コメントを落としてから見る**（[`powershell_code`] の限界の節も読むこと）。
    let code = powershell_code(script).join("\n");
    declares_parameter(&code, &variable) && variable_uses(&code, &variable) >= 2
}

/// ソースに現れる `scripts/….ps1` の綴りを全部返す（先頭の `./` は含まない）。
fn script_paths(source: &str) -> BTreeSet<String> {
    const HEAD: &str = "scripts/";
    let mut found = BTreeSet::new();
    for (at, _) in source.match_indices(HEAD) {
        let rest = &source[at..];
        let len = rest
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || "/._-".contains(ch)))
            .unwrap_or(rest.len());
        let path = &rest[..len];
        if path.ends_with(".ps1") {
            found.insert(path.to_owned());
        }
    }
    found
}

/// PowerShell スクリプトが要求する版の**メジャー**を読む（書いていなければ `None`）。
fn required_powershell_major(script: &str) -> Option<u32> {
    const KEY: &str = "#Requires -Version ";
    let at = script.find(KEY)?;
    let rest = &script[at + KEY.len()..];
    let end = rest
        .find(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .unwrap_or(rest.len());
    rest[..end].split('.').next()?.parse().ok()
}

/// ソースが**名指ししている** PowerShell の版（メジャー）を全部返す。
///
/// # なぜ「含んでいるか」では足りないか（**変異検査を組む途中で気付いた**）
///
/// 要求の版が別のところにも書いてあると、**片方だけ間違えても
/// 「その綴りが在る」は成立し続ける**（CLAUDE.md「その行が出た、と、
/// その行がこう言っている、は別の主張である」）。
/// 集合で見れば、**1箇所でも別の版を名乗った時点で鳴る。**
fn powershell_versions_named(source: &str) -> BTreeSet<u32> {
    const KEY: &str = "PowerShell ";
    let mut found = BTreeSet::new();
    for (at, _) in source.match_indices(KEY) {
        let rest = &source[at + KEY.len()..];
        let end = rest
            .find(|ch: char| !ch.is_ascii_digit())
            .unwrap_or(rest.len());
        if let Ok(major) = rest[..end].parse::<u32>() {
            found.insert(major);
        }
    }
    found
}

/// 「`<パス>.rs` の `<定数名>`」という**指し先**を、書いてある順に返す。
///
/// バッククォートで囲まれた隣り合う2つを見る。値そのものではなく
/// **値の在処**を書かせるための走査で、README が値を写していないことと対にして使う。
fn rust_const_pointers(source: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for line in source.lines() {
        let segments = backticked(line);
        for pair in segments.windows(2) {
            let (path, name) = (pair[0], pair[1]);
            let named = name.starts_with(|ch: char| ch.is_ascii_uppercase())
                && name
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_');
            if path.ends_with(".rs") && named {
                found.push((path.to_owned(), name.to_owned()));
            }
        }
    }
    found
}

/// Rust のソースから `pub const <名前> … = <値>;` の値を読む（空白と `"` は落とす）。
fn declared_const_value<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    let at = source.find(&format!("pub const {name}"))?;
    let rest = &source[at..];
    let equals = rest.find("= ")?;
    let end = rest.find(';')?;
    if equals > end {
        return None;
    }
    Some(rest[equals + 2..end].trim().trim_matches('"'))
}

/// ソースに現れる `PERSONACORE_…` の綴りを全部拾う。
///
/// 大文字・数字・下線が続く限り**後ろへ伸ばす** —— `PERSONACORE_PORTX` を
/// `PERSONACORE_PORT` として通すと、綴り間違いを見逃す。
fn personacore_env_names(source: &str) -> BTreeSet<&str> {
    let mut found = BTreeSet::new();
    for (at, _) in source.match_indices(ENV_NAME_PREFIX) {
        let rest = &source[at..];
        let len = rest
            .find(|ch: char| !(ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_'))
            .unwrap_or(rest.len());
        found.insert(&rest[..len]);
    }
    found
}

/// `config.rs` が `pub const ENV_… = "…";` で定義している綴りを全部拾う。
fn defined_env_names(source: &str) -> BTreeSet<&str> {
    let mut found = BTreeSet::new();
    for (at, _) in source.match_indices(ENV_CONST_PREFIX) {
        let rest = &source[at..];
        let Some(end) = rest.find(';') else {
            continue;
        };
        let statement = &rest[..end];
        let Some(open) = statement.find('"') else {
            continue;
        };
        let inside = &statement[open + 1..];
        let Some(close) = inside.find('"') else {
            continue;
        };
        found.insert(&inside[..close]);
    }
    found
}

/// ソースに現れる IPv4 の**リテラル**を全部拾う。
///
/// **`[System.Net.IPAddress]::Any` のように数字で書かれていない宛先は拾えない。**
fn ipv4_literals(source: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let bytes = source.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if !bytes[at].is_ascii_digit() {
            at += 1;
            continue;
        }
        let start = at;
        while at < bytes.len() && (bytes[at].is_ascii_digit() || bytes[at] == b'.') {
            at += 1;
        }
        let run = source[start..at].trim_end_matches('.');
        if let Ok(addr) = run.parse::<Ipv4Addr>() {
            found.insert(addr.to_string());
        }
    }
    found
}

/// ソースの中に綴りが何回現れるか。**「宣言されているか」ではなく「使われているか」を数える。**
fn occurrences(source: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    source.match_indices(needle).count()
}

// ----------------------------------------------------------------------
// 実ファイルの読み込み。**絶対パスで組む**（相対パスはプロセスの作業ディレクトリで
// 解決され、別のツリーを読みうる / CLAUDE.md の `A-1a`）。
// ----------------------------------------------------------------------

fn repo_path(relative: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for part in relative.split('/') {
        path.push(part);
    }
    path
}

fn read_repo_file(relative: &str) -> String {
    let path = repo_path(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{} を読めること: {err}", path.display()))
}

fn readme() -> String {
    read_repo_file(README)
}

fn start_script() -> String {
    read_repo_file(START_SCRIPT)
}

// ----------------------------------------------------------------------
// 走査本体の対照（**植えたソースを、実ファイルと同じ本体に通す**）。
// ----------------------------------------------------------------------

/// `relative_links` は、渡されたソースのリンクを読むこと。
#[test]
fn the_link_scan_reads_the_source_it_is_given() {
    assert_eq!(
        relative_links("[植えた案内](docs/planted.md) を読む"),
        vec![("植えた案内".to_owned(), "docs/planted.md".to_owned())],
        "植えたリンクを拾うこと"
    );
    assert_eq!(
        relative_links("[節](docs/planted.md#見出し)"),
        vec![("節".to_owned(), "docs/planted.md".to_owned())],
        "錨は落として、ファイルの部分を返すこと"
    );
    assert!(
        relative_links("[外](https://example.test/planted)").is_empty(),
        "外部の URL は行き先として数えないこと"
    );
    assert!(
        relative_links("[同じ文書の中](#植えた節)").is_empty(),
        "同じ文書の中の錨は数えないこと"
    );
    assert!(
        relative_links("リンクの無い行").is_empty(),
        "無いソースからは1件も拾わないこと"
    );

    // **実ファイルも同じ本体を通ること。**
    assert!(
        !relative_links(&readme()).is_empty(),
        "実ファイルからも拾えること"
    );
}

/// `commands_in` は、渡されたソースのコードブロックだけを読むこと。
#[test]
fn the_command_scan_reads_the_source_it_is_given() {
    assert_eq!(
        commands_in("```bash\ncargo planted --flag\n```\n", "cargo"),
        vec!["cargo planted --flag".to_owned()],
        "コードブロックの中の行を拾うこと"
    );
    assert!(
        commands_in("散文の中で cargo planted と書いただけ\n", "cargo").is_empty(),
        "コードブロックの外は拾わないこと"
    );
    assert!(
        commands_in("```bash\ncargoplanted\n```\n", "cargo").is_empty(),
        "綴りが続いているだけの行を、その命令と数えないこと"
    );
    assert_eq!(
        commands_in("```bash\n  cargo planted\n```\n", "cargo"),
        vec!["cargo planted".to_owned()],
        "前後の空白は落とすこと"
    );

    // **実ファイルも同じ本体を通ること。**
    assert!(
        !commands_in(&readme(), "cargo").is_empty(),
        "実ファイルからも拾えること"
    );
}

/// `command_is_required` は、渡された要求と**行として**突き合わせること。
#[test]
fn the_command_membership_reads_the_requirement_it_is_given() {
    let required = vec![
        "cargo planted   # ← 注記".to_owned(),
        "cargo other --flag".to_owned(),
    ];

    assert!(
        command_is_required("cargo planted", &required),
        "権威の側に注記が続く形は、同じ命令と数えること"
    );
    assert!(
        command_is_required("cargo other --flag", &required),
        "そのままの行は同じ命令と数えること"
    );
    assert!(
        !command_is_required("cargo other --fla", &required),
        "**接頭辞を同じ命令と数えないこと**（`-D warnings` を `-D warning` に縮める変異が\
         ここを通り抜けた）"
    );
    assert!(
        !command_is_required("cargo other --flags", &required),
        "後ろへ伸ばした綴りも同じ命令と数えないこと"
    );
    assert!(
        !command_is_required("cargo planted", &[]),
        "要求が空なら、何も同じ命令ではないこと"
    );
}

/// `script_paths` は、渡されたソースの綴りを読むこと。
#[test]
fn the_script_path_scan_reads_the_source_it_is_given() {
    assert_eq!(
        script_paths("pwsh ./scripts/planted.ps1"),
        BTreeSet::from(["scripts/planted.ps1".to_owned()]),
        "植えた綴りを拾うこと（先頭の `./` は含まない）"
    );
    assert_eq!(
        script_paths("[投入](scripts/planted.ps1) を使う"),
        BTreeSet::from(["scripts/planted.ps1".to_owned()]),
        "リンクの中の綴りも拾うこと（`)` で切れる）"
    );
    assert!(
        script_paths("scripts/planted.md").is_empty(),
        "スクリプトでないものは拾わないこと"
    );
    assert!(
        script_paths("何も無い").is_empty(),
        "無いソースからは1件も拾わないこと"
    );

    // **実ファイルも同じ本体を通ること。**
    assert!(
        !script_paths(&readme()).is_empty(),
        "実ファイルからも拾えること"
    );
}

/// `options_named_on_lines_with` は、渡されたソースの引数を読むこと。
#[test]
fn the_option_scan_reads_the_source_it_is_given() {
    assert_eq!(
        options_named_on_lines_with("`scripts/start.ps1` は `-Planted` を取る", OPTION_CONTEXT),
        BTreeSet::from(["-Planted".to_owned()]),
        "植えた引数を拾うこと"
    );
    assert!(
        options_named_on_lines_with("`-Planted` とだけ書いた行", OPTION_CONTEXT).is_empty(),
        "目印の無い行は読まないこと"
    );
    assert!(
        options_named_on_lines_with("start.ps1 と README-start.md の話", OPTION_CONTEXT).is_empty(),
        "囲まれていない `-` を引数と数えないこと"
    );
    assert!(
        options_named_on_lines_with("`start.ps1` は `-Port 9000` のように使う", OPTION_CONTEXT)
            .is_empty(),
        "値まで一緒に囲んだ綴りを引数名と数えないこと"
    );

    // **実ファイルも同じ本体を通ること。**
    assert!(
        !options_named_on_lines_with(&readme(), OPTION_CONTEXT).is_empty(),
        "実ファイルからも拾えること"
    );
}

/// `powershell_code` / `invokes` は、渡されたソースのコメントを落として読むこと。
#[test]
fn the_powershell_code_scan_reads_the_source_it_is_given() {
    let planted = "<#\n    説明の中で cargo planted と書く\n#>\n\
                   # 行頭のコメントでも cargo planted と書く\n\
                   Write-Host 'メッセージの中の cargo planted'\n    \
                   cargo planted\n";

    assert_eq!(
        powershell_code(planted),
        vec![
            "Write-Host 'メッセージの中の cargo planted'",
            "    cargo planted",
        ],
        "ブロックコメントと行頭コメントを落とすこと"
    );
    assert!(
        invokes(planted, "cargo planted"),
        "その命令だけの行を「呼んでいる」と数えること"
    );
    assert!(
        !invokes(
            "<#\n    cargo planted\n#>\nWrite-Host 'cargo planted'\n",
            "cargo planted"
        ),
        "**コメントとメッセージの中の言及を「呼んでいる」と数えないこと**\
         （数えると、本物の呼び出しを消しても緑になる）"
    );

    // **実ファイルも同じ本体を通ること。**
    assert!(
        !powershell_code(&start_script()).is_empty(),
        "実ファイルからも拾えること"
    );
}

/// `option_is_declared_and_used` は、渡されたスクリプトの配線を読むこと。
///
/// # ⚠ ここが囮の置き場所である
///
/// 実ファイル（`scripts/start.ps1`）は
/// **コメントに、針に当たる綴り（`$Port` / `$SkipWebBuild`）を1つも持っていない**ので
/// （`$` を含むコメント自体は在る —— [`powershell_code`] の限界の節の訂正）、
/// **コメント落としの効き目を実ファイル側では観測できない**
/// （[`powershell_code`] の限界の節）。
/// **囮を植えたソースの側に置くのが、この検査の存在理由。**
///
/// [`powershell_code`]: powershell_code
#[test]
fn the_option_wiring_scan_reads_the_script_it_is_given() {
    assert!(
        option_is_declared_and_used(
            "param(\n    [switch]$Planted\n)\nif ($Planted) { }\n",
            "-Planted"
        ),
        "宣言して使っている引数を、そう読むこと"
    );
    assert!(
        !option_is_declared_and_used(
            "param(\n    [switch]$Planted\n)\nif ($false) { }\n",
            "-Planted"
        ),
        "**宣言だけで使っていない引数を「使っている」と数えないこと**\
         （F-1 のレビューが見つけた形）"
    );

    // **囮1: コメントの中だけで使われている**（コメント落としが効いていないと通る）。
    assert!(
        !option_is_declared_and_used(
            "<#\n.PARAMETER Planted\n    $Planted に入る。$Planted は既定で false。\n#>\n\
             param(\n    [switch]$Planted\n)\n",
            "-Planted"
        ),
        "**コメントの中の言及を「使っている」と数えないこと**\
         （`.PARAMETER` の説明文が変数名で書かれた瞬間、これが唯一の防ぎ手になる）"
    );

    // **囮2: 後ろへ伸びた別の綴り**（接頭辞で数えていると通る / レビューが実測した形）。
    assert!(
        !option_is_declared_and_used(
            "param(\n    [switch]$PlantedMode\n)\nif ($PlantedMode) { }\n",
            "-Planted"
        ),
        "**後ろへ伸びた綴りを同じ引数と数えないこと**\
         （まとめて改名する変異が 1797 全緑を通った形）"
    );

    assert!(
        !option_is_declared_and_used("param(\n    [switch]$Planted\n)\n", "Planted"),
        "`-` で始まらない綴りは引数として読まないこと"
    );

    // **実ファイルも同じ本体を通ること。**
    assert!(
        option_is_declared_and_used(&start_script(), "-Port"),
        "実ファイルでも配線を読めること"
    );
}

/// `required_powershell_major` は、渡されたソースの版を読むこと。
#[test]
fn the_powershell_version_scan_reads_the_source_it_is_given() {
    assert_eq!(
        required_powershell_major("#Requires -Version 9.3\nparam()\n"),
        Some(9),
        "植えた版のメジャーを拾うこと"
    );
    assert_eq!(
        required_powershell_major("#Requires -Version 11\n"),
        Some(11),
        "小数点が無くても読めること（2桁を1桁に丸めない）"
    );
    assert_eq!(
        required_powershell_major("param()\n"),
        None,
        "書いていないソースを、黙って何かに解釈しないこと"
    );

    // **実ファイルも同じ本体を通ること。**
    assert!(
        required_powershell_major(&start_script()).is_some(),
        "実ファイルからも拾えること"
    );
}

/// `powershell_versions_named` は、渡されたソースが名乗る版を全部読むこと。
#[test]
fn the_powershell_mention_scan_reads_the_source_it_is_given() {
    assert_eq!(
        powershell_versions_named("PowerShell 9 以上"),
        BTreeSet::from([9]),
        "植えた版を拾うこと"
    );
    assert_eq!(
        powershell_versions_named("PowerShell 9 が要る。むかしは PowerShell 5 だった"),
        BTreeSet::from([5, 9]),
        "**2箇所を別々に拾うこと**（拾えなければ、片方だけ古びても鳴らない）"
    );
    assert_eq!(
        powershell_versions_named("PowerShell 11 以上"),
        BTreeSet::from([11]),
        "2桁を1桁に丸めないこと"
    );
    assert!(
        powershell_versions_named("PowerShell がある").is_empty(),
        "版を名乗っていない言及は拾わないこと"
    );

    // **実ファイルも同じ本体を通ること。**
    assert!(
        !powershell_versions_named(&readme()).is_empty(),
        "実ファイルからも拾えること"
    );
}

/// `rust_const_pointers` は、渡されたソースの指し先を読むこと。
#[test]
fn the_const_pointer_scan_reads_the_source_it_is_given() {
    assert_eq!(
        rust_const_pointers("値は `src/planted/file.rs` の `PLANTED_VALUE`。"),
        vec![("src/planted/file.rs".to_owned(), "PLANTED_VALUE".to_owned())],
        "植えた指し先を拾うこと"
    );
    assert!(
        rust_const_pointers("`src/planted/file.rs` の `planted_value`").is_empty(),
        "小文字の綴りを定数と数えないこと"
    );
    assert!(
        rust_const_pointers("`docs/planted.md` の `PLANTED_VALUE`").is_empty(),
        "Rust のソースでない指し先は拾わないこと"
    );
    assert!(
        rust_const_pointers("`PLANTED_VALUE` だけの行").is_empty(),
        "在処の書いていない綴りは拾わないこと"
    );

    // **実ファイルも同じ本体を通ること。**
    assert!(
        !rust_const_pointers(&readme()).is_empty(),
        "実ファイルからも拾えること"
    );
}

/// `declared_const_value` は、渡されたソースの値を読むこと。
#[test]
fn the_declared_const_scan_reads_the_source_it_is_given() {
    assert_eq!(
        declared_const_value("pub const PLANTED: u16 = 4242;\n", "PLANTED"),
        Some("4242"),
        "数はそのまま拾うこと"
    );
    assert_eq!(
        declared_const_value(
            "pub const PLANTED: &str = \"http://planted:1\";\n",
            "PLANTED"
        ),
        Some("http://planted:1"),
        "文字列は引用符を落として拾うこと"
    );
    assert_eq!(
        declared_const_value("const PLANTED: u16 = 4242;\n", "PLANTED"),
        None,
        "公開されていない定義は拾わないこと"
    );
    assert_eq!(
        declared_const_value("pub const PLANTED: u16 = 4242\n", "PLANTED"),
        None,
        "`;` で終わっていない綴りは「見つからない」と言うこと"
    );

    // **実ファイルも同じ本体を通ること。**
    assert!(
        declared_const_value(&read_repo_file(CONFIG_SOURCE), "DEFAULT_PORT").is_some(),
        "実ファイルからも拾えること"
    );
}

/// `personacore_env_names` は、渡されたソースの綴りを読むこと。
#[test]
fn the_readme_env_name_scan_reads_the_source_it_is_given() {
    assert_eq!(
        personacore_env_names("`PERSONACORE_PLANTED` を立てる"),
        BTreeSet::from(["PERSONACORE_PLANTED"]),
        "植えた綴りを拾うこと"
    );
    assert_eq!(
        personacore_env_names("PERSONACORE_PLANTEDX"),
        BTreeSet::from(["PERSONACORE_PLANTEDX"]),
        "後ろへ伸ばすこと（綴り間違いを既知の綴りに丸めない）"
    );
    assert!(
        personacore_env_names("何も無い").is_empty(),
        "無いソースからは1件も拾わないこと"
    );

    // **実ファイルも同じ本体を通ること。**
    assert!(
        !personacore_env_names(&readme()).is_empty(),
        "実ファイルからも拾えること"
    );
}

/// `defined_env_names` は、渡されたソースの定義を読むこと。
#[test]
fn the_readme_defined_env_scan_reads_the_source_it_is_given() {
    assert_eq!(
        defined_env_names("pub const ENV_PLANTED: &str = \"PERSONACORE_PLANTED\";\n"),
        BTreeSet::from(["PERSONACORE_PLANTED"]),
        "植えた定義を拾うこと"
    );
    assert!(
        defined_env_names("/// `PERSONACORE_PLANTED` の話\n").is_empty(),
        "doc の中の言及を「定義」と読まないこと"
    );

    // **実ファイルも同じ本体を通ること。** ここが「1件も無いから緑」を潰す。
    assert!(
        defined_env_names(&read_repo_file(CONFIG_SOURCE)).contains(ENV_LLM_ENDPOINT),
        "実ファイルから {ENV_LLM_ENDPOINT} を拾えること（走査が空振りしていない）"
    );
}

/// `ipv4_literals` は、渡されたソースの宛先を読むこと。
#[test]
fn the_readme_address_scan_reads_the_source_it_is_given() {
    assert_eq!(
        ipv4_literals("画面は http://127.0.0.1:8080/ に出る"),
        BTreeSet::from(["127.0.0.1".to_owned()]),
        "植えた宛先を拾うこと"
    );
    assert_eq!(
        ipv4_literals("外へ出すなら 0.0.0.0 に待ち受ける"),
        BTreeSet::from(["0.0.0.0".to_owned()]),
        "**外へ出す綴りを拾うこと**（拾えなければ、この検査は何もしていない）"
    );
    assert_eq!(
        ipv4_literals("cargo 1.97.1 / ポート 8080"),
        BTreeSet::new(),
        "IPv4 として読めない数の並びは拾わないこと"
    );

    // **実ファイルも同じ本体を通ること。**
    assert!(
        !ipv4_literals(&readme()).is_empty(),
        "実ファイルからも拾えること"
    );
}

/// `occurrences` は、渡されたソースを数えること。
#[test]
fn the_readme_occurrence_scan_counts_the_source_it_is_given() {
    assert_eq!(
        occurrences("$Planted\n$Planted\n", "$Planted"),
        2,
        "植えた重複を数えること"
    );
    assert_eq!(occurrences("$Planted\n", "$Planted"), 1, "1回なら 1");
    assert_eq!(
        occurrences("$Other\n", "$Planted"),
        0,
        "別の綴りは数えないこと"
    );
    assert_eq!(
        occurrences("何でも", ""),
        0,
        "空の綴りを「全部一致」と読まないこと"
    );

    // **実ファイルも同じ本体を通ること。**
    assert_eq!(
        occurrences(&start_script(), "#Requires"),
        1,
        "実ファイルでも数えられること"
    );
}

// ----------------------------------------------------------------------
// 本命 —— README と実体の突き合わせ。
// ----------------------------------------------------------------------

/// **README のリンクは、全部実在するものを指していること。**
///
/// 「実在しない操作を案内しない」の、いちばん機械的に見える部分。
/// **⚠ 見出しの錨は落として読む**ので、節が消えても鳴らない。
#[test]
fn every_link_in_the_readme_points_at_something_that_exists() {
    let links = relative_links(&readme());

    assert!(
        links.len() >= LEAST_LINKS,
        "README の相対リンクが {} 件しかない（記録では最低 {LEAST_LINKS} 件）\
         ——案内が丸ごと消えても気付けない",
        links.len()
    );

    for (label, target) in &links {
        let path = repo_path(target);
        assert!(
            path.exists(),
            "README の「{label}」が指す {target} がリポジトリに無い（{}）",
            path.display()
        );
    }
}

/// **README が名指しする起動スクリプトは、実在すること。**
#[test]
fn the_readme_only_names_scripts_that_exist() {
    let named = script_paths(&readme());

    assert!(
        !named.is_empty(),
        "README がスクリプトを1つも名指ししていない（走査が空振りしている疑い）"
    );
    assert!(
        named.contains(START_SCRIPT),
        "README が {START_SCRIPT} を名指ししていない（起動の案内が消えている）"
    );

    for path in &named {
        assert!(
            repo_path(path).exists(),
            "README が {path} を案内しているが、リポジトリに無い"
        );
    }
}

/// **README が名指しする引数は、起動スクリプトが宣言し、しかも使っていること。**
///
/// # なぜ「宣言」だけでは足りないか（F-1 のレビューが実測）
///
/// **宣言はそのままで使用箇所だけを差し替える変異は、宣言を見る検査を全部通り抜けた**
/// （フルスイート 1754 全緑）。**死んだ引数になり、案内だけが正しく見える。**
/// なので `$名前` が**2回以上**現れることを数える（宣言 ＋ 最低1回の使用）。
///
/// # ⚠ それでも綴りの検査でしかない
///
/// **その行が効いていること**は見ていない。走らせて確かめる形は別の作業単位。
#[test]
fn the_readme_only_names_options_the_start_script_declares_and_uses() {
    let named = options_named_on_lines_with(&readme(), OPTION_CONTEXT);
    let script = start_script();

    assert!(
        named.len() >= LEAST_OPTIONS,
        "README が名指しする引数が {} 件しかない（記録では最低 {LEAST_OPTIONS} 件）",
        named.len()
    );

    for option in &named {
        assert!(
            option_is_declared_and_used(&script, option),
            "README が `{option}` を案内しているが、{START_SCRIPT} が\
             その名前で宣言して使っていない（案内だけが正しく見えて、指定しても何も起きない）"
        );
    }
}

/// **README が名指しする環境変数は、全部サーバが実際に読むものであること。**
///
/// 綴りを1文字間違えた環境変数は**黙って無視される** —— 起動は通り、
/// 現れ方は「指定したはずの設定が効いていない」になる。
#[test]
fn the_readme_only_names_environment_variables_the_server_reads() {
    let config = read_repo_file(CONFIG_SOURCE);
    let defined = defined_env_names(&config);
    let readme = readme();
    let named = personacore_env_names(&readme);

    assert!(
        !named.is_empty(),
        "README が環境変数を1つも名指ししていない（走査が空振りしている疑い）"
    );

    for name in &named {
        assert!(
            defined.contains(name),
            "README が `{name}` を案内しているが、{CONFIG_SOURCE} にその綴りの定義が無い\
             （サーバは黙って無視する）"
        );
    }
}

/// **README のコードブロックに書いてある `cargo` の行は、CLAUDE.md が要求するものであること。**
///
/// README は**開発の作法の権威ではない。** §2 の写しである。
/// 権威の側を変えたときに README だけが古びるのを、ここで止める。
#[test]
fn every_cargo_command_in_the_readme_is_one_the_guide_requires() {
    let commands = commands_in(&readme(), "cargo");
    let guide = read_repo_file(GUIDE);

    assert!(
        commands.len() >= LEAST_CARGO_COMMANDS,
        "README の `cargo` の行が {} 本しかない（記録では最低 {LEAST_CARGO_COMMANDS} 本）",
        commands.len()
    );

    let required = commands_in(&guide, "cargo");
    assert!(
        required.len() >= LEAST_CARGO_COMMANDS,
        "{GUIDE} の `cargo` の行が {} 本しかない（権威の側を読めていない疑い）",
        required.len()
    );

    for command in &commands {
        assert!(
            command_is_required(command, &required),
            "README の `{command}` が {GUIDE} の要求に無い\
             （権威の側が変わったか、README が古びた）。要求は {required:?}"
        );
    }
}

/// **README は既定値を写さず、在処を指すこと。**
///
/// # なぜ
///
/// 写すと、Rust 側で既定を変えたときに **README だけが黙って古びる**
/// ——現れ方は「書いてあった URL を開いたら、何も居ない」。
/// `scripts/start.ps1` と `web/vite.config.ts` は「読む」形にしてあるが、
/// **markdown は読めない。** なので**書かない**ことで揃える。
#[test]
fn the_readme_points_at_the_defaults_instead_of_copying_them() {
    let readme = readme();
    let pointers = rust_const_pointers(&readme);

    for name in POINTED_CONSTANTS {
        let (path, _) = pointers
            .iter()
            .find(|(_, pointed)| pointed == name)
            .unwrap_or_else(|| panic!("README が `{name}` の在処を指していない"));

        let source = read_repo_file(path);
        let value = declared_const_value(&source, name)
            .unwrap_or_else(|| panic!("README が指す {path} に `pub const {name}` が無い"));

        // **指し先が、本当にその値を持っていること。**
        // （在処を間違えても、そこに同名の定数があれば読めてしまうので、実体と突き合わせる）
        let actual = match name {
            "DEFAULT_PORT" => DEFAULT_PORT.to_string(),
            "DEFAULT_ENDPOINT" => DEFAULT_ENDPOINT.to_owned(),
            other => panic!("帳簿にある `{other}` の突き合わせ相手が書かれていない"),
        };
        assert_eq!(
            value, actual,
            "README が指す {path} の `{name}` が、実体（{actual}）と違う"
        );

        // **写していないこと。** スキームを落とした綴り（`127.0.0.1:<ポート>`）も写しに数える。
        for copy in copied_forms(&actual) {
            assert!(
                !readme.contains(&copy),
                "README が `{name}` の値を `{copy}` の形で写している（在処を指す形にすること）"
            );
        }
    }
}

/// 既定値の**写しとして数える綴り**を返す: 値そのものと、`://` があればその後ろ（末尾の `/` を落とす）。
///
/// # なぜスキームを落とした形も数えるか（O-10 のレビューの是正）
///
/// 値そのもの（`http://…`）だけを見ていたので、**`(127.0.0.1:<ポート>)` と書く写しは緑**だった。
/// 読者にとっては同じ宛先で、Rust 側で既定を変えたときに同じように古びる。
fn copied_forms(value: &str) -> Vec<String> {
    let mut forms = vec![value.to_owned()];
    if let Some((_, rest)) = value.split_once("://") {
        let rest = rest.trim_end_matches('/');
        if !rest.is_empty() {
            forms.push(rest.to_owned());
        }
    }
    forms
}

/// `copied_forms` は、渡された値から写しの綴りを作ること（植えた値）。
#[test]
fn the_copied_forms_include_the_value_without_its_scheme() {
    assert_eq!(
        copied_forms("http://192.0.2.1:9/"),
        vec!["http://192.0.2.1:9/".to_owned(), "192.0.2.1:9".to_owned()]
    );
    assert_eq!(copied_forms("9999"), vec!["9999".to_owned()]);
    assert_eq!(copied_forms("http://"), vec!["http://".to_owned()]);
}

/// **README がたどらせる先も、既定ポートを写さず在処を指すこと**（作業単位 B-30）。
///
/// # なぜ根の README の見張り番が、別のファイルまで見るのか
///
/// [`the_readme_points_at_the_defaults_instead_of_copying_them`] が言えるのは
/// **そのファイルについてだけ**である。ところが根の README は
/// `web/README.md` を**3回リンクしている**ので、
/// **読者がたどった先には写しがある**（F-2 のレビューの指摘）。
/// **読者から見れば、リンクの先は案内の続き**であって、別の文書ではない。
///
/// 実際に写っていた —— `cd .. && cargo run       # http://127.0.0.1:8080/ で画面が出る`。
/// **`src/server/config.rs` の `DEFAULT_PORT` を変えても、何一つ落ちなかった。**
///
/// # 何を見ているか（4つ）
///
/// 1. 根の README が、そのファイルを**実際にリンクしている**こと。
///    **リンクが消えたら、この検査の前提（読者がたどる）も消える**ので一緒に鳴らす。
/// 2. そのファイルが `DEFAULT_PORT` の**在処を指している**こと。
/// 3. **どの指し先も、本当にその値を持っている**こと（在処を間違えても、
///    そこに同名の定数があれば読めてしまう）。**最初の1つではなく全部見る。**
/// 4. そのファイルが**値を写していない**こと。
///
/// # ⚠ このうち、B-30 が**新しく**守ったのは 1・3・4 である
///
/// **2 は B-30 より前から満たされていた**（レビューの指摘 / 実測）——
/// `web/README.md` は **dev server の proxy 先の話**として
/// 「`src/server/config.rs` の `DEFAULT_PORT` を読んで決めている」と**既に書いていた。**
/// **`HEAD` にも在る行**なので、**「在処を指している」だけなら B-30 の前から緑だった。**
///
/// **3 は「全部見る」にして初めて新しくなった。** 最初の1つだけを見ていた版では、
/// **2つ目以降の指し先は何を書いても緑**で、
/// **`DEFAULT_PORT` を宣言していない実在のファイルへ書き換える変異が 1827 全緑**だった。
///
/// **⚠ 「B-30 で足した引用ブロックが在ること」は、誰も見ていない**（下記「守っていないもの」）。
///
/// # ⚠ 守っていないもの（**先に書く**）
///
/// * **「いまの既定を写した」しか捕まえない。** `9999` のような**別の値**を
///   書く形は素通りする（[`the_start_script_reads_the_default_port_from_the_rust_source`]
///   の「写していないこと」と同じ限界で、この家の既定の作法）。
///   **写しは必ず「そのとき正しい値」から始まる**ので、捕まえたいのはこちらである。
/// * **`5173`（Vite の dev server）は見ていない。**
///   **この値の権威はリポジトリのどこにも無い** —— `web/vite.config.ts` は
///   `server.port` を設定しておらず、Vite の組み込みの既定である。
///   走査をそこまで広げると、**テストが「誰も宣言していない値」を写す**ことになり、
///   直そうとしている病気そのものになる（F-2 の作業者の判断をそのまま引き継ぐ）。
///
///   **⚠ 「`vite.config.ts` に `server.port` を書いて権威を作る」案は退けた。**
///   **理由は挙動ではない**（F-7c のレビューの訂正）—— `strictPort` は既定 `false` なので、
///   **`port` を明示しても、塞がっていれば Vite は次の番号へずらす**
///   （確信度: 中 / 未実測）。**挙動は実質変わらない。**
///   **理由は規則の向き** —— **写しを正当化するために権威を作るのは、向きが逆である。**
///   `vite.config.ts` はいま `server.port` を必要としておらず、書けば
///   **「散文が写せるようにするためだけに存在する定数」が1つ増える。**
/// * **⚠ 「在処を指す行が在ること」は見ているが、「どの行が指しているか」は見ていない。**
///   **B-30 で足した引用ブロックを丸ごと消しても緑**である（実測）——
///   **proxy の節の行が残っているので、文書としては在処を指したままだから。**
///   **これは正しい振る舞いであって、穴ではない。**
///   赤にするには「指し先が2つ以上」のような**床**が要り、それは
///   [`LEAST_LINKS`] と同じ「**床は効くが、床を下げる手を止めるものが無い**」形になる。
///   **足していない**（帳簿を増やすのは司令塔の採否を仰ぐべき話なので、勝手に足さない）。
/// * **帳簿に挙げた1つしか見ていない。** 他の doc は範囲外である。
/// * **`DEFAULT_ENDPOINT` は見ていない**（`web/README.md` は上流の話をしない）。
/// * **散文は1つも見ていない。** 「pnpm dev が URL を表示する」が本当かは、
///   ここでは検査できない。
/// * **帳簿そのものは無防備**（module doc の表の5行目）。
///
/// # 実測（B-30 の変異検査 / 複製したツリー / 2026-09-02）
///
/// | 変異 | 赤 |
/// |---|---|
/// | `web/README.md` に `http://127.0.0.1:8080/` を書き戻す | **1**（この1本だけ） |
/// | 根の README の `](web/README.md)` を3件とも別の行き先に | **1**（この1本 —— 「リンクしていない」で落ちる） |
/// | **引用ブロックの在処を `src/llm/mod.rs`（`DEFAULT_PORT` を宣言していない実在のファイル）へ** | **1**（**`find` だった頃は 1827 全緑** / レビューの指摘） |
/// | **proxy の節の在処を同じく書き換える** | **1** |
/// | `rust_const_pointers` の `.rs` を `.rss` に | **3**（この1本 ＋ 在処の本 ＋ **植えたソースの対照**） |
/// | `declared_const_value` の `pub const {name}` を `{name}` に | **1**（**植えたソースの対照だけ** —— 実ファイルでは等価） |
/// | 引用ブロックを丸ごと削除（493 文字） | **0**（**1827 全緑** / 理由は上の「守っていないもの」） |
/// | [`PORT_WATCHED_LINKED_DOCUMENTS`] を空に | **0**（**1827 全緑** / module doc の表に転記した） |
///
/// **⚠ `src/server/config.rs` の `DEFAULT_PORT` を `9090` にする変異では、ここは鳴らない**
/// （実測: 赤くなったのは
/// `tests/start_script.rs` の `the_seed_script_talks_to_the_port_the_server_listens_on` **1本だけ**）。
/// **それが正しい** —— 写していない文書は、権威が動いても古びようがない。
/// **「両側から鳴る」ではなく「片側が鳴らなくなった」がこの作業単位の成果である。**
///
/// [`the_readme_points_at_the_defaults_instead_of_copying_them`]:
///     the_readme_points_at_the_defaults_instead_of_copying_them
/// [`the_start_script_reads_the_default_port_from_the_rust_source`]:
///     tests/start_script.rs
#[test]
fn the_documents_the_readme_links_to_point_at_the_default_port_instead_of_copying_it() {
    let linked: BTreeSet<String> = relative_links(&readme())
        .into_iter()
        .map(|(_, target)| target)
        .collect();
    let actual = DEFAULT_PORT.to_string();

    for relative in PORT_WATCHED_LINKED_DOCUMENTS {
        // 1. **読者がたどる先であること。**
        assert!(
            linked.contains(relative),
            "README が {relative} をリンクしていない\
             （リンクが無いなら、この検査は「案内の続き」を見ていない）"
        );

        let document = read_repo_file(relative);

        // 2. **在処を指していること。**
        let pointers = rust_const_pointers(&document);
        let pointing: Vec<&(String, String)> = pointers
            .iter()
            .filter(|(_, pointed)| pointed == PORT_CONSTANT)
            .collect();
        assert!(
            !pointing.is_empty(),
            "{relative} が `{PORT_CONSTANT}` の在処を指していない"
        );

        // 3. **指し先が、本当にその値を持っていること。全部見る。**
        //
        // **⚠ `find` で最初の1つだけを見ていた**（B-30 のレビューが実測）。
        // **後ろの指し先は何を書いても緑**で、
        // **`DEFAULT_PORT` を宣言していない実在のファイルへ書き換える変異が
        // 1827 全緑を通った** ——**読者がたどると行き止まりになる**という実害そのもの。
        for (path, _) in &pointing {
            let source = read_repo_file(path);
            let value = declared_const_value(&source, PORT_CONSTANT).unwrap_or_else(|| {
                panic!("{relative} が指す {path} に `pub const {PORT_CONSTANT}` が無い")
            });
            assert_eq!(
                value, actual,
                "{relative} が指す {path} の `{PORT_CONSTANT}` が、実体（{actual}）と違う"
            );
        }

        // 4. **写していないこと。**
        assert!(
            !document.contains(&actual),
            "{relative} が既定ポート（{actual}）を写している（在処を指す形にすること）"
        );
    }
}

/// **README が名指しする宛先は、ループバックだけであること。**
///
/// # ⚠ これは「外部公開を防いでいる」ではない
///
/// 待ち受け先を緩める手段は**そもそも無い** —— [`BindAddr::loopback`] が唯一の
/// コンストラクタで、IP を受け取らない。**守っているのは型のほう。**
///
/// ここが見ているのは **README が読者に見せる宛先の綴り**で、
/// 「読者を別の宛先へ案内する」形を落とす。
#[test]
fn the_readme_names_no_address_other_than_the_loopback() {
    let named = ipv4_literals(&readme());
    let loopback = BindAddr::loopback(DEFAULT_PORT)
        .socket_addr()
        .ip()
        .to_string();

    assert_eq!(
        named,
        BTreeSet::from([loopback.clone()]),
        "README が {loopback} 以外の宛先を名指ししている"
    );
}

/// **README が名乗る PowerShell の版は、スクリプトが要求する版だけであること。**
///
/// 揃っていないと、現れ方は「前提を満たしたのに `#Requires` で撥ねられる」
/// （または、その逆の「満たしていないのに案内が通ったと言う」）。
///
/// # ⚠ 「その綴りが在るか」では足りない
///
/// README は版を**2箇所**で名乗っている（前提の表と、走らせて確かめた機の注記）。
/// **`contains` で見ると、片方だけ間違えても、もう片方が成立させてしまう**
/// ——この作業単位の変異検査を組む途中で実際に見つけた形。
/// **集合で突き合わせる**ので、どちらを間違えても鳴る。
#[test]
fn the_readme_names_only_the_powershell_version_the_script_requires() {
    let major = required_powershell_major(&start_script())
        .unwrap_or_else(|| panic!("{START_SCRIPT} が要求する版を読めること"));
    let named = powershell_versions_named(&readme());

    assert_eq!(
        named,
        BTreeSet::from([major]),
        "README が名乗る PowerShell の版が、{START_SCRIPT} の要求（{major}）と揃っていない"
    );
}

/// **README が「これ1本で立ち上がる」と言う相手が、実際に画面を作ってサーバを起動する形であること。**
///
/// # ⚠ これは綴りの検査でしかない
///
/// 走らせてはいない。**言えるのは「その行がスクリプトにある」まで。**
/// それでも、**この2行が消えたら README の主張は嘘になる**ので置いてある
/// （F-1 の作業者とレビュアーが実際に走らせて確かめた範囲は
/// `scripts/README-start.md` の「見張り番」節）。
///
/// # ⚠ `contains` では駄目だった（**変異検査で実測**）
///
/// `cargo run` は先頭の散文コメントにも、`pnpm run build` は
/// `Write-Host` のメッセージにも実在する。**本物の呼び出しを消しても
/// そちらが検査を成立させる。** [`invokes`] は
/// **コメントを落としたうえで「その命令だけの行」**を見る。
///
/// [`invokes`]: invokes
#[test]
fn the_script_the_readme_points_at_builds_the_ui_and_runs_the_server() {
    let script = start_script();

    assert!(
        invokes(&script, "pnpm run build"),
        "{START_SCRIPT} が画面をビルドしていない（README の「画面のビルド」が嘘になる）"
    );
    assert!(
        invokes(&script, SERVER_RUN),
        "{START_SCRIPT} がサーバを起動していない（README の「これ1本で立ち上がる」が嘘になる）"
    );
}

// ----------------------------------------------------------------------
// **素の `cargo run` が曖昧にならないこと**（作業単位 F-4b-1 のレビュー2周目 / 2026-09-19）
// ----------------------------------------------------------------------

/// マニフェスト（`[[bin]]` と `default-run` を見る）。
const MANIFEST: &str = "Cargo.toml";

/// `[[bin]]` の節の頭。
const BIN_SECTION: &str = "[[bin]]";

/// 既定の実行ファイルを決める鍵。
const DEFAULT_RUN_KEY: &str = "default-run";

/// **この家の `cargo run` が起こすべき実行ファイル。**
const DEFAULT_BIN: &str = "personacore";

/// cargo が**宣言なしで**実行ファイルとして拾う綴り。
const AUTO_MAIN: &str = "src/main.rs";

/// cargo が**宣言なしで**実行ファイルとして拾うディレクトリ。
const AUTO_BIN_DIR: &str = "src/bin";

/// マニフェストの**コメントを落とした行**だけを返す。
///
/// # ⚠ コメントを落とすのは、この家の doc が綴りを引用するから（CLAUDE.md `B-21d`）
///
/// **`Cargo.toml` の注記は `[[bin]]` と `default-run` を現に引用している。**
/// **素で数えると、注記の言及が「節がもう1つある」に化ける。**
/// **TOML の `#` は行コメント**で、この家のマニフェストに `#` を含む値は無い
/// （下の対照が、その前提ごと測る）。
fn manifest_code(source: &str) -> String {
    source
        .lines()
        .map(|line| match line.find('#') {
            Some(at) => &line[..at],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// **cargo が見る実行ファイルの名前**を、宣言と自動発見の両方から集める。
///
/// # ⚠ `[[bin]]` の節を数えるだけでは足りない（**変異検査で実測 / 2026-09-19**）
///
/// **`src/main.rs` は宣言なしで実行ファイルになる。**
/// このリポジトリの `Cargo.toml` に `[[bin]]` は**1つしか書かれていない**
/// （偽の `llama`）のに、**cargo から見える実行ファイルは2つ**である。
///
/// **初版は節の数だけを見ていたので、`default-run` を落とす変異が 2161 全緑を通った**
/// ——**実機で現に止まる形なのに、見張り番が黙っていた。**
///
/// `auto_main` は `src/main.rs` が在るか、`auto_bins` は `src/bin/*.rs` の名前。
/// **どちらも引数で受け取る**ので、実ファイルと植えたソースを同じ本体に通せる（`15b`）。
fn binary_names(
    source: &str,
    package: &str,
    auto_main: bool,
    auto_bins: &[String],
) -> BTreeSet<String> {
    let code = manifest_code(source);
    let mut names = BTreeSet::new();

    // **宣言された `[[bin]]`** —— 節の中の最初の `name = "…"` を読む。
    let mut inside = false;
    for line in code.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            inside = trimmed == BIN_SECTION;
            continue;
        }
        if !inside {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=')
            && key.trim() == "name"
        {
            names.insert(value.trim().trim_matches('"').to_owned());
            inside = false;
        }
    }

    // **自動で拾われるもの。** 同じ名前なら宣言と重なるだけ（集合なので増えない）。
    if auto_main {
        names.insert(package.to_owned());
    }
    for name in auto_bins {
        names.insert(name.clone());
    }
    names
}

/// `name = "…"` を `[package]` から読む。**`[[bin]]` の `name` と取り違えない。**
fn package_name(source: &str) -> Option<String> {
    let code = manifest_code(source);
    let mut inside = false;
    for line in code.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            inside = trimmed == "[package]";
            continue;
        }
        if !inside {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=')
            && key.trim() == "name"
        {
            return Some(value.trim().trim_matches('"').to_owned());
        }
    }
    None
}

/// `default-run = "…"` の値。**無ければ `None`。コメントの中は読まない。**
fn default_run(source: &str) -> Option<String> {
    manifest_code(source).lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        if key.trim() != DEFAULT_RUN_KEY {
            return None;
        }
        Some(value.trim().trim_matches('"').to_owned())
    })
}

/// このリポジトリで cargo が自動発見する実行ファイルの名前（`src/bin/*.rs`）。
fn auto_bin_names() -> Vec<String> {
    let dir = repo_path(AUTO_BIN_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension()? == "rs")
                .then(|| path.file_stem()?.to_str().map(str::to_owned))
                .flatten()
        })
        .collect();
    names.sort();
    names
}

/// **実行ファイルが2つ以上あるなら、`default-run` が既定を名乗ること。**
///
/// # なぜ要るか（**実機で踏んだ** / 2026-09-19）
///
/// **実行ファイルが2つ以上あると、素の `cargo run` は
/// 「どれを走らせるか決められない」と言って exit 101 で止まる。**
/// **`required-features` は候補から外す効果を持たない** ——
/// 条件を満たさない bin も、曖昧さの数には入る。
///
/// **偽の `llama`（`tests/support/fake_llama.rs`）を `[[bin]]` に足した瞬間、
/// `pwsh ./scripts/start.ps1 -Upstream …` が exit 101 になった。**
/// **統合テストは `CARGO_BIN_EXE_personacore` で実行ファイルを直に起こす**ので、
/// **`cargo run` を通る唯一の経路（`start.ps1`）を誰も走らせていなかった**
/// （CLAUDE.md §4「走らせてはいけない経路がある単位では、その経路だけが無防備になる」の再演。
/// **今回は走らせてよい経路だった**）。
///
/// # ⚠ 守っていないもの
///
/// * **`cargo run` を実際には回していない。** 言えるのは「マニフェストがそう書いてある」まで。
///   **実際に回した記録は `scripts/README-start.md` の「見張り番」節にはない**
///   ——**この作業単位の報告に転記してある**（1回だけ手で回した）。
/// * **`default-run` が指す名前の実行ファイルが在ることは見ていない**
///   ——それは `cargo` 自身が起動時に落とす。
/// * **`build.rs` / `examples/` / `benches/` は数えていない**（`cargo run` の曖昧さに関わらない）。
#[test]
fn the_manifest_names_a_default_binary_when_more_than_one_exists() {
    let manifest = read_repo_file(MANIFEST);
    let package = package_name(&manifest).expect("[package] の name を読めること");
    let auto_main = repo_path(AUTO_MAIN).is_file();
    let binaries = binary_names(&manifest, &package, auto_main, &auto_bin_names());

    // **囮**: 既定の実行ファイルが数えられていないなら、この検査は何も見ていない。
    assert!(
        binaries.contains(DEFAULT_BIN),
        "{MANIFEST} から `{DEFAULT_BIN}` を実行ファイルとして拾えていない\
         （走査が空振りしている疑い）: {binaries:?}"
    );

    if binaries.len() >= 2 {
        assert_eq!(
            default_run(&manifest).as_deref(),
            Some(DEFAULT_BIN),
            "実行ファイルが {} 個（{binaries:?}）あるのに `{DEFAULT_RUN_KEY}` が \
             `{DEFAULT_BIN}` を名乗っていない\
             （素の `cargo run` が「どれを走らせるか決められない」と言って止まる）",
            binaries.len()
        );
    }
}

/// 走査本体の対照（**植えたソースを、実ファイルと同じ本体に通す** / CLAUDE.md `15b`）。
///
/// **実ファイルだけを見る対照は自己成就する** ——
/// 走査の綴りを壊す変異は、壊した綴りを `Cargo.toml` に書き込むだけで緑になる。
#[test]
fn the_manifest_scan_reads_the_source_it_is_given() {
    let none: Vec<String> = Vec::new();

    // **`[[bin]]` が2つで `default-run` 無し**（自動発見は無し）。
    let two_without = "[package]\nname = \"x\"\n\n[[bin]]\nname = \"a\"\n\n[[bin]]\nname = \"b\"\n";
    assert_eq!(
        binary_names(two_without, "x", false, &none),
        BTreeSet::from(["a".to_owned(), "b".to_owned()])
    );
    assert_eq!(default_run(two_without), None);

    // **`[[bin]]` が1つで `default-run` 無し**（曖昧さが無いので、要求もしない形）。
    let one_without = "[package]\nname = \"x\"\n\n[[bin]]\nname = \"a\"\n";
    assert_eq!(
        binary_names(one_without, "x", false, &none),
        BTreeSet::from(["a".to_owned()])
    );

    // **`[[bin]]` が2つで `default-run` 有り。**
    let two_with = "[package]\nname = \"x\"\ndefault-run = \"a\"\n\n[[bin]]\nname = \"a\"\n\n[[bin]]\nname = \"b\"\n";
    assert_eq!(binary_names(two_with, "x", false, &none).len(), 2);
    assert_eq!(default_run(two_with).as_deref(), Some("a"));

    // **⚠ いちばん大事な形**: **`[[bin]]` は1つだが、`src/main.rs` も在る**
    // （このリポジトリの形。**節を数えるだけの走査は、ここで 1 と答えて見逃す**）。
    assert_eq!(
        binary_names(one_without, "x", true, &none),
        BTreeSet::from(["a".to_owned(), "x".to_owned()]),
        "src/main.rs を実行ファイルとして数えていない"
    );

    // **`src/bin/*.rs` も数える。**
    assert_eq!(
        binary_names("[package]\nname = \"x\"\n", "x", true, &["tool".to_owned()]),
        BTreeSet::from(["tool".to_owned(), "x".to_owned()])
    );

    // **同じ名前の宣言と自動発見は重ならない**（集合なので1つ）。
    assert_eq!(
        binary_names(
            "[package]\nname = \"x\"\n\n[[bin]]\nname = \"x\"\n",
            "x",
            true,
            &none
        ),
        BTreeSet::from(["x".to_owned()])
    );

    // **`[package]` の `name` と `[[bin]]` の `name` を取り違えない。**
    assert_eq!(package_name(two_without).as_deref(), Some("x"));

    // **コメントの中の綴りは数えない**（この家のマニフェストは `[[bin]]` を注記で引用している）。
    let commented = "[package]\n# [[bin]] を足すと曖昧になる。default-run = \"nope\" が守る\nname = \"x\"\ndefault-run = \"a\"\n\n[[bin]]\nname = \"a\"\n";
    assert_eq!(
        binary_names(commented, "x", false, &none),
        BTreeSet::from(["a".to_owned()]),
        "コメントの中の `[[bin]]` を節として数えている"
    );
    assert_eq!(
        default_run(commented).as_deref(),
        Some("a"),
        "コメントの中の `default-run` を値として読んでいる"
    );

    // **鍵の後ろへ伸びた綴りは別物**（`default-runner = "x"` を拾わない）。
    let lookalike =
        "[package]\ndefault-runner = \"b\"\ndefault-run = \"a\"\n\n[[bin]]\nname = \"a\"\n";
    assert_eq!(default_run(lookalike).as_deref(), Some("a"));

    // **⚠ CRLF でも同じ答えを返すこと**（CLAUDE.md `E-1`）。
    //
    // **`main` へ入った `Cargo.toml` は checkout が CRLF にする。**
    // **作業ツリーの LF だけで測ると、そこだけ緑のまま `main` で落ちる形**が実際に出ている。
    let crlf = two_with.replace('\n', "\r\n");
    assert_eq!(
        binary_names(&crlf, "x", true, &["tool".to_owned()]),
        binary_names(two_with, "x", true, &["tool".to_owned()]),
        "CRLF の入力で実行ファイルの数え方が変わっている"
    );
    assert_eq!(
        default_run(&crlf).as_deref(),
        Some("a"),
        "CRLF の入力で `default-run` の値に CR が残っている"
    );
}

// ----------------------------------------------------------------------
// **英語の README**（作業単位 O-10 / Issue origin#104）
//
// 根の README に当てている突き合わせのうち、英語の README が名指しするもの
// （リンク・環境変数・起動スクリプト・宛先・PowerShell の版・既定値の在処）を、
// **同じ走査の本体で**英語の README にも当てる。加えて、根の README が見ていない
// **実行ファイルの旗**を見る（英語の README は配布物の exe を主に案内するため）。
//
// # ⚠ 守っていないもの（先に書く）
//
// * **散文は1つも見ていない。** 英語の文が日本語の README と同じことを言っているかは、
//   ここでは検査できない（人のレビューが受け持つ）。
// * **引数（`-Port` 等）の案内と `cargo` の並びは見ていない** —— 英語の README は
//   どちらも書かず、権威の文書（`scripts/README-start.md` / `CLAUDE.md`）へ案内するだけである。
//   **書き足したら、ここにも突き合わせを足すこと。**
// * **床（[`LEAST_EN_LINKS`]）は、根の README の床と同じく無防備**（module doc の表と同じ形）。
// ----------------------------------------------------------------------

/// 英語の README。
const README_EN: &str = "README.en.md";

/// 英語の README の相対リンクの最低本数。
///
/// **実数（23）に近い床**（作業単位 O-10 のレビューの是正の後に数えた値。着手時は 22）。
const LEAST_EN_LINKS: usize = 20;

/// 値を取る旗に渡す、見本のプロファイル名。
const SAMPLE_FLAG_VALUE: &str = "sample";

/// **実行ファイルがその旗を受け付けるか**を、本物の解析（`personacore::server::parse_args`）に
/// 通して答える。値を取らない形（`[旗]`）か、値を1つ取る形（`[旗, 名前]`）のどちらかが
/// 通れば受け付ける。
///
/// # なぜ定数の一覧と比べないか（O-10 のレビューの是正）
///
/// 旗の定数を並べて比べる形では、**解析から腕を1つ消しても定数は残るので緑のまま**だった
/// （`--no-ui` の腕を消すと、案内どおりに打った起動が「引数を解釈できませんでした」で止まる）。
fn executable_accepts(flag: &str) -> bool {
    parse_args([flag]).is_ok() || parse_args([flag, SAMPLE_FLAG_VALUE]).is_ok()
}

fn readme_en() -> String {
    read_repo_file(README_EN)
}

/// ソースが名指しする `--<英字>…` の旗を全部返す（空白で区切った語の、前後の記号を落としたもの）。
///
/// `---`（区切り線）と表の `|---|` は、`--` の後に英字が続かないので拾わない。
fn flags_named(source: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for word in source.split_whitespace() {
        let word = word.trim_matches(|ch: char| "`*(),.;:\"'<>[]|".contains(ch));
        let Some(rest) = word.strip_prefix("--") else {
            continue;
        };
        if rest.starts_with(|ch: char| ch.is_ascii_alphabetic()) {
            found.insert(word.to_owned());
        }
    }
    found
}

/// `flags_named` は、渡されたソースの旗を読むこと（植えたソース / LF と CRLF）。
#[test]
fn the_flag_scan_reads_the_source_it_is_given() {
    let planted = "Run `--no-ui` (or `--upstream <name>`).\n\n---\n\n| a | b |\n|---|---|\n\
                   ```powershell\n.\\personacore.exe --fresh\n```\n`--headless`, --x.\n";
    let expected = BTreeSet::from([
        "--fresh".to_owned(),
        "--headless".to_owned(),
        "--no-ui".to_owned(),
        "--upstream".to_owned(),
        "--x".to_owned(),
    ]);
    assert_eq!(flags_named(planted), expected);
    assert_eq!(flags_named(&planted.replace('\n', "\r\n")), expected);
    assert!(flags_named("--- と --1 は旗ではない\n").is_empty());
}

/// **根の README と英語の README が、互いをリンクしていること。**
///
/// 片方からしかたどれないと、もう片方は**入口から見えない文書**になる。
#[test]
fn the_two_readmes_link_to_each_other() {
    let targets = |source: &str| -> BTreeSet<String> {
        relative_links(source)
            .into_iter()
            .map(|(_, target)| target)
            .collect()
    };
    assert!(
        targets(&readme()).contains(README_EN),
        "{README} が {README_EN} をリンクしていない"
    );
    assert!(
        targets(&readme_en()).contains(README),
        "{README_EN} が {README} をリンクしていない"
    );
}

/// **英語の README のリンクは、全部実在するものを指していること。**
///
/// **⚠ 見出しの錨は落として読む**ので、節が消えても鳴らない（根の README と同じ限界）。
#[test]
fn every_link_in_the_english_readme_points_at_something_that_exists() {
    let links = relative_links(&readme_en());

    assert!(
        links.len() >= LEAST_EN_LINKS,
        "{README_EN} の相対リンクが {} 件しかない（記録では最低 {LEAST_EN_LINKS} 件）",
        links.len()
    );

    for (label, target) in &links {
        let path = repo_path(target);
        assert!(
            path.exists(),
            "{README_EN} の「{label}」が指す {target} がリポジトリに無い（{}）",
            path.display()
        );
    }
}

/// **英語の README が名指しする環境変数は、全部サーバが実際に読むものであること。**
#[test]
fn the_english_readme_only_names_environment_variables_the_server_reads() {
    let config = read_repo_file(CONFIG_SOURCE);
    let defined = defined_env_names(&config);
    let readme_en = readme_en();
    let named = personacore_env_names(&readme_en);

    assert!(
        !named.is_empty(),
        "{README_EN} が環境変数を1つも名指ししていない（走査が空振りしている疑い）"
    );
    for name in &named {
        assert!(
            defined.contains(name),
            "{README_EN} が `{name}` を案内しているが、{CONFIG_SOURCE} にその綴りの定義が無い\
             （サーバは黙って無視する）"
        );
    }
}

/// **英語の README が名指しする旗は、全部実行ファイルが受け付けるものであること。**
///
/// 知らない旗は起動を止める（`src/server/fresh.rs`）ので、綴りを間違えた案内は
/// 「書いてあるとおりに打ったら起動しない」になる。
#[test]
fn the_english_readme_only_names_flags_the_executable_accepts() {
    let named = flags_named(&readme_en());

    assert!(
        named.len() >= 2,
        "{README_EN} が旗を {} 個しか名指ししていない（走査が空振りしている疑い）",
        named.len()
    );
    for flag in &named {
        assert!(
            executable_accepts(flag),
            "{README_EN} が `{flag}` を案内しているが、実行ファイル（parse_args）はその旗を受け付けない"
        );
    }
}

/// `executable_accepts` は、本物の解析の答えを返すこと（受ける旗・値を取る旗・知らない旗）。
#[test]
fn the_flag_acceptance_asks_the_real_parser() {
    assert!(executable_accepts("--no-ui"), "値を取らない旗");
    assert!(executable_accepts("--upstream"), "値を1つ取る旗");
    assert!(!executable_accepts("--headless"), "知らない旗");
    assert!(!executable_accepts("--fresh=1"), "受け付けない綴り");
}

/// **英語の README が名指しする起動スクリプトは、実在すること。**
#[test]
fn the_english_readme_only_names_scripts_that_exist() {
    let named = script_paths(&readme_en());

    assert!(
        named.contains(START_SCRIPT),
        "{README_EN} が {START_SCRIPT} を名指ししていない（走査が空振りしている疑い）"
    );
    for path in &named {
        assert!(
            repo_path(path).exists(),
            "{README_EN} が {path} を案内しているが、リポジトリに無い"
        );
    }
}

/// **英語の README が名指しする宛先は、ループバックだけであること。**
#[test]
fn the_english_readme_names_no_address_other_than_the_loopback() {
    let named = ipv4_literals(&readme_en());
    let loopback = BindAddr::loopback(DEFAULT_PORT)
        .socket_addr()
        .ip()
        .to_string();

    assert_eq!(
        named,
        BTreeSet::from([loopback.clone()]),
        "{README_EN} が {loopback} 以外の宛先を名指ししている"
    );
}

/// **英語の README が名乗る PowerShell の版は、スクリプトが要求する版だけであること。**
#[test]
fn the_english_readme_names_only_the_powershell_version_the_script_requires() {
    let major = required_powershell_major(&start_script())
        .unwrap_or_else(|| panic!("{START_SCRIPT} が要求する版を読めること"));

    assert_eq!(
        powershell_versions_named(&readme_en()),
        BTreeSet::from([major]),
        "{README_EN} が名乗る PowerShell の版が、{START_SCRIPT} の要求（{major}）と揃っていない"
    );
}

/// **英語の README も既定値を写さず、在処を指すこと**（根の README と同じ帳簿 [`POINTED_CONSTANTS`]）。
#[test]
fn the_english_readme_points_at_the_defaults_instead_of_copying_them() {
    let readme_en = readme_en();
    let pointers = rust_const_pointers(&readme_en);

    for name in POINTED_CONSTANTS {
        let pointing: Vec<&(String, String)> = pointers
            .iter()
            .filter(|(_, pointed)| pointed == name)
            .collect();
        assert!(
            !pointing.is_empty(),
            "{README_EN} が `{name}` の在処を指していない"
        );

        let actual = match name {
            "DEFAULT_PORT" => DEFAULT_PORT.to_string(),
            "DEFAULT_ENDPOINT" => DEFAULT_ENDPOINT.to_owned(),
            other => panic!("帳簿にある `{other}` の突き合わせ相手が書かれていない"),
        };
        // **指し先が、本当にその値を持っていること。全部見る。**
        for (path, _) in &pointing {
            let source = read_repo_file(path);
            let value = declared_const_value(&source, name).unwrap_or_else(|| {
                panic!("{README_EN} が指す {path} に `pub const {name}` が無い")
            });
            assert_eq!(
                value, actual,
                "{README_EN} が指す {path} の `{name}` が、実体（{actual}）と違う"
            );
        }

        // **写していないこと。** スキームを落とした綴りも写しに数える（[`copied_forms`]）。
        for copy in copied_forms(&actual) {
            assert!(
                !readme_en.contains(&copy),
                "{README_EN} が `{name}` の値を `{copy}` の形で写している（在処を指す形にすること）"
            );
        }
    }
}

// ----------------------------------------------------------------------
// **AI 協働の開示の文書**（作業単位 O-3 / Issue origin#50）
//
// 根の `NOTICE-AI-COLLABORATION.md`。`LICENSE` が名指し（名前の一致と承認した版の指紋は
// `tests/license.rs`）、配る zip にも入る（`tests/distribution.rs`）。ここが見るのは2つ:
//
// * **両方の README と docs の索引から案内されていること**（origin#50 のコメント /
//   O-10 のレビューからの申し送り）。
// * **文書の中のリンクが、全部実在するものを指していること**（同じ文書の中の錨も、
//   その錨の `id` が文書の中に在ること）。
//
// # ⚠ 守っていないもの（先に書く）
//
// * **散文は1つも見ていない**（数・期間・分担の記述が事実どおりかは、承認した利用者が受け持つ）。
// * **リンクの表示文字列が行き先と合っているかは見ていない。**
// * **zip の中では、文書の中の相対リンクは行き先を持たない**（zip には `LICENSE` 以外の
//   行き先が入らない）。ここが見ているのはリポジトリの中での実在だけである。
// * **床（[`LEAST_DISCLOSURE_LINKS`]）は、根の README の床と同じく無防備**（module doc の表と同じ形）。
// ----------------------------------------------------------------------

/// AI 協働の開示の文書。
const DISCLOSURE: &str = "NOTICE-AI-COLLABORATION.md";

/// docs の索引（入口の一覧）。リンクは `docs/` からの相対で書く。
const DOCS_INDEX: &str = "docs/README.md";

/// docs の索引から AI 協働の開示の文書を指すときの綴り。
const DOCS_INDEX_TO_DISCLOSURE: &str = "../NOTICE-AI-COLLABORATION.md";

/// AI 協働の開示の文書の相対リンクの最低本数。
///
/// **実数（26。日本語 13・英語 13）に近い床**（作業単位 O-3 の着手時に数えた値）。
const LEAST_DISCLOSURE_LINKS: usize = 24;

/// markdown の**同じ文書の中の錨**（`](#…)` の `…`）を、書いてある順に返す。
fn same_document_anchors(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (at, _) in source.match_indices("](#") {
        let rest = &source[at + 3..];
        let Some(end) = rest.find(')') else {
            continue;
        };
        found.push(rest[..end].to_owned());
    }
    found
}

/// ソースの中で `id="…"` と宣言された錨の名前を全部返す。
fn declared_ids(source: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for (at, needle) in source.match_indices("id=\"") {
        let rest = &source[at + needle.len()..];
        if let Some(end) = rest.find('"') {
            found.insert(rest[..end].to_owned());
        }
    }
    found
}

/// `same_document_anchors` / `declared_ids` は、渡されたソースを読むこと（植えたソース / LF と CRLF）。
#[test]
fn the_anchor_scans_read_the_source_they_are_given() {
    let planted = "[下へ](#planted) と [外](docs/x.md#not-this)\n\n<a id=\"planted\"></a>\n\
                   <a id=\"other\"></a>\n";
    for source in [planted.to_owned(), planted.replace('\n', "\r\n")] {
        assert_eq!(same_document_anchors(&source), vec!["planted".to_owned()]);
        assert_eq!(
            declared_ids(&source),
            BTreeSet::from(["planted".to_owned(), "other".to_owned()])
        );
    }
    assert!(same_document_anchors("錨の無い行\n").is_empty());
    assert!(declared_ids("錨の無い行\n").is_empty());
}

/// **両方の README と docs の索引が、AI 協働の開示の文書へ案内していること**（O-3 / Issue origin#50）。
///
/// 案内が無いと、`LICENSE` からしかたどれない（リポジトリを開いた人の入口から見えない）。
#[test]
fn the_readmes_and_the_docs_index_point_at_the_ai_collaboration_disclosure() {
    let targets = |source: &str| -> BTreeSet<String> {
        relative_links(source)
            .into_iter()
            .map(|(_, target)| target)
            .collect()
    };
    assert!(
        targets(&readme()).contains(DISCLOSURE),
        "{README} が {DISCLOSURE} を案内していない"
    );
    assert!(
        targets(&readme_en()).contains(DISCLOSURE),
        "{README_EN} が {DISCLOSURE} を案内していない"
    );
    assert!(
        targets(&read_repo_file(DOCS_INDEX)).contains(DOCS_INDEX_TO_DISCLOSURE),
        "{DOCS_INDEX} が {DOCS_INDEX_TO_DISCLOSURE} を案内していない"
    );
    // 索引の綴りが、根の文書そのものを指していること（`docs/` からの相対で解く）。
    assert_eq!(
        DOCS_INDEX_TO_DISCLOSURE,
        format!("../{DISCLOSURE}"),
        "索引の綴りが根の {DISCLOSURE} を指していない"
    );
    assert!(
        repo_path("docs").join(DOCS_INDEX_TO_DISCLOSURE).is_file(),
        "{DOCS_INDEX} の {DOCS_INDEX_TO_DISCLOSURE} が実在しない"
    );
}

/// **AI 協働の開示の文書のリンクは、全部実在するものを指していること**（O-3 / Issue origin#50）。
///
/// 相対リンクは行き先のファイル・ディレクトリが在ること、同じ文書の中の錨は
/// その `id` が文書の中に在ること。
#[test]
fn every_link_in_the_ai_collaboration_disclosure_points_at_something_that_exists() {
    let disclosure = read_repo_file(DISCLOSURE);
    let links = relative_links(&disclosure);

    assert!(
        links.len() >= LEAST_DISCLOSURE_LINKS,
        "{DISCLOSURE} の相対リンクが {} 件しかない（記録では最低 {LEAST_DISCLOSURE_LINKS} 件）",
        links.len()
    );
    for (label, target) in &links {
        let path = repo_path(target);
        assert!(
            path.exists(),
            "{DISCLOSURE} の「{label}」が指す {target} がリポジトリに無い（{}）",
            path.display()
        );
    }

    let anchors = same_document_anchors(&disclosure);
    assert!(
        !anchors.is_empty(),
        "{DISCLOSURE} に同じ文書の中の錨が無い（英語の節への案内が消えた疑い）"
    );
    let ids = declared_ids(&disclosure);
    for anchor in &anchors {
        assert!(
            ids.contains(anchor),
            "{DISCLOSURE} の錨 #{anchor} の行き先（id=\"{anchor}\"）が文書の中に無い"
        );
    }
}
