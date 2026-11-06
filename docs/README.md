# personacore ドキュメント

ローカルLLMキャラクター会話基盤のオーケストレーション層。

読む順序に迷ったら [01-overview.md](01-overview.md) から。実装中の日常的な参照先は
リポジトリ直下の [CLAUDE.md](../CLAUDE.md)。

**使う → 設計を知る → 手を入れる**の順に並べ、開発の過程の記録はその後ろに分けて置いた。

## 使う

| ドキュメント | 内容 | いつ読むか |
|---|---|---|
| [09-setup.md](09-setup.md) | **各環境のセットアップ**。**要求**（環境が満たすこと）と**歩いた道**（実際に確かめた1通り）を分けて書く | 別の環境で動かすとき |
| [../scripts/README-start.md](../scripts/README-start.md) | **起動のしかた**。`start.ps1` の引数、**上流プロファイル（`upstreams.json`）の書き方**、DB の置き場所 | リポジトリから動かすとき・上流を替えるとき |
| [../scripts/README-dist.md](../scripts/README-dist.md) | **配布物（zip）に入る利用者向けの説明**。配った実行ファイルだけで遊ぶ人が読む | 配布物を作るとき・配られた側の目線を確かめるとき |
| [../README.en.md](../README.en.md) | **英語の入口**（English README）。何をするものか・何で動くか・配布物の入手と起動・上流・ライセンス。詳細は写さず、この表の文書と API ガイドへ案内する（O-10 / Issue origin#104） | 英語で読む人に、最初に渡すとき |
| [api/README.md](api/README.md) | **英語の API ガイド**（英語の案内は、これと根の英語の README の2つだけ。ライセンスの文書と、日英併記の AI 協働の開示の文書の英語の節を除く / [ADR-0047](adr/0047-the-api-is-the-product.md) ② P-2c）。画面なしの起動・`model` の綴り・既存のチャットクライアントから使うときの注意・管理の口・版と互換の方針。**綴りは `src/server/openapi/guide_tests.rs` が仕様書と定数に突き合わせる** | personacore を自分のソフトや既存のクライアントから使うとき |

## 設計を知る

| ドキュメント | 内容 | いつ読むか |
|---|---|---|
| [01-overview.md](01-overview.md) | 何を作るのか、なぜ作るのか、長期目標 | 最初に一度 |
| [02-architecture.md](02-architecture.md) | **1ターンで何が起きるか**（図解）、二段 OpenAI 互換構成、モジュール境界、SSE 中継、予算の枠 | 設計判断のとき |
| [03-data-model.md](03-data-model.md) | SQLite スキーマと**不変条件** | DB を触るとき（必読） |
| [06-roadmap.md](06-roadmap.md) | フェーズ分割、箱の定義、**順番の合意とその理由**。**開いている作業単位の一覧ではない**（それは GitHub Issue / [ADR-0043](adr/0043-work-units-open-and-close-as-issues.md)） | スコープと順番を確認するとき |
| [07-prompt-compiler.md](07-prompt-compiler.md) | 画像生成タグのコンパイラ（フェーズ3。**型とデータモデルは早期に**） | 画像生成に触るとき |
| [adr/](adr/README.md) | 設計判断の記録 | 「なぜこうなっている」を知りたいとき |

## 手を入れる

| ドキュメント | 内容 | いつ読むか |
|---|---|---|
| [../CLAUDE.md](../CLAUDE.md) | **絶対制約**・コマンド・モジュール構成・開発フロー | 書く前に必ず |
| [05-development.md](05-development.md) | TDD 方針、セキュア実装、3ロール体制の運用 | 実装を始める前 |
| [04-environment.md](04-environment.md) | ハード、**Windows ツールチェーン競合**、ログ、上流（llama.cpp 直）と特殊トークンの実測、**別の機へ持ち出すときのつまみ**、**モデルを差し替えるときの手順**。LM Studio で測った記録は [archive/04-environment-lm-studio.md](archive/04-environment-lm-studio.md) | 環境が壊れたとき／**機かモデルを替えるとき** |

