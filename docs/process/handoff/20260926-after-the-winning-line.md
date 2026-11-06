# 引き継ぎ: 勝ちの線のあと（2026-09-26 / 司令塔 Opus 5.5 の compact 前）

**読む順**: この文書 → `CLAUDE.md` → `gh issue list` → `docs/06-roadmap.md` の「現在地」。
**進捗の権威はリポジトリと Issue**。ここは compact で消える推論のうち、次の回に要るものだけを書く。前の引き継ぎは [20260923-fable51-to-opus55.md](20260923-fable51-to-opus55.md)。

## 1. いまの状態

- **勝ちの線は済んだ**（origin#22 → origin#23 → origin#2 → origin#24 → origin#20 → origin#25、2026-09-25）。Release は [v0.1.0](../../../README.md#untraceable-references)（タグは `fb865e9`、非公開リポジトリ）。
- 使い始めて出たものも閉じた: origin#28（最後に選んだプロファイルで起きる）/ origin#29（起動の引数の1行書きを断る）/ origin#30（キャラクター作成の記入例と識別子の規則）。
- **`main` = `a94fb6a` 以降、作業ツリーは空、ワークツリー 0、進行中の Issue 0。**
- **v0.1.0 の zip の後に入ったもの**（次の Release に載る）: origin#28 / origin#29 / origin#30 と、README の「PowerShell の窓を閉じても止まる」。

## 2. 次の一手

**利用者が遊んで引っかかったところから。** 利用者は配布 exe で遊び始めている（環境の事実は司令塔のメモリ `personacore-user-setup`）。候補:

- 本文の見た目: origin#5（`**` を履歴で見続ける）/ origin#6（見出し・箇条書きが画面に残る）/ origin#7（公式 Qwen の `‘ ’`）/ origin#8（記憶の重複）。
- 画面: origin#3（D-12b / 骨組みの作り直し）/ origin#4（D-12c / 会話履歴の導線）。
- 次の Release（修正を入れた zip）。作るなら `pwsh ./scripts/package.ps1` → zip を展開して歩く → 利用者に確かめてから `gh release create`。**`--target` は完全なハッシュ**（短いハッシュは 422）。
- 待ち: origin#27（公式の配布物の llama.cpp で歩く。Mac 版のときか、llama.cpp を入れ直すとき）。

## 3. この期間に分かった、運用の形

- **サブエージェントは階層ごとに定義を分けた**（`docs/process/model-tiering-log.md`）。設計が要る単位は `rust-implementer-design`、仕様が明確なら `rust-implementer`、レビューは `rust-reviewer`。**定型の A/B（`rust-routine-opus` / `-sonnet`）はまだ一度も使っていない。** エスカレーション（Fable）は利用者の承認が要る。
- **定義や CLAUDE.md を書き換えたら、アプリを再起動してから効く。** 効いたかは、サブエージェントの会話ログの `"effort"` の欄と、自己申告のモデル ID で確かめる。
- **レビューは毎回1〜2周で、1周目に要修正が出なかった単位は無い。** 「最後の小さな是正」は3周目を回さず、司令塔が変異を1本当てて確かめる形で回した。
- **実機の確認は `LOCALAPPDATA` を一時ディレクトリへ向ける。** 司令塔のシェルは Claude のアプリ（MSIX）の配下で、`%LOCALAPPDATA%` の書き込みがパッケージ専用の場所へ振り替えられ、本物より優先して見える（CLAUDE.md §4）。利用者の本物のファイルは、利用者が起こした exe の API（`GET /admin/upstream/profiles`）で読む。
- **利用者の exe が 8080 を使っていることがある。** 実機の確認は `PERSONACORE_PORT` を別の番号にし、`--no-browser` で起こす。
- **PowerShell の `Remove-Item` を変数や複数の操作と同じ1行に入れると、安全装置が「`/` の削除」と誤って全体を止める。** 止まったときは何も実行されていない。削除は `-LiteralPath '<固定の文字列>'` で単独の呼び出しにする。
- 内蔵ブラウザの窓が描画されないときは、座標のクリックが失敗する。`javascript_tool` で DOM を操作して確かめる。

## 4. 未決・懸念

| 何 | どこ |
|---|---|
| 起動ログの見本の一言がまだ「`--upstream` を付けて起動し直して」と言う（画面は「直す」「切り替える」を案内） | origin#23 のコメント |
| 既定の上流が LM Studio を指したまま（覚えた名前が無い初回だけ効く） | Release の説明 / origin#28 |
| 画面で読み直した後も、前の「保存しました」の一言が残る | origin#24 のコメント |
| 画面から profile を消す操作が無い（ADR-0042 §決定8。要ると分かってから） | ADR-0042 |
| `tests/upstream_launch.rs` の偽 llama を待つテストが負荷で揺れる | CLAUDE.md の既知の揺れ |
