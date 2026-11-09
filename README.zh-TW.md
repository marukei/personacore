# personacore

日本語：[README.md](README.md) · English：[README.en.md](README.en.md)

personacore 是一個小型伺服器，為本機 LLM 提供**角色與記憶**。

它位於聊天前端與你自行執行的推論伺服器之間。每一輪對話，它會依據角色的人設、個性數值、記憶與近期歷史紀錄組出提示詞，送往上游，將回覆拆分為對白與敘述，並在事後於背景從對話中擷取新的記憶。角色、對話、記憶與個性數值都儲存在 SQLite 中。隨著對話進行，這些數值也可以緩慢地漂移；漂移功能仍屬實驗性質，預設為關閉（`PERSONACORE_STAT_DRIFT`；請參閱 API 指南）。
**兩側都使用與 OpenAI 相容的 API**，因此前端可以是內建的網頁介面，也可以是現有的聊天用戶端。

```
前端（內建的網頁介面 / 現有的 OpenAI 相容用戶端）
    ↓ POST /v1/chat/completions
personacore (Rust / axum)  ←→  SQLite
    ↓ POST /v1/chat/completions
上游推論伺服器
```

## 適用對象

在自己的電腦上執行語言模型，並希望透過內建網頁介面、現有的聊天用戶端或自己的程式，與具備人設並記得對話內容的角色交談的人。

## 它不是什麼

