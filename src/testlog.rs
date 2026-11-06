//! ログ観測テストの共通土台。**テスト専用**（`#[cfg(test)]`）。
//!
//! `tracing` のイベントを捕まえて中身を検査するテストは、`server::body` /
//! `server::context` / `server::telemetry` / `server::usage` / `llm::response` に
//! 散らばっている。**捕捉ヘルパを5箇所に複製していた**ものをここへ寄せた。
//!
//! # `#[cfg(feature = "testing")]` にしない理由
//!
//! `testing` フィーチャは dev-dependency の自己参照で有効になるため、
//! **本番コードがここを呼んでも `cargo test` / `cargo clippy --all-targets` は通る**
//! （CLAUDE.md 2節）。`#[cfg(test)]` なら本番ビルドに存在しないことが型で保証される。
//!
//! # `tests/` とは「フィーチャ」ではなく「ファイル」を共有する（作業単位 B-3a）
//!
//! 統合テスト（`tests/**`）は別クレートなので、この `#[cfg(test)]` の中は見えない。
//! **だからといって `testing` の下へ出さない**（上の理由がそのまま当てはまる）。
//!
//! 行と欄の突き合わせ（[`line_containing`] / [`line_with_field`] / [`has_field`]）だけを
//! [`assert`] に切り出し、`tests/common/mod.rs` が
//! `#[path = "../../src/testlog/assert.rs"]` で**同じファイル**を読む。
//! **ライブラリの公開面は1バイトも増えない**ので、本番コードからは相変わらず呼べない
//! （素の `cargo build` はこのファイルをコンパイルすらしない）。
//!
//! 見張り番は4本、下の `mod tests` にある:
//!
//! * [`the_testlog_module_is_never_exposed_by_a_feature`] ——
//!   `src/lib.rs` の宣言が `#[cfg(test)]` から離れたら落ちる。
//! * [`the_integration_tests_share_this_file_instead_of_copying_it`] ——
//!   **`#[path]` が共有の1本から離れたら**、または
//!   **`tests/**` に `SHARED_HELPERS` の綴りの定義が戻ったら**落ちる
//!   （2本目は作業単位 B-13、3本目は B-12b、**器の型は B-3**、
//!   **「欄で行を選ぶ」方は B-21b** で足した）。
//!   **名前しか見ていない**（そちらの doc の表）。
//! * [`no_integration_test_copies_a_shared_body_under_another_name`] ——
//!   **名前を変えただけの写し**が `tests/**` に戻ったら落ちる（作業単位 B-17）。
//!   **本文の断片で見る**ので名前に依らないが、**本文を書き直した別名は捕まらない**
//!   （[`SHARED_BODIES`] の doc の表に、捕まえない形を全部並べてある）。
//! * [`no_library_module_copies_a_shared_body_under_another_name`] ——
//!   **同じことを `src/**` について見る**（作業単位 B-36）。
//!   **写しが `src/**` に生えるのはこの道具が生まれた理由そのもの**なのに、
//!   B-36 まで走査が1本も無かった。
//!
//! [`no_integration_test_copies_a_shared_body_under_another_name`]:
//!     tests::no_integration_test_copies_a_shared_body_under_another_name
//! [`no_library_module_copies_a_shared_body_under_another_name`]:
//!     tests::no_library_module_copies_a_shared_body_under_another_name
//! [`SHARED_BODIES`]: tests::SHARED_BODIES
//!
//! # ログを溜める器も共有した（作業単位 B-3）
//!
//! **`SharedBuffer` と `Write` / `MakeWriter` の実装は、B-3 より前は計4つ複製されていた**
//! （ここと `tests/` の3ファイル）。**同じ [`assert`] へ畳んだ。**
//! グローバルの subscriber を差す手順（`install_global_log`）も同じ1本にある
//! ——`set_global_default` が「1プロセス1回」なのは**共有を妨げない**（あちらの doc）。
//!
//! **このモジュールが使うのは器だけ**で、差し込みの側は使わない。
//! ここの捕捉（[`captured_with_filter`] と番人）は**スレッドローカルな `with_default`** で、
//! 統合テストの**グローバルな `set_global_default`** とは別の仕掛けのままである。
//!
//! [`the_testlog_module_is_never_exposed_by_a_feature`]:
//!     tests::the_testlog_module_is_never_exposed_by_a_feature
//! [`the_integration_tests_share_this_file_instead_of_copying_it`]:
//!     tests::the_integration_tests_share_this_file_instead_of_copying_it
//!
//! # 見張り番が「赤くならずに黙る」壊れ方
//!
//! **`tracing` の callsite interest はプロセス全体で1つのキャッシュ。**
//! `tracing-core` 0.1.36 の実装は次のとおり（`callsite.rs` / `dispatcher.rs`）:
//!
//! 1. マクロを最初に踏んだときだけ `callsite::register` が走り、
//!    そのとき決まった `Interest` が **以後ずっと再利用される**。
//! 2. 登録済みの `Dispatch`（`with_default` が作る）が **1つ以下**のときは
//!    `Rebuilder::JustOne` が選ばれ、interest は
//!    **登録した側のスレッドが見ている dispatcher** だけで決まる。
//! 3. subscriber を差していないスレッドが最初にその行を踏むと、
//!    `NoSubscriber` の答え（`Interest::never()`）が**グローバルに焼き付く**。
//!
//! この状態で `output.is_empty()` や `!output.contains(..)` を主張するテストは
//! **「何も出なかった」ので緑**になる。**危険なのは偽陽性ではなく偽陰性。**
//!
//! 2026-08-15 の実測（無変異のツリー / 同じテスト exe を 8 本同時に走らせる負荷）で
//! `server::context::tests::the_fallback_is_visible_under_the_default_log_filter` が
//! 落ちることを確認している。落ちるのは肯定側なので気付けたが、
//! **同じ機構が否定側では黙って通る。**
//!
//! # 打つ手: `Interest::sometimes()` を返す番人を常駐させる
//!
//! [`pin_callsite_interest`] が上の 2. / 3. を構造的に潰す。
//!
//! `Interest::and` は「意見が割れたら `sometimes`」なので、
//! **常に `sometimes` と答える dispatcher が1つでも生きていれば、
//! どの callsite も `never` に焼き付けられない**（`sometimes` は
//! 「イベントごとに、そのスレッドの subscriber へ `enabled()` を訊く」＝正しい挙動）。
//!
//! # 却下した手: 捕捉の窓を `Mutex` で直列化する
//!
//! 「`with_default` の出入りが重ならないようにすればよい」は**自然に思いつく案**で、
//! 実際に一度は番人と併用していた。**外した。同じものを再び足さないこと。**
//!
//! 1. **単体では逆効果。** 直列化すると生きている `Dispatch` が常に1つ以下に保たれ、
//!    `has_just_one`（＝ `Rebuilder::JustOne` を選ぶ条件）が **true に張り付く。**
//!    つまり上の 2. の条件をむしろ維持してしまう。焼き付きを潰しているのは番人だけ。
//! 2. **番人がある今は中立で、益を測れない。** `lock()` の戻り値をその場で `drop` して
//!    直列化を実質無効化する変異を入れても **431 件全緑**（2026-08-15 実測）。
//!    **効果を見張るテストが原理的に書けない**——このプロジェクトは
//!    効果を測れない仕掛けを残さない（CLAUDE.md 4節）。
//! 3. **害はある。** 「直列化しているから安全」という誤った安心感と、
//!    捕捉ヘルパの中からさらに捕捉ヘルパを呼ぶと即デッドロックする罠。
//!
//! **外した副作用が1つある。** 錠は `has_just_one` を true に張り付かせていたので、
//! 焼き付きを丸ごと再現する見張り番
//! （`a_callsite_registered_by_a_bare_thread_is_not_burned_to_never`）が
//! 既定の走らせ方でも決定的に赤になっていた。錠が無いと他のテストの dispatcher が
//! 同居して `has_just_one` が false になり、**その見張り番は条件付きになる。**
//! 代わりに `the_sentinels_are_registered_and_always_ask` を置いてある
//! （番人の常駐と回答を直接見るので、並行状況に依らず決定的）。
//!
//! # 併せて必要なこと（このモジュールでは面倒を見られない）
//!
//! **否定側のテストは「先に `trace` フィルタでイベント自体が出ている」ことを
//! 確かめてから否定を主張すること**（[`VERBOSE_FILTER`]）。番人があっても、
//! 実装から `debug!` が消えたといった別の理由で黙る事故は拾えない。
//!
//! この「先出し」は飾りではない。**番人を外した exe を 16 本同時に 400 実行**した
//! 負荷（2026-08-15 実測）で 28 件が赤になり、うち 22 件は
//! `llm::response::tests::an_unreadable_usage_stays_out_of_the_default_log_filter` の
//! **先出しのアサート自体**（`response.rs` の「前提: 現に出せること」）だった。
//! **先出しが無ければ、その 22 件は「何も出なかった」まま緑を通り抜けていた。**
//! 同じ負荷を番人つきで 1600 実行すると 0 件（この節の対照実験）。

use std::sync::OnceLock;

use tracing::span::{Attributes, Record};
use tracing::subscriber::Interest;
use tracing::{Dispatch, Event, Id, Metadata, Subscriber};
use tracing_subscriber::EnvFilter;

/// 常駐させる番人。**落とすと interest の焼き付きが復活する。**
static SENTINELS: OnceLock<Vec<Dispatch>> = OnceLock::new();

/// 番人が実際に interest を訊かれた callsite の `Metadata`。**見張り番のための足場。**
///
/// `&'static Metadata<'static>` は自前で組み立てるのが面倒（`Callsite` の実装が要る）で、
/// かつ「**番人が現に訊かれている**」ことも別途確かめたい。両方をここで賄う
/// ——[`the_sentinels_are_registered_and_always_ask`] が使う。
///
/// [`the_sentinels_are_registered_and_always_ask`]:
///     tests::the_sentinels_are_registered_and_always_ask
static OBSERVED_CALLSITE: OnceLock<&'static Metadata<'static>> = OnceLock::new();

/// 番人の数。**2 は原理的に窓を残さないための保険。**
///
/// `has_just_one` は「retain したあと新しい dispatcher を push した件数 ≦ 1」で決まるので、
/// 常駐が1つだけだと**その1つを登録した瞬間**に true へ戻る。
///
/// **ただしこの値は見張れない。** `1` にしても全緑になる（2026-08-15 実測）。
/// `1` で `has_just_one` が true でいるのは
/// **[`pin_callsite_interest`] の直後から最初の `with_default` までの数マイクロ秒だけ**で、
/// しかもその窓で焼き付いても直後の `Dispatch::new` が全 callsite を rebuild して解く。
/// **`1` でも実害は観測されていない。** それでも 2 にしてあるのは、
/// 窓の存在そのものを消しておく方が安いから。
const SENTINEL_COUNT: usize = 2;

/// 何を訊かれても「毎回訊いてくれ」と答えるだけの subscriber。
///
/// # 呼ばれるのは2つだけ。残りは**到達しない**
///
/// 番人は [`Dispatch::new`] で**登録されるだけ**で、
/// `with_default` にも `set_global_default` にも渡していない。
/// つまり**どのスレッドの既定 dispatcher にもならない。**
/// `tracing-core` が登録済み dispatcher に対して呼ぶのは
/// `register_callsite`（interest の再計算）と `max_level_hint`（最大水準の再計算）だけで、
/// `enabled` / `event` / `new_span` / `record` / `enter` / `exit` へは**来ない。**
///
/// **これは実装が守っているのではなく配線が保証している。**
/// `enabled` を `true`、`event` を `panic!` に変えても全件緑になる（2026-08-15 実測）。
/// 到達不能なので、どんなテストを書いてもここは赤にできない
/// ——`Subscriber` を実装するために置いてあるだけの本体だと思って読むこと。
///
/// # `max_level_hint` を既定の `None` のままにしてある
///
/// `callsite.rs` の `rebuild_interest` は登録済み全 dispatcher の
/// `max_level_hint().unwrap_or(TRACE)` の **MAX** を取って `LevelFilter::set_max` する。
/// **MAX なので、ここで何を申告してもプロセス全体の上限は押し下がらない**
/// （`Some(OFF)` を返しても、捕捉側の `trace` フィルタが TRACE を申告する）。
///
/// 起きているのは逆で、**`None`（＝ TRACE 扱い）のまま番人を常駐させることが
/// `LevelFilter::current()` を TRACE に固定する。** マクロの水準チェックが素通りに
/// なるぶん、無効な `trace!` / `debug!` の行もフィールド組み立てまで進む。
/// これがコストの正体で、テスト一式で **21.7s → 22.1s**（2026-08-15 実測）。
/// 焼き付きによる偽陰性と引き換えなら安い。
struct AlwaysAsk;

impl Subscriber for AlwaysAsk {
    /// **ここがこの型の存在理由。** `Interest::and` は意見が割れたら
    /// `sometimes` になるので、これが1つ生きていれば `never` は焼き付かない。
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        // 見張り番の足場。**戻り値には影響しない**（[`OBSERVED_CALLSITE`] の doc）。
        // 初期化済みの `OnceLock::set` は原子読みの速い経路なので、
        // rebuild のたびに全 callsite を通っても実測できるコストにならない。
        let _ = OBSERVED_CALLSITE.set(metadata);
        Interest::sometimes()
    }

    // --- ここから下は到達しない（型の doc を参照）。 ---

    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        false
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        // `Id` は 0 を作れないので 1 を返す（panic しない）。
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, _event: &Event<'_>) {}

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

