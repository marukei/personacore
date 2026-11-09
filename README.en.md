# personacore

日本語: [README.md](README.md) · 繁體中文: [README.zh-TW.md](README.zh-TW.md)

personacore is a small server that gives a local LLM **characters and memories**.

It sits between a chat front end and an inference server you run yourself. For every
turn it builds the prompt from the character's persona, personality stats, memories and
recent history, sends it upstream, splits the reply into dialogue and narration, and
afterwards extracts new memories from the conversation in the background. Characters,
conversations, memories and personality stats are stored in SQLite. The stats can also
drift slowly as the conversation goes on; drift is experimental and off by default
(`PERSONACORE_STAT_DRIFT`; see the API guide).
**It speaks the OpenAI-compatible API on both sides**, so the front end can be the
bundled web UI or an existing chat client.

```
front end (the bundled web UI / an existing OpenAI-compatible client)
    ↓ POST /v1/chat/completions
personacore (Rust / axum)  ←→  SQLite
    ↓ POST /v1/chat/completions
upstream inference server
```

## Who it is for

People who run a language model on their own PC and want to talk to characters that keep
a persona and remember the conversation, from the bundled web UI, an existing chat
client or their own program.

## What it is not

- **Not an inference engine.** Inference is left to the upstream server.
- **Not a public server.** It listens only on `127.0.0.1` and has no authentication.
  Do not expose it to other machines.
- **Not finished.** What works is in GitHub Releases; what is not done yet is in GitHub
  Issues (the order and the reasons are in [docs/06-roadmap.md](docs/06-roadmap.md)).
