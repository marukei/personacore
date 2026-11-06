//! **DB の置き場所は起動時に作る**（[ADR-0041](../docs/adr/0041-one-executable-per-os.md) §決定3 /
//! 作業単位 F-4a / レビュー指摘4）。**LM Studio には繋がない。**
//!
//! # なぜ1本要るのか
//!
//! **既定が OS の作法の場所になったので、初回の起動ではそのディレクトリがまだ無い。**
//! **作る1行が落ちれば、初回起動が全部落ちる。**
//!
//! **⚠ その1行に見張り番が1つも無かった**（レビュアーが実測 —— `create_dir_all` を
//! 殺す変異が **2096 全緑**）。**既存のサーバテストはどれも
//! `tempfile::tempdir()` が作ったディレクトリの直下を指していた**ので、
//! **親は必ず先に在り、あの行は一度も仕事をしていなかった。**
//!
//! **本番の入口（`App::bind`）を通す**——`bind_with_clients`（モック注入）だけを通すと、
//! 「テスト専用の入口を作ると本番の入口が観測対象から外れる」形になる（CLAUDE.md）。

use personacore::server::{App, AppConfig, BindAddr, ENV_LLM_ENDPOINT};

/// **上流に繋ぎに行かせない宛先**（閉じている loopback のポート）。
///
/// **既定（LM Studio）を指させない**——テストが実機の推論サーバに触る
/// （docs/05-development.md）。**繋がらなくても起動は止まらない。**
const CLOSED_UPSTREAM: &str = "http://127.0.0.1:9";

/// 指定した DB のパスで、**本番の入口から**待ち受けて、すぐ止める。
async fn bind_and_stop(database: &std::path::Path) {
    let config =
        AppConfig::from_lookup(|key| (key == ENV_LLM_ENDPOINT).then(|| CLOSED_UPSTREAM.to_owned()))
            .expect("設定を組み立てられること")
            .with_bind(BindAddr::loopback(0))
            .with_database_path(database);

    let app = App::bind(&config).await.expect("待ち受けを開始できること");
    app.serve(std::future::ready(()))
        .await
        .expect("停止まで通ること");
}

/// **まだ無いディレクトリを指しても、起動が作る**（ADR-0041 §決定3）。
///
/// # なぜ2段の入れ子か
///
/// **既定は `<データディレクトリ>/personacore/personacore.sqlite`** で、
/// **初回に足りないのは1段だけ**だが、**`create_dir` ではなく `create_dir_all`**
/// であることまで見たい——**`PERSONACORE_DB_PATH` に深い場所を書く利用者がいる。**
///
/// # なぜ「作られたこと」と「DB が在ること」の両方を見るか
///
/// **ディレクトリだけ作って DB を開けていない**形（= `Store::connect` の前で
/// 早期 return する変異）を、**ディレクトリの有無だけでは見分けられない。**
#[tokio::test]
async fn the_database_directory_is_created_at_startup() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let nested = dir.path().join("not-yet").join("deeper");
    let database = nested.join("personacore.sqlite");

    // **前提**——まだ1段も無いこと（在ると、この検査は何も言わない）。
    assert!(
        !dir.path().join("not-yet").exists(),
        "前提: 置き場所がまだ無いこと"
    );

    bind_and_stop(&database).await;

    assert!(
        nested.is_dir(),
        "起動が置き場所を作っていない: {}",
        nested.display()
    );
    assert!(
        database.is_file(),
        "置き場所は出来たのに DB が無い: {}",
        database.display()
    );
}

/// **対比: 既に在るディレクトリでも、起動は変わらず通る。**
///
/// これが無いと、「常に失敗する」実装と区別が付かない
/// （上の1本は「作られた」しか見ていない）。
#[tokio::test]
async fn an_existing_database_directory_is_left_alone() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let existing = dir.path().join("already-there");
    std::fs::create_dir(&existing).expect("先に作れること");
    // **中身を置いておく**——作り直して消す実装なら、これが消える。
    let bystander = existing.join("do-not-touch.txt");
    std::fs::write(&bystander, "BYSTANDER").expect("書けること");

    bind_and_stop(&existing.join("personacore.sqlite")).await;

    assert_eq!(
        std::fs::read_to_string(&bystander).expect("隣のファイルを読めること"),
        "BYSTANDER",
        "既に在る置き場所の中身を触らないこと"
    );
    assert!(existing.join("personacore.sqlite").is_file());
}
