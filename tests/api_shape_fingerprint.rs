//! **API の形の指紋**（[ADR-0047](../docs/adr/0047-the-api-is-the-product.md) §② P-2b「版の上げ方」/
//! 作業単位 O-7・Issue origin#91）。
//!
//! # なぜ在るか
//!
//! ADR-0047 は「作業単位が API の形を変えたら、その単位の中で版を上げる」と決めていた。
//! **それを見る見張り番が無かった**ので、v0.2.0（API `1.3`）の後に形を変えた単位が
//! どれも版を上げず、**v0.3.0 / v0.3.1 は形が変わっていたのに `1.3` を名乗って出た。**
//! 版の綴りの走査（`the_adr_names_the_same_api_version` 等）は「定数と文書が同じ版を言うか」を
//! 見るだけで、「形が変わったのに版が同じか」は見ていなかった。
//!
//! # 何を固定するか
//!
//! | 主張 | 破れると |
//! |---|---|
//! | いまの形の指紋 == 表（`tests/fixtures/api_shape_fingerprints.tsv`）の [`API_VERSION`] の行 | 形を変えた単位が、版を上げずに通る |
//! | [`API_VERSION`] の行が表に在り、表の最後の行である | 版だけを上げて、その形を記録しない |
//! | **失敗の形の一覧**（[`error_catalog`] の状態コード・`type`・`code`）も指紋に入る | 新しい `code`・状態コードの変更が版を上げずに通る（仕様書の失敗は `default` → `ErrorBody` だけで、`code` は `enum` の無い文字列） |
//! | 実験中の口（`x-personacore-stability: experimental`）と、**その口だけが指す型**は指紋に入らない | 互換の約束の外の口を直すたびに版が上がる |
//! | `info` の版・ライセンス・説明、口の `summary`、`example` / `examples` は指紋に入らない | 版を上げただけで、または説明の文を直しただけで指紋が変わる |
//!
//! **表の既存の行は書き換えない**（表の先頭のコメントと ADR-0047 の版の上げ方の段落）。
//! 形が変わって赤になったら、版を上げて**新しい行を足す。**
//!
//! # 正規化（[`fingerprint_input`] → [`canonical`] → SHA-256）
//!
//! 仕様書（[`shape`]）:
//! * `info` から `version`（版そのもの）・`license`・`description`（`Cargo.toml` から来る）を外す。
//! * **位置を決めて** `summary` を外す（`paths.<パス>.summary` と `paths.<パス>.<メソッド>.summary`）。
//!   **`properties` の下の `summary` という名前の欄は残る**（それは形）。
//! * `example` / `examples` の**キーワード**を外す（名前かどうかは本番の `strip_descriptions` と同じ
//!   `NAME_MAPS` の判定 —— [`strip_keywords_for_tests`]）。**版のヘッダの値の例もこれで外れる。**
//! * 実験中の口を外す（そのパスに口が残らなければ、パスごと外す）。
//! * `components.schemas` から、**実験中の口からは辿れて、それ以外からは辿れない型**を外す。
//! * **`operationId` と型の名前は残す**（コード生成の使い手には改名に当たる）。
//!
//! 失敗の形（[`error_shapes`]）: [`error_catalog`] の `(状態コード, type, code)` を重複なく並べる。
//!
//! 両方を1つの JSON にし、鍵を並べ、空白を入れずに綴る（[`canonical`]。**配列の順は保つ**）。
//! **鍵を自分で並べる**のは、`serde_json` の `Map` の並びがフィーチャに依って変わるため
//! （`preserve_order` がどこかの依存で有効になると、挿入順になる）。
//!
//! # ⚠ 見ていないこと
//!
//! * **意味だけの変更**（欄の形はそのままで値の作り方が変わる。例: B-71 の `endpoint` からクエリを落とした・
//!   B-61 の `dialogue` / `narration`）は指紋が変わらない。それは今までどおり、差分を読んで決める。
//! * **どの口がどの失敗を返すか**は見ていない。見ているのは「失敗の形の集合」だけで、
//!   既にある `code` を別の口が返し始めても指紋は変わらない。
//! * 失敗の**文言**（`message`）と、`ApiError` を通らない応答（hyper の 400 等）は見ていない。
//! * **既存の行が書き換えられたこと**は見えない（約束で守る）。
//! * **綴りだけの変化**（依存の更新で仕様書の書き方が変わった・Rust の関数や型を改名して
//!   `operationId` / 型の名前が変わった）も赤になる。そのときも版の小さい側を上げて行を足す（ADR-0047）。

