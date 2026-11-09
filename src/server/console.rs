//! **起動に失敗したとき、ダブルクリックで開いた窓を、理由と一緒に消さない**
//! （作業単位 B-76 / marukei/personacore#13）。
//!
//! エクスプローラから `personacore.exe` をダブルクリックすると、Windows はこの exe のためだけに
//! コンソールの窓を作る。**exe が終わると窓も閉じる**ので、起動に失敗したときの文は、
//! 読む前に窓と一緒に消える（利用者には「一瞬見えて閉じた」としか見えない）。
//!
//! そこで、**この exe が窓を独りで使っていて、かつ標準入力が端末のときだけ**、
//! エラー文を出した後に Enter を1回待つ。
//!
//! | 起こし方 | 窓に繋がっているプロセスの数 | 標準入力 | 待つか |
//! |---|---|---|---|
//! | エクスプローラからダブルクリック | 1（この exe だけ） | 端末 | **待つ** |
//! | PowerShell / cmd から | 2 以上（シェルも繋がっている） | 端末 | 待たない（窓は閉じない） |
//! | コンソールが無い（数えられない） | 0 | — | 待たない |
//! | テスト・CI・パイプ | — | 端末でない | 待たない（待つと返らない） |
//! | Windows 以外 | — | — | 待たない（窓を作る起こし方が無い） |
//!
//! **判定は [`should_wait_before_closing`]（純粋関数）にある。** OS に問い合わせる部分は
//! [`console_process_count`] の1か所だけにし、薄く保つ。
//!
//! # 何を守っていないか（**先に書く**）
//!
//! * **本物のダブルクリックは、テストから作れない。** 判定の全部の腕は単体テストが見る。
//!   実行ファイルを起こすテスト（`tests/upstream_cli.rs` の
//!   `an_occupied_port_names_the_port_and_the_setting_and_never_waits`）が見るのは
//!   **「無条件に待つ」形ではないことだけ**である —— テストが起こした exe はテストと同じ窓に繋がる
//!   （数が 2 以上。窓の無い CI では 0）ので、**標準入力の条件を見る前に「待たない」が決まる。**
//!   **[`wait_before_closing_if_alone`] が `is_terminal()` を判定へ正しく渡しているか（配線）は、
//!   どのテストも観測しない**（`is_terminal()` を `true` に置き換えても緑のまま）。
//!   **数が 1 になる起こし方で本当に待つことは、司令塔の実機で確かめた**（2026-11-09。新しい窓に独りで
//!   起こすと、使用中のポートで止まった後も 8 秒生きていた。同じ窓で起こすと終了コード 1 ですぐ終わった）。
//! * **失敗した時点で、起こした子（llama 等）がまだ窓に繋がっていれば、数は 2 以上になって待たない。**
//!   待ち受けを開けない失敗は子を起こす前に起きる（`App::bind_with` の doc）ので、この節の本命には当たらない。

use std::io::{BufRead as _, IsTerminal as _, Write as _};

/// **待つときに出す一文。** エラー文の後に、標準エラーへ出す。
pub const PRESS_ENTER_TO_CLOSE: &str = "Enter キーを押すと、この窓を閉じます";

/// **どの OS で動いているか**（判定に OS を引数として渡すための型）。
///
/// **`cfg` で関数ごと消さない**——消すと「Windows 以外でも待つ」形の誤りを、
/// Windows の上の単体テストで観測できなくなる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Platform {
    Windows,
    Other,
}

/// この実行ファイルが組まれた OS。
const THIS_PLATFORM: Platform = if cfg!(windows) {
    Platform::Windows
} else {
    Platform::Other
};

/// **閉じる前に Enter を待つか。純粋関数。**
///
/// * `console_processes` —— この exe の窓（コンソール）に繋がっているプロセスの数。
///   **1 は「この exe だけ」**＝エクスプローラから起こした。**0 はコンソールが無いか、数えられない。**
/// * `stdin_is_terminal` —— 標準入力が端末か。**端末でなければ、読んでも誰も Enter を押さない。**
fn should_wait_before_closing(
    platform: Platform,
    console_processes: u32,
    stdin_is_terminal: bool,
) -> bool {
    platform == Platform::Windows && console_processes == 1 && stdin_is_terminal
}

