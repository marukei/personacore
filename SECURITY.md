# セキュリティの報告 / Security policy

日本語の後に英語の版があります。 / English follows the Japanese.

## 報告のしかた

**脆弱性は、公開の Issue に書かないでください。**
GitHub の非公開の報告（Private vulnerability reporting）で知らせてください。
<https://github.com/marukei/personacore/security/advisories/new>

報告には次のことを書いてもらえると助かります。

- 影響を受ける版（リリースの版、またはコミット）と OS
- 再現の手順（要求の中身、上流プロファイルの書き方など）
- 何が起きるか（何が読める・書ける・実行できるようになるか）

personacore は個人が趣味で開発しています。返事と修正の早さは約束できませんが、届いた報告には必ず目を通します。
修正の版を出すまでは、内容を公開しないでください。

## 対象の範囲

対象はこのリポジトリのコード（`src/`・`web/`・`scripts/`）の `main` の最新と、[Releases](https://github.com/marukei/personacore/releases) で配っている最新の版です。古い版は直しません。

personacore は同じ機の中のクライアントのためのサーバです。次のことを前提にしています。

- サーバは `127.0.0.1` にだけ待ち受け、認証を持ちません。同じ機で動くプログラムは、どれも API に届きます（これは設計どおりです）。
- 要求の `Host` / `Origin` をすべての経路の手前で検査します（ブラウザで開いた外のページからの DNS rebinding を止めるため）。
- 管理の API には上流の推論サーバを起動するコマンドを保存し、実行する口があります。
- 上流への鍵は上流プロファイル（`upstreams.json`）に平文で書きます。値が出るのはその上流への送信のヘッダだけです。API の応答と画面には末尾4文字だけを見せ（8文字未満の鍵では見せません）、ログには載せません。
- 推論サーバ（LLM）の出力は信頼できない入力として扱います。

特に、次のような報告を歓迎します。

- ブラウザで開いた外のページから、管理の API（とくに起動するコマンドの保存と実行）に届くこと
- `Host` / `Origin` の検査を通り抜けられること
- 上流への鍵が、ログ・応答・画面・別の宛先へ漏れること
- LLM の出力や要求の値が、SQL・シェル・ファイルのパスとして解釈されること
- エラーの応答に、内部のパス・接続文字列・スタックトレースが載ること

次のものは対象外です。

- personacore を `127.0.0.1` 以外で待ち受けさせたり、プロキシなどで外部に公開したりした構成（サポートしていません）
- 上流の推論サーバ（llama.cpp など）やモデルそのものの脆弱性（それぞれの開発元へ報告してください）
- モデルが不適切な文を生成すること（安全性のフィルタは personacore の役目ではありません）

---

## Reporting a vulnerability

**Please do not report vulnerabilities in public issues.**
Use GitHub's private vulnerability reporting instead:
<https://github.com/marukei/personacore/security/advisories/new>

It helps if your report includes:

- the affected version (release or commit) and your OS
- steps to reproduce (the request, how the upstream profile is written, and so on)
- what happens (what becomes readable, writable or executable)

personacore is a hobby project maintained by one person. I cannot promise how quickly I will reply or ship a fix, but I will read every report.
Please keep the details private until a fixed version is released.

## Scope

In scope are the latest code on `main` in this repository (`src/`, `web/`, `scripts/`) and the latest version distributed on [Releases](https://github.com/marukei/personacore/releases). Older versions will not be fixed.

personacore is a server for clients on the same machine. It assumes the following:

- The server listens only on `127.0.0.1` and has no authentication. Any program running on the same machine can reach the API (this is by design).
- It checks the `Host` and `Origin` of every request before any route runs (to stop DNS rebinding from pages opened in a browser).
- The admin API can save, and run, the command that starts an upstream inference server.
- Keys for upstreams are written in plain text in the upstream profile (`upstreams.json`). The value appears only in the header of requests to that upstream. API responses and the screens show only the last four characters (nothing for keys shorter than eight characters), and logs never include it.
- Output from the inference server (the LLM) is treated as untrusted input.

Reports like these are especially welcome:

- a page opened in a browser reaching the admin API (in particular, saving or running the launch command)
- getting past the `Host` / `Origin` check
- a key for an upstream leaking into logs, responses, the screens or another destination
- LLM output or request values being interpreted as SQL, shell commands or file paths
- error responses that include internal paths, connection strings or stack traces

Out of scope:

- setups that make personacore listen on anything other than `127.0.0.1`, or expose it through a proxy or similar (not supported)
- vulnerabilities in upstream inference servers (such as llama.cpp) or in the models themselves (please report them to their developers)
- a model generating inappropriate text (content filtering is not personacore's job)
