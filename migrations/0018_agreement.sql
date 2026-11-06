-- キャラクターごとの取り決め（約束・決まり）と、その状態の移り変わり
-- （ADR-0051 §決定1 / 作業単位 A-12a / Issue origin#61）。
--
-- ============================================================================
-- 設計はここに書かない
-- ============================================================================
--
-- 「何が入るか」「なぜ表が2枚あるのか」「1行壊れたときに何が止まるか」は
-- docs/03-data-model.md の character_agreement の節が正。
--
-- 適用済みのマイグレーションは二度と編集できない（sqlx のチェックサム）ので、
-- ここに設計判断を写すと、判断が変わった日に古い写しが永久に残る。
-- 残すのは「この SQL を読むときにだけ要る注意」だけにする。
--
-- ============================================================================
-- 既存の DB の振る舞いを変えない
-- ============================================================================
--
-- 2枚とも空で始まる。既存の表には1列も足さない。
--
-- ============================================================================
-- 名前に既存の走査の語を含めない
-- ============================================================================
--
-- 既存の SQL 走査（src/store の記憶・票・属性・軸の候補）は、テーブル名の語で
-- migrations/** を走査する。その語を含むテーブルを作ると誤検出される（0009〜0016 と同じ注意）。
--
-- ============================================================================
-- 書き換えてよいのは投影の状態の列だけ。履歴の表は追記だけ
-- ============================================================================
--
-- 行は消さない（終わった取り決めも残る / ADR-0051 §決定1）。消えるのは、キャラクターを
-- 完全に消したときのカスケードだけ（ADR-0049）。
-- 見張り番は src/store/agreement.rs の走査で、src/** と migrations/** の両方を見る。
--
-- ============================================================================
-- 状態の綴りには「提案中」「却下」の分も残してある
-- ============================================================================
--
-- proposed / rejected はいまの domain には無い（A-12b の後に足す / ADR-0051 §決定2）。
-- CHECK は列を作った後に変えられない（表の作り直しが要る）ので、綴りだけ先に入れておく。
-- いまの読み手（domain::AgreementState::from_label）はこの2つを読めず、
-- 読み出しは InvalidStoredValue に落ちる（知らない綴りと同じ扱い）。
-- 書き手はこの2つを書かない。
--
-- ============================================================================
-- created_at の綴りを 0010 / 0011 / 0013 / 0015 と揃える
-- ============================================================================
--
-- 既定を CURRENT_TIMESTAMP にすると綴りが2通りになり、読み出しのデコードが
-- 行ごとに割れる（CLAUDE.md の D-5a の実測）。

-- 取り決め1件につき1行。state はいまの状態の投影（履歴の表と同じトランザクションで書く）。
--
-- content の CHECK は 0009 の value と同じ4条件（typeof / NUL / 空白のみ / 長さ）。
-- 長さ 60 は domain::AgreementContent::MAX_CHARS の、移行した時点の写し。
-- 縮める向きは取れない（保存済みの行が読めなくなる）。
--
-- source_session_id は生まれた会話。手で書いたものは NULL。
-- 参照アクションは付けない（既定の NO ACTION）。いまは会話を1つだけ消す経路が無く、
-- キャラクターを消すときは同じ文のカスケードで取り決めも会話も消える。
-- 会話を1つ消す経路を足す日に、ここをどうするかを決めること（黙って NULL にしない）。
CREATE TABLE character_agreement (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    character_id INTEGER NOT NULL REFERENCES character(id) ON DELETE CASCADE,
    content TEXT NOT NULL CHECK (
        typeof(content) = 'text'
        AND instr(content, char(0)) = 0
        AND length(trim(
            content,
            ' ' || char(9) || char(10) || char(11) || char(12) || char(13) || char(12288) || char(160)
        )) > 0
        AND length(content) <= 60
    ),
    kind TEXT NOT NULL CHECK (kind IN ('promise', 'rule')),
    keeper TEXT NOT NULL CHECK (keeper IN ('user', 'character', 'both')),
    state TEXT NOT NULL CHECK (
        state IN ('proposed', 'active', 'rejected', 'fulfilled', 'broken', 'withdrawn')
    ),
    source_session_id INTEGER REFERENCES session(id),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- 「そのキャラクターの有効な取り決め」を毎ターン引くための索引。
CREATE INDEX character_agreement_character_idx ON character_agreement (character_id, state);

-- 状態の移り変わり（取り決め1件につき1行以上）。追記だけ。
--
-- before が NULL の行は「書かれた」（無かった → after）。
-- 遷移の規則（有効 → 果たした／破った／取り消した）は CHECK では書けない（行をまたぐ）。
-- 書き手が BEGIN IMMEDIATE の中で投影の状態を確かめてから、投影と同じトランザクションで足す。
CREATE TABLE agreement_transition (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agreement_id INTEGER NOT NULL REFERENCES character_agreement(id) ON DELETE CASCADE,
    before TEXT CHECK (
        before IS NULL
        OR before IN ('proposed', 'active', 'rejected', 'fulfilled', 'broken', 'withdrawn')
    ),
    after TEXT NOT NULL CHECK (
        after IN ('proposed', 'active', 'rejected', 'fulfilled', 'broken', 'withdrawn')
    ),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- 「そのキャラクターの履歴を新しい順に」は取り決めの表と結んで引く。取り決めごとに id 順で辿る。
CREATE INDEX agreement_transition_agreement_idx ON agreement_transition (agreement_id, id);