- **不是推論引擎。** 推論交由上游伺服器處理。
- **不是公開伺服器。** 它只監聽 `127.0.0.1`，且沒有身分驗證。請勿將它開放給其他機器存取。
- **尚未完成。** 已能運作的部分見 GitHub Releases；尚未完成的部分見 GitHub Issues（順序與理由見 [docs/06-roadmap.md](docs/06-roadmap.md)）。
- **尚未支援多語言。** personacore 所產生的提示詞、網頁介面與錯誤訊息皆為日文。回覆是以日文的直角引號（「」）拆分為對白與敘述。請參閱 API 指南中的[已知限制](docs/api/README.md#known-limitations)。

## 執行環境

- **Windows。** 目前只在搭載 PowerShell 7 的 Windows 11 上執行過；尚未在其他作業系統或其他機器上嘗試
  （[docs/adr/0036-the-range-of-machines-is-a-constraint.md](docs/adr/0036-the-range-of-machines-is-a-constraint.md)）。
- **一個上游推論伺服器**，有兩種方式（見下文）：由 personacore 為你啟動 `llama serve`（llama.cpp），或連線到已在執行中的 OpenAI 相容伺服器（LM Studio、位於其他伺服器前方的代理伺服器等）。
- **自備的模型**（供 llama.cpp 使用的 gguf 檔案）。personacore 不附帶模型。

## 快速開始

從 GitHub Releases 下載 Windows 的 zip 檔（`personacore-<version>-windows-x64.zip`），並解壓縮到任意位置。其中包含 `personacore.exe`、以日文撰寫的 `README.md`、第三方聲明以及授權文件。執行它**不需要 Rust 與 pnpm**；網頁介面已內建於執行檔中。

簡要步驟如下（zip 內的 `README.md` 有完整的日文說明；其原始檔為 [scripts/README-dist.md](scripts/README-dist.md)）：

1. 從 llama.cpp 的官方發行管道安裝 llama.cpp。
2. 下載 gguf 格式的模型（例如從 Hugging Face 下載）。
3. 在 PowerShell 中啟動 personacore：

   ```powershell
   .\personacore.exe
   ```

   它會以預設瀏覽器開啟網頁介面。如果沒有開啟，請自行開啟 `url=` 之後所顯示的 URL。按 **Ctrl+C** 即可停止；這也會一併停止它所啟動的 llama.cpp。
4. 在網頁介面中開啟上游畫面（上流モデル），將你的 gguf 檔案路徑寫入某個設定檔（profile），儲存後切換到該設定檔。

執行該執行檔時，資料會存放在 `%LOCALAPPDATA%\personacore\` 底下：SQLite 資料庫、上游設定檔（`upstreams.json`），以及 personacore 所啟動的 llama.cpp 處理程序的日誌。（下文的 `scripts/start.ps1` 則預設改為將資料庫與日誌存放在儲存庫內的 `data/` 底下；請參閱 [scripts/README-start.md](scripts/README-start.md)。）`PERSONACORE_DB_PATH` 可移動資料庫位置。連接埠在啟動時依下列順序決定：`PERSONACORE_PORT`、在網頁介面的設定畫面（設定）中儲存的連接埠、預設值。儲存的連接埠從下次啟動起生效。
預設連接埠在 `src/server/config.rs` 中以 `DEFAULT_PORT` 設定，啟動日誌也會印出位址。

其他旗標：`--no-ui`（僅提供 API：不提供網頁介面，也不開啟瀏覽器）、`--no-browser`、`--upstream <name>`（啟動時啟動已儲存的設定檔）、`--fresh`（將目前的資料庫移到一旁而不刪除，並以全新的空資料庫啟動）、`--version`。

**從原始碼執行**請使用 `pwsh ./scripts/start.ps1`。它會建置網頁介面、檢查上游、準備資料庫並啟動伺服器。這需要 [rust-toolchain.toml](rust-toolchain.toml) 中指定的 Rust 工具鏈、pnpm 與 PowerShell 7。其選項請見 [scripts/README-start.md](scripts/README-start.md)（日文）。

## 上游推論伺服器

上游設定檔有兩側：聊天，以及背景作業（記憶擷取與漂移）。每一側各屬於兩種類型之一
（[docs/adr/0042-the-executable-owns-the-upstream-it-starts.md](docs/adr/0042-the-executable-owns-the-upstream-it-starts.md)）：

- **指定了 gguf 檔案的一側** — personacore 會為它啟動 `llama serve`，並在關閉時，只停止由它自己啟動的處理程序。
- **未指定 gguf 檔案的一側** — personacore 只會連線到**已在執行中**的伺服器。它既不啟動也不停止該伺服器，且該伺服器可以在 personacore 之後才啟動。

**除非你要求，否則 personacore 不會啟動上游**，因為載入模型會佔用 GPU。你可以用以下方式提出要求：啟動時使用 `--upstream <name>`、在網頁介面中切換設定檔（`POST /admin/upstream/profile`），或按下等待啟動（`not_started`）的那一側的啟動按鈕。若啟動時未加旗標，personacore 會載入你上次選取的設定檔，但不會啟動它
（[docs/adr/0055-do-not-start-the-remembered-upstream-at-launch.md](docs/adr/0055-do-not-start-the-remembered-upstream-at-launch.md)）。

沒有設定檔時，請求會送往 LM Studio 的預設位址，該位址在 `src/llm/lmstudio.rs` 中以 `DEFAULT_ENDPOINT` 設定。若要指向其他位置，請設定 `PERSONACORE_LLM_ENDPOINT`（設定檔的優先順序高於它）。`PERSONACORE_SUB_LLM_ENDPOINT` 只會將背景作業送往另一個上游；未設定時，背景作業會送往聊天所用的上游。

**上游的金鑰會以純文字形式儲存在 `upstreams.json` 中。** 將檔案交給他人之前，請移除每一側的 `api_key`（以及 `allow_key_over_plain_http`）。

設定檔的撰寫方式請見 [scripts/README-start.md](scripts/README-start.md)（日文）。personacore 對上游的需求列於
[docs/adr/0038-what-personacore-requires-of-an-upstream.md](docs/adr/0038-what-personacore-requires-of-an-upstream.md)。

## 使用 API

HTTP API 才是產品本身；內建的網頁介面只是它的第一個用戶端
（[docs/adr/0047-the-api-is-the-product.md](docs/adr/0047-the-api-is-the-product.md)）。
**API 指南（英文）為 [docs/api/README.md](docs/api/README.md)**：不使用介面的執行方式、`model` 欄位、從現有聊天用戶端使用 personacore、管理路由、錯誤，以及版本編號與相容性。執行中的伺服器也會在 `GET /openapi.json` 提供其 OpenAPI 文件。

## 文件

除了本檔案、API 指南，以及 AI 協作揭露文件的英文部分之外，**文件皆以日文撰寫。**

- [docs/README.md](docs/README.md) — 設計文件的索引。
- [docs/adr/](docs/adr/README.md) — 設計決策及其理由的紀錄。
- [docs/process/](docs/process/) — 與 AI 協作打造 personacore 的過程紀錄：各角色、運作上的經驗教訓，以及交接內容。使用 personacore 並不需要它。
- [NOTICE-AI-COLLABORATION.md](NOTICE-AI-COLLABORATION.md) — AI 協作揭露文件：誰決定了什麼、誰撰寫了什麼，以及使用了哪些模型。提供日文與英文。
- [CLAUDE.md](CLAUDE.md) — 修改程式碼之前，請先閱讀其中的絕對限制（絶対制約）。

<a id="untraceable-references"></a>
**無法從這裡追溯的參照。** 文件、程式碼註解與紀錄中，提到了 `origin#N` 形式的 issue、以雜湊值（約七位十六進位數字）標示的 commit，以及 v0.1.x 版本。這些全都指向本儲存庫據以公開的私有原始儲存庫（origin），無法從這個公開儲存庫追溯：此公開儲存庫的 issue 重新從 1 開始編號，其歷史則從單一 commit 開始（[ADR-0048](docs/adr/0048-open-source-policy.md) §2，日文）。

## 參與貢獻

pull request 的運作方式、commit 訊息的撰寫方式，以及如何記錄 AI 的參與，請見 [CONTRIBUTING.md](CONTRIBUTING.md)。若要回報安全漏洞，請依照 [SECURITY.md](SECURITY.md) 所述以私下方式回報，不要在公開的 issue 中提出。

## 授權

`MIT OR Apache-2.0 OR LicenseRef-ACCRL-0.3`

下列段落照錄 LICENSE 的英文原文，不另行翻譯，以免與作者簽署的文字產生出入：

personacore is available under your choice of the MIT License (LICENSE-MIT),
the Apache License, Version 2.0 (LICENSE-APACHE), or the AI Collective Creativity
Respect License (ACCRL) v0.3-PROTOTYPE (LICENSE-ACCRL.md).

- MIT License（[LICENSE-MIT](LICENSE-MIT)）
- Apache License, Version 2.0（[LICENSE-APACHE](LICENSE-APACHE)）
- AI Collective Creativity Respect License (ACCRL) v0.3-PROTOTYPE
  （[LICENSE-ACCRL.md](LICENSE-ACCRL.md)；英文譯本：
  [LICENSE-ACCRL.en.md](LICENSE-ACCRL.en.md)）

若選擇 MIT 或 Apache-2.0，你可以在 OSI 核可的授權之下，將 personacore 納入你自己的軟體中。ACCRL 的採用聲明與誓約見 [LICENSE](LICENSE)；誓約的內文為日文。
