//! **生成している間にキャラクターが完全に消されたら、往復の保存の失敗は `error` にしない**
//! （ADR-0049 §決定3 / 作業単位 A-10 のレビュー / Issue origin#55）。
//!
//! 上流（モック）が答えている間に、しまって → 消す。会話もカスケードで消えるので、
//! 往復の保存（`server::turn`）は `UnknownSession` で落ちる。**会話が本当に無いことを引き直して
//! 確かめてから** `info`（「キャラクターが消えていたので、会話の往復を保存しませんでした」）に落とす。
//! **応答は今までどおり 200 で返る。**
//!
//! 本物の故障（DB が閉じている等）が今までどおり `error` になることは
//! `tests/server_chat_parse_log.rs::a_failure_to_persist_is_recorded_as_an_error` が見ている。
//!
//! **グローバルの subscriber を差すので、このファイルにはこの1本だけを置く**（他の行が混ざらない）。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use personacore::domain::{CharacterUpdate, DisplayName, Handle, Persona};
use personacore::llm::{
    ChatRequest, ChatStream, Completion, FinishReason, LlmClient, LlmError, LlmFuture,
};
use personacore::server::{App, AppConfig, BindAddr, DEFAULT_LOG_FILTER};
use personacore::store::Store;
use reqwest::StatusCode;
use serde_json::json;

mod common;

use common::{fixture_scale, install_global_log};

/// **答えている間に、`aoi` をしまって完全に消す**上流。
struct DeletingLlm {
    store: Mutex<Option<Store>>,
    calls: Mutex<usize>,
}

impl LlmClient for DeletingLlm {
    fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        *self.calls.lock().expect("ロックを取れること") += 1;
        let store = self.store.lock().expect("ロックを取れること").clone();
        Box::pin(async move {
            if let Some(store) = store {
                let handle = Handle::parse("aoi").expect("正当な識別子");
                store
                    .update_character(
                        &handle,
                        &CharacterUpdate {
                            archived: Some(true),
                            ..CharacterUpdate::default()
                        },
                    )
                    .await
                    .expect("しまえること");
                store
                    .delete_character(&handle)
                    .await
                    .expect("しまったキャラクターは消せること");
            }
            Ok(Completion {
                content: "「はい」".to_owned(),
                finish_reason: Some(FinishReason::Stop),
                usage: None,
            })
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        Box::pin(async move { Err(LlmError::MissingContent) })
    }
}

#[tokio::test]
async fn a_turn_whose_character_was_deleted_meanwhile_is_dropped_without_an_error() {
    let buffer = install_global_log(DEFAULT_LOG_FILTER);

    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let config = AppConfig::from_lookup(|_| None)
        .expect("既定値だけで設定を組み立てられること")
        .with_bind(BindAddr::loopback(0))
        .with_database_path(dir.path().join("personacore.sqlite"));
    let llm = Arc::new(DeletingLlm {
        store: Mutex::new(None),
        calls: Mutex::new(0),
    });
    let app = App::bind_with_client(&config, llm.clone())
        .await
        .expect("待ち受けを開始できること");
    let addr: SocketAddr = app.local_addr();
    let store = app.store().clone();
    store
        .create_character(
            &Handle::parse("aoi").expect("正当な識別子"),
            &DisplayName::parse("葵").expect("正当な表示名"),
            &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナ"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");
    *llm.store.lock().expect("ロックを取れること") = Some(store.clone());

    let (shutdown, rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        app.serve(async move {
            let _ = rx.await;
        })
        .await
    });

    let mark = buffer.mark();
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/v1/chat/completions"))
        .json(&json!({
            "model": "aoi@new",
            "messages": [{"role": "user", "content": "こんにちは"}],
        }))
        .send()
        .await
        .expect("サーバへ接続できること");
    let status = response.status();
    let body = response.text().await.expect("本文を読めること");
    assert_eq!(status, StatusCode::OK, "応答は返ること: {body}");
    assert_eq!(
        *llm.calls.lock().expect("ロック"),
        1,
        "前提: 上流を1回呼んだこと"
    );

    let output = buffer.contents_since(mark);
    assert!(
        !output.contains("会話の往復を保存できませんでした"),
        "消えたキャラクターの往復の保存の失敗を、故障として出さないこと: {output}"
    );
    assert!(
        !output.contains(" ERROR "),
        "error の行を1本も出さないこと: {output}"
    );
    assert!(
        output.lines().any(|line| line.contains(" INFO ")
            && line.contains(
                "キャラクターが消えていたので、会話の往復を保存しませんでした（応答は返します）"
            )),
        "保存しなかったことを info で残すこと: {output}"
    );
    let messages: i64 = sqlx::query_scalar("SELECT count(*) FROM message")
        .fetch_one(store.pool())
        .await
        .expect("数えられること");
    assert_eq!(messages, 0, "何も保存されていないこと");

    shutdown.send(()).expect("停止シグナルを送れること");
    server
        .await
        .expect("サーバタスクが panic しないこと")
        .expect("正常に終了すること");
}
