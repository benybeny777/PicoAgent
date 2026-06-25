# PicoAgent 仕様書

対象: PicoAgent（コネクタ駆動デスクトップペット / Tauri）
最終更新: 2026-06-25

---

## 概要

PicoAgent は透明な **Tauri** ウィンドウに常駐する、キャラクターとフキダシだけのデスクトップペット。
バックエンドは Rust（`src-tauri/`）、フロントは webview（`ui/`）。
「叩く AI（LLM バックエンド）」と「エージェント機能（ツール）」をコネクタとして拡張できる。

- **叩く AI** → プロバイダコネクタ（`src-tauri/src/providers/`）。**既定は `local`（同梱 llama-server、Ollama 不要）**、Ollama / OpenAI 互換も同梱。
- **エージェント機能** → スキル（ツール）コネクタ（連結ファイルはプロジェクト直下の `skills/`、トレイトは `src-tauri/src/tools/mod.rs`）。モデルが function calling で呼び出す。
- **キャラクター** → 正本はプロジェクト直下の `characters/index.json` と各 `character.json`。`sync-characters.mjs` が `ui/assets/characters/` へ同期し webview が配信。見た目・アニメーション・人格を一体で切り替える。
- **両者をつなぐ中核** → エージェントループ（`src-tauri/src/agent.rs`）。

> [!NOTE]
> 起動・セットアップ・設定キーは **[README.md](README.md)**、作業ルールは **[AGENTS.md](AGENTS.md)** を参照。本書は仕様の正本。

---

## 📑 目次