- **Not multilingual yet.** The prompts personacore writes, the web UI and the error
  messages are in Japanese. Replies are split into dialogue and narration by Japanese
  corner brackets (「」). See the
  [known limitations](docs/api/README.md#known-limitations) in the API guide.

## What it runs on

- **Windows.** It has been run only on Windows 11 with PowerShell 7; no other OS or
  machine has been tried
  ([docs/adr/0036-the-range-of-machines-is-a-constraint.md](docs/adr/0036-the-range-of-machines-is-a-constraint.md)).
- **An upstream inference server**, in one of two ways (see below): personacore starts
  `llama serve` (llama.cpp) for you, or it connects to an OpenAI-compatible server that
  is already running (LM Studio, a proxy in front of another server, ...).
- **A model you bring** (a gguf file for llama.cpp). personacore does not ship one.

## Getting started

Download the Windows zip (`personacore-<version>-windows-x64.zip`) from GitHub Releases
and extract it anywhere. It contains `personacore.exe`, a `README.md` written in
Japanese, the third-party notices and the license documents. **Rust and pnpm are not
needed** to run it; the web UI is built into the executable.

The steps, in short (the zip's `README.md` has them in full, in Japanese; its source is
[scripts/README-dist.md](scripts/README-dist.md)):

1. Install llama.cpp from its official distribution.
2. Download a model in gguf format (for example from Hugging Face).
3. Start personacore in PowerShell:

   ```powershell
   .\personacore.exe
   ```

   It opens the web UI in your default browser. If it does not, open the URL printed
   after `url=` yourself. Stop it with **Ctrl+C**; that also stops the llama.cpp it
   started.
4. In the web UI, open the upstream screen (上流モデル), write the path of your gguf
   file into a profile, save it, and switch to that profile.

When you run the executable, data lives under `%LOCALAPPDATA%\personacore\`: the
SQLite database, the upstream profiles (`upstreams.json`) and the logs of the llama.cpp
processes personacore started. (`scripts/start.ps1`, below, keeps the database and the
logs under `data/` in the repository by default instead; see
[scripts/README-start.md](scripts/README-start.md).) `PERSONACORE_DB_PATH` moves the
database. The port is chosen at start, in this order: `PERSONACORE_PORT`, then the port
saved on the settings screen (設定) in the web UI, then the default. A saved port takes
effect from the next start. If the saved port or the default port is taken, personacore
starts on the next free port (up to ten above it) and says so in a banner at the top of
the web UI and in the startup log. A port set with `PERSONACORE_PORT` is never moved; if
it is taken, personacore stops.
The default port is set in `src/server/config.rs` as `DEFAULT_PORT`, and the startup
log prints the address.

Other flags: `--no-ui` (API only: no web UI and no browser), `--no-browser`,
`--upstream <name>` (start a saved profile at launch), `--fresh` (move the current
database aside, without deleting it, and start with an empty one), `--version`.

**Running from source** is `pwsh ./scripts/start.ps1`. It builds the web UI, checks the
upstream, prepares the database and starts the server. It needs the Rust toolchain named
in [rust-toolchain.toml](rust-toolchain.toml), pnpm and PowerShell 7. Its options are in
[scripts/README-start.md](scripts/README-start.md) (Japanese).

## Upstream inference servers

An upstream profile has two sides: chat, and background work (memory extraction and
drift). Each side is one of two kinds
([docs/adr/0042-the-executable-owns-the-upstream-it-starts.md](docs/adr/0042-the-executable-owns-the-upstream-it-starts.md)):

- **A side that names a gguf file** — personacore starts `llama serve` for it and, when
  it shuts down, stops only the processes it started itself.
- **A side without a gguf file** — personacore only connects to a server that is
  **already running**. It neither starts nor stops it, and the server may be started
  after personacore.

**personacore does not start an upstream unless you ask**, because loading a model
occupies the GPU. You ask with `--upstream <name>` at launch, by switching profiles in
the web UI (`POST /admin/upstream/profile`), or with the start button of a side that is
waiting to be started (`not_started`). Launched without the flag, personacore loads the
profile you selected last time but does not start it
([docs/adr/0055-do-not-start-the-remembered-upstream-at-launch.md](docs/adr/0055-do-not-start-the-remembered-upstream-at-launch.md)).

Without a profile, requests go to LM Studio's default address, which is set in
`src/llm/lmstudio.rs` as `DEFAULT_ENDPOINT`. Set `PERSONACORE_LLM_ENDPOINT` to point
elsewhere (a profile takes precedence over it). `PERSONACORE_SUB_LLM_ENDPOINT` sends
only the background work to another upstream; when it is not set, the background work
goes to the chat upstream.

**A key for an upstream is stored in plain text in `upstreams.json`.** Remove each
side's `api_key` (and `allow_key_over_plain_http`) before giving the file to someone.

How to write profiles is in [scripts/README-start.md](scripts/README-start.md)
(Japanese). What personacore needs from an upstream is listed in
[docs/adr/0038-what-personacore-requires-of-an-upstream.md](docs/adr/0038-what-personacore-requires-of-an-upstream.md).

## Using the API

The HTTP API is the product; the bundled web UI is just its first client
([docs/adr/0047-the-api-is-the-product.md](docs/adr/0047-the-api-is-the-product.md)).
**The API guide, in English, is [docs/api/README.md](docs/api/README.md)**: running
without the UI, the `model` field, using personacore from an existing chat client, the
admin routes, errors and versioning. A running server also serves its OpenAPI document
at `GET /openapi.json`.

## Documentation

Apart from this file, the API guide, and the English section of the AI collaboration
disclosure, **the documentation is written in Japanese.**

- [docs/README.md](docs/README.md) — the index of the design documents.
- [docs/adr/](docs/adr/README.md) — the records of design decisions and their reasons.
- [docs/process/](docs/process/) — the record of how personacore was built in
  collaboration with AI: the roles, the lessons from running them, and the hand-offs.
  You do not need it to use personacore.
- [NOTICE-AI-COLLABORATION.md](NOTICE-AI-COLLABORATION.md) — the AI collaboration
  disclosure: who decided what, who wrote what, and which models were used. In Japanese
  and English.
- [CLAUDE.md](CLAUDE.md) — read its absolute constraints (絶対制約) before changing code.

<a id="untraceable-references"></a>
**References you cannot follow from here.** The documents, code comments and records
mention issues in the form `origin#N`, commits by their hash (about seven hexadecimal
digits), and the v0.1.x releases. All of them point to the private original repository
(origin) that this repository was published from, and none of them can be followed from
this public repository: its issues are numbered from 1 again, and its history starts from
a single commit ([ADR-0048](docs/adr/0048-open-source-policy.md) §2, in Japanese).

## Contributing

How pull requests work, how to write commit messages, and how to record AI involvement
are in [CONTRIBUTING.md](CONTRIBUTING.md). Please report vulnerabilities privately as
described in [SECURITY.md](SECURITY.md), not in public issues.

## License

`MIT OR Apache-2.0 OR LicenseRef-ACCRL-0.3`

personacore is available under your choice of the MIT License (LICENSE-MIT),
the Apache License, Version 2.0 (LICENSE-APACHE), or the AI Collective Creativity
Respect License (ACCRL) v0.3-PROTOTYPE (LICENSE-ACCRL.md).

- MIT License ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- AI Collective Creativity Respect License (ACCRL) v0.3-PROTOTYPE
  ([LICENSE-ACCRL.md](LICENSE-ACCRL.md); English translation:
  [LICENSE-ACCRL.en.md](LICENSE-ACCRL.en.md))

If you choose MIT or Apache-2.0, you can include personacore in your own software under
an OSI-approved license. The adoption statement and pledge for the ACCRL are in
[LICENSE](LICENSE); the text of the pledge is in Japanese.