use std::collections::BTreeSet;

use personacore::server::{
    API_VERSION, API_VERSION_HEADER, ErrorShape, api_document, error_catalog,
    strip_keywords_for_tests,
};
use serde_json::{Value, json};

#[path = "common/sha256.rs"]
mod sha256;

use sha256::sha256_hex;

/// 版 → 形の指紋の表。
const TABLE: &str = include_str!("fixtures/api_shape_fingerprints.tsv");

/// 実験中の口の印（`src/server/openapi.rs` の `STABILITY_EXTENSION` と同じ綴り。
/// あちらは `#[cfg(test)]` の私有なので、ここに写す —— 綴りが食い違えば実験中の口が指紋に入り、
/// 下の前提の assert が赤にする）。
const STABILITY_EXTENSION: &str = "x-personacore-stability";

/// 口を置くメソッドの鍵（OpenAPI の Path Item）。
const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// 外す例のキーワード（OpenAPI の `example` は 3.1 以降は非推奨だが、書かれうる）。
const EXAMPLE_KEYWORDS: [&str; 2] = ["example", "examples"];

const COMPONENT_PREFIX: &str = "#/components/schemas/";

fn document() -> Value {
    api_document().expect("仕様書は JSON にできること")
}

// ----------------------------------------------------------------------
// 正規化
// ----------------------------------------------------------------------

/// `value` の中の `$ref` が指す `components.schemas` の名前を、`found` に足す。
fn references(value: &Value, found: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                if key == "$ref"
                    && let Some(name) = inner
                        .as_str()
                        .and_then(|r| r.strip_prefix(COMPONENT_PREFIX))
                {
                    found.insert(name.to_owned());
                }
                references(inner, found);
            }
        }
        Value::Array(items) => {
            for item in items {
                references(item, found);
            }
        }
        _ => {}
    }
}

/// `roots` から `$ref` を辿って届く型の名前（推移的に）。
fn reachable(roots: &[&Value], schemas: &serde_json::Map<String, Value>) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for root in roots {
        references(root, &mut found);
    }
    let mut frontier: Vec<String> = found.iter().cloned().collect();
    while let Some(name) = frontier.pop() {
        let Some(schema) = schemas.get(&name) else {
            continue;
        };
        let mut next = BTreeSet::new();
        references(schema, &mut next);
        for name in next {
            if found.insert(name.clone()) {
                frontier.push(name);
            }
        }
    }
    found
}