- [システム構成](#システム構成)
- [メッセージ正規形](#メッセージ正規形コネクタ間の共通言語)
- [エージェントループ](#エージェントループ-agentrs)
- [プロバイダコネクタ](#プロバイダコネクタ-srcproviders)
- [スキル（ツール）コネクタ](#スキルツールコネクタ)
- [新しいコネクタの作り方](#新しいコネクタの作り方)
- [設定（config と設定画面）](#設定config-と設定画面)
- [設定キー一覧](#設定キー一覧)
- [フロント連携と製品化](#フロント連携と製品化)

---

## システム構成

```mermaid
graph TD
    U[ユーザー入力<br/>フキダシ] --> R[ui/renderer.js]
    R -->|invoke pet_send| CMD[commands.rs]
    S[設定ウィンドウ<br/>ui/settings.js] -->|invoke settings_save| CMD
    CMD <-->|cfg.get / set_all| C[(config.json<br/>設定の正本)]
    CMD --> A[agent.rs<br/>エージェントループ]
    A -->|chat| P[プロバイダコネクタ<br/>providers/*]
    P -->|reqwest| LLM[(叩く AI<br/>Ollama / OpenAI互換)]
    LLM -->|tool_calls| A
    A -->|dispatch| T[スキルコネクタ<br/>skills/*]
    T -->|結果| A
    A -->|最終応答| CMD -->|return| R
```

- コネクタは **`inventory` でコンパイル時自動登録**。`build.rs` が `src/providers/*.rs`・ルートの `skills/*.rs` を走査して
  `#[path] pub mod` を生成するため、**`.rs` を1枚置くだけ**でコネクタ追加（`mod.rs` 編集不要）。
- `AppState`（`commands.rs`）が選択中プロバイダ・モデル・会話履歴を保持。プロバイダとモデルは設定画面から切替。
- 設定の正本は `config.rs`（`app_config_dir/config.json`）。コネクタは `cfg.get()` で都度読むため、設定変更は再起動なしに反映。

---

## メッセージ正規形（コネクタ間の共通言語）

`agent.rs` はプロバイダ固有のワイヤ形式を一切知らない。各プロバイダが自分の API 形式へ相互変換する（`message.rs`）。

| role | 形（serde） |
|---|---|
| system / user | `{ role, content }` |
| assistant | `{ role:"assistant", content, tool_calls?: [{ id, name, arguments(Value) }] }` |
| tool（結果） | `{ role:"tool", tool_call_id, name, content }` |

- `arguments` は **`serde_json::Value`（オブジェクト）** で持つ（OpenAI の JSON 文字列はプロバイダ内で相互変換）。
- Ollama は tool 結果を id で紐付けず位置・名前で扱うため `tool_call_id` は無視（正規形には残す）。

---

## エージェントループ（`agent.rs`）

`run(cfg, provider, model, messages, max_steps = 5) -> Result<String>`

1. `provider.chat()` を呼び、返った assistant を `messages` に積む。
2. `tool_calls` が無ければ、その `content` を最終応答として返す。
3. `tool_calls` があれば、各呼び出しを `tools::dispatch(cfg, name, arguments)` で実行し、結果を `tool` メッセージとして積み 1 に戻る。
4. `max_steps` を超えたら打ち切り、最後の文面を返す（暴走防止）。

すべてのユーザー入力で、ローカルモデルの `tool_calls` 対応状況に依存させず、最初の `chat()` より前に
`web_search` を必ず実行する。検索結果は外部の信頼できない参考データとして区切ってユーザーメッセージへ
付加し、結果内の指示には従わないよう明記する。検索失敗もモデルへ渡し、ユーザーへ明示させる。
参照元はMarkdownハイパーリンクで回答させ、フキダシはMarkdownリンクと通常のURLを安全なDOMリンクへ変換する。

`messages` は `&mut Vec<Message>` を破壊的に伸ばす。

---

## プロバイダコネクタ（`src-tauri/src/providers/`）

`Provider` トレイトを実装し、`inventory::submit!` で登録する。

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    fn config_schema(&self) -> Vec<Field> { Vec::new() }
    fn default_model(&self, cfg: &Config) -> String;
    async fn list_models(&self, cfg: &Config) -> anyhow::Result<Vec<String>>;
    async fn is_up(&self, cfg: &Config) -> bool;  // 接続不可・キー無は false に正規化してよい
    async fn chat(&self, cfg: &Config, model: &str, messages: &[Message], tools: &[Value]) -> anyhow::Result<Message>;
}
inventory::submit! { ProviderReg { make: || Box::new(MyProvider) } }
```

> 接続先・APIキー・モデルは各メソッド内で `cfg.get()` から都度読む（フィールドに固定しない）。設定変更を再起動なしに反映するため。

### 同梱プロバイダ

| id | 叩く先 | 接続条件 |
|---|---|---|
| `local`（既定） | 同梱 llama-server `/v1/chat/completions`（OpenAI 互換） | エンジン同梱＋モデル取得済（`engine.rs` が管理）。Ollama 不要 |
| `ollama` | ローカル Ollama `/api/chat` | `ollama serve` 起動 |
| `openai` | OpenAI 互換 `/chat/completions` | `OPENAI_API_KEY` 設定（未設定なら `is_up()=false`） |

`openai` は `OPENAI_BASE_URL` を変えれば OpenAI 本体に限らず OpenAI 形式の任意ゲートウェイに接続できる。
`local` は `openai.rs` の `to_wire`/`from_wire` を再利用し、tools 非対応テンプレートのモデルでも会話が成立するよう tools 付きで失敗したら tools 無しで再試行する。

### 同梱推論エンジン（`engine.rs`）

`local` プロバイダのために、Ollama の代わりに llama.cpp の `llama-server` をアプリに同梱して使う。

- エンジン本体（`llama-server` + ggml DLL）はリポジトリに含めず、`scripts/fetch-engine.mjs`（`npm run setup:engine`）で
  `src-tauri/resources/engine/` に取得。`tauri.conf.json` の `bundle.resources` で配布物に同梱する。
- 起動時（既定が `local` のとき）`engine::ensure_and_start` を非同期実行:
  1. モデル GGUF（既定: Gemma 3n E2B Q4_K_M）が `app_data_dir/models/` に無ければ `CODA_MODEL_URL` から
     ストリーミング DL（進捗を `engine:status {phase:"download", percent}` で UI へ）。
  2. `llama-server -m <model> --host 127.0.0.1 --port <CODA_LOCAL_PORT> -c <CODA_LOCAL_CTX> --jinja` を spawn
     （Windows は CREATE_NO_WINDOW）。`/health` が 200 になったら `engine:status {phase:"ready"}`。
- 子プロセスは `RunEvent::Exit` で停止（`engine::stop`）。状態は `engine_status` command でも取得可能。

---

## スキル（ツール）コネクタ

連結ファイルはプロジェクト直下の `skills/`、`Tool` トレイト・レジストリ本体は `src-tauri/src/tools/mod.rs`。

`Tool` トレイトを実装し、`inventory::submit!` で登録する。

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> String;
    fn parameters(&self) -> Value;            // JSON Schema
    fn config_schema(&self) -> Vec<Field> { Vec::new() }
    async fn run(&self, cfg: &Config, args: Value) -> String;  // 失敗もエラー文字列で返す
}
inventory::submit! { ToolReg { make: || Box::new(MyTool) } }
```

- `tools::build_schemas()` が `{ type:"function", function:{name,description,parameters} }` 形式の配列を生成しプロバイダへ渡す。
- `tools::dispatch(cfg, name, args)` が実行を仲介する。未登録ツールもエラー文字列で返す（エージェントが回復できるように）。

### 同梱ツール

| name | 機能 | 安全策 |
|---|---|---|
| `get_time` | 現在の日付・時刻（任意で IANA タイムゾーン、`chrono-tz`） | 副作用なし |
| `read_file` | ローカルのテキストファイル読込 | 許可ルート `CODA_TOOLS_ROOT`（既定はホーム）配下に限定、`..` 等のルート外を拒否、64KB で打ち切り |
| `web_search` | Web検索結果のタイトル・URL・概要を返す。すべての回答前に必ず実行 | 検索語500文字、1〜8件、待ち時間3〜60秒、応答32KBで制限 |

---

## 新しいコネクタの作り方

### 叩く AI を増やす（例: Anthropic Claude）

1. `src-tauri/src/providers/anthropic.rs` を作り、`Provider` を実装して `inventory::submit!` する。`chat()` 内で正規形 ⇄ Anthropic Messages API 形式を変換。
2. 再ビルドすると設定画面のプロバイダ一覧に自動で現れる（`build.rs` が `pub mod` を生成、`inventory` が登録）。
3. `SPEC.md` の同梱プロバイダ表と、必要な設定キーを `README.md` に追記。

### エージェント機能を増やす（例: Web 検索）

1. `skills/web_search.rs` を作り、`Tool` を実装して `inventory::submit!` する。副作用があるなら境界（タイムアウト・件数上限）を設ける。
2. 再ビルドすると、ツールスキーマが自動でモデルへ渡る。
3. `SPEC.md` の同梱ツール表を更新。

どちらも **`mod.rs` の編集は不要**。フォルダに `.rs` を置くだけで検出される。設定値が要るなら `config_schema()` で申告すれば設定画面に項目が並ぶ。

---

## 設定（config と設定画面）

### 設定の正本（`config.rs`）

- 値の優先順位: **永続ファイル `app_config_dir/config.json` > 環境変数 > ハードコード既定値**。
- API: `load(path)` / `get(key, fallback)` / `get_bool(key, fallback)` / `get_all()` / `set_all(patch)`。
- 空文字は「未設定」とみなし env → 既定値へフォールバックする。

### 設定画面（右クリック →「設定…」）

- `ui/settings.{html,css,js}` の独立 Tauri ウィンドウ。`ui/coda.js` の `window.coda.settings.*` →
  `commands.rs` の `settings_load` / `settings_list_models` / `settings_save` command で構成。
- 画面は **基本設定** ＋ **導入済みスキル（エージェント機能）一覧** ＋ **導入済みキャラクター一覧** ＋ **各コネクタの `config_schema()`** を動的に描画する。コネクタを足すと一覧も設定項目も自動で増える。
- スキル一覧は `settings_load` が返す各ツールの `name` / `label` / `description` を読み取り専用カードで表示する（設定項目を持たないツールもここに並ぶ）。キャラクター一覧は `assets/characters/index.json` から表示し、使用中のキャラに「（使用中）」を付ける。
- `Field` の項目: `{ key, label, type:"text"|"password"|"textarea", placeholder }`。
- 保存は `config.set_all()` で永続化し、実行中の状態（プロバイダ・モデル・システムプロンプト・常に手前）へ即反映する。コネクタ固有設定は `cfg.get()` 都度読みのため自動反映。

---

## 設定キー一覧

| キー | 既定 | 区分 | 用途 |
|---|---|---|---|
| `CODA_PROVIDER` | `local` | 基本 | 選択中の叩く AI の id |
| `CODA_MODEL` | プロバイダ既定 | 基本 | 選択中モデル |
| `CODA_SYSTEM_PROMPT` | 既定プロンプト | 基本 | system メッセージ |
| `CODA_BUBBLE_FONT_PX` | `13` | 基本 | フキダシの文字サイズ(px)。`pet:ui` で即反映 |
| `CODA_BUBBLE_WIDTH_PX` | 自動 | 基本 | フキダシ右下のドラッグで保存する幅(px)。`pet:ui` で即反映 |
| `CODA_BUBBLE_HEIGHT_PX` | 自動 | 基本 | フキダシ右下のドラッグで保存する高さ(px)。`pet:ui` で即反映 |
| `CODA_ALWAYS_ON_TOP` | `true` | 基本 | 常に手前に表示 |
| `CODA_CHARACTER` | `mugi` | 基本 | 選択中キャラクターのID |
| `CODA_CHARACTER_OVERRIDES` | `{}` | 基本 | キャラクターIDごとの人格設定上書き |
| `CODA_MODEL_URL` | gemma-3n-E2B Q4_K_M GGUF | local | 同梱エンジンが取得するモデル URL |
| `CODA_LOCAL_PORT` | `11435` | local | 同梱エンジンのポート |
| `CODA_LOCAL_CTX` | `4096` | local | コンテキスト長 |
| `OLLAMA_HOST` | `http://127.0.0.1:11434` | ollama | Ollama 接続先 |
| `OLLAMA_MODEL` | `qwen3:8b` | ollama | Ollama 既定モデル |
| `OPENAI_BASE_URL` | `https://api.openai.com/v1` | openai | OpenAI 互換エンドポイント |
| `OPENAI_API_KEY` | （なし） | openai | OpenAI 互換の API キー。未設定なら選択不可 |
| `OPENAI_MODEL` | `gpt-4o-mini` | openai | OpenAI 互換の既定モデル |
| `CODA_TOOLS_ROOT` | ユーザーホーム | read_file | `read_file` が読み取れるルート |
| `CODA_WEB_SEARCH_MAX_RESULTS` | `5` | web_search | Web検索で返す件数（1〜8） |
| `CODA_WEB_SEARCH_TIMEOUT_SECS` | `15` | web_search | Web検索の待ち時間（秒、3〜60） |

各キーは環境変数としても渡せる（既定値としての参照のみ。保存先は常に `config.json`）。

---

## フロント連携と製品化

### キャラクターパック

- `characters/index.json` に `{ id, name, manifest }` を追加し、各フォルダへ `character.json` と連番フレームを置く（`ui/assets/characters/` へは `sync-characters.mjs` が同期）。
- `character.json` は `name`、`description`、`greeting`、`persona`、`animations` を持つ。人格には性格・口調・一人称・ユーザーの呼び方・追加指示を定義できる。
- 各アニメーションは `path`、`frames`、`fps`、`motion` を持つ。フロントは画像フレームとCSS補間動作を組み合わせる。
- 選択中キャラの人格は `pet_set_character_profile` でシステムプロンプトへ追加する。設定画面のキャラ別上書きは `CODA_CHARACTER_OVERRIDES` に保存する。

- フロント（`ui/`）は Vite 等のバンドラを使わず、Tauri の静的 `frontendDist` で配信。`withGlobalTauri:true` で
  `window.__TAURI__` を有効化し、`ui/coda.js` が旧 `window.coda`（init/send/menu/on/settings.*）を再現する。
  これにより `renderer.js` / `settings.js` は Electron 版とほぼ同じコードのまま動く。
- フキダシ右下のリサイズつまみは Pointer Events で幅・高さを変更し、確定時に `pet_set_bubble_size` で設定へ永続化する。
- キャラクターのダブルクリックはウィンドウをフルスクリーン化（`getCurrentWindow().setFullscreen`）し、その中でフキダシを大きく広げる（`.bubble.fullscreen`）。右側にキャラ分の余白を残して口元からしっぽが伸びる吹き出しの形を保ち、キャラ全身と送信ボタンに被らないようにする。解除はキャラ／フキダシのダブルクリックまたは `Esc` でフルスクリーンを解除する。ドラッグ領域のダブルクリックによるネイティブのウィンドウ最大化は `maximizable:false` で無効化している。
- ウィンドウ: 透明・枠なし・常に手前・タスクバー非表示・最大化不可（`tauri.conf.json`）。ドラッグは `data-tauri-drag-region`。
- 権限は `src-tauri/capabilities/default.json`（start-dragging / set-always-on-top / set-fullscreen / event / webview 生成）。
- 製品化: `productName=PicoAgent` / `identifier=com.coda.app`（既存ユーザーのデータ互換のため識別子は据え置き）。Windows バンドルは NSIS（`bundle.targets`）。
  アイコンは `src-tauri/icons/`（`npx tauri icon src-tauri/icon-source.png` で再生成）。
  署名（Authenticode）・自動更新（`tauri-plugin-updater`）はリリース時に鍵・証明書を用意して設定する（現状未設定）。
