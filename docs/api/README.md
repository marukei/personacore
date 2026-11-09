# personacore API guide

personacore is a small server that gives a local LLM **characters and memories**.
It sits between a chat front end and an OpenAI-compatible inference server
(LM Studio, `llama serve`, ...). For every turn it builds the prompt itself from the
character's stored persona, personality stats, attributes, memories and recent history,
forwards it upstream, returns the reply, stores the exchange in SQLite, and afterwards
extracts new memories from the conversation in the background.

The HTTP API is the product; the bundled web UI is just its first client and uses
nothing but the routes described here. This guide is for people who want to drive
personacore from their own program or from an existing chat client.

The field-level contract is the OpenAPI document the running server serves
(`GET /openapi.json`). This guide explains how the pieces fit together and does not
repeat every field.

> The rest of the repository documentation is written in Japanese.
> Apart from the license texts, the only other English documents are the top-level README
> ([README.en.md](../../README.en.md)) and the English section of the AI collaboration disclosure
> ([NOTICE-AI-COLLABORATION.md](../../NOTICE-AI-COLLABORATION.md)); design decisions are
> recorded in `docs/adr/`.

## Contents

- [Security model: local only, no authentication](#security-model-local-only-no-authentication)
- [Running personacore without the UI](#running-personacore-without-the-ui)
- [The OpenAPI document and the version header](#the-openapi-document-and-the-version-header)
- [Chat: the OpenAI-compatible endpoint](#chat-the-openai-compatible-endpoint)
- [Using personacore from SillyTavern or Open WebUI](#using-personacore-from-sillytavern-or-open-webui)
- [Managing characters](#managing-characters)
- [Personality stats and drift](#personality-stats-and-drift)
- [Attributes](#attributes)
- [Promises and rules](#promises-and-rules)
- [Memories](#memories)
- [Conversation history](#conversation-history)
- [Upstream inference servers](#upstream-inference-servers)
- [The listening port](#the-listening-port)
- [Errors](#errors)
- [Versioning and compatibility](#versioning-and-compatibility)
- [Walkthrough with curl](#walkthrough-with-curl)
- [Known limitations](#known-limitations)

## Security model: local only, no authentication

personacore **binds only to `127.0.0.1`**. The listen address cannot be configured;
only the port can. There is **no authentication** of any kind.

Binding to `127.0.0.1` keeps other machines out. It does **not** keep out other
programs on the same machine. **Any program running on the same machine can call every
route**, and that includes programs run by other user accounts on that machine.

Those routes include saving an upstream profile, which stores a command line, and
selecting it, which **runs that command** (see
[Upstream inference servers](#upstream-inference-servers)). It also includes reading and
changing every character, memory and conversation.

### Host and Origin checks

To keep web pages open in a browser on the same machine out (for example through DNS
rebinding, or a page on another site sending requests to `127.0.0.1`), personacore
checks the host the request is addressed to and its `Origin` header on **every**
request, API and UI alike. `<port>` below is the port personacore is actually listening
on.

- **`Host`**: if present, it must be `127.0.0.1:<port>` or `localhost:<port>`.
  The name is not case-sensitive. A name with a trailing dot (`localhost.`), any other
  name or address (including `[::1]` and other `127.x.x.x` addresses), a different port,
  or more than one `Host` header is refused. A request without a `Host` header is
  accepted: browsers always send `Host`, so such a request cannot come from a web page.
- **The host in the request target**: an HTTP/2 request (`:authority`) or an HTTP/1.1
  request line with a full URL (`GET http://127.0.0.1:<port>/v1/models`) names the host
  there too. It follows the same rule as `Host`; a request that does not name one there
  is accepted.
- **`Origin`**: if present, it must be `http://127.0.0.1:<port>` or
  `http://localhost:<port>`, with nothing after the port. `Origin: null`, `https`, any
  other origin, or more than one `Origin` header is refused.
- **A missing port means port 80**, in `Host`, in the request target and in `Origin`
  alike: `Host: 127.0.0.1` and `Origin: http://localhost` are accepted only when
  personacore listens on port 80.

A refused request gets **403** with `error.code` `request_origin_not_allowed` (see
[Errors](#errors)); the headers that arrived are written to the server log, not to the
response.

What this means for integrators:

- Call personacore as `http://127.0.0.1:<port>` or `http://localhost:<port>`. Other
  names that resolve to `127.0.0.1` (an entry in the hosts file, for example) are
  refused.
- A program that runs outside a browser can call every route as before; most HTTP
  clients send the `Host` above on their own.
- A web page served from **another origin** cannot send write requests to personacore
  or read its responses from the browser.
  Serve your page through personacore's own origin, or make the calls from a program
  (a server-side proxy that sends the `Host` above and no foreign `Origin`).

These checks only keep out web pages. They are **not authentication**: any program on
the same machine, including one run by another user account, can still call every
route.

**Do not run personacore on a machine you do not trust**, or on a machine shared with
people who should not be able to do the above.

The consequence for integrators: **your program must run on the same machine as
personacore.** Do not put it behind a reverse proxy or port forwarding to expose it;
nothing in the API is designed for that. If personacore ever needs to be reachable from
elsewhere, that will be a separate design decision, and authentication would come with it.

## Running personacore without the UI

The release is a single executable. To run it as an API server only, pass `--no-ui`:

```sh
./personacore --no-ui
```

```powershell
.\personacore.exe --no-ui
```

With `--no-ui` the web UI is not served (`/` answers 404) and no browser is opened.
The API behaves exactly the same with or without the flag.

Other flags: `--no-browser` (serve the UI but do not open a browser),
`--upstream <name>` (start the upstream described by a saved profile, see
[Upstream inference servers](#upstream-inference-servers)), `--fresh` (move the
current database aside and start with an empty one), `--version`.

**Port.** The default port is `8080`. Set `PERSONACORE_PORT` to change it:

```sh
PERSONACORE_PORT=8123 ./personacore --no-ui
```

```powershell
$env:PERSONACORE_PORT = "8123"; .\personacore.exe --no-ui
```

A port can also be saved through the API (`PUT /admin/server/port`, see
[The listening port](#the-listening-port)); it is used from the next start. The order is
`PERSONACORE_PORT`, then the saved port, then `8080`.

The startup log prints the address it listens on.

**Where data lives.** By default everything is under a `personacore` directory in the
operating system's per-user data directory:

| OS | Directory |
|---|---|
| Windows | `%LOCALAPPDATA%\personacore\` |
| macOS | `~/Library/Application Support/personacore/` |
| Other Unix | `$XDG_DATA_HOME/personacore/` (or `~/.local/share/personacore/`) |

It holds the SQLite database (`personacore.sqlite`), the upstream profiles
(`upstreams.json`), the name of the last selected profile (`selected-upstream.txt`) and
the saved listening port (`listening-port.txt`, only after a port has been saved).
Only Windows has actually been run.

`PERSONACORE_DB_PATH` moves **only the database**. The upstream profiles stay in the
data directory unless you also set `PERSONACORE_UPSTREAMS` to another file. This matters
when you run a second instance for testing: with only `PERSONACORE_DB_PATH` set, it
still reads your real profiles and selects the profile you last selected (and listens on
your saved port unless `PERSONACORE_PORT` is set). It does not
start that profile's process at launch, but a `POST /admin/upstream/profile` with the
same name, or `--upstream <name>`, starts a process from your real profiles. Setting
`PERSONACORE_UPSTREAMS` to a file that does not exist gives an instance that starts no
upstream and neither reads nor saves a listening port.

**Upstream.** personacore does not ship a model. By default it sends requests to an
OpenAI-compatible server on the same machine (LM Studio's default address); set
`PERSONACORE_LLM_ENDPOINT` to point elsewhere. personacore starts without an upstream,
and the upstream may be started later; only chat needs it. Managing characters,
memories and history works without one.

## The OpenAPI document and the version header

```sh
curl -s "http://127.0.0.1:8080/openapi.json"
```

`GET /openapi.json` returns the OpenAPI document for the running server, including
itself. It is served with `--no-ui` too, because it is part of the API, not of the UI.

- It is **OpenAPI 3.2**. 3.2 is needed to describe the server-sent event stream item by
  item (`itemSchema`). Some tools do not read 3.2 yet.
- Every operation has a one-line English `summary`. The document carries no longer
  descriptions; this guide is the prose.
- Closed sets of values are listed as `enum`. In responses, an `enum` means "the values
  that can be returned today", not a closed promise; see
  [Versioning and compatibility](#versioning-and-compatibility).
- `x-personacore-stability: experimental` marks routes outside the compatibility promise.

**The version header.** Every response to a request that reaches personacore's router
carries the API version:

```
x-personacore-api-version: 1.5
```

That includes errors, 404 and 405 responses, the event stream and `/openapi.json`.
A response **without** `X-Personacore-Api-Version` was not produced by the personacore
API. For example, a request that is not valid HTTP is rejected with 400 by the HTTP
layer before it reaches the router, and that response has no version header.

The API version is separate from the application version: `personacore --version`
names the build, `X-Personacore-Api-Version` names the contract.

## Chat: the OpenAI-compatible endpoint

`POST /v1/chat/completions` follows the OpenAI Chat Completions shape, both
non-streaming (`application/json`) and streaming (`"stream": true`, `text/event-stream`
ending with `data: [DONE]`). `GET /v1/models` lists the characters as models.

### Choosing the character and the conversation: `model`

The `model` field does not name an inference model. It names a **character**, and
optionally a conversation (a "session") of that character:

| `model` | Meaning |
|---|---|
| `aoi` | Continue the latest conversation of the character `aoi`; create one if there is none |
| `aoi@new` | Start a new conversation. Plain `aoi` continues it once one turn has been stored in it |
| `aoi@42` | Continue conversation 42 of `aoi` (it must belong to `aoi`) |

"Latest" means the conversation with messages that was written to most recently.
The conversation is created before the upstream is called, so an `aoi@new` request that
fails (for example with 503 because no upstream is running) still leaves an empty
conversation behind. Plain `aoi` prefers conversations that have messages, so an empty
one is continued only when the character has no other.
`GET /v1/models` lists characters only (the `id` is the handle to put in `model`);
`@new` and `@<session-id>` are accepted but never listed.

The `model` field of every response is `<handle>@<session-id>` for the conversation that
was actually used. Send that value back as `model` to stay in the same conversation even
if another client starts a new one meanwhile.

An unknown character is rejected with 404 before anything is stored. An archived character
(see [Archiving and deleting characters](#archiving-and-deleting-characters)) is rejected
with 409 and `error.code` `model_archived`, also before anything is stored, and is not
listed by `GET /v1/models`.

### What personacore does with your request

personacore keeps the conversation history in its own database and builds the prompt
itself. From the request it uses only these fields:

| Field | What happens |
|---|---|
| `model` | Selects the character and conversation (above) |
| `messages` | **Only the last message whose `role` is exactly `user` is used**, and its `content` must be a string. The content of every other message is not used: all `system` messages, all `assistant` messages, and earlier `user` messages. Every element must still be an object with a string `role`; otherwise the whole request is rejected with 400 (`invalid_body`) |
| `temperature` | Passed to the upstream as is |
| `max_tokens` | Used as an upper bound, capped at the fixed reply reserve (currently 1024 tokens; a larger value is silently lowered to it). `0` is rejected. It must be a non-negative integer (`-1` is rejected with 400) |
| `stream` | `true` streams; absent or `false` does not |

Everything else (`top_p`, `stop`, `presence_penalty`, `n`, `tools`,
`max_completion_tokens`, ...) is accepted and ignored; unknown fields do not cause an
error. In particular `max_completion_tokens` has **no effect**; only `max_tokens` is
read. personacore sends its own fixed sampling settings upstream so that results are
reproducible.

If there is no `user` message the request is rejected with 400. If the last `user`
message has non-string content (for example the multi-part array used for images), the
request is also rejected with 400; personacore does not fall back to an earlier message.

The prompt is: a system message built from the character's display name, persona,
attributes, non-zero personality stats, active promises and rules (see
[Promises and rules](#promises-and-rules)), memories **about the user** (selected to fit the
budget; memories about the character itself are not used in the chat prompt, see
[Memories](#memories)) and output-format instruction (`output_format_instruction`, when
set), then the recent history of this conversation from the database, then the text of
your last `user` message. After the fixed part (name, persona, attributes, stats,
instructions) and the reply reserve, the latest exchange of the conversation is kept if
it fits; then the active promises and rules are taken, and if they do not all fit, the
oldest are left out. The room left after that is shared between memories and history,
half each; whatever one side does not use goes to the other, and each drops its oldest
entries first.

After the reply is complete, the pair (your message, the reply) is stored and memory
extraction starts in the background. In streaming mode the pair is stored only when the
upstream finished normally; **if the client stops the stream, nothing is stored.**
The reply text is returned exactly as the model produced it.

**If the upstream fails in the middle of a stream**, the 200 status has already been
sent. personacore then sends one final event whose `data` is an error object (the same
shape as in [Errors](#errors)) and **does not send `data: [DONE]`**. The chunks sent
before it stay as they were, and the turn is not stored. A stream that ends without
`[DONE]` did not complete.

The response has no `usage` field.

## Using personacore from SillyTavern or Open WebUI

Point the client's OpenAI-compatible connection at personacore:

- Base URL: `http://127.0.0.1:<port>/v1` (or `localhost` instead of `127.0.0.1`; other
  names are refused, see [Host and Origin checks](#host-and-origin-checks)). The
  connection works when the client's own server process makes the request; a request
  made from the client's web page carries that page's `Origin` and is refused. Which
  of the two a client does has **not been verified** by this project.
- API key: personacore does not check it; enter any value if the client requires one.
- Model: the character handle from the model list, or `<handle>@new` if the client lets
  you type a model name.

### The prompt is assembled twice; only personacore's version reaches the model

A chat client builds its own prompt from its character card, system prompt, lorebook
and its copy of the history, and sends that as `messages`. personacore then **throws
almost all of it away**: it keeps only the text of the last `user` message and builds a
new prompt from the character stored in personacore (see the table above). As a result:

- **The client's character card, system prompt, lorebook entries and author's notes have
  no effect** when they are sent as `system` or `assistant` messages or as earlier
  messages. Define the character in personacore (`POST /admin/characters`) instead.
  A near-empty card on the client side is enough.
- **Anything the client adds to the last user message is kept, stored and fed to memory
  extraction.** Features that append instructions or retrieved documents to the user's
  message will end up in personacore's history and memories. Turn them off.
- **Editing or deleting messages in the client does not change personacore's history.**
  Regenerating or "swiping" sends the same last user message again, and personacore
  stores it as another turn. To correct history, start a new conversation (`@new`).
- **Background requests are treated as conversation turns.** If the client sends extra
  requests to the same model (for example to generate a chat title, tags or follow-up
  suggestions), each one becomes a turn in the character's latest conversation and is
  fed to memory extraction. Disable those features or send them to a different model.
- Only `temperature` and `max_tokens` from the client's sampler settings have an effect.
- personacore sends its own conversation history, so the client's context size setting
  does not limit what the model sees; personacore budgets the prompt itself.

The exact names of these settings differ between clients and versions and have **not
been verified** against SillyTavern or Open WebUI by this project. The list above
describes what personacore does with whatever the client sends; check your client's
documentation for where each feature is switched off.

## Managing characters

| Route | Purpose |
|---|---|
| `GET /admin/characters` | List characters, archived ones included (rows that cannot be read are listed separately under `unreadable`) |
| `POST /admin/characters` | Create a character (201) |
| `PATCH /admin/characters/{handle}` | Change some fields, or archive or restore the character (`archived`). Fields you omit are unchanged; `null` clears `output_format_instruction` and `stat_drift_mode` and leaves the other fields unchanged |
| `DELETE /admin/characters/{handle}` | Delete a character for good (204). Allowed when it has no conversation or when it is archived; otherwise refused with 409 |

The **handle** (`handle`) is the identifier used in URLs and in `model`. It is fixed at
creation and cannot be changed; it may not contain `@`. The **display name**
(`display_name`) is what the model is told the character is called, and can be changed.
`base_persona` is the free-text description of the character. Every free-text field has
a length limit, so one oversized value cannot make every later prompt exceed the budget.

There is no `PUT` on a character (405): replacing a character wholesale is not offered.

All `/admin` request bodies reject unknown fields with 400, so a misspelled field name
is reported instead of silently ignored. (`/v1/chat/completions` is the opposite: it
accepts unknown fields, as OpenAI clients expect.)

### Archiving and deleting characters

Removing a character takes two steps, so that one mistaken request cannot lose its
conversations and memories:

1. **Archive** it: `PATCH /admin/characters/{handle}` with `{"archived": true}`. The
   character's `archived_at` becomes the time it was archived (it is `null` while the
   character is not archived; archiving it again keeps the first time). An archived
   character is not listed by `GET /v1/models`, and chat requests for it are rejected
   with 409 (`model_archived`). **Nothing else changes**: its conversations, memories,
   stats, attributes, promises and rules are kept, and the admin routes (conversation
   history, memories, stats, attributes, promises and rules, drift) still answer for it,
   including the ones that write (replacing stats or attributes, correcting a memory,
   writing or ending a promise or rule). A chat turn that was already past
   this check when the character was archived is still stored. `{"archived": false}`
   restores it. `null` or leaving the field out keeps the current state. The handle stays
   in use while the character is archived, so creating another character with the same
   handle is refused with 409.
2. **Delete** it: `DELETE /admin/characters/{handle}`. For an archived character this
   **permanently deletes the character together with everything that belongs to it**:
   conversations, messages, memories, stats, attributes and their history, promises and
   rules and their history, drift votes and proposed axes. No backup is kept; copy the
   database file first if you want one. After that the handle can be used again.

A character that has never had a conversation (not even an empty one left by a failed
`<handle>@new` turn) can still be deleted directly, without archiving it first. A character that has conversations and is not archived is refused
with 409 and `error.code` `character_has_sessions`, and nothing is deleted.

There is no route that deletes a single conversation.

## Personality stats and drift

| Route | Purpose |
|---|---|
| `GET /admin/characters/{handle}/stats` | Get the stats |
| `PUT /admin/characters/{handle}/stats` | Replace all stats at once |
| `GET /admin/characters/{handle}/drift` | How the stats are drifting: accumulated pressure per axis, recent votes and the effective drift mode |

A character has a list of named axes, each with an integer value from 0 to the
character's `stat_scale`. Axes with value 0 are **left out of the chat prompt entirely**
(writing "Passion: 0" makes models overplay the absence of passion).

**Drift** lets the stats change slowly as the conversation goes on. How it runs is set
per character with the `stat_drift_mode` field on `POST /admin/characters` and
`PATCH /admin/characters/{handle}`:

| `stat_drift_mode` | Meaning |
|---|---|
| `"off"` | Drift does not run |
| `"shadow"` | Drift is evaluated and recorded, but the stats do not change |
| `"on"` | Drift runs and changes the stats |
| `null` | Follow the server default (`PERSONACORE_STAT_DRIFT`, off when unset) |

A change takes effect from the next turn and does not reset anything that has
accumulated. `GET /admin/characters/{handle}/drift` reports the effective mode, whether
it came from the character or the server default, and the server default itself.

`GET /admin/characters/{handle}/axis-candidates` lists new axes that drift has
proposed. It is **experimental** (see [Versioning and compatibility](#versioning-and-compatibility)).

## Attributes

| Route | Purpose |
|---|---|
| `GET /admin/characters/{handle}/attributes` | Get the attributes |
| `PUT /admin/characters/{handle}/attributes` | Replace all attributes at once |
| `GET /admin/characters/{handle}/attributes/revisions` | The history of changes to the attributes |

Attributes are short key/value facts that are always in the prompt, each marked as being
about the user or about the character (`subject`). `PUT` replaces the whole set: an
attribute you leave out is removed. Every change is recorded. The revisions route
returns only the most recent changes, up to a fixed limit, and says how many there are
in total (`total_revisions`); there is no paging, so older changes cannot be read
through the API.

## Promises and rules

| Route | Purpose |
|---|---|
| `GET /admin/characters/{handle}/agreements` | List the promises and rules, active and ended, in the order they were written |
| `POST /admin/characters/{handle}/agreements` | Write one by hand (201). It starts active |
| `POST /admin/characters/{handle}/agreements/{agreement_id}/transitions` | End one: `{"state": "fulfilled"}`, `"broken"` or `"withdrawn"` |
| `GET /admin/characters/{handle}/agreements/transitions` | The history of the promises and rules: when each was written and ended |
| `POST /admin/characters/{handle}/agreements/{agreement_id}/adoption` | Adopt a proposed one, optionally editing `content`, `kind` or `keeper` first. It becomes active |
| `POST /admin/characters/{handle}/agreements/{agreement_id}/rejection` | Reject a proposed one. It stays in the list as `rejected` |

A character can keep promises (`kind: "promise"`, something to do once) and rules
(`kind: "rule"`, something to keep doing), written as short statements. Each one says who
keeps it (`keeper`): the user, the character, or both. **Active ones are in the chat
prompt on every turn**, grouped by who keeps it; ended ones are not. They are placed
before memories and history, but if the context window is too small for all of them,
the oldest active ones are left out of that turn. The list reports what happened on the
character's most recent turn as `last_turn` (`included` and `dropped` counts and the
time); it is `null` until the character has had a turn since personacore started.

Ending one does not delete it: it stays in the list with its new `state`, and every
change is recorded in the history. There is no route that edits or deletes a promise or
rule; to change one, end it and write a new one. Only deleting the whole character
removes them.

The number of active ones is limited (the list says the limit as `active_limit`). When
the limit is reached, writing another is refused with 409 and `error.code`
`agreement_limit_reached`, and nothing is stored; end an old one first. Ending one that
is not active (already ended, proposed or rejected) is refused with 409
(`agreement_not_active`). The history route returns only the most recent changes, up to a
fixed limit, together with their total count (`total_transitions`).

**Proposals.** After each chat turn, in the background and after memory extraction, the
sub model looks at that turn for a new promise or rule and, one turn later, checks what it
found (with one more check when the first two disagree). These requests go to the sub model
one at a time, in the same background queue as memory extraction, and are skipped for that
turn when the background is still busy with the previous turn or when a request would be
too large for the sub model's window. What survives the checks is stored as `proposed`,
together with the conversation it came from (`source_session_id`). **Proposed ones are not
in the chat prompt and do not count toward `active_limit`.** List them with the list route
(their `state` is `proposed`). Rejected ones stay in the list as `rejected`, and the
proposals that are already in the list (proposed, rejected or active) are shown to the
check so that the same one is not proposed again.

Adopting a proposal makes it active (send `{}` to adopt it unchanged, or any of `content`,
`kind` and `keeper` to change them first); it is refused with 409 `agreement_limit_reached`
when the limit is reached (the same refusal as for writing one: end an old one first, and the
proposal stays proposed). Rejecting it keeps it in the list as `rejected`. Both are
recorded in the history. Adopting or rejecting something that is not proposed is refused
with 409 (`agreement_not_proposed`). Proposals always have `kind: "rule"` (the check does
not tell promises from rules, so the kind of a proposal is not a judgement); edit it when
you adopt one if needed. The proposals are a help, not a record: the model misses many
promises and rules, so writing them by hand is still the main way.

## Memories

| Route | Purpose |
|---|---|
| `GET /admin/characters/{handle}/memories` | List current memories; add `?include=superseded` to also list the most recent of the ones that were replaced |
| `POST /admin/characters/{handle}/memories/{memory_id}/corrections` | Replace a memory with a corrected version (201) |

Memories are extracted automatically from conversations in the background after each
turn. Each memory says whether it is about the user or about the character (`subject`).
**Only memories about the user (`subject: user`) are used in the chat prompt.** Memories
about the character are still extracted, listed here and used by background requests,
but not in the chat prompt (they carried the character's own stock phrases back into
every reply). Because a correction states the `subject`, correcting a memory to the
other subject moves it into or out of the chat prompt.

**Memories are never deleted or edited in place.** A correction **appends** a new memory
and marks the old one as superseded by it. Only memories that have not been superseded
are used. The superseded ones are kept in the database, so the history of
what the character believed is not lost. `?include=superseded` returns only the most
recently written of them (by when the memory was written, not when it was replaced), up
to a fixed limit, together with their total count (`total`); there is
no paging, so older superseded memories cannot be read through the API. There is no route
that deletes or rewrites a memory, on purpose. The only way memories go away is deleting
the whole character (see [Archiving and deleting characters](#archiving-and-deleting-characters)).

## Conversation history

| Route | Purpose |
|---|---|
| `GET /admin/characters/{handle}/sessions` | List the newest conversations of a character. The first entry is the one that plain `<handle>` continues |
| `GET /admin/characters/{handle}/sessions/{session_id}/messages` | Read one page of a conversation |

History is read-only; conversations are written only by chat turns.
The list of conversations returns only the newest ones, up to a fixed limit, and says how
many there are in total (`total_sessions`); there is no paging, so older conversations
cannot be listed through the API (a known conversation id can still be read).
Each message carries the reply exactly as the model produced it (`raw_content`) and,
for replies, personacore's split into dialogue and narration.

Messages are paged: the response says which `limit` it used and whether older messages
exist (`has_older`). To read further back, pass `?before=<id>` with the id of the oldest
message you already have. You may pass `?limit=` up to a maximum; a larger value is
rejected with 400 rather than silently cut.

## Upstream inference servers

| Route | Purpose |
|---|---|
| `GET /admin/upstream` | What personacore knows about its upstreams: endpoint, model, what the upstream answered, whether personacore started it |
| `POST /admin/upstream/check` | Ask both upstreams now whether they answer; returns the same body as `GET /admin/upstream` |
| `GET /admin/upstream/profiles` | The saved upstream profiles (`upstreams.json`) |
| `PUT /admin/upstream/profiles/{name}` | Create or replace one profile; allowed only when the profiles live in the default place |
| `POST /admin/upstream/profile` | Switch to a saved profile while running, by name |

A profile describes an upstream: where it listens, which model it serves, and optionally
how personacore should start it (for example `llama serve` with a model file).
personacore can use one upstream for chat and a second one for background work
(memory extraction and drift).

A profile can also state each upstream's context length (`context_tokens` in
`upstreams.json`) for upstreams that do not report it. Such a value is used as is: it
takes precedence over `PERSONACORE_CONTEXT_TOKENS`, it follows the profile when you
switch, and personacore never replaces it with what the upstream reports (the upstream
is still asked right after launch, every five minutes, right after a switch, once a
started upstream has loaded, and on `POST /admin/upstream/check`, but only to record
what it says).

The profile API carries this field on each side of a profile: `GET /admin/upstream/profiles`
lists it (`null` when not written), and `PUT /admin/upstream/profiles/{name}` accepts it
(omit it or send `null` to leave it unwritten, which removes it from the file). An integer
from 0 to 2^64−1 that is not larger than the response reserve (1024 tokens), or does not fit
in 32 bits, is refused with 422 `upstream_profile_invalid`, reason `context_tokens_unusable`,
before anything is written. A value that is not a non-negative integer, or is larger than
2^64−1, cannot be read as a request and is refused with 400.

A profile can also carry a key for each upstream (`api_key` in `upstreams.json`) for
upstreams that require one. personacore then sends `Authorization: Bearer <key>` with
every request to that upstream (model list, chat, and token counting). The key belongs
to one side of a profile: if the background side (`sub`) has no key, nothing is sent to
it, even when the chat side has one. There is no environment variable for the key. A key
must be 1 to 4096 bytes of visible ASCII (no spaces or line breaks). A side with a key
must not put user information in its endpoint (`http://user:pass@host`), because two
`Authorization` headers would be sent; such a profile is refused when the profiles are read.
The key is stored in plain text in `upstreams.json`; remove it before giving the file to
someone else.

**When a side with a key points to a plain `http://` endpoint on another machine, the key
crosses the network unencrypted.** personacore checks where it is about to connect, every
time it opens a connection:

- `https://` endpoints and loopback endpoints (`127.0.0.0/8`, `::1`, `localhost`) are used
  as before.
- If every address is loopback or on the local network (`10.0.0.0/8`, `172.16.0.0/12`,
  `192.168.0.0/16`, `169.254.0.0/16`, `fc00::/7`, `fe80::/10`), the key is sent with a
  warning.
- If any address is outside those ranges (this includes `100.64.0.0/10`), the key is sent,
  with a warning, only when that side says `"allow_key_over_plain_http": true` in
  `upstreams.json`. Otherwise nothing is sent: the request stops before connecting, and chat
  answers 502 with `error.code` `upstream_key_not_approved` (in both the non-streaming and
  the streaming form). This is different from `upstream_unauthorized`: the upstream never
  saw the key.
- For a host name, the decision uses the addresses that name resolves to when the
  connection is opened, so a name that later resolves elsewhere is checked again.
- Requests on a side with a key and a plain `http://` endpoint that is not loopback do not
  go through an HTTP proxy (such as `HTTP_PROXY`), because the proxy, not the checked
  address, would receive the key.

`allow_key_over_plain_http` must be `true` or `false`; anything else is refused when the
profiles are read (reason `not_a_boolean`). It belongs to one side of a profile, like the
key. `GET /admin/upstream/profiles` lists it for each side (`false` when it is not
written). In `PUT /admin/upstream/profiles/{name}`, send `true` to write the approval
(also when the save changes the endpoint: the client asked for the new address), send
`false` to remove it, or omit it (or send `null`) to keep the value already written for
that profile and side **only when that side's `endpoint` is unchanged** (compared as
written); when such a save changes the endpoint, the approval is removed, because it was
given for the old address. **A side that has no key after the save never keeps an
approval**, whatever the request says (it is dropped, not refused): otherwise a key added
later to the same address would be sent as already approved.

`GET /admin/upstream` reports this for each side as `key_over_plain_http`:
`local_network`, `approved` (sent outside with the approval) or `not_approved` (not sent).
**The field is omitted when there is nothing to warn about** (no key, `https://`, loopback,
or a host name that has not been resolved yet). Like `unauthorized`, it is updated by
model list requests; for an address written as an IP address it is known from the start.

**The key is never returned by the API.** `GET /admin/upstream/profiles` lists, for each
side, `api_key_set` (whether a key is written) and `api_key_hint` (its last four
characters, or `null` when no key is written or the key is shorter than eight
characters). In `PUT /admin/upstream/profiles/{name}`, `api_key` works differently from
the other fields: **omit it to keep the key that is already saved for that profile and
side**, send `null` to remove it, or send a string to replace it. So a client that only
edits the other fields keeps the key. An unusable key is refused with 422
`upstream_profile_invalid`, reason `api_key_unusable` (`key` is `api_key`); a key on a
side whose endpoint carries user information is refused with reason
`api_key_with_userinfo` (`key` is `endpoint`).

When an upstream refuses the key (it answers 401 or 403), chat answers 502 with
`error.code` `upstream_unauthorized` instead of `upstream_failed`, in both the
non-streaming and the streaming form. `GET /admin/upstream` reports, for each side,
`unauthorized`: `true` when the last model list request to that upstream was refused
this way. It reflects model list requests only; `check_result` (below) says whether one
has been made yet.

personacore asks each upstream for its model list at launch (for a side whose profile
states its context length, right after launch, without delaying it; a side that
personacore starts itself is asked once it has loaded), every five minutes, and when
you send `POST /admin/upstream/check`. `GET /admin/upstream` reports,
for each side, when the last request was made in `checked_at` (RFC 3339, `null` before
the first one) and how it went in `check_result`:

| `check_result` | Meaning |
|---|---|
| `not_checked` | Not asked yet (`checked_at` is `null`) |
| `answered` | The upstream answered |
| `no_answer` | No answer: not reachable, or silent until the request was cut off |
| `no_model_list` | The upstream was reached, but its model list could not be read (for example, it has no model list endpoint). A model list is optional for an upstream, so chat may still work |
| `refused` | The upstream refused the key (`unauthorized` is `true`) |
| `key_not_sent` | The key was not sent, because sending it unencrypted outside the local network is not approved |

`POST /admin/upstream/check` asks both sides at once and answers when both requests
have finished (each is cut off after a few seconds if the upstream stays silent). If a
check of both sides is already running, it waits for that one and returns its result
instead of asking again (if that check does not finish, for example because its own
request was cancelled, it asks itself); it does not answer 409.

**When a profile says how to start its upstream, selecting it starts that process,
which loads a model onto the GPU.** When the profiles live in the default place, the
last profile you selected (with `--upstream <name>` or `POST /admin/upstream/profile`)
is remembered and selected again at the next launch without any flag.

**A profile selected again this way is not started at launch.** Its endpoint, model and
context length are loaded, but the process is not started, because something else may
be using the GPU by then. To start it, send `POST /admin/upstream/profile` with the same
name. This is the same request as a switch: one at a time, 409 while another switch is
running, and the request finishes even if the client disconnects. Until then, chat
answers with the same upstream error as while a model is loading (503
`upstream_unavailable`). `--upstream <name>` still starts the process at launch.

`GET /admin/upstream` reports, for each of the two upstreams, what personacore has done
with it in `launch.state`:

| `launch.state` | Meaning |
|---|---|
| `external` | personacore does not start this upstream (no model file in the profile, or no profile) |
| `not_started` | The profile says how to start it, but it has not been started yet; the same-name `POST /admin/upstream/profile` starts it |
| `loading` | Started, not answering with the model yet (`launch.since` is when it was started) |
| `ready` | Answering with the model file it was started with (`launch.answered_name`) |
| `failed` | Exited on its own, answered with another model, or could not be started (`launch.reason`, `launch.log`) |
| `stopped` | Stopped by personacore when it shut down |

The same response also carries `selected_profile`: the name of the profile currently
selected, which is the one chosen at launch (`--upstream <name>` or the remembered name)
or the name of the last switch that was accepted. A refused switch (404, 409, 422) does
not change it. It is `null` when no profile has been selected (the launch used the
environment variables and nothing has been switched since). The built-in UI's start
button sends this name to `POST /admin/upstream/profile` when the main upstream is
`not_started`; a client of your own can do the same.

Sending the name of the profile that is already selected, when nothing is `not_started`
and the profile has not been edited since, leaves the processes personacore started as
they are. That includes one that is `failed` because it exited on its own or answered
with another model; to start it again, switch to another profile and back. An upstream
that is `failed` because it could not be started at all (for example, the command does
not exist) is started again by sending the same name.

**`PUT /admin/upstream/profiles/{name}` accepts a command, its arguments, a model file,
an endpoint, a model name and a context length, and saves them.** Selecting that profile with
`POST /admin/upstream/profile` then **runs the saved command**. So anything that can call
the API can make personacore run a program of its choosing; see
[Security model](#security-model-local-only-no-authentication). The switch request
itself carries only a profile name.

## The listening port

| Route | Purpose |
|---|---|
| `GET /admin/server` | The port personacore is listening on, where it came from, the saved port, and the port the next start will use |
| `PUT /admin/server/port` | Save a port for the next start (`{"port": 8090}`), or clear the saved port (`{"port": null}`) |

The port is decided at start, in this order: `PERSONACORE_PORT`, then the saved port,
then `8080`. **Saving does not move the running server**; the saved port is used from
the next start.

`GET /admin/server` returns:

| Field | Meaning |
|---|---|
| `listening_port` | The port personacore is actually listening on (with `PERSONACORE_PORT=0`, the port the operating system chose) |
| `listening_port_source` | Where that port came from: `environment` (`PERSONACORE_PORT`), `saved` or `default` |
| `saved_port` | The saved port, or `null` when none is saved, the file cannot be read, or ports cannot be saved in this run |
| `saved_port_unreadable` | `true` when the saved port file exists but cannot be read (not a single number from 1024 to 65535, or it cannot be opened); `false` when there is no file, the file is empty, or ports cannot be saved in this run. Use it to tell "nothing saved" from "a broken file is there" |
| `next_port` | The port the next start will use, worked out now from this process's `PERSONACORE_PORT` and the saved port (`null` if `PERSONACORE_PORT` cannot be read as a port, in which case the next start stops) |
| `next_port_source` | Where `next_port` comes from (same values as `listening_port_source`) |
| `editable` | Whether a port can be saved in this run |

`PUT /admin/server/port` takes `{"port": <number or null>}`; `port` cannot be omitted.
A saved port must be from 1024 to 65535; `0` (let the operating system choose) can only
be given through `PERSONACORE_PORT`. The response has the same fields as
`GET /admin/server`, plus `saved_port_in_use`: whether another program on this machine
is using the saved port on `127.0.0.1` right now (`false` for the port this personacore
is listening on, `null` after clearing). It is worked out by trying to open the port, so
it is also `true` for a port inside a range Windows has excluded (opening it is refused
with a permission error), since the next start cannot listen there either. A port in use
is still saved.

| Status | When |
|---|---|
| 200 | Saved or cleared |
| 400 `invalid_body` | The body cannot be read, `port` is missing, or `port` is not an integer (including an integer too large for a signed 64-bit integer) |
| 400 `invalid_listening_port` | `port` is outside 1024 to 65535 |
| 409 `listening_port_read_only` | Ports cannot be saved in this run (`editable` is `false`) |
| 500 `listening_port_write_failed` | The file could not be written or removed; what was saved before is kept |

The saved port lives in `listening-port.txt` next to `upstreams.json` (see
[Running personacore without the UI](#running-personacore-without-the-ui)). Like the
remembered profile name, it is read and written **only when the profiles live in the
default place**: with `PERSONACORE_UPSTREAMS` set, the file is not read and saving
answers 409. A file that cannot be read (not a single number from 1024 to 65535) is
reported once in the startup log and ignored; it is not deleted. While it stays broken,
`GET /admin/server` says `saved_port_unreadable: true`; saving a port replaces the file,
and clearing removes it. When `PERSONACORE_PORT` is set, it wins over the saved port, and
`next_port_source` says `environment`.

## Errors

Errors from the API (every route under `/v1/*` and `/admin/*`, `/openapi.json`, and
unknown paths under `/v1` and `/admin`) use the OpenAI error shape:

```json
{"error": {"message": "...", "type": "invalid_request_error", "code": "..."}}
```

- `error.message` is a human-readable **Japanese** sentence. Do not parse it.
- Branch on the HTTP status and on `error.code` (present on most errors).
- Error bodies never contain internal paths or stack traces; details go to the server log.
- Saving an upstream profile can also answer 422 with a body that says which field of
  the profile is wrong; its shape is in the OpenAPI document.
- Any path, UI paths included, can answer 403 with `error.code`
  `request_origin_not_allowed` in this shape when the `Host` or `Origin` header is not
  personacore's own (see [Host and Origin checks](#host-and-origin-checks)). The OpenAPI
  document covers it with each route's `default` response.

Treat a status code you do not know as belonging to its class (4xx or 5xx). `error.code`
is an open set: treat a code you do not know as a failure of that status. A later version
may also split the code of an existing condition into a finer one; the list of changes in
[Versioning and compatibility](#versioning-and-compatibility) names the code it replaces.

Responses outside the API do not use this shape: for example a 404 for `/` with
`--no-ui`, or for a missing UI file, has an empty body. They still carry the version
header.

## Versioning and compatibility

The current API version is `1.5`.

The API version is kept separately from the application version and is stated in the
OpenAPI document (`info.version`) and in the `X-Personacore-Api-Version` header. The
paths themselves carry no version: moving to a new API version does not put a version
segment (such as `v1`) into the paths under `/admin`.

### Compatibility policy

| Change | Version |
|---|---|
| Adding a response field, adding an **optional request field**, adding a route | `1.x` (minor) |
| **Adding a value to a response `enum`** | `1.x` (minor). **Clients must accept values they do not know** |
| Widening what a request accepts (adding an `enum` value, raising a length limit, and so on) | `1.x` (minor) |
| Removing, renaming or changing the meaning of a field | `2.0` (major). **The old form is kept for one release** |
| **Adding a required request field, or making an optional field required** | `2.0` (major): every request from an older client would fail with 400 |
| **Narrowing what a request accepts** (removing an `enum` value, lowering a length limit, rejecting a form that used to be accepted, and so on) | `2.0` (major) |
| **Changing an existing status code** (a success from 200 to 201, a failure from 404 to 409, and so on) | `2.0` (major): clients branch on status codes |
| **Adding a new failure status** (returning 409 for a condition that could not occur before, and so on), or adding a value to `error.code` in error bodies | `1.x` (minor). **Clients must treat an unknown status code as its class (4xx / 5xx).** However, making a request that used to succeed fail narrows what is accepted, so that is `2.0` |
| **Splitting the `error.code` of an existing condition into a finer one** (the status code and `type` stay the same) | `1.x` (minor). `error.code` is an open set, and clients must treat a code they do not know as a failure of its status. The changes for that version name the code it replaces |

- **A response `enum` lists "the values that can be returned today"; it is not a closed
  promise.** The document puts `enum` on fields whose spelling is a closed set, so a
  client that validates strictly would see a new value as a breaking change. It is still
  a minor change: if every new state spelling bumped the major version, the version would
  stop meaning anything. Clients are expected to treat an unknown value as an unknown
  state.
- **On the request side the line is "does every request that used to succeed still
  succeed?"** Adding an optional field or an `enum` value is `1.x` (older clients simply
  do not send it). **Adding a required field or narrowing the accepted range makes
  requests that used to succeed fail with 400**, so it is major.
- **Exception: closing, for security, a way of reaching the API that this guide never
  promised does not change the version.** Requests made as this guide describes keep
  working; requests that reached personacore some other way (for example through another
  host name) may start to be refused within `1.x`. The `Host` / `Origin` checks were added
  this way.

Changes in `1.5` (from `1.4`): the two listening port routes, `GET /admin/server` and
`PUT /admin/server/port` (see [The listening port](#the-listening-port)), with the fields
`listening_port`, `listening_port_source`, `saved_port`, `saved_port_unreadable`,
`next_port`, `next_port_source` and `editable` (the save response adds
`saved_port_in_use`), and their new failures: 400 `invalid_listening_port`, 409
`listening_port_read_only` and 500 `listening_port_write_failed`. No existing route
changed.

Changes in `1.4` (from `1.3`): on the upstream routes (see
[Upstream inference servers](#upstream-inference-servers)), the new route
`POST /admin/upstream/check`; in `GET /admin/upstream` and the routes that return the
same body, the `selected_profile` field, the `unauthorized`, `key_over_plain_http`,
`checked_at` and `check_result` fields of each side, and the `not_started` value of
`launch.state`; in the upstream profiles, the `context_tokens`, `api_key_set`,
`api_key_hint` and `allow_key_over_plain_http` fields of each side, the optional
`context_tokens`, `api_key` and `allow_key_over_plain_http` request fields, and new `key`
and `reason` values in profile problems and in 422 `upstream_profile_invalid`; and two new
chat failures, 502 `upstream_unauthorized` (previously `upstream_failed`) and 502
`upstream_key_not_approved`. A remembered profile is no longer started at launch
(`not_started`); until it is started, chat answers 503 `upstream_unavailable`, as while a
model is loading. The `endpoint` of each side no longer carries a query string or a
fragment (user information was already shown as `***@`), and an endpoint that cannot be
read as a URL with a host is shown as `<読めない宛先>` ("unreadable endpoint"). In
`GET /admin/characters/{handle}/sessions/{session_id}/messages`, the `dialogue` and
`narration` of replies written from now on no longer include thinking that the upstream
leaked into the reply; `raw_content` is still the reply as received. Nothing was removed
or renamed.
Releases v0.3.0 and v0.3.1 still said `1.3` although they already carried these changes
(v0.3.0 part of them, v0.3.1 all of them).

Changes in `1.3` (from `1.2`): the two proposal routes (adoption and rejection; see
[Promises and rules](#promises-and-rules)), the `proposed` and `rejected` values of an
agreement's `state` (proposals are written in the background after each chat turn), and
the new failure 409 `agreement_not_proposed`. The message of `agreement_not_active` now
also covers proposed and rejected ones; its code and status did not change. No existing
route changed shape.

Changes in `1.2` (from `1.1`): the four promise and rule routes (see
[Promises and rules](#promises-and-rules)), the list's `last_turn` field, and their new failures: 409
`agreement_limit_reached` and `agreement_not_active`, 404 `agreement_not_found`, and 400
for an invalid `content` or an unknown `kind`, `keeper` or ending `state`. No existing
route changed.

Changes in `1.1` (from `1.0`): the optional `archived` request field and the
`archived_at` response field on characters, 409 `model_archived` for chat with an
archived character, and `DELETE /admin/characters/{handle}` accepting archived
characters that have conversations.

### Stability per route

| Routes | Promise |
|---|---|
| `/v1/chat/completions`, `/v1/models` | Stable as OpenAI-compatible routes (they follow the OpenAI-compatible shape; adding personacore-specific fields follows the table above) |
| `/admin/*`, `/openapi.json` | Stable within the version (the table above) |
| Routes marked `x-personacore-stability: experimental` in the OpenAPI document (currently only `/admin/characters/{handle}/axis-candidates`) | **Outside the compatibility promise.** Their shape may change without a version change |

## Walkthrough with curl

The examples assume a POSIX shell (Git Bash, WSL, macOS, Linux) and a server started
with `--no-ui`. Set the port first; the rest of the commands only use `$BASE`.

```sh
PORT=8080
BASE="http://127.0.0.1:$PORT"
```

**Fetch the OpenAPI document** (and see the version header):

```sh
curl -s -D - -o openapi.json "$BASE/openapi.json"
```

**Create a character:**

```sh
curl -s -X POST "$BASE/admin/characters" \
  -H 'Content-Type: application/json' \
  -d '{"handle": "aoi", "display_name": "Aoi", "base_persona": "A cheerful university student who loves astronomy."}'
```

**List characters:**

```sh
curl -s "$BASE/admin/characters"
```

**Give the character some personality stats and turn drift on for it:**

```sh
curl -s -X PUT "$BASE/admin/characters/aoi/stats" \
  -H 'Content-Type: application/json' \
  -d '{"stats": [{"axis": "Kindness", "value": 7}, {"axis": "Curiosity", "value": 5}]}'

curl -s -X PATCH "$BASE/admin/characters/aoi" \
  -H 'Content-Type: application/json' \
  -d '{"stat_drift_mode": "on"}'
```

**One chat turn, non-streaming** (needs a running upstream; without one it answers 503):

```sh
curl -s "$BASE/v1/chat/completions" \
  -H 'Content-Type: application/json' \
  -d '{"model": "aoi@new", "messages": [{"role": "user", "content": "Hi! What are you reading?"}]}'
```

The `model` of the response is `aoi@<session-id>`. Note the session id.

**List the conversations, then read the messages of one:**

```sh
curl -s "$BASE/admin/characters/aoi/sessions"

SESSION_ID=1   # the id from the chat response or the list above
curl -s "$BASE/admin/characters/aoi/sessions/$SESSION_ID/messages?limit=20"
```

**List the memories** (extraction runs in the background after each turn, so they may
appear a little later):

```sh
curl -s "$BASE/admin/characters/aoi/memories"
```

**Archive the character, then delete it for good** (this removes its conversations and
memories too, and cannot be undone):

```sh
curl -s -X PATCH "$BASE/admin/characters/aoi" \
  -H 'Content-Type: application/json' \
  -d '{"archived": true}'

curl -s -X DELETE "$BASE/admin/characters/aoi"
```

## Known limitations

- **The prompt personacore writes is in Japanese**, and the split of replies into
  dialogue and narration recognises Japanese corner brackets (「」) as dialogue. With
  other languages the reply is still returned unchanged, but the stored split will
  usually put everything in narration.
- Error messages are in Japanese (branch on `error.code`).
- Only Windows has been run. The macOS and Linux data directories above are what the
  code does, not something that has been tried.
- Tool calls, multi-part (image) user content and `n > 1` are not supported.
- **There is no authentication.** The `Host` and `Origin` checks keep out web pages,
  but any program on the same machine, including one run by another user account, can
  call every route (see [Security model](#security-model-local-only-no-authentication)).
- Some lists are bounded and have no paging: the list of conversations, superseded
  memories, attribute revisions and the history of promises and rules return only the
  most recent entries (with a total count). Older entries cannot be read through the API.