/// **仕様書の、指紋を取る形**（モジュールの doc の「正規化」）。`document` は変えない。
fn shape(document: &Value) -> Value {
    let mut shape = document.clone();

    if let Some(info) = shape.get_mut("info").and_then(Value::as_object_mut) {
        for key in ["version", "license", "description"] {
            info.remove(key);
        }
    }

    // **例のキーワードを外す**（名前の位置の `example` / `examples` は残す。
    // 版のヘッダの `schema.examples` —— 値は版そのもの —— もこれで外れる）。
    strip_keywords_for_tests(&mut shape, &EXAMPLE_KEYWORDS);

    // 実験中の口を抜き出す（**抜いた口は、型の到達の根として使う**）。
    let mut experimental = Vec::new();
    if let Some(paths) = shape.get_mut("paths").and_then(Value::as_object_mut) {
        for item in paths.values_mut() {
            let Some(item) = item.as_object_mut() else {
                continue;
            };
            // **位置を決めて** summary を外す（Path Item の直下）。
            item.remove("summary");
            for method in METHODS {
                if item.get(method).is_some_and(|operation| {
                    operation[STABILITY_EXTENSION] == json!("experimental")
                }) && let Some(operation) = item.remove(method)
                {
                    experimental.push(operation);
                }
            }
            // **位置を決めて** summary を外す（口の直下）。
            for method in METHODS {
                if let Some(operation) = item.get_mut(method).and_then(Value::as_object_mut) {
                    operation.remove("summary");
                }
            }
        }
        paths.retain(|_, item| METHODS.iter().any(|method| item.get(method).is_some()));
    }

    // 実験中の口だけが指す型を外す。
    let schemas = shape
        .pointer("/components/schemas")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    // 安定な側の根は「型の定義を除いた仕様書の全部」（実験中の口は上で抜いてある）。
    // **型の定義を根に入れない** —— 入れると、実験中の口だけが指す型の中の `$ref` が
    // 安定な側から届いたことになる。
    let without_schemas = {
        let mut rest = shape.clone();
        if let Some(components) = rest.get_mut("components").and_then(Value::as_object_mut) {
            components.remove("schemas");
        }
        rest
    };
    let stable = reachable(&[&without_schemas], &schemas);
    let experimental_roots: Vec<&Value> = experimental.iter().collect();
    let only_experimental: BTreeSet<String> = reachable(&experimental_roots, &schemas)
        .difference(&stable)
        .cloned()
        .collect();
    if let Some(schemas) = shape
        .pointer_mut("/components/schemas")
        .and_then(Value::as_object_mut)
    {
        schemas.retain(|name, _| !only_experimental.contains(name));
    }

    shape
}