/// 番人を常駐させる。**冪等**（`OnceLock` で1回だけ）。
///
/// `Dispatch::new` はその場で
/// **登録済みの全 callsite の interest を計算し直す**ので、
/// この呼び出しより前に `never` で焼き付いていた行もここで解ける。
fn pin_callsite_interest() {
    SENTINELS.get_or_init(|| {
        (0..SENTINEL_COUNT)
            .map(|_| Dispatch::new(AlwaysAsk))
            .collect()
    });
}

/// 組み立て済みの [`EnvFilter`] を差してイベントを流し、出力された本文を返す。
///
/// `server::telemetry` は「環境変数からフィルタを組み立てる関数」そのものを
/// 検査対象にしているので、文字列ではなく `EnvFilter` を受け取る口が要る。
pub(crate) fn captured_with_env_filter(filter: EnvFilter, emit: impl FnOnce()) -> String {
    pin_callsite_interest();

    let buffer = SharedBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(buffer.clone())
        .finish();

    tracing::subscriber::with_default(subscriber, emit);

    buffer.contents()
}

/// 指定したフィルタを差してイベントを流し、出力された本文を返す。
pub(crate) fn captured_with_filter(filter: &str, emit: impl FnOnce()) -> String {
    captured_with_env_filter(EnvFilter::new(filter), emit)
}

/// **既定のログフィルタを差して**イベントを流し、出力された本文を返す。
///
/// 見たいのは深刻度そのものではなく「既定の設定で運用者の目に入るか」なので、
/// `DEFAULT_LOG_FILTER` を通した結果を観測する。
pub(crate) fn captured_under_default_filter(emit: impl FnOnce()) -> String {
    captured_with_filter(crate::server::DEFAULT_LOG_FILTER, emit)
}

/// 何もかも通すフィルタ。**否定側テストの「先出し」に使う。**
///
/// 「既定フィルタで出ない」を主張する前に、**そもそもイベントが出ている**ことを
/// これで確かめる。確かめないと、実装から `debug!` を消しただけで
/// 否定側が「何も出なかった」まま緑になる。
pub(crate) const VERBOSE_FILTER: &str = "trace";

/// **`debug!` の行の肯定側に使うフィルタ**（作業単位 G-5 の是正 / 引き上げは G-8/G-9 の是正）。
///
/// # ⚠ [`VERBOSE_FILTER`]（= `trace`）を使わないこと
///
/// **あちらは運用者が実際に打つフィルタより広い。** 実測（G-5 の是正）:
/// `trace` で捕まえていると、**`debug!` を `trace!` へ落とす変異が 1945 全緑**で通る
/// ——**テストは両向きとも緑のまま、運用者からはその行が消える。**
///
/// **コードも doc も `debug` と名乗っており、`RUST_LOG` に `debug` を渡すのが
/// その行を出す唯一の自然な手である。** 物差しはそちらに合わせる。
///
/// **⚠ 値の綴りは `personacore=debug` であって `debug` ではない**
/// （G-5 の是正3 / 2026-09-06 / **doc の本文だけが `RUST_LOG=debug` と書いていた**）。
///
/// # 否定側には使わない
///
/// personacore の行については [`crate::server::DEFAULT_LOG_FILTER`] と同じ水準で、
/// 違うのは **personacore 以外の層に土台を置いていない**点だけ
/// （あちらは先頭に `warn` を敷いている）。
/// **この定数を使うのは肯定側（「その行は現に出ている」）だけ**なので、
/// **見るのは personacore の行そのもの**であり、土台の差は主張に効かない。
/// **否定側は [`crate::server::DEFAULT_LOG_FILTER`]**——運用者が実際に受け取る綴り。
///
/// # ⚠ ここに置いてある理由（**写しを1つにする**）
///
/// **G-8/G-9 の着手時、同じ値と同じ理由が `llm::lmstudio::tests` と
/// `llm::response::tests` に別々の定数として置かれていた。**
/// **同じ発見を2箇所に書かない**（CLAUDE.md）——**片方を直しても、
/// もう片方が黙って古びる。**
///
/// # ⚠ 再発経路は開いたままである（G-9 のレビューが実測 / 2026-09-07）
///
/// **一本化したのは `llm` の2つだけ。** **同じ値の3本目が残っている**
/// ——`src/server/relay.rs` の `PERSIST_ORDER_FILTER`。
/// **そして4本目が生えても、誰も赤にしない**
/// （レビュアーが実際に植えて**全緑**を実測。
/// **走査の帳簿は関数と型しか見ていないので、定数の写しには当たらない**）。
///
/// **「写しを1つにした」ではなく「2つを1つにしただけ」。**
/// **この段は塞いでいない。守れていないことを、そう書いてある。**
pub(crate) const DEBUG_FILTER: &str = "personacore=debug";

/// 行と欄の突き合わせ**と、ログを溜める器**。**`tests/` と共有する1本**
/// （作業単位 B-3a / B-3）。
///
/// `tests/common/mod.rs` が `#[path]` で同じファイルを読む。詳細はそちらの doc。
mod assert;

pub(crate) use assert::{
    bracketed_field, has_field, has_key, line_containing, line_with_field, quoted_field,
};

// 出力を溜めるライタ。**このモジュールの外へは出さない**
// （捕捉は必ず上の関数を通す。直に `with_default` を張られると番人が効かない）。
//
// **B-3 で `tests/` と共有した**（それまでは同じ型が計4つ複製されていた）。
use assert::SharedBuffer;

#[cfg(test)]
mod tests {
    use super::*;

    /// **番人を常駐させても、捕捉したい側のイベントは殺されない。**
    ///
    /// 番人は全 callsite の interest（`Interest::sometimes()`）と
    /// プロセス全体の最大水準（`max_level_hint`）に口を出す。ここを
    /// `Interest::never()` や低い `max_level_hint` にすると、
    /// **焼き付き対策のつもりで捕捉そのものを黙らせる**ことになる。
    ///
    /// **番人の `enabled` / `event` を見ているのではない**（そこは到達しない。
    /// [`AlwaysAsk`] の doc を参照）。見ているのは番人の**副作用**の方。
    #[test]
    fn the_sentinel_does_not_suppress_capture_via_interest_or_max_level() {
        let output = captured_with_filter(VERBOSE_FILTER, || {
            tracing::warn!("番人の下でも出ること");
            tracing::trace!("最大水準を下げられていないこと");
        });

        assert!(output.contains("番人の下でも出ること"), "{output:?}");
        assert!(
            output.contains("最大水準を下げられていないこと"),
            "`LevelFilter::current()` を押し下げていないこと: {output:?}"
        );
    }

    /// 焼き付き検査専用の callsite。**他から呼ばないこと**
    /// （先に登録されると、下のテストが仕込む順序が崩れる）。
    fn emit_probe(tag: &str) {
        tracing::warn!(%tag, "焼き付き検査のマーカー");
    }

    /// **番人が常駐していれば `sometimes` と答える。** 決定的に確かめる側。
    ///
    /// 下の [`a_callsite_registered_by_a_bare_thread_is_not_burned_to_never`] は
    /// 焼き付きを丸ごと再現する代わりに、**他のテストと同時に走ると緑を通り抜ける**
    /// （そちらの doc を参照）。番人の欠落を**必ず**捕まえるのはこちら。
    ///
    /// 見ているのは2つだけ:
    ///
    /// 1. 番人が**1つ以上**常駐していること。
    /// 2. その番人が `Interest::sometimes()` と答えること。
    ///
    /// 変異検査（2026-08-15 / `cargo test --lib` 既定 32 スレッド）:
    /// `SENTINEL_COUNT = 0` で **8/8 赤**、`Interest::never()` で **3/3 赤**。
    ///
    /// `&'static Metadata` は [`OBSERVED_CALLSITE`] から借りる。
    /// **番人が現に訊かれていなければそこが空**なので、
    /// 「番人が登録すらされていない」もここで赤になる。
    ///
    /// [`a_callsite_registered_by_a_bare_thread_is_not_burned_to_never`]:
    ///     a_callsite_registered_by_a_bare_thread_is_not_burned_to_never
    #[test]
    fn the_sentinels_are_registered_and_always_ask() {
        pin_callsite_interest();

        // 番人が居る状態で新しい callsite を1つ登録させる。この登録が起こす
        // rebuild で、登録済み dispatcher（＝番人）の `register_callsite` が呼ばれる。
        tracing::warn!("番人の配線を確かめる");

        let sentinels = SENTINELS.get().expect("番人を常駐させてあること");
        assert!(
            !sentinels.is_empty(),
            "番人が1つも居なければ interest の焼き付きが復活する"
        );

        let metadata = OBSERVED_CALLSITE
            .get()
            .copied()
            .expect("番人が callsite の interest を訊かれていること（訊かれないなら効いていない）");

        for (index, sentinel) in sentinels.iter().enumerate() {
            assert!(
                sentinel.register_callsite(metadata).is_sometimes(),
                "番人 #{index} は `sometimes` と答えること（`never` を混ぜると自分で焼き付ける）"
            );
        }
    }

    /// **番人が居なければ落ちる形で、焼き付きそのものを再現する。**
    ///
    /// 捕捉の窓の中から、**まだ一度も登録されていない callsite** を
    /// subscriber を差していないスレッドに先に踏ませる。
    /// 番人を外すと `has_just_one` が true のままなので、
    /// その素のスレッドの `NoSubscriber` が `Interest::never()` を焼き付け、
    /// **続く捕捉側の同じ行が丸ごと消える**（＝この assert が赤になる）。
    ///
    /// # このテストだけでは番人の欠落を捕まえきれない
    ///
    /// **他のログ観測テストと同時に走ると、その `with_default` の dispatcher が
    /// 登録済み一覧に残っているせいで `has_just_one` が false になり、
    /// 焼き付きが起こらない。** `SENTINEL_COUNT = 0` の変異での実測（2026-08-15）:
    ///
    /// | 走らせ方 | 結果 |
    /// |---|---|
    /// | `cargo test --lib`（既定 32 スレッド）× 8 | **0/8 赤**（全部素通り） |
    /// | `cargo test --lib -- --test-threads=1` | 赤 |
    /// | このテストだけを名前で絞って実行 | 赤 |
    ///
    /// 以前は捕捉を `Mutex` で直列化していたので `has_just_one` が true に
    /// 張り付き、既定の走らせ方でも決定的に赤だった。**錠を外した副作用で
    /// この見張り番は条件付きになった**（モジュール doc の「却下した手」も参照）。
    /// 決定的な番はひとつ上の [`the_sentinels_are_registered_and_always_ask`] が担う。
    ///
    /// 偽陽性（番人が居るのに赤）にはならないので、条件付きのまま残してある。
    ///
    /// [`the_sentinels_are_registered_and_always_ask`]:
    ///     the_sentinels_are_registered_and_always_ask
    #[test]
    fn a_callsite_registered_by_a_bare_thread_is_not_burned_to_never() {
        let output = captured_with_filter(VERBOSE_FILTER, || {
            std::thread::spawn(|| emit_probe("bare"))
                .join()
                .expect("スレッドが完了すること");
            emit_probe("captured");
        });

        assert!(
            output.contains("焼き付き検査のマーカー"),
            "素のスレッドが先に登録した行を never に焼き付けさせないこと: {output:?}"
        );
        // **欄は、その文言を出した行の中で見る**（作業単位 B-21d）。
        // [`emit_probe`] は文言と `tag=` を**同じイベント**に載せるので、
        // 「文言が出た」と「捕捉側だと言っている」を塊に対して別々に主張すると、
        // **別々の行で満たされても緑**になる（`has_field` は `split_whitespace` で
        // 改行をまたぐ / [`assert::has_field`] の doc）。
        //
        // **欄はトークンとして突き合わせる**（作業単位 B-12）。
        // **ここは [`has_field`] の振る舞いを検査している側ではない**ので、
        // 他の呼び出し側と同じく共有の1本を通す
        // （[`has_field`] 自身の押さえは下の
        // [`has_field_matches_a_whole_token_and_nothing_longer`] が
        // **関数を通さないべた書き**で持っている）。
        //
        // [`has_field_matches_a_whole_token_and_nothing_longer`]:
        //     has_field_matches_a_whole_token_and_nothing_longer
        let probe = line_containing(&output, "焼き付き検査のマーカー");
        assert!(
            has_field(probe, "tag=captured"),
            "捕捉側のイベントであること: {probe:?}"
        );
    }

    // ------------------------------------------------------------------
    // 行の切り出し（作業単位 B-2）
    // ------------------------------------------------------------------

