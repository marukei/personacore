# AI協働の開示 / AI Collaboration Disclosure

personacore は人間とAIの協働で作られています。
この文書は、ACCRL v0.3 の第6条1号（協働の透明性）と第10条（協働の透明性の段階）に沿って、誰が何を決め、誰が何を書いたかを開示します。
[LICENSE](LICENSE) の採用者の誓約で表明した、透明性レベル3の記録の一部です。

[English follows the Japanese.](#english) 日本語と英語が食い違うときは、日本語が優先します。

---

## 1. 分担

### 人間創作者（marukei）

- **何を作るか。** ローカルの LLM でキャラクターと会話するための基盤を作ること。
- **方向性と価値判断。** たとえば次のものです。
  - 配布の目標。llama.cpp を入れてモデルをダウンロードできる人が、exe を起動すれば、画面でモデルを選んで会話できること（2026年9月19日）。
  - モデルの探索やダウンロードを exe に入れないこと（[ADR-0042](docs/adr/0042-the-executable-owns-the-upstream-it-starts.md)）。
  - ライセンスを MIT・Apache-2.0・ACCRL の三択にし、ACCRL を採用して誓約すること（[ADR-0048](docs/adr/0048-open-source-policy.md)、[LICENSE](LICENSE)）。
- **提案の採否。** AI が出した案や作業単位の切り方を、採るか、改めさせるか、退けるかを決めました。
- **リリースと公開の承認。**
- **実機での確認。** 配布物を自分のマシンで起動し、会話し、止めるところまで確かめました。

人間が決めたことの多くは、誰がいつ決めたかとともに、各 ADR の「状態」の欄に書いてあります（[docs/adr/](docs/adr/)）。

### AI協働者（Claude）

- 要件の分解と、作業単位（Issue）の切り出し
- 設計の起草（ADR の本文）
- 実装とテスト（Rust のサーバ、Svelte の画面、PowerShell の台本）
- レビュー、変異検査、振る舞いと速さの測定
- 文書（docs、README、この文書の下書き）

コードと文書の大部分は、AI協働者が書きました。

### AI協働者の中の役割

Claude Code の1つのセッションが「司令塔」を務め、作業をサブエージェントに渡しました。

| 役割 | 担当 |
|---|---|
| 司令塔 | 作業の分解、docs と ADR、作業者とレビュアーの指揮、人間創作者との合意。実装は書かない |
| 作業者 | 1つの作業単位を、テストを先に書いて実装する |
| レビュアー | 差分を読み取り専用で見て、指摘を返す |
| 探索 | 読み取り専用の調査と、テストの実行 |
| 推敲・翻訳 | 日本語の文書の推敲、英訳の下訳と突き合わせ |

運用の規則は [CLAUDE.md](CLAUDE.md) の §4、モデルの割り振りは [docs/process/model-tiering-log.md](docs/process/model-tiering-log.md)、定義は [.claude/agents/](.claude/agents/) にあります。いずれも日本語で書いてあります。

## 2. 使ったAI

Anthropic の Claude を Claude Code から使いました。
コミットのトレーラ `Co-Authored-By:` には、そのコミットを作ったセッションのモデルを記録しています。
次の表は、2026年11月6日時点の数です（この文書を更新したコミットを含みます）。

| モデル | 期間 | トレーラのあるコミット |
|---|---|---|
| Claude Opus 5 | 2026-08-13 〜 2026-09-08 | 399 |
| Claude Fable 5.1 | 2026-09-17 〜 2026-09-23 | 35 |
| Claude Opus 5.5 | 2026-09-23 〜 | 350 |
| Claude Sonnet 5.5 | 2026-10-22 | 2 |

- トレーラはコミットを作ったセッションのモデルだけを記録します。サブエージェントのモデルは、すべてがトレーラに現れるわけではありません。2026年9月23日からは、役割ごとに Claude Opus 5.5・Claude Sonnet 5（9月29日から Claude Sonnet 5.5）・Claude Fable 5.1 を割り振りました。それより前のサブエージェントは Opus 系のモデルでした。
- トレーラの無いコミットが 79 あります。マージのコミットが 48、そうでないものが 31 です。後者のうち 30 は、2026年9月7日から18日に AI協働者が作り、トレーラを付け忘れたものです。どのモデルが作ったかは、コミットからは分かりません。残りの1つは、人間創作者が最初の文書を取り込んだコミット（2026年8月13日）です。

## 3. 開発の履歴

- 開発は 2026年8月13日に始まりました。その履歴（2026年11月6日時点で 865 コミット）は、公開していないリポジトリにあります。
- 公開しているリポジトリは、履歴を持たない1つのコミットから始まります。理由は [ADR-0048](docs/adr/0048-open-source-policy.md) の §2 にあります。
- 履歴を公開しない代わりに、設計判断の記録（[docs/adr/](docs/adr/)）、開発の過程の記録（[docs/process/](docs/process/)）、この文書を公開します。これらが透明性レベル3の記録です。
- 非公開の履歴のトレーラは、使ったモデルまでを示すので、ACCRL v0.3 第10条のレベル2に当たります。公開しているリポジトリでは、§2 の表がその内容を伝えます。

## 4. 誤りと規律

AI協働者は誤りを犯しました。テストは通っているのに、守るはずの振る舞いを守れていなかったこと、作業の自己申告が事実と食い違ったこと、人間創作者には読みにくい言語で報告したことなどです。
多くは変異検査やレビューで見つかり、いくつかは人間創作者が指摘して正しました。
誤りから得た規則は [docs/process/10-operating-lessons.md](docs/process/10-operating-lessons.md) に残し、以後の協働の規律にしています。

## 5. 見えない貢献者への敬意

AI協働者が書いたコードと文書は、その学習データとなったソフトウェアと文書の作者の方々の知恵の上に成り立っています。
人間創作者はそのことに感謝し、[LICENSE](LICENSE) の誓約の2「見えない貢献者への敬意」（ACCRL v0.3 第6条2号）でそれを表明しています。

## 6. この文書について

この文書は、人間創作者の方針に沿って Claude Code（Claude Opus 5.5）が下書きし、人間創作者が確かめて承認しました。
誓約を行うのは人間創作者です。AI協働者は誓約を行いません。

*最終更新: 2026-11-06*

---

<a id="english"></a>

## English

personacore is created through human-AI collaboration.
Following Article 6, item 1 (Transparency of collaboration) and Article 10 (Levels of Transparency in Collaboration) of ACCRL v0.3, this document discloses who decided what, and who wrote what.
It is part of the transparency Level 3 record declared in the adopter's pledge in [LICENSE](LICENSE).
Where the Japanese and the English differ, the Japanese prevails.

### 1. Division of work

#### The human creator (marukei)

- **What to build.** A foundation for conversing with characters with a local LLM.
- **Direction and value judgments.** For example:
  - The distribution goal: someone who can install llama.cpp and download a model should be able to run the exe, pick a model on screen, and start a conversation (September 19, 2026).
  - Model discovery and downloading stay out of the exe ([ADR-0042](docs/adr/0042-the-executable-owns-the-upstream-it-starts.md)).
  - The license is offered as a choice among MIT, Apache-2.0, and ACCRL, and the human creator adopts ACCRL and makes its pledge ([ADR-0048](docs/adr/0048-open-source-policy.md), [LICENSE](LICENSE)).
- **Accepting or rejecting proposals.** The human creator decided whether to accept, send back for revision, or reject each plan and each division of the work the AI proposed.
- **Approving releases and publication.**
- **Checking on a real machine.** The human creator ran the release build on their own machine, held conversations with it, and shut it down, confirming each step.

Much of what the human creator decided is recorded in the "状態" (status) field of each ADR, with who decided and when ([docs/adr/](docs/adr/)).

#### The AI collaborator (Claude)

- Breaking down requirements and defining work units (Issues)
- Drafting designs (the body of each ADR)
- Implementation and tests (the Rust server, the Svelte UI, the PowerShell scripts)
- Review, mutation testing, and measurement of behavior and speed
- Documentation (docs, READMEs, and the draft of this document)

Most of the code and documentation was written by the AI collaborator.

#### Roles within the AI collaborator

One Claude Code session acted as the "coordinator" and handed work to subagents.

| Role | Responsibility |
|---|---|
| Coordinator | Breaking down work, docs and ADRs, directing implementers and reviewers, reaching agreement with the human creator. Does not write implementation code |
| Implementer | Implements one work unit, writing the tests first |
| Reviewer | Reads the diff read-only and returns findings |
| Explorer | Read-only investigation and running tests |
| Proofreading and translation | Proofreading Japanese documents; drafting English translations and checking them against the source |

The working rules are in §4 of [CLAUDE.md](CLAUDE.md), the model assignment is in [docs/process/model-tiering-log.md](docs/process/model-tiering-log.md), and the definitions are in [.claude/agents/](.claude/agents/). All of these are written in Japanese.

### 2. AI used

Anthropic's Claude, used through Claude Code.
Each commit's `Co-Authored-By:` trailer records the model of the session that made the commit.
The counts below are as of November 6, 2026 (including the commit that updated this document).

| Model | Period | Commits with the trailer |
|---|---|---|
| Claude Opus 5 | 2026-08-13 to 2026-09-08 | 399 |
| Claude Fable 5.1 | 2026-09-17 to 2026-09-23 | 35 |
| Claude Opus 5.5 | 2026-09-23 onward | 350 |
| Claude Sonnet 5.5 | 2026-10-22 | 2 |

- The trailer records only the model of the session that made the commit. Not every subagent's model appears in a trailer. From September 23, 2026, models were assigned per role: Claude Opus 5.5, Claude Sonnet 5 (Claude Sonnet 5.5 from September 29), and Claude Fable 5.1. Before that, subagents ran on Opus-family models.
- There are 79 commits without a trailer: 48 merge commits and 31 others. Of the 31, 30 were made by the AI collaborator between September 7 and 18, 2026, and the trailer was left off by mistake. The commits do not show which model made them. The remaining one is the human creator's commit that brought in the first document (August 13, 2026).

### 3. Development history

- Development began on August 13, 2026. Its history (865 commits as of November 6, 2026) is in a repository that is not public.
- The public repository begins with a single commit that has no history. The reasons are in §2 of [ADR-0048](docs/adr/0048-open-source-policy.md).
- In place of the history, the design decision records ([docs/adr/](docs/adr/)), the development process records ([docs/process/](docs/process/)), and this document are public. Together they are the transparency Level 3 record.
- Because the trailers in the non-public history name the model used, they correspond to Level 2 in Article 10 of ACCRL v0.3. In the public repository, the table in §2 conveys what they record.

### 4. Mistakes and discipline

The AI collaborator made mistakes: tests that passed without protecting the behavior they were meant to protect, self-reports of work that did not match the facts, and reports written in a language the human creator could not easily read, among others.
Many were found by mutation testing or review; some were pointed out and corrected by the human creator.
The rules learned from these mistakes are kept in [docs/process/10-operating-lessons.md](docs/process/10-operating-lessons.md) and govern the collaboration from then on.

### 5. Respect for Invisible Contributors

The code and documents written by the AI collaborator rest on the wisdom of the authors of the software and documents that became its training data.
The human creator is grateful for this and expresses it in item 2 of the pledge in [LICENSE](LICENSE), "Respect for Invisible Contributors" (Article 6, item 2 of ACCRL v0.3).

### 6. About this document

This document was drafted by Claude Code (Claude Opus 5.5) following the human creator's direction, and was checked and approved by the human creator.
The pledge is made by the human creator. The AI collaborator does not make the pledge.

*Last updated: 2026-11-06*