/// **失敗の形の一覧を、重複なく並べた JSON**（`[状態コード, type, code]` の配列）。
fn error_shapes(catalog: &[(&'static str, ErrorShape)]) -> Value {
    let shapes: BTreeSet<ErrorShape> = catalog.iter().map(|(_, shape)| *shape).collect();
    Value::Array(
        shapes
            .into_iter()
            .map(|shape| json!([shape.status, shape.kind, shape.code]))
            .collect(),
    )
}

/// **指紋の入力**: 仕様書の形と、失敗の形の一覧。
fn fingerprint_input(document: &Value, catalog: &[(&'static str, ErrorShape)]) -> Value {
    json!({
        "openapi_shape": shape(document),
        "error_shapes": error_shapes(catalog),
    })
}

/// **鍵を並べ、空白を入れずに綴る。** 配列の順は保つ。文字列と数は `serde_json` の綴り。
fn canonical(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut sorted: Vec<(&String, &Value)> = map.iter().collect();
            sorted.sort_unstable_by_key(|(key, _)| *key);
            out.push('{');
            for (index, (key, inner)) in sorted.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                write_canonical(inner, out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
}

/// 形の指紋（SHA-256 の 16 進、小文字）。
fn fingerprint(document: &Value, catalog: &[(&'static str, ErrorShape)]) -> String {
    sha256_hex(canonical(&fingerprint_input(document, catalog)).as_bytes())
}

// ----------------------------------------------------------------------
// 表
// ----------------------------------------------------------------------

/// 表の1行（`版<TAB>指紋`）。
#[derive(Debug, PartialEq, Eq)]
struct Row {
    version: (u32, u32),
    spelled: String,
    fingerprint: String,
}

/// `major.minor` を読む（`1.4` → `(1, 4)`）。
fn parse_version(text: &str) -> Option<(u32, u32)> {
    let (major, minor) = text.split_once('.')?;
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !digits(major) || !digits(minor) {
        return None;
    }
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// 表を読む。**`#` で始まる行と空行は読まない。CRLF も読む。**
/// 版が昇順でない・同じ版が2行ある・指紋が 64 桁の小文字の 16 進でない行は `Err`。
fn parse_table(text: &str) -> Result<Vec<Row>, String> {
    let mut rows: Vec<Row> = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let at = number + 1;
        let mut columns = line.split('\t');
        let (Some(spelled), Some(fingerprint), None) =
            (columns.next(), columns.next(), columns.next())
        else {
            return Err(format!("{at} 行目: `版<TAB>指紋` の2列でない: {line:?}"));
        };
        let version = parse_version(spelled)
            .ok_or_else(|| format!("{at} 行目: 版が読めない: {spelled:?}"))?;
        if fingerprint.len() != 64
            || !fingerprint
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(format!(
                "{at} 行目: 指紋が 64 桁の小文字の 16 進でない: {fingerprint:?}"
            ));
        }
        if let Some(last) = rows.last()
            && last.version >= version
        {
            return Err(format!(
                "{at} 行目: 版が昇順でない（{} の後に {spelled}）",
                last.spelled
            ));
        }
        rows.push(Row {
            version,
            spelled: spelled.to_owned(),
            fingerprint: fingerprint.to_owned(),
        });
    }
    Ok(rows)
}

/// **いまの指紋と表から、見張り番の判定を作る**（赤の文面もここで作る）。
fn verdict(rows: &[Row], version: &str, now: &str) -> Result<(), String> {
    let Some(index) = rows.iter().position(|row| row.spelled == version) else {
        return Err(format!(
            "表（tests/fixtures/api_shape_fingerprints.tsv）に版 {version} の行が無い。\
             ADR-0047 の版の上げ方に従って版を上げたなら、表の最後に行を足すこと。\
             いまの形の指紋: {now}\n足す行: {version}\t{now}"
        ));
    };
    // **形の食い違いを先に言う**（位置の食い違いより直す手順に近い）。
    let recorded = &rows[index].fingerprint;
    if recorded != now {
        return Err(format!(
            "API の形が変わった。ADR-0047 の版の上げ方に従って版を上げ、表に行を足すこと\
             （既存の行は書き換えない）。依存の更新や改名で綴りだけが変わったときも、\
             版の小さい側を上げて行を足す。\
             いまの版: {version} / 表のその行の指紋: {recorded} / いまの形の指紋: {now}"
        ));
    }
    if index + 1 != rows.len() {
        return Err(format!(
            "版 {version} の行が表の最後の行でない（最後は {}）。版は表に昇順で足し、\
             いまの版が最後の行になる。いまの形の指紋: {now}",
            rows[rows.len() - 1].spelled
        ));
    }
    Ok(())
}

// ----------------------------------------------------------------------
// 見張り番
// ----------------------------------------------------------------------

/// 実験中の口（`(パス, メソッド)`）。
fn experimental_operations(document: &Value) -> Vec<(String, String)> {
    document["paths"]
        .as_object()
        .into_iter()
        .flatten()
        .flat_map(|(path, item)| {
            METHODS
                .iter()
                .filter(|method| item[**method][STABILITY_EXTENSION] == json!("experimental"))
                .map(move |method| (path.clone(), (*method).to_owned()))
        })
        .collect()
}

/// **いまの形 == 表の、いまの版の行。** 形を変えて版を上げなければ赤。
#[test]
fn the_api_shape_matches_the_fingerprint_recorded_for_the_current_version() {
    let rows = parse_table(TABLE).unwrap_or_else(|err| panic!("表が読めない: {err}"));
    let document = document();
    assert!(
        !experimental_operations(&document).is_empty(),
        "前提: 実験中の口の印を読めていること（印の綴りが変わると、実験中の口が指紋に入る）"
    );
    let catalog = error_catalog();
    assert!(
        catalog.len() >= 90,
        "前提: 失敗の一覧が読めていること（{}）",
        catalog.len()
    );
    if let Err(message) = verdict(&rows, API_VERSION, &fingerprint(&document, &catalog)) {
        panic!("{message}");
    }
}

// ----------------------------------------------------------------------
// 対照
// ----------------------------------------------------------------------

/// **綴りの形**: 鍵は並び、空白は無く、配列の順は保たれ、文字列は JSON の綴り。
///
/// **鍵を挿入順と逆に入れた `Map` でも並ぶ**（`preserve_order` が有効な構成でもこの検査は意味を持つ。
/// いまの構成では `Map` は `BTreeMap` なので、並べ替えを外す変異はここでは緑 —— 条件付き等価）。
#[test]
fn the_canonical_form_sorts_keys_and_keeps_array_order() {
    let mut map = serde_json::Map::new();
    map.insert("b".to_owned(), json!([3, 1, 2]));
    map.insert(
        "a".to_owned(),
        json!({"z": null, "y": true, "x": "引用\"と\\と改行\n"}),
    );
    map.insert("c".to_owned(), json!(1.5));
    assert_eq!(
        canonical(&Value::Object(map)),
        r#"{"a":{"x":"引用\"と\\と改行\n","y":true,"z":null},"b":[3,1,2],"c":1.5}"#
    );
}

/// 全部の口の全部の応答の、版のヘッダの値の例を `to` にする。書き換えた数を返す。
fn restamp_version_header(document: &mut Value, to: &str) -> usize {
    let mut stamped = 0;
    let Some(paths) = document.get_mut("paths").and_then(Value::as_object_mut) else {
        return 0;
    };
    for item in paths.values_mut() {
        let Some(item) = item.as_object_mut() else {
            continue;
        };
        // **Path Item の直下にはメソッド以外の鍵（`parameters` / `summary` 等）も来る**ので、
        // `get_mut` で辿る（IndexMut は鍵が無いと `null` を作り、配列だと panic する）。
        for operation in item.values_mut() {
            let Some(responses) = operation
                .get_mut("responses")
                .and_then(Value::as_object_mut)
            else {
                continue;
            };
            for response in responses.values_mut() {
                if let Some(examples) =
                    response.pointer_mut(&format!("/headers/{API_VERSION_HEADER}/schema/examples"))
                {
                    assert_eq!(
                        *examples,
                        json!([API_VERSION]),
                        "前提: 版のヘッダの例は版そのもの"
                    );
                    *examples = json!([to]);
                    stamped += 1;
                }
            }
        }
    }
    stamped
}

/// **版だけを変えても指紋は変わらない。欄を1つ足すと変わる**（本物の仕様書に植える）。
#[test]
fn the_fingerprint_ignores_the_version_but_not_a_field() {
    let document = document();
    let catalog = error_catalog();
    let before = fingerprint(&document, &catalog);

    let mut renamed = document.clone();
    renamed["info"]["version"] = json!("0.0-planted");
    renamed["info"]["description"] = json!("planted description");
    let stamped = restamp_version_header(&mut renamed, "0.0-planted");
    assert!(
        stamped >= 40,
        "前提: 全部の応答の版のヘッダを書き換えたこと（{stamped}）"
    );
    assert_ne!(
        canonical(&renamed),
        canonical(&document),
        "前提: 植えた差が綴りに出る"
    );
    assert_eq!(fingerprint(&renamed, &catalog), before);

    let mut widened = document.clone();
    widened["components"]["schemas"]["ErrorBody"]["properties"]["planted"] =
        json!({"type": "string"});
    assert_ne!(fingerprint(&widened, &catalog), before);
}

/// **`summary` と `example` / `examples` を変えても指紋は変わらない。**
/// **`summary` / `example` という名前の欄を足すと変わる。`operationId` を変えても変わる。**
#[test]
fn the_fingerprint_ignores_summaries_and_examples_but_not_fields_named_so() {
    let document = document();
    let catalog = error_catalog();
    let before = fingerprint(&document, &catalog);

    // 前提: 安定な口に summary があること（無ければ外す処理を試していない）。
    assert!(
        document["paths"]["/v1/models"]["get"]["summary"].is_string(),
        "前提: 安定な口が summary を持つ"
    );

    let mut reworded = document.clone();
    reworded["paths"]["/v1/models"]["get"]["summary"] = json!("planted summary");
    reworded["paths"]["/v1/models"]["summary"] = json!("planted path summary");
    reworded["components"]["schemas"]["ErrorBody"]["example"] = json!({"planted": true});
    reworded["components"]["schemas"]["ErrorBody"]["examples"] = json!([{"planted": true}]);
    reworded["components"]["schemas"]["ErrorDetail"]["properties"]["message"]["examples"] =
        json!(["planted"]);
    assert_ne!(
        canonical(&reworded),
        canonical(&document),
        "前提: 植えた差が綴りに出る"
    );
    assert_eq!(fingerprint(&reworded, &catalog), before);

    for (name, planted) in [
        ("summary", json!({"type": "string"})),
        ("example", json!({"type": "string"})),
        ("examples", json!({"type": "array"})),
    ] {
        let mut named = document.clone();
        named["components"]["schemas"]["ErrorDetail"]["properties"][name] = planted;
        assert_ne!(
            fingerprint(&named, &catalog),
            before,
            "{name} という名前の欄は形（外さない）"
        );
    }

    let mut renamed = document.clone();
    renamed["paths"]["/v1/models"]["get"]["operationId"] = json!("planted_operation_id");
    assert_ne!(
        fingerprint(&renamed, &catalog),
        before,
        "operationId は残す（コード生成の使い手には改名に当たる）"
    );
}

/// **失敗の形の一覧が指紋に入る**: `code` を1つ足す・綴りを変える・状態コードを変えると変わり、
/// 文言だけ・並び・重複では変わらない。
#[test]
fn the_fingerprint_folds_in_the_error_shapes() {
    let document = document();
    let catalog = error_catalog();
    let before = fingerprint(&document, &catalog);

    let mut added = catalog.clone();
    added.push((
        "planted",
        ErrorShape {
            status: 409,
            kind: "invalid_request_error",
            code: Some("planted_code"),
        },
    ));
    assert_ne!(fingerprint(&document, &added), before);

    let respelled: Vec<(&'static str, ErrorShape)> = catalog
        .iter()
        .map(|(name, shape)| {
            let mut shape = *shape;
            if shape.code == Some("upstream_unauthorized") {
                shape.code = Some("upstream_unauthorised");
            }
            (*name, shape)
        })
        .collect();
    assert_ne!(respelled, catalog, "前提: 綴りを変えた行がある");
    assert_ne!(fingerprint(&document, &respelled), before);

    let restatused: Vec<(&'static str, ErrorShape)> = catalog
        .iter()
        .map(|(name, shape)| {
            let mut shape = *shape;
            if shape.code == Some("agreement_not_proposed") {
                shape.status = 422;
            }
            (*name, shape)
        })
        .collect();
    assert_ne!(restatused, catalog, "前提: 状態コードを変えた行がある");
    assert_ne!(fingerprint(&document, &restatused), before);

    // 並び・重複・名前は形ではない。
    let mut shuffled = catalog.clone();
    shuffled.reverse();
    shuffled.push(("duplicate", catalog[0].1));
    assert_eq!(fingerprint(&document, &shuffled), before);
}

/// 実験中の口だけが指す型の名前。
fn only_experimental_schemas(document: &Value) -> BTreeSet<String> {
    let shaped = shape(document);
    let all: BTreeSet<&String> = document["components"]["schemas"]
        .as_object()
        .expect("components.schemas")
        .keys()
        .collect();
    let kept: BTreeSet<&String> = shaped["components"]["schemas"]
        .as_object()
        .expect("components.schemas")
        .keys()
        .collect();
    all.difference(&kept).map(|name| (*name).clone()).collect()
}

/// **実験中の口と、その口だけが指す型を変えても指紋は変わらない。
/// 両方から指される型（`ErrorBody`）を変えると変わる。**
#[test]
fn the_fingerprint_ignores_only_the_experimental_route() {
    let document = document();
    let catalog = error_catalog();
    let before = fingerprint(&document, &catalog);
    let operations = experimental_operations(&document);
    let dropped = only_experimental_schemas(&document);
    assert_eq!(
        operations,
        vec![(
            "/admin/characters/{handle}/axis-candidates".to_owned(),
            "get".to_owned()
        )],
        "前提: 実験中の口は軸の候補だけ（ADR-0047 §② の口ごとの安定度）"
    );
    assert!(
        dropped.contains("AxisCandidatesView"),
        "前提: 実験中の口の応答の型が外れること: {dropped:?}"
    );
    assert!(
        !dropped.contains("ErrorBody"),
        "安定な口も指す型は外さない: {dropped:?}"
    );

    let (path, method) = &operations[0];
    let mut changed = document.clone();
    changed["paths"][path][method]["responses"]["299"] = json!({});
    changed["paths"][path][method]["operationId"] = json!("planted_operation_id");
    for name in &dropped {
        changed["components"]["schemas"][name]["properties"]["planted"] = json!({"type": "string"});
    }
    assert_ne!(
        canonical(&changed),
        canonical(&document),
        "前提: 植えた差が綴りに出る"
    );
    assert_eq!(fingerprint(&changed, &catalog), before);

    let mut shared = document.clone();
    shared["components"]["schemas"]["ErrorBody"]["required"] = json!(["planted"]);
    assert_ne!(fingerprint(&shared, &catalog), before);

    // 安定な口へ印を付けると（安定 → 実験中は約束を狭める）、指紋は変わる。
    let mut demoted = document.clone();
    demoted["paths"]["/v1/models"]["get"][STABILITY_EXTENSION] = json!("experimental");
    assert_ne!(fingerprint(&demoted, &catalog), before);
}

/// **型の到達を推移的に辿り、summary と例を位置で外す**（植えた仕様書）。
/// 実験中の口からだけ届く `B` → `C` は外れ、安定な口からも届く `S` と、`$ref` の連鎖で
/// 安定な口から届く `D` は残る。
#[test]
fn the_shape_follows_references_transitively_and_strips_by_position() {
    let planted = json!({
        "info": {"title": "t", "version": "9.9"},
        "paths": {
            "/stable": {
                "summary": "path item summary",
                "get": {
                    "summary": "operation summary",
                    "operationId": "stable_get",
                    "responses": {"200": {"content": {"application/json": {
                        "schema": {"$ref": "#/components/schemas/S"},
                        "example": {"x": 1},
                    }}}},
                },
            },
            "/exp": {
                "parameters": [],
                "get": {
                    STABILITY_EXTENSION: "experimental",
                    "responses": {"200": {"content": {"application/json": {
                        "schema": {"$ref": "#/components/schemas/B"}}}}},
                },
            },
            "/mixed": {
                "get": {STABILITY_EXTENSION: "experimental", "responses": {}},
                "post": {"responses": {}},
            },
        },
        "components": {"schemas": {
            "S": {
                "properties": {
                    "d": {"$ref": "#/components/schemas/D"},
                    "summary": {"type": "string", "examples": ["x"]},
                    "example": {"type": "string"},
                },
                "examples": [{"d": "x"}],
            },
            "D": {"type": "string", "example": "x"},
            "B": {"properties": {"c": {"$ref": "#/components/schemas/C"},
                                 "s": {"$ref": "#/components/schemas/S"}}},
            "C": {"type": "integer"},
            "Orphan": {"type": "boolean"},
        }},
    });
    let shaped = shape(&planted);
    let names: Vec<&String> = shaped["components"]["schemas"]
        .as_object()
        .expect("schemas")
        .keys()
        .collect();
    assert_eq!(names, vec!["D", "Orphan", "S"]);
    let paths: Vec<&String> = shaped["paths"].as_object().expect("paths").keys().collect();
    assert_eq!(paths, vec!["/mixed", "/stable"], "口の残らないパスは外れる");
    assert!(shaped["paths"]["/mixed"]["get"].is_null());
    assert!(shaped["paths"]["/mixed"]["post"].is_object());
    assert_eq!(shaped["info"], json!({"title": "t"}));

    // 位置で外したもの・残したもの。
    assert_eq!(
        shaped["paths"]["/stable"],
        json!({"get": {
            "operationId": "stable_get",
            "responses": {"200": {"content": {"application/json": {
                "schema": {"$ref": "#/components/schemas/S"}}}}},
        }})
    );
    assert_eq!(
        shaped["components"]["schemas"]["S"],
        json!({"properties": {
            "d": {"$ref": "#/components/schemas/D"},
            "summary": {"type": "string"},
            "example": {"type": "string"},
        }})
    );
    assert_eq!(
        shaped["components"]["schemas"]["D"],
        json!({"type": "string"})
    );
}

/// **表の読み方と、判定の3つの腕**（植えた表）。
#[test]
fn the_table_and_the_verdict_read_a_planted_table() {
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    let planted = format!("# 注釈\r\n\r\n1.3\t{a}\r\n1.10\t{b}\n");
    let rows = parse_table(&planted).expect("読めること");
    assert_eq!(
        rows,
        vec![
            Row {
                version: (1, 3),
                spelled: "1.3".to_owned(),
                fingerprint: a.clone()
            },
            Row {
                version: (1, 10),
                spelled: "1.10".to_owned(),
                fingerprint: b.clone()
            },
        ]
    );

    assert_eq!(verdict(&rows, "1.10", &b), Ok(()));
    let changed = verdict(&rows, "1.10", &a).expect_err("形が変わった");
    assert!(changed.starts_with("API の形が変わった。"), "{changed}");
    assert!(changed.contains("ADR-0047"), "{changed}");
    assert!(changed.contains("綴りだけが変わったときも"), "{changed}");
    assert!(
        changed.contains(&format!("いまの形の指紋: {a}")),
        "{changed}"
    );
    let missing = verdict(&rows, "1.11", &b).expect_err("行が無い");
    assert!(missing.contains("版 1.11 の行が無い"), "{missing}");
    assert!(missing.contains(&format!("1.11\t{b}")), "{missing}");
    let stale = verdict(&rows, "1.3", &a).expect_err("最後の行でない");
    assert!(stale.contains("最後の行でない"), "{stale}");
    let stale_and_changed = verdict(&rows, "1.3", &b).expect_err("形も位置も違う");
    assert!(
        stale_and_changed.starts_with("API の形が変わった。"),
        "形の食い違いを先に言う: {stale_and_changed}"
    );

    for broken in [
        format!("1.3 {a}\n"),
        format!("1.3\t{a}\textra\n"),
        format!("v1.3\t{a}\n"),
        format!("1.3\t{}\n", "A".repeat(64)),
        format!("1.3\t{}\n", "a".repeat(63)),
        format!("1.4\t{a}\n1.3\t{b}\n"),
        format!("1.3\t{a}\n1.3\t{b}\n"),
    ] {
        assert!(parse_table(&broken).is_err(), "読めてしまった: {broken:?}");
    }
}

/// **実物の表が読めて、少なくとも1行ある**（読めない表で見張り番が空振りしない）。
#[test]
fn the_recorded_table_is_readable() {
    let rows = parse_table(TABLE).unwrap_or_else(|err| panic!("表が読めない: {err}"));
    assert!(!rows.is_empty(), "表に行が無い");
}