    /// **植えた出力**。実ログを模してあるが、[`line_containing`] を通さずに書いてある。
    ///
    /// **2行とも `WARN` を持ち、2行とも `session_id=` を持つ**——
    /// これが 24-2 と B-1-2 で実際に通り抜けた形（無関係の行が答えを供給する）。
    /// **狙いの行だけが `session_id=7`** で、他の行は `session_id=1`。
    const PLANTED: &str = "\
2026-08-26T00:00:00Z  WARN personacore::a: 別の警告 session_id=1 outcome=applied
2026-08-26T00:00:01Z  INFO personacore::b: 狙いの記録 session_id=7 outcome=skipped
2026-08-26T00:00:02Z  WARN personacore::c: また別の警告 session_id=1 outcome=applied";

    /// **手掛かりの行だけを返す。出力全体を返さない。**
    ///
    /// # なぜ「関数の外の値」で押さえるのか
    ///
    /// [`line_containing`] は `src/**` と `tests/**` のログ観測テストが**全員**使う
    /// （B-3a で `tests/` も同じファイルを読むようになった）。
    /// 呼び出し側は例外なくこの関数の戻り値に対して主張を書くので、
    /// **ここが緩めば呼び出し側の期待値も一緒に緩み、全部緑のまま**になる
    /// （CLAUDE.md「写しを1つにしても、その1つは自動的には守られない」）。
    ///
    /// **`tests/` 側にこの押さえの写しは置かない。**
    /// 押さえるべき対象が[同じ1本のファイル](super::assert)なので、
    /// ここが赤くなれば `tests/` 側の道具も同時に壊れている
    /// （**`#[path]` が離れていないこと**と
    /// **`fn line_containing` の定義が戻っていないこと**は
    /// [`the_integration_tests_share_this_file_instead_of_copying_it`] が見張り、
    /// **名前を変えただけの写し**は
    /// [`no_integration_test_copies_a_shared_body_under_another_name`] が見張る。
    /// **本文まで書き直した別名はそれでも捕まらない**——両方の doc の表を読むこと）。
    ///
    /// [`no_integration_test_copies_a_shared_body_under_another_name`]:
    ///     no_integration_test_copies_a_shared_body_under_another_name
    ///
    /// [`the_integration_tests_share_this_file_instead_of_copying_it`]:
    ///     the_integration_tests_share_this_file_instead_of_copying_it
    ///
    /// 期待値は [`PLANTED`] の2行目を**べた書きした文字列**にしてある。
    /// `output` をそのまま返す変異も、`lines()` の探索を先頭行に決め打つ変異も、
    /// この `assert_eq!` で必ず赤くなる。
    #[test]
    fn line_containing_returns_only_the_line_that_carries_the_marker() {
        let line = line_containing(PLANTED, "狙いの記録");

        assert_eq!(
            line,
            "2026-08-26T00:00:01Z  INFO personacore::b: 狙いの記録 session_id=7 outcome=skipped",
            "手掛かりを含む1行だけを返すこと"
        );
        assert!(!line.contains('\n'), "複数行を返さないこと: {line:?}");
    }

    /// **他の行の深刻度と欄を拾わせない**（この関数の存在理由そのもの）。
    ///
    /// 前提として、[`PLANTED`] 全体に対して `contains` で見ると
    /// **どちらの主張も満たされてしまう**ことを先に確かめる。
    /// この前提が偽になったら、下の主張は「そもそも紛れが無い」を見ているだけになる。
    #[test]
    fn line_containing_does_not_let_a_neighbouring_line_answer() {
        // **前提: 出力全体で見れば、間違った答えが手に入る。**
        assert!(
            PLANTED.contains("WARN"),
            "前提: 出力のどこかに WARN が居ること"
        );
        assert!(
            PLANTED.contains("session_id=1"),
            "前提: 出力のどこかに別の往復の欄が居ること"
        );

        let line = line_containing(PLANTED, "狙いの記録");

        assert!(
            !line.contains("WARN"),
            "隣の行の深刻度を拾わないこと（狙いの行は INFO）: {line:?}"
        );
        assert!(
            !line.contains("session_id=1"),
            "隣の行の欄を拾わないこと（狙いの行は session_id=7）: {line:?}"
        );
        assert!(
            line.contains("session_id=7"),
            "狙いの行の欄はそのまま読めること: {line:?}"
        );
    }

    /// **手掛かりが無ければ落とす。**
    ///
    /// `Option` を返して呼び出し側が `if let` で畳むと、
    /// **イベントが丸ごと消えた変異が「行が無いので何も主張しない」で緑**になる。
    #[test]
    #[should_panic(expected = "を含む行があること")]
    fn line_containing_panics_when_the_marker_is_absent() {
        let _ = line_containing(PLANTED, "この文言はどの行にも無い");
    }

    // ------------------------------------------------------------------
    // 欄で行を選ぶ（作業単位 B-21b）
    // ------------------------------------------------------------------

    /// **植えた出力。狙いの行を、端でも唯一でもない位置に置いてある**（作業単位 B-21b）。
    ///
    /// | 行 | 役目 |
    /// |---|---|
    /// | 1 | **囮**（キーの側が伸びた欄） |
    /// | 2 | **囮**（値の側が伸びた欄） |
    /// | 3 | **狙い**——**最初に一致する行。期待値はこれ** |
    /// | 4 | **同じ欄をもう1本**（`session_id` が別なので3行目と区別できる） |
    ///
    /// **⚠ 4行目はレビューの指摘で足した**（B-21b / 2026-08-28）。
    /// 無かった版では**後ろから探す変異（`.rev()`）が振る舞いのテストを1本も赤にせず**、
    /// **唯一の赤は写し検出の文字列走査**だった（＝向きの権威がどこにも無かった）。
    /// **端は元から塞がっていた**——「常に先頭」は肯定側が、「常に末尾」は
    /// [`line_with_field_panics_when_the_field_is_absent`] の対照が赤にする。
    /// **塞がっていなかったのは「複数一致のときどちらを返すか」だけ。**
    ///
    /// [`line_with_field_panics_when_the_field_is_absent`]:
    ///     line_with_field_panics_when_the_field_is_absent
    ///
    /// 囮は**実際に狙っている相手と同じ形**にしてある
    /// （CLAUDE.md「囮は、実際に狙っている相手と同じ形でなければ意味がない」/ B-12b の再発）。
    /// 欄を素の部分一致で探したときに掴む相手は、**2種類しかない:**
    ///
    /// | 囮 | 何が伸びたか |
    /// |---|---|
    /// | `x_outcome=skipped` | **キーの側**（`x_` を付ける変異が作る形） |
    /// | `outcome=skipped_x` | **値の側**（ラベルを伸ばす変異が作る形） |
    ///
    /// **⚠ 囮も「同じ欄のもう1本」も、置く位置が要点。** `lines().find(..)` は先頭から探すので、
    /// 判定が `contains` へ緩めば**囮の行が返る。**
    /// 囮を後ろに置くと、緩めても狙いの行が先に当たって**緑のまま**になる。
    /// 逆に「同じ欄のもう1本」は**狙いより後ろ**でなければならない
    /// ——前に置くと、それが「最初の一致」になって狙いの行が観測できない。
    const PLANTED_WITH_DECOYS: &str = "\
2026-08-26T00:00:00Z  INFO personacore::a: 囮その1 x_outcome=skipped session_id=1
2026-08-26T00:00:01Z  INFO personacore::b: 囮その2 outcome=skipped_x session_id=2
2026-08-26T00:00:02Z  INFO personacore::c: 狙いの記録 outcome=skipped session_id=7
2026-08-26T00:00:03Z  INFO personacore::d: 後から出た同じ欄 outcome=skipped session_id=9";

    /// **欄をトークンとして丸ごと持つ行だけを返す。伸びた欄の行は掴まない。**
    ///
    /// # なぜ「関数の外の値」で押さえるのか
    ///
    /// [`line_containing`] の押さえと同じ理由——
    /// **呼び出し側は例外なくこの関数の戻り値に対して主張を書く**ので、
    /// ここが緩めば呼び出し側の期待値も一緒に緩み、**全部緑のまま**になる。
    /// 期待値は [`PLANTED_WITH_DECOYS`] の3行目を**べた書きした文字列**にしてある。
    ///
    /// # ここで赤くなる変異
    ///
    /// | 変異 | どの assert が捕まえるか |
    /// |---|---|
    /// | 判定を `line.contains(field)` へ緩める | **囮の1行目**が返るので `assert_eq!` |
    /// | 探索を先頭行に決め打つ | 同上 |
    /// | `output` をそのまま返す | 改行を含むので `assert_eq!` と最後の assert |
    /// | **後ろから探す**（`.rev()`） | **4行目**が返るので `assert_eq!`（**レビュー指摘で足した行**） |
    /// | 最後の一致を返す（`.last()`） | 同上 |
    ///
    /// # これは [`line_with_field_panics_when_the_field_is_absent`] の**対照**でもある
    ///
    /// `#[should_panic]` は「落ちた」しか言わない（CLAUDE.md / B-3）。
    /// **「常に落ちる」実装**をこちらが赤にすることで、あちらの主張が
    /// 「**その入力で**落ちる」になる。
    ///
    /// [`line_with_field_panics_when_the_field_is_absent`]:
    ///     line_with_field_panics_when_the_field_is_absent
    #[test]
    fn line_with_field_returns_only_the_line_whose_field_matches_as_a_whole_token() {
        // **前提: 素の部分一致なら囮が先に当たる。**
        // これが偽になったら、下の主張は「そもそも紛れが無い」を見ているだけになる。
        let decoys: Vec<&str> = PLANTED_WITH_DECOYS.lines().take(2).collect();
        assert_eq!(decoys.len(), 2, "前提: 囮が2行あること");
        for decoy in &decoys {
            assert!(
                decoy.contains("outcome=skipped"),
                "前提: 囮は部分一致では当たること（当たらないなら囮になっていない）: {decoy:?}"
            );
        }

        // **前提: 同じ欄を丸ごと持つ行が2本ある**（1本しか無いと「最初を返す」が観測できない）。
        // **`has_field` を通さずに見る**——道具が緩んだときに前提まで一緒に緩まないように。
        let lines: Vec<&str> = PLANTED_WITH_DECOYS.lines().collect();
        assert_eq!(lines.len(), 4, "前提: 植えた出力は4行であること");
        assert!(
            lines[2].contains(" outcome=skipped ") && lines[3].contains(" outcome=skipped "),
            "前提: 3行目と4行目が同じ欄を丸ごと持つこと: {lines:?}"
        );
        assert_ne!(lines[2], lines[3], "前提: その2本は互いに別の行であること");

        let line = line_with_field(PLANTED_WITH_DECOYS, "outcome=skipped");

        assert_eq!(
            line,
            "2026-08-26T00:00:02Z  INFO personacore::c: 狙いの記録 outcome=skipped session_id=7",
            "欄をトークンとして丸ごと持つ1行だけを返すこと（囮を掴まないこと）"
        );
        assert!(!line.contains('\n'), "複数行を返さないこと: {line:?}");
    }

    /// **その欄を持つ行が無ければ落とす。**
    ///
    /// `Option` を返して呼び出し側が `if let` で畳むと、
    /// **欄が丸ごと消えた変異が「行が無いので何も主張しない」で緑**になる
    /// （[`line_containing`] の押さえと同じ理由）。
    ///
    /// **囮は残したまま**問い合わせる——`x_outcome=skipped` も `outcome=skipped_x` も
    /// **`outcome=applied` ではない**ので、落ちるのが正しい。
    #[test]
    #[should_panic(expected = "を持つ行があること")]
    fn line_with_field_panics_when_the_field_is_absent() {
        let _ = line_with_field(PLANTED_WITH_DECOYS, "outcome=applied");
    }

    // ------------------------------------------------------------------
    // 欄の突き合わせ（作業単位 B-2-2）
    // ------------------------------------------------------------------

    /// **`キー=値` はトークンとして完全一致。伸びた値には当たらない。**
    ///
    /// # 期待値を [`has_field`] から作らない
    ///
    /// 入力も期待値も**べた書きの文字列**にしてある。
    /// 関数を通して期待値を組み立てると、**関数が緩んだときに期待値も一緒に緩み、
    /// この見張り番ごと緑になる**（CLAUDE.md「写しを1つにしても、その1つは
    /// 自動的には守られない」）。
    ///
    /// # 何を捕まえるためのものか
    ///
    /// `line.contains(field)` へ緩める変異は、**最初の否定**（`a=10` に `a=1` が
    /// 当たる）で赤くなる。`starts_with` / `ends_with` へ緩める変異も
    /// それぞれ下の否定のどれかで赤くなる。
    #[test]
    fn has_field_matches_a_whole_token_and_nothing_longer() {
        // 肯定: トークンとして在れば、行のどこにあっても見つかる。
        assert!(has_field("a=1 b=2", "a=1"));
        assert!(has_field("a=1 b=2", "b=2"));

        // 否定: **桁が伸びた値**（`件数=1` と `件数=10`）。
        assert!(!has_field("a=10", "a=1"));
        // 否定: 逆向き（探す側が長い）。
        assert!(!has_field("a=1", "a=10"));
        // 否定: **接尾辞が伸びた値**（`結末=structured` と `結末=structured_nested`）。
        assert!(!has_field("a=1_x", "a=1"));
        // 否定: **キーの側が伸びた**（`session_id=1` と `x_session_id=1`）。
        assert!(!has_field("xa=1", "a=1"));
        // 否定: 値を落とした手掛かりは当たらない（`a=1` は `a` ではない）。
        assert!(!has_field("a=1", "a"));
    }