/// **窓をこの exe が独りで使っているなら、閉じる前に Enter を待つ。**
///
/// **エラー文を出した「後」に呼ぶこと**（待つ前に読める文が窓に出ている必要がある）。
/// **正常に終わったとき（Ctrl+C）には呼ばない**——`main` が失敗の枝でだけ呼ぶ。
///
/// **書けない・読めないは、待たずに終わるだけ。** panic しない（`println!` を使わない）。
pub fn wait_before_closing_if_alone() {
    let wait = should_wait_before_closing(
        THIS_PLATFORM,
        console_process_count(),
        std::io::stdin().is_terminal(),
    );
    if !wait {
        return;
    }
    {
        let mut stderr = std::io::stderr().lock();
        if writeln!(stderr, "{PRESS_ENTER_TO_CLOSE}").is_err() || stderr.flush().is_err() {
            return;
        }
    }
    let mut line = String::new();
    // **読めなくても（EOF・エラー）待たずに終わる。** 中身は使わない。
    let _ = std::io::stdin().lock().read_line(&mut line);
}

/// **この exe の窓（コンソール）に繋がっているプロセスの数**（Windows の `GetConsoleProcessList`）。
///
/// **0 はコンソールが無いか、問い合わせに失敗した**（どちらも待たない側に倒れる）。
#[cfg(windows)]
fn console_process_count() -> u32 {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        /// <https://learn.microsoft.com/windows/console/getconsoleprocesslist>
        ///
        /// 戻り値は繋がっているプロセスの数（**配列より多ければ、要る要素数**）。失敗は 0。
        fn GetConsoleProcessList(process_list: *mut u32, process_count: u32) -> u32;
    }

    /// 受け取る配列の長さ。**数だけが要る**ので、溢れても構わない（戻り値は数のまま）。
    const CAPACITY: usize = 4;
    let mut ids = [0_u32; CAPACITY];
    // SAFETY: `ids` は `CAPACITY` 個の `u32` を持つ、この関数の中で生きている可変の配列で、
    // 第2引数にその要素数をそのまま渡している。API は渡した要素数までしか書かない。
    // 引数のほかに触る状態は無く、戻り値は数値だけ（ポインタを返さない）。
    unsafe { GetConsoleProcessList(ids.as_mut_ptr(), CAPACITY as u32) }
}

/// **Windows 以外では数えない**（[`should_wait_before_closing`] は OS で先に断る）。
#[cfg(not(windows))]
fn console_process_count() -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **待つのは「Windows・数が 1・端末」の1点だけ。** 全部の腕を並べる。
    #[test]
    fn it_waits_only_when_this_executable_owns_the_console_alone_and_stdin_is_a_terminal() {
        let cases = [
            // (OS, 数, 端末か, 待つか, 何の場合か)
            (Platform::Windows, 1, true, true, "ダブルクリック"),
            (
                Platform::Windows,
                1,
                false,
                false,
                "独りだが標準入力がパイプ（テスト・CI）",
            ),
            (
                Platform::Windows,
                0,
                true,
                false,
                "コンソールが無い・数えられない",
            ),
            (Platform::Windows, 0, false, false, "コンソールも端末も無い"),
            (
                Platform::Windows,
                2,
                true,
                false,
                "PowerShell / cmd から起こした",
            ),
            (
                Platform::Windows,
                2,
                false,
                false,
                "シェルからパイプで起こした",
            ),
            (
                Platform::Windows,
                3,
                true,
                false,
                "シェルの中のシェルから起こした",
            ),
            (Platform::Windows, u32::MAX, true, false, "数の上端"),
            (
                Platform::Other,
                1,
                true,
                false,
                "Windows 以外（数が 1 でも）",
            ),
            (Platform::Other, 0, true, false, "Windows 以外"),
            (Platform::Other, 2, false, false, "Windows 以外"),
        ];
        for (platform, count, terminal, expected, what) in cases {
            assert_eq!(
                should_wait_before_closing(platform, count, terminal),
                expected,
                "{what}（{platform:?} / 数 {count} / 端末 {terminal}）"
            );
        }
    }

    /// **組んだ OS の綴りが、本当にその OS を指していること**（`cfg!` の向きの取り違えを止める）。
    #[test]
    fn this_platform_is_the_one_it_was_built_for() {
        assert_eq!(THIS_PLATFORM == Platform::Windows, cfg!(windows));
    }

    /// **テストの中では待たない**（`cargo test` は標準入力が端末でも、窓をシェルと共有している）。
    ///
    /// 戻ってくることが主張である（待つと、ここで返らなくなる）。
    #[test]
    fn the_test_runner_never_waits() {
        assert_ne!(
            console_process_count(),
            1,
            "テストを走らせている窓を、テストのプロセスが独りで使っていることは無い"
        );
        wait_before_closing_if_alone();
    }
}