## 開発の過程の記録（AI 協働の記録）

司令塔・作業者・レビュアーの3ロール体制（[CLAUDE.md](../CLAUDE.md) §4）で進めた開発の、経緯・運用の教訓・引き継ぎを
[process/](process/) に分けて置く。ただし AI 協働の開示の文書（表の1行目）だけは、`LICENSE` が名指すのでリポジトリの根に置いている。
**使う・設計を知るためには読まなくてよい。**

| ドキュメント | 内容 | いつ読むか |
|---|---|---|
| [../NOTICE-AI-COLLABORATION.md](../NOTICE-AI-COLLABORATION.md) | **AI 協働の開示**（日本語と英語）。人間と AI の分担・使ったモデルとコミットの数・公開しない履歴の扱い。`LICENSE` の誓約が名指す文書で、利用者が承認した文面（O-3 / Issue origin#50） | 誰が何を決め、誰が何を書いたかを知りたいとき |
| [process/08-work-log.md](process/08-work-log.md) | **2026-08-27 までに済んだ作業単位の経緯**とマイルストーンの測定記録。**予定は入っていない。それ以降の経緯は Issue のコメント**（[ADR-0043](adr/0043-work-units-open-and-close-as-issues.md)） | 昔の判断の理由を追うとき |
| [process/10-operating-lessons.md](process/10-operating-lessons.md) | CLAUDE.md §4 の規則の**経緯と実測**（事例 ID・撤回・訂正）。コードにある `CLAUDE.md「…」` の参照の本文 | 規則の理由を知りたいとき |
| [process/model-tiering-log.md](process/model-tiering-log.md) | **サブエージェントの階層**（どれを使うか）と、定型階層の A/B 記録 | サブエージェントを出すとき |
| [process/handoff/](process/handoff/20261023-v020.md)（最新。前の回は各文書の頭からたどる） | **司令塔のモデルを切り替えるときの引き継ぎ**。推論の中にしか無かった判断・未解決の懸念（確度つき）・サブエージェントの使い方の実態。**進捗の権威はリポジトリと Issue で、ここは書いた日の写し** | 司令塔が交代した最初の回 |

## 原典と、移した記録（archive/）

**ここに置いたものは歴史的資料であり、更新しない。**

チャットで詰めた内容を Claude Code へ引き渡すために書かれた元ドキュメント群:

| ファイル | 内容 |
|---|---|
| [archive/handoff-brief.md](archive/handoff-brief.md) | 実装ブリーフ本体（2026-08-13） |
| [archive/addendum-01-prompt-compiler.md](archive/addendum-01-prompt-compiler.md) | 追補1・プロンプトコンパイラ設計（2026-08-13） |

内容は上記の各ドキュメントへ分割・展開済みで、
記述が食い違う場合は分割後のドキュメントが正。ただし原典に書かれた
「なぜそう決めたか」の記述には分割後に落とした細部があるため、判断の背景を辿るときは有用。

肥大した文書から移した記録（作業単位 O-9 / 2026-11-04）:

| ファイル | 内容 |
|---|---|
| [archive/04-environment-lm-studio.md](archive/04-environment-lm-studio.md) | 環境の文書の「LM Studio」の節の大半（2026-08 の実測） |
| [archive/06-roadmap-done.md](archive/06-roadmap-done.md) | ロードマップの、2026-10-21 で止まった「現在地」と、取り消し線の付いた済んだ節 |

## 書き方の方針

- 1ファイルが長大になったら階層を切る。1ファイルに詰め込まない。
- 「決定」は ADR に、「現状の説明」は各ドキュメントに書く。両方に同じことを書かない。
- 未決事項は未決と明記する。決まったふりをして書かない。