    /// **`has_key` は値を一切見ない。キーが伸びた別の欄には当たらない**（作業単位 B-21d）。
    ///
    /// # 期待値を [`has_key`] から作らない
    ///
    /// 入力も期待値も**べた書きの文字列**にしてある（[`has_field`] の押さえと同じ理由）。
    ///
    /// # 何を捕まえるためのものか
    ///
    /// | 変異 | 赤にする否定 |
    /// |---|---|
    /// | `has_field` と同じ**完全一致**へ狭める | 最初の肯定（`a=2` を `a` で探す） |
    /// | 素の `contains` へ緩める | **キーが伸びた**否定（`xa=1`） |
    /// | `=` を付けずに前方一致する | **キーが前方一致で伸びた**否定（`ab=1`） |
    /// | 値の側まで見てしまう | 2つ目の肯定（`a=` だけで在ると答える） |
    #[test]
    fn has_key_ignores_the_value_and_never_matches_a_longer_key() {
        // 肯定: **値が何であっても在ると答える。** ここが `has_field` との差。
        assert!(has_key("a=1 b=2", "a"));
        assert!(has_key("a=2 b=2", "a"));
        assert!(has_key("a=10 b=2", "a"));
        // 肯定: 値が空でも、欄としては在る。
        assert!(has_key("a= b=2", "a"));

        // 否定: **キーの側が伸びた別の欄**（`extracted=` と `x_extracted=`）。
        // **これが素の `contains("a=")` との差**——あちらは当たって誤って赤くする。
        assert!(!has_key("xa=1", "a"));
        // 否定: **キーが前方一致で伸びた別の欄**（`a` と `ab`）。`=` まで見ている証拠。
        assert!(!has_key("ab=1", "a"));
        // 否定: そのキーがそもそも無い。
        assert!(!has_key("b=1", "a"));
        // 否定: **`=` が付いていない**（値ではなく本文に現れただけ）。
        assert!(!has_key("a b=1", "a"));
    }

    // ------------------------------------------------------------------
    // 値に空白を含む欄（作業単位 B-12b）
    // ------------------------------------------------------------------

    /// **`キー=[…]` は閉じ括弧までを1つの値として読む。1文字でも伸びたら受理しない。**
    ///
    /// # 期待値を [`bracketed_field`] から作らない
    ///
    /// 入力も期待値も**べた書きの文字列**にしてある（[`has_field`] の押さえと同じ理由）。
    /// 関数を通して期待値を組み立てると、**関数が緩んだときに期待値も一緒に緩む。**
    ///
    /// # 何を捕まえるためのものか（**全部、実測して当てた形**）
    ///
    /// | 変異 | ここで赤くなる主張 |
    /// |---|---|
    /// | 欄名に接頭辞（`x_` を足す） | 「キーの側が伸びた」の否定**2本** |
    /// | 値の末尾に1文字（`["A"]` → `["A"]!`） | 「1文字長い値」の否定 |
    /// | 門1を「もっともらしい別規則」へ（`!before.is_alphanumeric()`） | **`x_a=` の側**（`xa=` では素通りする） |
    /// | 探索を先頭の一致だけに畳む（`match_indices(..).take(1)`） | **`x_a=[…] a=[…]` を読む1本**（一致が2回ある唯一の入力） |
    ///
    /// **`line.contains(&format!("{key}={value}"))` へ退化させる変異は、
    /// 最初の2つの否定でどちらも赤になる。**
    ///
    /// **⚠ 下の2行は B-12b のレビューで足した。** それまで**囮が英数字の `xa=` だけ**で、
    /// **この作業単位が現に狙う `x_axis_leak_narration`（アンダースコア）と形が違っていた**
    /// ——門1を `!is_alphanumeric()` へ緩める変異が**単独で 1584 全緑**、
    /// **本番の接頭辞変異と組にすると 7 赤が 2 赤まで落ちた**（残るのは `has_field` の側だけ）。
    ///
    /// **⚠ 是正しても「組にしたときの呼び出し側の赤」は戻らない**（是正後に実測: **3 赤** ——
    /// `has_field` の2本と**この見張り番**。帰属の本命・新テスト・統合の2本は緑のまま）。
    /// **道具が緩めば、道具に判断を預けている呼び出し側は一緒に緩む**
    /// （CLAUDE.md「写しを1つにしても、その1つは自動的には守られない」）。
    /// **だから道具の権威はこの見張り番自身**で、**単独で赤になること**が要件になる。
    #[test]
    fn bracketed_field_reads_to_the_closing_bracket_and_no_further() {
        // 肯定: **値に空白が入っても**1つの値として読める（`has_field` が使えない形）。
        assert_eq!(bracketed_field(r#"a=["x y"] b=2"#, "a"), Some(r#"["x y"]"#));
        // 肯定: **2件以上**（`Debug` の列は `, ` で区切られる）。
        assert_eq!(
            bracketed_field(r#"a=["x", "y"] b=2"#, "a"),
            Some(r#"["x", "y"]"#)
        );
        // 肯定: 空の欄も読める。
        assert_eq!(bracketed_field("a=[] b=2", "a"), Some("[]"));
        // 肯定: **行末**にあっても読める（閉じ括弧の後ろに何も無い）。
        assert_eq!(bracketed_field(r#"b=2 a=["x"]"#, "a"), Some(r#"["x"]"#));
        // 肯定: 手前に**別の欄の値**があっても、そちらを答えない。
        assert_eq!(bracketed_field(r#"b=["y"] a=["x"]"#, "a"), Some(r#"["x"]"#));

        // 否定: **1文字長い値**（`["x"]` と `["x"]!`）。
        assert_eq!(bracketed_field(r#"a=["x"]! b=2"#, "a"), None);
        // 否定: **キーの側が伸びた**（`a=…` と `xa=…`）。
        assert_eq!(bracketed_field(r#"xa=["x"]"#, "a"), None);
        // 否定: **キーの側が `_` で伸びた**——**この作業単位が現に捕まえる形**
        // （本番の変異は `x_axis_leak_narration`）。**`xa=` の囮だけでは覆えない**
        // ——門を「もっともらしい別規則」（`!before.is_alphanumeric()`）へ緩める変異は、
        // **英数字の囮を素通りする**（B-12b のレビューで実測: 単独で 1584 全緑）。
        // **⚠ 囮は、現に狙っている相手と同じ形でなければ意味がない。**
        assert_eq!(bracketed_field(r#"x_a=["x"]"#, "a"), None);
        // 否定 兼 **「囮を飛び越えて本物を読む」**。
        // **`a=[` が2回一致する唯一の入力**なので、探索を先頭の一致だけに畳む変異
        // （`match_indices(..).take(1)` ＝ 素の `find` 相当）もここで赤くなる。
        // **上の `b=["y"] a=["x"]` は一致が1つしかなく、その枝を通らない**（同上の実測）。
        assert_eq!(
            bracketed_field(r#"x_a=["y"] a=["x"]"#, "a"),
            Some(r#"["x"]"#)
        );
        // 否定: 閉じ括弧が無い（読めないなら答えない）。
        assert_eq!(bracketed_field(r#"a=["x""#, "a"), None);
        // 否定: **値の中に `]` がある**（`StatAxis` は `]` を許すので、これは作れる）。
        // **読めないなら黙って半分を答えず、`None` で呼び出し側を落とす**
        // ——偽陽性はうるさく落ちるだけ。偽陰性（伸びた値を同じと読む）は黙って通る。
        assert_eq!(bracketed_field(r#"a=["x]y"] b=2"#, "a"), None);
        // 否定: 括弧で始まらない値（そちらは [`has_field`] の領分）。
        assert_eq!(bracketed_field("a=1 b=2", "a"), None);
        // 否定: その欄が無い（値を捏造しないこと）。
        assert_eq!(bracketed_field(r#"b=["y"]"#, "a"), None);
    }

    /// **`キー="…"` は閉じ引用符までを1つの値として読む。名前が伸びた欄からは読まない。**
    /// （作業単位 B-32）
    ///
    /// # 期待値を [`quoted_field`] から作らない
    ///
    /// 入力も期待値も**べた書きの文字列**にしてある（[`has_field`] の押さえと同じ理由）。
    ///
    /// # 何を捕まえるためのものか
    ///
    /// | 変異 | ここで赤くなる主張 |
    /// |---|---|
    /// | 規則 1 を落とす（素の `find` へ退化） | 「キーの側が伸びた」の否定**2本** |
    /// | 規則 1 を「もっともらしい別規則」へ（`!before.is_alphanumeric()`） | **`x_a=` の側**（`xa=` では素通りする） |
    /// | 規則 3 を落とす | 「1文字長い値」の否定 |
    /// | エスケープを見ない | **`\"` を含む値**の肯定（手前で切れる） |
    /// | エスケープを**復号**する | 同上（`\n` が生の改行になったら値が変わる） |
    /// | 探索を先頭の一致だけに畳む（`match_indices(..).take(1)`） | **`x_a="y" a="x"` を読む1本** |
    /// | 引用符ごと返す | すべての肯定 |
    ///
    /// **`server::drift` の [`raw_field`] が実際に踏んだのは1行目**
    /// ——**素の `find` が `x_` の付いた囮の中から値を読んでいた**（B-21e の実測 / B-32 の出発点）。
    ///
    /// [`raw_field`]: crate::server::drift
    #[test]
    fn quoted_field_reads_to_the_closing_quote_and_no_further() {
        // 肯定: **値に空白が入っても**1つの値として読める（[`has_field`] が使えない形）。
        assert_eq!(quoted_field(r#"a="x y" b=2"#, "a"), Some("x y"));
        // 肯定: 空の値も読める。
        assert_eq!(quoted_field(r#"a="" b=2"#, "a"), Some(""));
        // 肯定: **行末**にあっても読める（閉じ引用符の後ろに何も無い）。
        assert_eq!(quoted_field(r#"b=2 a="x""#, "a"), Some("x"));
        // 肯定: 手前に**別の欄の値**があっても、そちらを答えない。
        assert_eq!(quoted_field(r#"b="y" a="x""#, "a"), Some("x"));
        // 肯定: **エスケープした引用符は値の一部**（`Debug` の書き方そのもの）。
        //       **復号しない**——`\` と `"` の2文字がそのまま返る。
        assert_eq!(quoted_field(r#"a="x\"y" b=2"#, "a"), Some(r#"x\"y"#));
        // 肯定: **改行のエスケープも2文字のまま**（生の改行にしない）。
        //       ここを復号すると「1イベントが1行に収まっている」を見る側が壊れる。
        assert_eq!(quoted_field(r#"a="x\ny" b=2"#, "a"), Some(r#"x\ny"#));
        // 肯定: **逆斜線が2つ**（エスケープされた逆斜線）。次の引用符は閉じ引用符。
        assert_eq!(quoted_field(r#"a="x\\" b=2"#, "a"), Some(r#"x\\"#));

        // 否定: **1文字長い値**（`"x"` と `"x"!`）。
        assert_eq!(quoted_field(r#"a="x"! b=2"#, "a"), None);
        // 否定: **キーの側が伸びた**（`a=…` と `xa=…`）。
        assert_eq!(quoted_field(r#"xa="x""#, "a"), None);
        // 否定: **キーの側が `_` で伸びた**——**本番で現に起きた形**
        //       （`x_` の接頭辞）。**`xa=` の囮だけでは覆えない**
        //       ——規則 1 を `!before.is_alphanumeric()` へ緩める変異は英数字の囮を素通りする。
        assert_eq!(quoted_field(r#"x_a="x""#, "a"), None);
        // 否定 兼 **「囮を飛び越えて本物を読む」**。
        // **`a="` が2回一致する唯一の入力**なので、探索を先頭の一致だけに畳む変異
        // （`match_indices(..).take(1)` ＝ 素の `find` 相当）もここで赤くなる。
        assert_eq!(quoted_field(r#"x_a="y" a="x""#, "a"), Some("x"));
        // 否定: **キーが前方一致で伸びた別の欄**（`a` と `ab`）。`=` まで見ている証拠。
        assert_eq!(quoted_field(r#"ab="x""#, "a"), None);
        // 否定: 閉じ引用符が無い（読めないなら答えない）。
        assert_eq!(quoted_field(r#"a="x"#, "a"), None);
        // 否定: **最後まで閉じないエスケープ**（`\"` は閉じ引用符ではない）。
        assert_eq!(quoted_field(r#"a="x\""#, "a"), None);
        // 否定: 引用符で始まらない値（そちらは [`has_field`] の領分）。
        assert_eq!(quoted_field("a=1 b=2", "a"), None);
        // 否定: **括弧で始まる値**（そちらは [`bracketed_field`] の領分）。
        assert_eq!(quoted_field(r#"a=["x"] b=2"#, "a"), None);
        // 否定: その欄が無い（値を捏造しないこと）。
        assert_eq!(quoted_field(r#"b="y""#, "a"), None);
    }

    // ------------------------------------------------------------------
    // ログを溜める器（作業単位 B-3）
    // ------------------------------------------------------------------

    /// **区間より前に書いた分は返さない。** [`SharedBuffer::contents_since`] の権威。
    ///
    /// # なぜここに要るのか（**呼び出し側では観測できない**）
    ///
    /// 区間の仕掛けを使うのは `tests/server_chat_parse_log.rs` の `LogSection` だけで、
    /// **あちらの見張り番は `tracing` のイベントを流して観測する。**
    /// つまり「区間が効いているか」と「イベントが出ているか」が同じ1本に混ざる。
    /// ここは **`tracing` を一切通さず、`Write` に直接バイト列を書く**ので、
    /// 区間の切り出しだけを裸で見られる。
    ///
    /// **期待値はべた書き**（関数を通して組み立てない / CLAUDE.md「写しを1つにしても…」）。
    ///
    /// 見ているもの:
    ///
    /// 1. **区間の後ろだけ**が返る（＝区間を無視して全体を返す実装は赤）。
    /// 2. **全体**には区間より前の分も入る（＝「いつも空」で 1. が成り立っているのではない）。
    /// 3. **2つが別の文字列**であること（前提。同値だと 1. が何も言っていない）。
    #[test]
    fn the_buffer_hands_out_only_what_was_written_after_the_mark() {
        use std::io::Write as _;

        let buffer = SharedBuffer::default();
        let mut writer = buffer.clone();

        writer.write_all(b"before\n").expect("書き込めること");
        let mark = buffer.mark();
        writer.write_all(b"after\n").expect("書き込めること");

        // 3. 前提: 区間と全体が別の文字列になる状況で測っていること。
        assert_ne!(
            buffer.contents_since(mark),
            buffer.contents(),
            "前提: 区間より前にも書いてあること（同値だと下の 1. が何も言わない）"
        );

        // 1. 区間の後ろだけ。
        assert_eq!(
            buffer.contents_since(mark),
            "after\n",
            "区間より後ろに書いた分だけを返すこと"
        );
        // 2. 全体には前の分も入る。
        assert_eq!(
            buffer.contents(),
            "before\nafter\n",
            "全体には区間より前の分も入ること"
        );
    }

    /// 区間の検査に使う多バイト文字。**`\u` の綴りで書く**（CLAUDE.md）。
    ///
    /// **生の文字をソースに置かないこと。** 編集の道具が非 ASCII を
    /// `\uXXXX` へ変換したり二重エンコードしたりする事故が実際に起きており、
    /// **振る舞いは正しいままなので緑を通る。**
    /// 綴りではなく**バイト列そのもの**を下の前提で固定してある。
    const MULTIBYTE: &str = "\u{3042}";

    /// **文字の途中で切ったら、黙って進まずに落ちる**（作業単位 B-3 / 司令塔の採否）。
    ///
    /// `tracing` の fmt 層は正当な UTF-8 しか書かないので、
    /// **不正な UTF-8 が出てくるのは [`SharedBuffer::contents_since`] が
    /// 文字の途中で切ったとき＝ `mark` の扱いが壊れたときだけ。**
    /// `from_utf8_lossy` だと **U+FFFD になって黙って進み**、
    /// 突き合わせがたまたま通れば**壊れた区間のまま緑**になる。
    ///
    /// 対照は下の [`a_mark_on_a_character_boundary_reads_the_multibyte_text_back`]
    /// ——**境目の `mark` では落ちない**（「いつも落ちる」で緑になっているのではない）。
    ///
    /// [`a_mark_on_a_character_boundary_reads_the_multibyte_text_back`]:
    ///     a_mark_on_a_character_boundary_reads_the_multibyte_text_back
    #[test]
    #[should_panic(expected = "UTF-8 として読めません")]
    fn a_mark_in_the_middle_of_a_character_fails_loudly() {
        use std::io::Write as _;

        // **前提はバイト列で固定する**（綴りが化けても、ここで気付ける）。
        assert_eq!(
            MULTIBYTE.as_bytes(),
            [0xE3, 0x81, 0x82],
            "前提: 3 バイトの文字であること"
        );

        let buffer = SharedBuffer::default();
        let mut writer = buffer.clone();
        writer
            .write_all(MULTIBYTE.as_bytes())
            .expect("書き込めること");

        // 1 バイト目と 2 バイト目の境目。**文字の途中。**
        let _ = buffer.contents_since(1);
    }

    /// **境目の `mark` なら、多バイト文字はそのまま読み戻せる**（上の対照）。
    ///
    /// これが無いと、`contents_since` を「常に落ちる」にする変異でも
    /// 上の [`a_mark_in_the_middle_of_a_character_fails_loudly`] は緑のまま通る。
    ///
    /// [`a_mark_in_the_middle_of_a_character_fails_loudly`]:
    ///     a_mark_in_the_middle_of_a_character_fails_loudly
    #[test]
    fn a_mark_on_a_character_boundary_reads_the_multibyte_text_back() {
        use std::io::Write as _;

        assert_eq!(
            MULTIBYTE.as_bytes(),
            [0xE3, 0x81, 0x82],
            "前提: 3 バイトの文字であること"
        );

        let buffer = SharedBuffer::default();
        let mut writer = buffer.clone();

        writer
            .write_all(MULTIBYTE.as_bytes())
            .expect("書き込めること");
        // **文字の境目**（1 文字ぶん書き終えたところ）。
        let mark = buffer.mark();
        assert_eq!(mark, 3, "前提: 区間の左端が文字の境目にあること");
        writer
            .write_all(MULTIBYTE.as_bytes())
            .expect("書き込めること");

        assert_eq!(
            buffer.contents_since(mark),
            MULTIBYTE,
            "境目で切れば、多バイト文字はそのまま読めること"
        );
        assert_eq!(
            buffer.contents().len(),
            6,
            "全体には 2 文字ぶん（6 バイト）入っていること"
        );
    }

    /// **subscriber へ渡す handle は、同じバッファを指している。**
    ///
    /// `tracing_subscriber` は `with_writer` に渡した値を保持し、
    /// イベントごとに [`MakeWriter::make_writer`] を呼んで書き先を作る。
    /// **そこが `Self::default()`（新しい空のバッファ）に化けると、
    /// 出力はどこにも溜まらず、ログを見るテストは全部「何も出なかった」で緑になる。**
    /// 偽陰性なので、実際に走らせても気付けない。
    ///
    /// **`tracing` を通さずに、その1点だけを見る**（複製の複製まで辿る）。
    ///
    /// [`MakeWriter::make_writer`]: tracing_subscriber::fmt::MakeWriter::make_writer
    #[test]
    fn the_writer_handed_to_the_subscriber_shares_the_buffer() {
        use std::io::Write as _;
        use tracing_subscriber::fmt::MakeWriter as _;

        let buffer = SharedBuffer::default();
        // `captured_with_env_filter` は `buffer.clone()` を subscriber へ渡し、
        // subscriber はイベントごとに `make_writer()` を呼ぶ。**その2段を辿る。**
        let handed = buffer.clone();
        let mut writer = handed.make_writer();

        writer
            .write_all("溜まること".as_bytes())
            .expect("書き込めること");

        assert_eq!(
            buffer.contents(),
            "溜まること",
            "subscriber へ渡した handle への書き込みが、元のバッファに溜まること"
        );
    }

    // ------------------------------------------------------------------
    // 露出のしかたを固定する（作業単位 B-3a）
    // ------------------------------------------------------------------

    /// `src/lib.rs` の中身。**`include_str!` はビルドの入力**なので、
    /// あちらを書き換えればこのテストバイナリは必ず再ビルドされる。
    const LIB_SOURCE: &str = include_str!("lib.rs");

    /// `tests/common/mod.rs` の中身。**同上**（書き換えれば再ビルドされる）。
    const TESTS_COMMON_SOURCE: &str = include_str!("../tests/common/mod.rs");

    /// `declaration` の行に**直前から付いている属性**を返す。**無ければ `None`。**
    ///
    /// **引数でソースを受け取る**——実ファイルだけを読む対照は自己成就する
    /// （`15b` の実測 / CLAUDE.md「走査本体を引数でソースを受け取る形にすれば、
    /// 実ファイルと植えたソースの両方を同じ判定に通せる」）。
    ///
    /// doc コメント（`///`）と空行は読み飛ばす。属性でない行に当たったら `None`。
    ///
    /// # **コメントに残った宣言を答えないこと**が、この関数の存在理由
    ///
    /// **ファイル全体に対する素の `contains` では務まらない。**
    /// B-3a のレビューで実測——`#[path]` を写しへ向け替え、
    /// **旧い宣言をコメントとして1行残しただけで、`contains` の検査は 1604 全緑**だった
    /// （共有は切れ、写し側の `has_field` は退化していたのに、誰も鳴らない）。
    /// **その綴りが出てくる最も自然な場所が、まさにその属性の真上の doc コメント。**
    ///
    /// ここは**現に効いている1行**だけを返すので、コメントでは満たせない。
    fn attribute_above<'a>(source: &'a str, declaration: &str) -> Option<&'a str> {
        let lines: Vec<&str> = source.lines().map(str::trim).collect();
        let index = lines.iter().position(|line| line.ends_with(declaration))?;

        lines[..index]
            .iter()
            .rev()
            .find(|line| !line.is_empty() && !line.starts_with("//"))
            .filter(|line| line.starts_with("#["))
            .copied()
    }

    /// `mod testlog;` の門。[`attribute_above`] の薄い別名。
    fn gate_of_testlog_declaration(source: &str) -> Option<&str> {
        attribute_above(source, "mod testlog;")
    }

    /// `tests/common/mod.rs` が共有の1本を読むために書く属性。**べた書き。**
    ///
    /// **[`attribute_above`] を通して組み立てないこと**——期待値を走査本体から作ると、
    /// 本体が緩んだときに期待値も一緒に緩む（CLAUDE.md「写しを1つにしても…」）。
    const SHARED_PATH_ATTRIBUTE: &str = r#"#[path = "../../src/testlog/assert.rs"]"#;

    /// **`testlog` はどのフィーチャでも本番ビルドに現れない。**
    ///
    /// `testing` の下へ出すと、本番コードがここを呼んでも
    /// `cargo test` と `cargo clippy --all-targets` は通る（CLAUDE.md 2節）。
    /// **素の `cargo build` だけが検出できる**——つまり
    /// **人が回し忘れた時点で誰も見ていない。** ここで宣言そのものを固定しておく。
    ///
    /// 植えたソースを本体に通す対照つき（実ファイルだけでは
    /// 「見つけられていないから緑」と区別できない）。
    #[test]
    fn the_testlog_module_is_never_exposed_by_a_feature() {
        // 1. 対照: 植えたソースで、本体が現に属性を読み分けられること。
        assert_eq!(
            gate_of_testlog_declaration("#[cfg(test)]\nmod testlog;\n"),
            Some("#[cfg(test)]"),
            "植えた `cfg(test)` を読めること"
        );
        assert_eq!(
            gate_of_testlog_declaration("/// doc\n#[cfg(test)]\n\nmod testlog;\n"),
            Some("#[cfg(test)]"),
            "doc と空行をまたいでも直前の属性を読めること"
        );
        assert_eq!(
            gate_of_testlog_declaration("#[cfg(feature = \"a\")]\nmod testlog;\n"),
            Some("#[cfg(feature = \"a\")]"),
            "フィーチャの門はそのまま見えること（見逃して緑にならない）"
        );
        assert_eq!(
            gate_of_testlog_declaration("mod testlog;\n"),
            None,
            "門が無ければ `None`（属性を捏造しないこと）"
        );
        assert_eq!(
            gate_of_testlog_declaration("#[cfg(test)]\nmod other;\n"),
            None,
            "別のモジュールの門を答えないこと"
        );

        // 2. 実ファイル。**`#[cfg(test)]` 以外は許さない。**
        assert_eq!(
            gate_of_testlog_declaration(LIB_SOURCE),
            Some("#[cfg(test)]"),
            "`mod testlog;` は `#[cfg(test)]` の下だけに置くこと\
             （`testing` フィーチャへ移すと、本番コードから呼べてしまう）"
        );
    }

    /// **統合テストは、写しではなく[同じファイル](super::assert)を読んでいる。**
    ///
    /// 写しに戻ると、上の押さえ
    /// （[`line_containing_returns_only_the_line_that_carries_the_marker`] /
    /// [`has_field_matches_a_whole_token_and_nothing_longer`]）が
    /// **`tests/` 側を守らなくなる**——B-3a より前がその状態で、
    /// `tests/` には `line_containing` の写しが3本あった。
    /// **食い違いが消えたことと、押さえが届いていることは別**なので、
    /// 「届いている根拠」であるこの1行を固定する。
    ///
    /// # 何を見ていて、何を見ていないか（**絞って書く**）
    ///
    /// | 検査 | 見ているもの | **見ていないもの** |
    /// |---|---|---|
    /// | 1 | `mod testlog_assert;` に**現に効いている `#[path]` の綴り** | ファイルの中身。**同じファイルを別の綴りで指せば赤**（偽陽性） |
    /// | 2 | `tests/**` に [`SHARED_HELPERS`] の**定義**が無いこと | **名前を変えた写し**（`fn line_with` 等）。**本文で見る方は別の見張り番** |
    ///
    /// **2 は [`SHARED_HELPERS`] に並べた綴りしか見ない**
    /// （B-13 で欄を突き合わせる方を、B-12b で閉じ括弧まで読む方を、
    /// **B-3 でログを溜める器の型**を足して4本になった）。
    /// B-3a のレビューで実測——`tests/server_chat.rs` に `fn line_with` という
    /// **別名の弱い写し**を戻すと**全緑**。
    /// **名前を列挙する限り、名前を変えれば外へ逃げられる。**
    ///
    /// **⚠ 名前を変えただけの写しは、B-17 で別の見張り番が見るようになった**
    /// （[`no_integration_test_copies_a_shared_body_under_another_name`] ——
    /// **本文の断片**で見るので名前に依らない）。
    /// **ここは名前の側のまま**にしてある——**本文を書き直した弱い写しは
    /// どちらでも捕まらない**ので、名前の走査を落とすと守りが減るだけになる。
    ///
    /// [`no_integration_test_copies_a_shared_body_under_another_name`]:
    ///     no_integration_test_copies_a_shared_body_under_another_name
    ///
    /// **⚠ 器を畳んでも、`tests/` の4本が「行で見ている」ことの押さえは増えない。**
    /// 器の写しを戻す変異で赤くなるのは**この走査1本だけ**で、
    /// 統合テストは全緑を通る（**写しは同じ振る舞いをするから**——B-3a で
    /// 行と欄を畳んだときと同じ形）。**その代わり、器そのものを壊す変異
    /// （`Write::write` が捨てる／`make_writer` が別のバッファを返す／
    /// `contents_since` が区間を無視する）は、4本のバイナリ全部に届く。**
    /// **走査は「写しに戻ったこと」だけを見る道具**で、振る舞いの権威ではない。
    ///
    /// [`SHARED_HELPERS`]: SHARED_HELPERS
    ///
    /// 1 の綴りの厳しさは**わざと**（実測: `../../src/testlog/../testlog/assert.rs` で赤）。
    /// 偽陽性は**うるさく落ちる**だけで、読めば1秒で分かる。
    /// **黙って写しに戻る方**が高くつく（B-3a より前がその状態だった）。
    ///
    /// [`line_containing_returns_only_the_line_that_carries_the_marker`]:
    ///     line_containing_returns_only_the_line_that_carries_the_marker
    /// [`has_field_matches_a_whole_token_and_nothing_longer`]:
    ///     has_field_matches_a_whole_token_and_nothing_longer
    #[test]
    fn the_integration_tests_share_this_file_instead_of_copying_it() {
        // 1. 対照: 植えたソースを、実ファイルと同じ本体に通す。
        assert_eq!(
            attribute_above(
                "#[path = \"copy.rs\"]\nmod testlog_assert;\n",
                "mod testlog_assert;"
            ),
            Some("#[path = \"copy.rs\"]"),
            "植えた属性を読めること"
        );
        // **旧い宣言がコメントで残っていても、現に効いている方を答える。**
        // これが素の `contains` で 1604 全緑を通した形そのもの（[`attribute_above`] の doc）。
        assert_eq!(
            attribute_above(
                "/// doc\n// #[path = \"../../src/testlog/assert.rs\"]\n#[path = \"copy.rs\"]\nmod testlog_assert;\n",
                "mod testlog_assert;"
            ),
            Some("#[path = \"copy.rs\"]"),
            "コメントに残った旧い宣言で満たされないこと"
        );
        assert_eq!(
            attribute_above("mod testlog_assert;\n", "mod testlog_assert;"),
            None,
            "属性が無ければ `None`（捏造しないこと）"
        );

        // 1. 実ファイル。**現に効いている `#[path]` が共有の1本を指していること。**
        assert_eq!(
            attribute_above(TESTS_COMMON_SOURCE, "mod testlog_assert;"),
            Some(SHARED_PATH_ATTRIBUTE),
            "`tests/common/mod.rs` が共有の1本を読んでいること\
             （写しへ向け替えると、`tests/` 側だけ押さえが外れる）"
        );

        // 2. `tests/**` のどこにも、共有している関数の**定義**が無い。
        //    対照は下の [`the_copy_scan_would_notice_a_definition`] が受け持つ。
        let copies: Vec<String> = integration_test_sources()
            .into_iter()
            .filter_map(|(path, source)| {
                defines_a_shared_helper(&source).map(|spelling| format!("{path}: {spelling}"))
            })
            .collect();
        assert!(
            copies.is_empty(),
            "`tests/` に写しが戻っています（共有の1本を使うこと）:\n{}",
            copies.join("\n")
        );
    }

    /// [`assert`] が公開しているものの綴り。**この定義が `tests/**` に在れば写し。**
    ///
    /// **⚠ 最後の1本は関数ではなく型**（作業単位 B-3 / `struct` で始まる）。
    /// 走査本体は「宣言の語 ＋ 名前」を素朴に探すだけなので、
    /// **`fn` で始まらない綴りもそのまま扱える**（[`defines_a_shared_helper`] は
    /// `SHARED_HELPERS` の要素を丸ごと `contains` するだけ）。
    /// 呼び出し側は `SharedBuffer::default()` や `common::SharedBuffer` の形になるので、
    /// **`struct` を付けた綴りは定義にしか当たらない**——他の3本と同じ理屈。
    ///
    /// **5本とも見る**（作業単位 B-12b で3本目、B-3 で器の型、**B-21b で「欄で行を選ぶ」方**）。
    /// B-3a では行を切り出す方だけを見ていて、
    /// 欄を突き合わせる方の写しが `tests/extraction_quality_harness.rs` に実在した
    /// ——**畳んでから、ここを広げた**（順序を逆にすると即赤になる）。
    ///
    /// **⚠ 「欄で行を選ぶ」方を `tests/**` に置かなかったのは、この走査があるから**（B-21b）。
    /// 本文が [`line_containing`] の双子（`lines` → `find` → `unwrap_or_else`）なので、
    /// **`tests/` に置けば `SHARED_BODIES` の走査でも `SHARED_HELPERS` の走査でも
    /// 捕まらない別名の写しが1本増える**（[`SHARED_BODIES`] の doc の表の
    /// 「弱く書き直した別名」がまさにその形）。**共有の1本に置いて、ここへ足した。**
    ///
    /// **値に空白を含む欄を読む方は `tests/server_chat_parse_log.rs` が現に使う**
    /// ——`src/` だけを締めた時点では、**同じ穴が統合側に残っていた**（実測: 欄名に
    /// 接頭辞を足す変異が、あちらの `contains` を緑のまま通り抜けた）。
    /// **名前を変えた写しはここでは捕まらない**のは他の2本と同じ
    /// （`the_integration_tests_share_this_file_instead_of_copying_it` の doc の表）。
    /// **そちらは [`SHARED_BODIES`] の走査が本文で見る**（作業単位 B-17）。
    ///
    /// **⚠ ここを増やしたら [`the_copy_scan_would_notice_a_definition`] にも
    /// 植えたソースを1本足すこと。** 足さないと、その綴りは
    /// **走査本体で壊しても誰も赤にしない**（`15b` の自己成就）——
    /// 実ファイル側は「1件も無い」が正解なので、**綴りが壊れていても緑のまま。**
    ///
    /// [`the_copy_scan_would_notice_a_definition`]: the_copy_scan_would_notice_a_definition
    const SHARED_HELPERS: [&str; 7] = [
        "fn line_containing",
        "fn line_with_field",
        "fn has_field",
        "fn bracketed_field",
        // **作業単位 B-21d。** 「そのキーが値によらず一切現れない」を言う3本目の欄の道具。
        "fn has_key",
        // **作業単位 B-32。** `Debug` の**文字列**を1つの値として読む4本目の欄の道具。
        "fn quoted_field",
        "struct SharedBuffer",
    ];

    /// **上の警告を、読まなくても守らせる門**
    /// （作業単位 B-12b / B-3 で 3 → 4、**B-21b で 4 → 5**、**B-32 で 6 → 7** に上げた）。
    ///
    /// 8本目を足すと**テストターゲットのコンパイルが止まる**ので、
    /// 植えたソースを足さずに素通りできない。
    ///
    /// **⚠ 素の `cargo build` は止まらない。** `src/lib.rs` の `mod testlog;` は
    /// `#[cfg(test)]` なので、**本番ビルドはこのファイルをコンパイルすらしない**
    /// （このモジュールの doc）。**止まるのは `cargo test` と
    /// `cargo clippy --all-targets` のとき**——B-13 のときに、
    /// 3本目を足した版で実測して確かめてある（**その3本目が B-12b で現に入った**）。
    ///
    /// **⚠ 植えたソースを [`SHARED_HELPERS`] から組み立てないこと**
    /// （`format!("{spelling}(..)")` のような形）。
    /// **綴りを壊す変異が植えたソースも一緒に壊して緑になり、`15b` の自己成就に戻る。**
    /// **べた書きのまま**にして、増えたことだけをここで止める。
    const _: () = assert!(
        SHARED_HELPERS.len() == 7,
        "SHARED_HELPERS を増やしたら the_copy_scan_would_notice_a_definition にも植えたソースを1本足すこと"
    );

    /// ソースが [`SHARED_HELPERS`] のどれかを**自分で定義**しているか。
    /// **見つけた綴りを返す**（どちらが戻ったのかが失敗の文面に出る）。
    ///
    /// **呼び出し（`common::line_containing(..)`）と定義を区別する**ので、
    /// `fn` の綴りまで見る。
    ///
    /// **⚠ コメントと本文は区別しない。** 走査対象（`tests/**`）の doc に
    /// `fn` とこれらの名前を続けて綴ると**そのファイルが写しと数えられて赤くなる**
    /// ——着手時に `tests/common/mod.rs` の doc で実際に踏んだ。
    /// **偽陽性はうるさく落ちるだけ。** 賢くして偽陰性の窓を開ける方が高くつくので、
    /// **素朴なまま残す**（言及する側が語を分けて書く / `store::memory` と同じ作法）。
    fn defines_a_shared_helper(source: &str) -> Option<&'static str> {
        SHARED_HELPERS
            .into_iter()
            .find(|spelling| source.contains(spelling))
    }

    /// **走査本体が現に定義を見分けられる**ことの対照（植えたソースを通す）。
    ///
    /// 実ファイルだけを見る検査は「1件も無いから緑」と
    /// 「そもそも探せていないから緑」を区別できない（`15b` の実測）。
    ///
    /// **[`SHARED_HELPERS`] の綴りを1本ずつ植える。**
    /// 片方だけを植えると、**もう片方は走査本体で壊しても赤にならない**
    /// ——実ファイル側は「1件も無い」が正解なので、綴りが壊れていても緑のまま。
    #[test]
    fn the_copy_scan_would_notice_a_definition() {
        assert_eq!(
            defines_a_shared_helper("fn line_containing<'a>(o: &'a str) -> &'a str { o }"),
            Some("fn line_containing"),
            "植えた「行を切り出す」定義を見つけること"
        );
        assert_eq!(
            defines_a_shared_helper("fn line_with_field<'a>(o: &'a str, f: &str) -> &'a str { o }"),
            Some("fn line_with_field"),
            "植えた「欄で行を選ぶ」定義を見つけること（B-21b で広げた側）"
        );
        assert_eq!(
            defines_a_shared_helper("fn has_field(t: &str, f: &str) -> bool { t.contains(f) }"),
            Some("fn has_field"),
            "植えた「欄を突き合わせる」定義を見つけること（B-13 で広げた側）"
        );
        assert_eq!(
            defines_a_shared_helper("fn bracketed_field<'a>(l: &'a str, k: &str) -> &'a str { l }"),
            Some("fn bracketed_field"),
            "植えた「閉じ括弧まで読む」定義を見つけること（B-12b で広げた側）"
        );
        assert_eq!(
            defines_a_shared_helper("fn has_key(t: &str, k: &str) -> bool { t.contains(k) }"),
            Some("fn has_key"),
            "植えた「キーの有無だけを見る」定義を見つけること（B-21d で広げた側）"
        );
        assert_eq!(
            defines_a_shared_helper(
                "fn quoted_field<'a>(l: &'a str, k: &str) -> Option<&'a str> {"
            ),
            Some("fn quoted_field"),
            "植えた「閉じ引用符まで読む」定義を見つけること（B-32 で広げた側）"
        );
        // **`fn` で始まらない綴り**（作業単位 B-3）。走査本体が型の宣言も扱えること。
        assert_eq!(
            defines_a_shared_helper("#[derive(Clone, Default)]\nstruct SharedBuffer(Vec<u8>);"),
            Some("struct SharedBuffer"),
            "植えた「ログを溜める器」の定義を見つけること（B-3 で広げた側）"
        );
        assert_eq!(
            defines_a_shared_helper("let line = common::line_containing(&logged, MARKER);"),
            None,
            "呼び出しを定義と数えないこと（数えると常に赤になって無意味）"
        );
        assert_eq!(
            defines_a_shared_helper("let line = common::line_with_field(&logged, FIELD);"),
            None,
            "欄で行を選ぶ側も、呼び出しを定義と数えないこと"
        );
        assert_eq!(
            defines_a_shared_helper("assert!(common::has_field(line, \"session_id=7\"));"),
            None,
            "欄の側も、呼び出しを定義と数えないこと"
        );
        assert_eq!(
            defines_a_shared_helper("let v = common::bracketed_field(line, \"axes\");"),
            None,
            "括弧の側も、呼び出しを定義と数えないこと"
        );
        assert_eq!(
            defines_a_shared_helper("let v = common::quoted_field(line, \"raw\");"),
            None,
            "引用符の側も、呼び出しを定義と数えないこと"
        );
        assert_eq!(
            defines_a_shared_helper("assert!(!common::has_key(&output, \"extracted\"));"),
            None,
            "キーの側も、呼び出しを定義と数えないこと"
        );
        assert_eq!(
            defines_a_shared_helper("let buffer: &common::SharedBuffer = shared_log();"),
            None,
            "器の側も、使う側の綴りを定義と数えないこと（`struct` が付かない）"
        );
    }

    // ------------------------------------------------------------------
    // 別名の写しを、名前ではなく**本文**で捕まえる（作業単位 B-17）
    // ------------------------------------------------------------------

    /// 共有している1本の中身。**`include_str!` はビルドの入力**なので、
    /// あちらを書き換えればこのテストバイナリは必ず再ビルドされる。
    const ASSERT_SOURCE: &str = include_str!("testlog/assert.rs");

    /// 共有している本体の**断片**。**どれも共有側の名前を含まない**ので、
    /// **名前を変えただけの写しにも当たる**（[`SHARED_HELPERS`] は当たらない）。
    ///
    /// # 何を捕まえて、何を捕まえないか（**広げていない範囲を先に書く**）
    ///
    /// | | この走査 |
    /// |---|---|
    /// | 名前だけ変えた写し（本文はそのまま） | **捕まえる**（B-12 が人手で見つけた `field_is_present` がこの形） |
    /// | 空白・改行・行頭コメントだけが違う写し | **捕まえる**（[`squeezed`] で潰してから比べる） |
    /// | **`src/**` に置かれた写し** | **捕まえる**（**B-36 で広げた**。下の節） |
    /// | **import した trait 名で書いた器** | **捕まえない**（**断片4が完全修飾**。下の節） |
    /// | **本文の綴りを変えた写し**（受け側や引数の名前を変える） | **捕まえない** |
    /// | **弱く書き直した別名**（`contains` へ退化した `line_with`） | **捕まえない**（B-3a のレビューが見つけた形） |
    /// | 行の途中のコメント（`let x = 1; // 注`）を挟んだ写し | **捕まえない**（[`squeezed`] は行頭コメントしか落とさない） |
    /// | ブロックコメント（`/* */`）で割った写し | **捕まえない** |
    ///
    /// **⚠ 「写しに戻せなくなった」とは読まないこと。** ここが塞ぐのは
    /// **`tests/**` に、コピー＆ペーストして名前だけ付け替えて置く**という、
    /// 実際に2回起きた形だけ。
    ///
    /// # 置き場所 —— **本文の走査は `src/**` も読む（B-36 で広げた）**
    ///
    /// **B-36 より前は、この走査も [`SHARED_HELPERS`] の走査も
    /// [`integration_test_sources`]（＝ `tests/**` の `.rs`）だけを読んでいた。**
    /// **B-17 のレビューが実測している**——`src/server/telemetry.rs` の末尾に
    /// `#[cfg(test)] mod` を足し、**本文逐語・別名**の写しを置いたら **1633 全緑。**
    ///
    /// **⚠ これは想定内の再発経路だった。** B-2 / B-3 が畳んだ元の問題は
    /// **「`src/**` に5箇所」「器は `src/` と `tests/` に計4つ」**である
    /// ——**写しが `src/**` に生えるのは、この道具が生まれた理由そのもの。**
    ///
    /// いまは [`no_library_module_copies_a_shared_body_under_another_name`] が
    /// `src/**` を同じ本体に通す（**名前の走査は広げていない。あちらは `tests/**` だけ**）。
    ///
    /// **⚠ 広げても、B-36 が現に見つけた2箇所は捕まらない**
    /// ——`src/server/mod.rs` の写しは**引数名が違った**（上の表の
    /// 「本文の綴りを変えた写し」）。**消したのは手であって、走査ではない。**
    ///
    /// [`integration_test_sources`]: integration_test_sources
    /// [`no_library_module_copies_a_shared_body_under_another_name`]:
    ///     no_library_module_copies_a_shared_body_under_another_name
    ///
    /// # 断片4の限界 —— **完全修飾でしか当たらない**
    ///
    /// 断片4は `impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for` と**完全修飾**なので、
    /// **`use tracing_subscriber::fmt::MakeWriter;` を書いて
    /// `impl<'a> MakeWriter<'a> for ...` と綴った器には当たらない**
    /// （B-17 のレビューで実測: **1633 全緑**）。
    ///
    /// **⚠ 断片を `MakeWriter<'a> for` へ縮めないこと。** 二重の副作用がある——
    /// 下の 40 文字の床に掛かるうえ、**将来の正当な `MakeWriter` 実装にも当たる。**
    ///
    /// **⚠ 「実際には起きない形だから軽い」とは書けない。実測して逆だった**
    /// （`git grep -n "MakeWriter" 129ed73^`。129ed73 が B-3 の畳み込み）:
    ///
    /// | 当時の写し | 綴り | 断片4が当たるか |
    /// |---|---|---|
    /// | `tests/` の4本（`server_chat` / `_parse_log` / `_stream` / `_context_refresh`） | **完全修飾** | **当たる** |
    /// | `src/testlog.rs` の1本 | **`use` して `impl<'a> MakeWriter<'a> for`** | **当たらない** |
    ///
    /// **つまり `use` する書き方は、この器では現に使われていた。**
    /// **軽いのは「`tests/` に限れば当時 0 件だった」ことまで**で、
    /// **形そのものが珍しいわけではない。**
    /// **しかもその1本は `src/**` にあった**——当時は上の置き場所とも**重なって**、
    /// **どちらの理由でも捕まらなかった。**
    /// **⚠ B-36 で置き場所の方は消えたが、この限界は残っている**
    /// ——`use` して綴った器は、`src/**` に置いても当たらない。
    ///
    /// # 断片の選び方（**除外一覧を作らないこと**が制約）
    ///
    /// `B-3` は「語を広げると除外一覧が要る」ので広げなかった。
    /// ここでは**本文をまるごと1つの塊として**見るので、**除外一覧は1つも要らない**
    /// ——着手時に実測した唯一の紛らわしい相手は
    /// `tests/server_context_refresh.rs` の「行が出るまで待って返す」道具で、
    /// **あれは本文が違う**（`unwrap_or_else` ではなく `if let` とループ）ので当たらない。
    /// **当たってしまう断片を選ぶと、除外一覧か、範囲外の畳み込みが要る。選ばないこと。**
    ///
    /// **⚠ 断片は [`ASSERT_SOURCE`] から組み立てず、べた書きにしてある**（`15b` の自己成就）。
    /// 代わりに [`every_shared_body_is_quoted_from_the_shared_file`] が
    /// **「この断片が現に共有の1本に在ること」**を見る——共有側を書き換えれば、
    /// そこが**うるさく赤くなる。**
    ///
    /// [`every_shared_body_is_quoted_from_the_shared_file`]:
    ///     every_shared_body_is_quoted_from_the_shared_file
    const SHARED_BODIES: [&str; 7] = [
        ".lines().find(|line| line.contains(marker)).unwrap_or_else(|| panic!(",
        // **作業単位 B-21b。** 1本目の双子（探すのが文言ではなく欄）。
        // **`contains` へ弱く書き直した別名は、これでも捕まらない**（上の表）。
        ".lines().find(|line| has_field(line, field)).unwrap_or_else(|| panic!(",
        ".split_whitespace().any(|token| token == field)",
        // **作業単位 B-21d。** 3本目の双子（値を見ず、キーの有無だけを見る）。
        // **上の断片と紛れない**——あちらは `== field`、こちらは `starts_with(&prefix)`。
        r#"let prefix = format!("{key}="); line.split_whitespace().any(|token| token.starts_with(&prefix))"#,
        r#"let opening = format!("{key}=["); line.match_indices(&opening).find_map(|(at, _)| {"#,
        // **作業単位 B-32。** 1つ上の断片の双子（開くのが `[` ではなく `"`）。
        // **上の断片と紛れない**——squeezed してもキーの直後の1文字が違う。
        r#"let opening = format!("{key}=\""); line.match_indices(&opening).find_map(|(at, _)| {"#,
        "impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for",
    ];

    /// **上の警告を、読まなくても守らせる門**（作業単位 B-17）。
    ///
    /// 5本目を足すと**テストターゲットのコンパイルが止まる**ので、
    /// 植えたソースを足さずに素通りできない。
    ///
    /// **⚠ 素の `cargo build` は止まらない**（[`SHARED_HELPERS`] の門と同じ理由。
    /// `src/lib.rs` の `mod testlog;` が `#[cfg(test)]` なので、
    /// 本番ビルドはこのファイルをコンパイルすらしない）。
    const _: () = assert!(
        SHARED_BODIES.len() == 7,
        "SHARED_BODIES を増やしたら the_body_scan_would_notice_a_renamed_copy にも植えたソースを1本足すこと"
    );

    /// **行頭のコメントを落として、空白をすべて潰す。**
    ///
    /// 写しは「改行の入れ方」と「行頭のコメント」だけを変えて逃げられるので、
    /// **両側を同じ形に潰してから**比べる。
    ///
    /// # 行の途中の `//` は落とさない（**わざと**）
    ///
    /// `format!("http://{addr}")` のような**文字列の中の `//`**が
    /// tests/ には実在する（着手時に実測）。行の途中で切ると、
    /// **その行の残りが走査から消える**——偽陰性の窓を自分で開けることになる。
    /// **落とすのは「行頭から `//`」だけ**にしてある。
    ///
    /// **代償は上の表のとおり**（行の途中にコメントを挟んだ写しは捕まらない）。
    /// **偽陰性どうしの交換なので、狭い方**を採った。
    fn squeezed(source: &str) -> String {
        source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .flat_map(str::chars)
            .filter(|character| !character.is_whitespace())
            .collect()
    }

    /// ソースが [`SHARED_BODIES`] のどれかを**本文ごと写している**か。
    /// **見つけた断片を返す**（どれが戻ったのかが失敗の文面に出る）。
    fn copies_a_shared_body(source: &str) -> Option<&'static str> {
        let haystack = squeezed(source);

        SHARED_BODIES
            .into_iter()
            .find(|body| haystack.contains(&squeezed(body)))
    }

    /// **断片は、現に共有の1本に在る文字列である。**
    ///
    /// # これが無いと、走査は黙って何も探さなくなる
    ///
    /// 実ファイル側の主張は「1件も見つからないこと」なので、
    /// **断片が古びても・壊されても緑のまま**通る（`15b` の自己成就）。
    /// ここが**唯一、断片そのものを見ている**。
    ///
    /// 共有側の本文を書き換えたら、ここが赤くなる。**それが正しい壊れ方**
    /// ——断片を書き直せ、という指示になる。
    ///
    /// **⚠ 期待値を [`squeezed`] の外に置けない**（比べる相手が実ファイルなので）。
    /// 代わりに [`the_body_scan_would_notice_a_renamed_copy`] が
    /// **植えたソース**で [`squeezed`] と [`copies_a_shared_body`] を裸で押さえる。
    ///
    /// [`the_body_scan_would_notice_a_renamed_copy`]: the_body_scan_would_notice_a_renamed_copy
    #[test]
    fn every_shared_body_is_quoted_from_the_shared_file() {
        let shared = squeezed(ASSERT_SOURCE);

        for body in SHARED_BODIES {
            let needle = squeezed(body);
            // **空に潰れた断片（`contains("")` は常に真）と、
            // 明らかに短すぎる断片を止めるだけの床。**
            //
            // **⚠ 「写しでないものに当たらない」は、この床では届かない。**
            // B-17 のレビューで実測——**縮めた断片は squeezed で 42 文字あり、
            // それでも `wait_for_line` に当たった。** 床を越えても誤爆はする。
            // **誤爆を防いでいるのは「本文をまるごと1つの塊として選ぶ」という
            // 断片の選び方のほう**（[`SHARED_BODIES`] の doc）。
            assert!(
                needle.len() >= 40,
                "断片が空か、明らかに短すぎます\
                 （これは床であって、誤爆しないことの保証ではありません）: {body:?}"
            );
            assert!(
                shared.contains(&needle),
                "この断片は `src/testlog/assert.rs` に在りません\
                 （共有側を書き換えたなら、断片も書き直すこと）: {body:?}"
            );
        }
    }

    /// **走査本体が現に「名前を変えただけの写し」を見分けられる**ことの対照。
    ///
    /// **[`SHARED_BODIES`] の断片を1本ずつ植える**（[`the_copy_scan_would_notice_a_definition`]
    /// と同じ理由——片方だけを植えると、もう片方は本体で壊しても赤にならない）。
    ///
    /// **植えたソースは全部べた書き**（[`SHARED_BODIES`] から組み立てない / `15b`）。
    /// **5本とも、関数名・型名は共有側と別**にしてある——
    /// [`SHARED_HELPERS`] の名前の走査では**1本も捕まらない**形。
    ///
    /// [`the_copy_scan_would_notice_a_definition`]: the_copy_scan_would_notice_a_definition
    #[test]
    fn the_body_scan_would_notice_a_renamed_copy() {
        // 1. 行を切り出す方。**B-3a のレビューが実際に戻した名前**（`line_with`）。
        assert_eq!(
            copies_a_shared_body(
                "fn line_with<'a>(output: &'a str, marker: &str) -> &'a str {\n\
                 output\n\
                 .lines()\n\
                 .find(|line| line.contains(marker))\n\
                 .unwrap_or_else(|| panic!(\"no line\"))\n}"
            ),
            Some(".lines().find(|line| line.contains(marker)).unwrap_or_else(|| panic!("),
            "改行を挟んだ別名の写しを見つけること"
        );
        // 2. **欄で行を選ぶ方**（作業単位 B-21b）。1本目の双子なので、
        //    **名前を変えただけで両方の走査をすり抜ける形**がいちばん出やすい。
        assert_eq!(
            copies_a_shared_body(
                "fn line_where<'a>(output: &'a str, field: &str) -> &'a str {\n\
                 output\n\
                 .lines()\n\
                 .find(|line| has_field(line, field))\n\
                 .unwrap_or_else(|| panic!(\"no line\"))\n}"
            ),
            Some(".lines().find(|line| has_field(line, field)).unwrap_or_else(|| panic!("),
            "欄で行を選ぶ側の、改行を挟んだ別名の写しを見つけること"
        );
        // 3. 欄を突き合わせる方。**B-12 が人手で見つけた実物の名前**（`field_is_present`）。
        assert_eq!(
            copies_a_shared_body(
                "fn field_is_present(line: &str, field: &str) -> bool {\n    \
                 line.split_whitespace().any(|token| token == field)\n}"
            ),
            Some(".split_whitespace().any(|token| token == field)"),
            "別名の写しを見つけること（走査ではなく人が見つけた形）"
        );
        // 3b. **キーの有無だけを見る方**（作業単位 B-21d）。3本目の双子なので、
        //     **`== field` の方と取り違えた別名**がいちばん出やすい。
        assert_eq!(
            copies_a_shared_body(
                "fn key_is_present(line: &str, key: &str) -> bool {\n    \
                 let prefix = format!(\"{key}=\");\n    \
                 line.split_whitespace().any(|token| token.starts_with(&prefix))\n}"
            ),
            Some(
                r#"let prefix = format!("{key}="); line.split_whitespace().any(|token| token.starts_with(&prefix))"#
            ),
            "キーの有無だけを見る側の、別名の写しを見つけること"
        );
        // 4. 閉じ括弧まで読む方。
        assert_eq!(
            copies_a_shared_body(
                "fn value_in_brackets<'a>(line: &'a str, key: &str) -> Option<&'a str> {\n    \
                 let opening = format!(\"{key}=[\");\n\n    \
                 line.match_indices(&opening).find_map(|(at, _)| { None })\n}"
            ),
            Some(
                r#"let opening = format!("{key}=["); line.match_indices(&opening).find_map(|(at, _)| {"#
            ),
            "空行をまたいだ別名の写しを見つけること"
        );
        // 4b. **閉じ引用符まで読む方**（作業単位 B-32）。4 の双子なので、
        //     **`[` の方と取り違えた別名**がいちばん出やすい。
        //     **植えたソースは生の文字列**（`\` を数え違えないため。中身はべた書き）。
        assert_eq!(
            copies_a_shared_body(
                r#"fn value_in_quotes<'a>(line: &'a str, key: &str) -> Option<&'a str> {
                       let opening = format!("{key}=\"");

                       line.match_indices(&opening).find_map(|(at, _)| { None })
                   }"#
            ),
            Some(
                r#"let opening = format!("{key}=\""); line.match_indices(&opening).find_map(|(at, _)| {"#
            ),
            "引用符の側の、別名の写しを見つけること"
        );
        // 5. ログを溜める器。**型名が別**（`SharedBuffer` の走査では捕まらない）。
        assert_eq!(
            copies_a_shared_body(
                "struct LogSink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);\n\
                 impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogSink {}"
            ),
            Some("impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for"),
            "別の型名で器を作り直した写しを見つけること"
        );

        // 否定: 呼び出しは写しではない。
        assert_eq!(
            copies_a_shared_body("let line = common::line_containing(&logged, MARKER);"),
            None,
            "呼び出しを写しと数えないこと（数えると常に赤になって無意味）"
        );
        assert_eq!(
            copies_a_shared_body("assert!(common::has_field(line, \"session_id=7\"));"),
            None,
            "欄の側も、呼び出しを写しと数えないこと"
        );
        // 否定: **行頭のコメントに綴った本文**は写しではない（コンパイルされない）。
        // [`squeezed`] が行頭コメントを落とすことの押さえ。
        assert_eq!(
            copies_a_shared_body(
                "/// 本文はこう書いてある:\n\
                 /// line.split_whitespace().any(|token| token == field)\n\
                 fn other() {}"
            ),
            None,
            "行頭コメントに綴った本文を写しと数えないこと"
        );
        // 否定: **本文が違う別の道具**。`tests/server_context_refresh.rs` の
        // 「行が出るまで待って返す」道具がこの形で、**写しではない**（ループと `if let`）。
        // **この走査の限界そのもの**——本文を書き直した別名は捕まらない
        // （[`SHARED_BODIES`] の表）。
        assert_eq!(
            copies_a_shared_body(
                "async fn wait_for_line(marker: &str) -> String {\n    loop {\n        \
                 let logged = shared_log().contents();\n        \
                 if let Some(line) = logged.lines().find(|line| line.contains(marker)) {\n            \
                 return line.to_owned();\n        }\n    }\n}"
            ),
            None,
            "本文が違う別の道具を写しと数えないこと"
        );
    }

    /// **`tests/**` のどこにも、共有している本体の写しが無い**（別名でも）。
    ///
    /// [`the_integration_tests_share_this_file_instead_of_copying_it`] の 2 は
    /// **名前の綴りしか見ない**ので、`fn field_is_present` のように
    /// **名前を変えただけの写しが素通りする**（B-3a と B-12 で2回起きた）。
    /// ここは**本文で**見るので、名前を変えても当たる。
    ///
    /// **対照は [`the_body_scan_would_notice_a_renamed_copy`]**
    /// （実ファイルだけを見る検査は「1件も無いから緑」と
    /// 「そもそも探せていないから緑」を区別できない / `15b`）。
    /// **断片そのものの権威は [`every_shared_body_is_quoted_from_the_shared_file`]。**
    ///
    /// [`the_integration_tests_share_this_file_instead_of_copying_it`]:
    ///     the_integration_tests_share_this_file_instead_of_copying_it
    /// [`the_body_scan_would_notice_a_renamed_copy`]: the_body_scan_would_notice_a_renamed_copy
    /// [`every_shared_body_is_quoted_from_the_shared_file`]:
    ///     every_shared_body_is_quoted_from_the_shared_file
    #[test]
    fn no_integration_test_copies_a_shared_body_under_another_name() {
        let copies: Vec<String> = integration_test_sources()
            .into_iter()
            .filter_map(|(path, source)| {
                copies_a_shared_body(&source).map(|body| format!("{path}: {body}"))
            })
            .collect();

        assert!(
            copies.is_empty(),
            "`tests/` に本体の写しが戻っています（名前を変えても写しです）:\n{}",
            copies.join("\n")
        );
    }

    /// **`src/**` のどこにも、共有している本体の写しが無い**（別名でも / 作業単位 B-36）。
    ///
    /// # なぜ足したのか（**実測されていた穴**）
    ///
    /// [`no_integration_test_copies_a_shared_body_under_another_name`] が読むのは
    /// `tests/**` **だけ**で、**`src/**` は1本も走査していなかった**
    /// （[`SHARED_BODIES`] の doc「置き場所の限界」が自分でそう書いている）。
    /// **B-17 のレビューが実測している** ——`src/server/telemetry.rs` に
    /// **本文逐語・別名**の写しを置いたら **1633 全緑**。
    ///
    /// **そして写しが `src/**` に生えるのは、この道具が生まれた理由そのもの**
    /// （B-2 が畳んだのは `src/**` の5箇所、B-3 が畳んだ器は `src/` と `tests/` に計4つ）。
    ///
    /// # ⚠ **B-36 が見つけた2箇所は、この走査でも捕まらない**
    ///
    /// `src/server/mod.rs` の `fields_of` と、その下の警告の行を選ぶ3行は、
    /// **どちらも引数名が `marker` ではなかった**（`message` / 定数）。
    /// [`SHARED_BODIES`] の断片は**本文の綴りをそのまま持つ**ので当たらない
    /// （あちらの表の「本文の綴りを変えた写し」）。
    /// **あの2箇所を消したのは手であって、この走査ではない。**
    ///
    /// **ここが塞ぐのは「コピー＆ペーストして名前だけ付け替える」形だけ。**
    /// それが実際に2回起きた形である（`field_is_present` / `line_with`）。
    ///
    /// # 除いてある2ファイル
    ///
    /// | ファイル | 理由 |
    /// |---|---|
    /// | `src/testlog/assert.rs` | **本体そのもの**（写しではない） |
    /// | `src/testlog.rs` | [`SHARED_BODIES`] の断片と、植えたソースを**文字列として持つ** |
    ///
    /// **除外はこの2本だけ。** 増やすと、増やした先が黙って写し置き場になる。
    ///
    /// **対照は [`the_body_scan_would_notice_a_renamed_copy`]**（走査本体は同じ1本）。
    ///
    /// [`no_integration_test_copies_a_shared_body_under_another_name`]:
    ///     no_integration_test_copies_a_shared_body_under_another_name
    /// [`the_body_scan_would_notice_a_renamed_copy`]: the_body_scan_would_notice_a_renamed_copy
    #[test]
    fn no_library_module_copies_a_shared_body_under_another_name() {
        let copies: Vec<String> = library_sources()
            .into_iter()
            .filter_map(|(path, source)| {
                copies_a_shared_body(&source).map(|body| format!("{path}: {body}"))
            })
            .collect();

        assert!(
            copies.is_empty(),
            "`src/` に本体の写しが戻っています（同じクレートなので `crate::testlog` から直に呼べます）:\n{}",
            copies.join("\n")
        );
    }

    /// `src/**` の Rust ソースを「パス, 中身」で返す。**共有の1本とこのファイルは除く。**
    ///
    /// 除く理由は [`no_library_module_copies_a_shared_body_under_another_name`] の doc。
    ///
    /// [`no_library_module_copies_a_shared_body_under_another_name`]:
    ///     no_library_module_copies_a_shared_body_under_another_name
    fn library_sources() -> Vec<(String, String)> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let exempt = [
            root.join("testlog.rs"),
            root.join("testlog").join("assert.rs"),
        ];
        // **除外の綴りが古びたら落とす。** 綴りが実在しないと、除外は黙って無効になり、
        // 「除外しているつもり」のまま**本体そのもの**が写しとして数えられる。
        for path in &exempt {
            assert!(
                path.is_file(),
                "除外するファイルが実在しないこと（綴りが古い）: {}",
                path.display()
            );
        }

        let mut sources = Vec::new();
        let mut stack = vec![root];

        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("走査対象を読めること") {
                let path = entry.expect("ディレクトリ項目を読めること").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !path.extension().is_some_and(|ext| ext == "rs") {
                    continue;
                }
                if exempt.contains(&path) {
                    continue;
                }
                let source = std::fs::read_to_string(&path).expect("ソースを読めること");
                sources.push((path.display().to_string(), source));
            }
        }

        // **実数（55）に近い床**（2026-09-02 に数えた値。除外した2本を引いた数）。
        // 緩い床は「守っている」と数えられるだけで何も守らない。
        assert!(
            sources.len() >= 54,
            "走査対象が少なすぎるなら、この走査は何も守っていない: {} 件",
            sources.len()
        );
        sources
    }

    /// `tests/**` の Rust ソースを「パス, 中身」で返す。**走査対象が空なら落とす。**
    fn integration_test_sources() -> Vec<(String, String)> {
        let mut sources = Vec::new();
        let mut stack = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")];

        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("走査対象を読めること") {
                let path = entry.expect("ディレクトリ項目を読めること").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !path.extension().is_some_and(|ext| ext == "rs") {
                    continue;
                }
                let source = std::fs::read_to_string(&path).expect("ソースを読めること");
                sources.push((path.display().to_string(), source));
            }
        }

        // **実数（29）に近い床**（B-3 で `tests/log_filter_wiring.rs` が増えて 28 → 29）。
        //
        // **⚠ 2026-08-28 に数え直した値。** それまでの「実数（27）」は**読まずに書いた数**で、
        // 実際は 28 だった（B-3 のレビュー指摘 / CLAUDE.md「書く数は自分が読んだ数だけにする」）。
        // 緩い床は「守っている」と数えられるだけで何も守らない
        // （`D-0b` のコーパス下限と同じ形 / B-3a のレビュー指摘3）。
        assert!(
            sources.len() >= 28,
            "走査対象が少なすぎるなら、この走査は何も守っていない: {} 件",
            sources.len()
        );
        sources
    }
}
