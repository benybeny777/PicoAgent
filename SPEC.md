# PicoAgent 仕様書

対象: PicoAgent（コネクタ駆動デスクトップペット / Tauri）
最終更新: 2026-06-25

---

## 概要

PicoAgent は透明な **Tauri** ウィンドウに常駐する、キャラクターとフキダシだけのデスクトップペット。
バックエンドは Rust（`src-tauri/`）、フロントは webview（`ui/`）。
「叩く AI（LLM バックエンド）」と「エージェント機能（ツール）」をコネクタとして拡張できる。

- **叩く AI** → プロバイダコネクタ（`src-tauri/src/providers/`）。**既定は `local`（同梱 llama-server、Ollama 不要）**、Ollama / OpenAI 互換も同梱。
- **エージェント機能** → スキル（ツール）コネクタ（連結ファイルはプロジェクト直下の `skills/<スキル名>/mod.rs`、トレイトは `src-tauri/src/tools/mod.rs`）。モデルが function calling で呼び出す。
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
    A -->|dispatch| T[スキルコネクタ<br/>skills/*/mod.rs]
    T -->|結果| A
    A -->|最終応答| CMD -->|return| R
```

- コネクタは **`inventory` でコンパイル時自動登録**。`build.rs` が `src/providers/*.rs`・ルートの `skills/<スキル名>/mod.rs` を走査して
  `#[path] pub mod` を生成するため、**プロバイダは `.rs` を1枚、スキルは `<スキル名>/mod.rs` を1つ置くだけ**でコネクタ追加（中央 `mod.rs` 編集不要）。
- 登録済みプロバイダ/スキルは `OnceLock` でプロセス内キャッシュされ、設定画面表示・ツール dispatch・プロバイダ取得のたびに `Box` を再生成しない。ツールスキーマも初回だけ生成して再利用する。会話時は全スキルを常にLLMへ渡さず、ユーザー発話のキーワードから候補スキーマを絞り込む（`get_time` / `web_search` / `web_fetch` は基本候補）。
- `AppState`（`commands.rs`）が選択中プロバイダ・モデル・会話履歴・直近の自発雑談を保持。プロバイダとモデルは設定画面から切替。
- 設定の正本は `config.rs`（`app_config_dir/config.json`）。コネクタは `cfg.get()` で都度読むため、設定変更は再起動なしに反映。
- 会話履歴は `history.rs` が `app_data_dir/history.json` に永続化する（対話データなので config とは分離）。`pet_send` のたびに「system + クリーンな直近発話（最大 24 件）」へ圧縮して保存し、起動時に末尾 20 件を文脈へ復元する。これにより再起動をまたいで直近の会話を踏まえて話す。途中の tool 呼び出し/結果は順序破綻を避けるため文脈・履歴に残さない。
- 暗黙的な長期メモリは `memory.rs` が `app_data_dir/memory.json` に保存する。ユーザーの自己情報・好み・継続指示らしい文だけを保守的に抽出し、起動時と設定変更時に system 文脈へ短く差し込む。人格やプロンプトの変更（`reset_conversation`）では会話の流れを保持し、右クリックメニュー「会話をリセット」（`clear_conversation_and_history`）で文脈・履歴ファイル・長期メモリ・直近雑談を消して `pet:reset` を通知する。

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

通常の質問では、ローカルモデルの `tool_calls` 対応状況に依存させず、最初の `chat()` より前に
`web_search` を実行する。天気の質問（`is_weather_query`）は `weather` スキルを、為替の質問（`currency_query` が通貨を読み取れたとき）は `currency_rates` スキルを、市場指数の質問（`market_query` が日経平均等を読み取れたとき）は `market_indices` スキルを**先に実行してその整形済み出力（天気アイコン・気温/為替・市場指数のスパークライン等）をそのまま返す**（小型モデルが tool_calls を出せず、かつチャートを再現できないため、モデルを介さず確実にグラフィカル表示する）。スキル直行応答も assistant 発話として履歴へ保存し、前回のユーザー発話だけが残って次の質問へ混ざらないようにする。代表的なリマインダー・集中タイマー・見守り・できたことメモ、クリップボード要約、おでかけ前チェックもスキルを先に実行する。その他のローカル操作系依頼では、明示的なWeb検索依頼がない限り事前Web検索を省き、モデルへスキル利用を優先する指示を追加する。為替は文中の通貨（ドル/円/ユーロ…）と「推移/チャート」「N日」を読み取って `base`/`symbols`/`chart`/`days` を組み立てる。短い挨拶は検索結果に引っ張られて表記解説へ逸れやすいため検索を省き、キャラクター本人として返す指示だけを追加する。検索結果は外部の信頼できない参考データとして区切ってユーザーメッセージへ
付加し、結果内の指示には従わないよう明記する。検索失敗もモデルへ渡し、ユーザーへ明示させる。
参照元はMarkdownハイパーリンクで回答させ、フキダシはMarkdownリンクと通常のURLを安全なDOMリンクへ変換する。さらに、事前検索で得た参照元URLのうち本文に未掲載のものを、最終応答の末尾へ `参考:` として確定的に添える（モデルがリンクを出し損ねても参照元が残る）。URL 行は読み上げ（`speech_text`）で除外されるため音声には乗らない。

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
| `gemini` | Gemini の OpenAI 互換 `/v1beta/openai/chat/completions` | `PICOAGENT_GEMINI_API_KEY` 設定（未設定なら `is_up()=false`） |

`openai` は `OPENAI_BASE_URL` を変えれば OpenAI 本体に限らず OpenAI 形式の任意ゲートウェイに接続できる。
`gemini` は Google の OpenAI 互換エンドポイントを使い、`openai.rs` の `to_wire`/`from_wire` をそのまま再利用する（裏は OpenAI 互換と同じ）。tools 付きで失敗したら tools 無しで一度だけ再試行する。
`local` は `openai.rs` の `to_wire`/`from_wire` を再利用し、tools 非対応テンプレートのモデルでも会話が成立するよう tools 付きで失敗したら tools 無しで再試行する。
Gemma 等のチャットテンプレートは user/assistant の厳密な交互（かつ user 始まり）を要求し、崩れると 400「Conversation roles must alternate」になる。自発雑談（assistant 発話の差し込み）や履歴復元で assistant が連続・先頭になり得るため、`normalize_for_local` で送信直前に正規化する（system は先頭へ集約、先頭の純 assistant は除去、連続する同 role の user/assistant は内容を連結して1つに畳む。tool 呼び出しを含むメッセージはそのまま通す）。

### 同梱推論エンジン（`engine.rs`）

`local` プロバイダのために、Ollama の代わりに llama.cpp の `llama-server` をアプリに同梱して使う。

- エンジン本体（`llama-server` + ggml DLL）はリポジトリに含めず、`scripts/fetch-engine.mjs`（`npm run setup:engine`）で
  `src-tauri/resources/engine/` に取得。`tauri.conf.json` の `bundle.resources` で配布物に同梱する。
- 起動時（既定が `local` のとき）`engine::ensure_and_start` を非同期実行:
  1. `PICOAGENT_LOCAL_MODEL_PRESET` が未設定または `auto` の場合、Windows では `Win32_VideoController.AdapterRAM`、共通でRAM総量を見て Gemma 4 系の `small` / `balanced` / `large` を初回自動選定し、`config.json` へ保存する。`qwen3` は手動選択用の追加プリセット、`custom` の場合だけ `PICOAGENT_MODEL_URL` を直接使う。
  2. 選定モデル GGUF が `app_data_dir/models/` に無ければプリセットURLまたは `PICOAGENT_MODEL_URL` からストリーミング DL（進捗を `engine:status {phase:"download", percent}` で UI へ）。
  3. `llama-server -m <model> --host 127.0.0.1 --port <PICOAGENT_LOCAL_PORT> -c <PICOAGENT_LOCAL_CTX> -ngl <PICOAGENT_LOCAL_NGL> --jinja`
     を spawn（`PICOAGENT_LOCAL_GPU_DEVICE` 指定時は `--device <値>` を追加。Windows は CREATE_NO_WINDOW）。
     既定の同梱ビルドは Vulkan(GPU) 版で、`-ngl 999` により全レイヤを GPU へオフロードする（GPU 非対応環境では自動 CPU フォールバック）。
     `/health` が 200 になったら `engine:status {phase:"ready"}`。
- 子プロセスは `RunEvent::Exit` で停止（`engine::stop`）。状態は `engine_status` command でも取得可能。
- 設定画面の「モデル」欄（`PICOAGENT_MODEL`）でローカルプリセット（small/balanced/large/qwen3）を選ぶと、`local::model_url` がその選択を取得URLへ反映する。`settings_save` は `PICOAGENT_MODEL` / `PICOAGENT_LOCAL_MODEL_PRESET` / `PICOAGENT_MODEL_URL` などローカル関連の変更でエンジンを停止→`ensure_and_start` し、未取得モデルなら自動でダウンロードしてから起動する。
- 同梱 Aivis の状態は `aivis_status` command で取得する（フロントの起動時キャラ描画ゲートに使用）。

### AivisSpeech Engine 連携（`aivis.rs` / `pet_speak`）

`PICOAGENT_TTS_ENABLED` が有効な場合、回答テキストを AivisSpeech Engine の `/audio_query` → `/synthesis` へ渡して WAV を生成し、フロントで再生する。音声再生中のみ会話アニメーションを維持する。起動時・キャラクター変更時に吹き出しへ出る挨拶（`character.json` の `greeting`）も同じ経路で読み上げる。

- 読み上げには同梱 Aivis だけを使い、接続先は内部固定の `http://127.0.0.1:10102` とする。`PICOAGENT_TTS_ENABLED` が有効な場合はアプリ起動時に同梱 AivisSpeech Engine を即 spawn し、設定画面で ON にした場合も保存後すぐ起動する。`pet_speak` 時点で `/version` が未応答なら ready まで待つ。
- 推論デバイスは `PICOAGENT_TTS_DEVICE` で `auto` / `cpu` / `gpu` を選ぶ。`gpu` は `--use_gpu`、`cpu` は `--no-use_gpu` で起動する。`auto` は起動前に GPU 候補を確認し、Windows では NVIDIA / Radeon・Vega・Graphics 表記のある AMD APU/表示アダプタ / Intel 系の表示アダプタを候補として、GPU 候補があれば `--use_gpu`、なければ `--no-use_gpu` で起動する。AMD CPU だけの環境は GPU 候補にしない。CPU/GPU の明示指定は自動判定より優先する。設定画面で ON/OFF やデバイスを変更した場合は、保存後に同梱 Aivis を停止/再起動して即時反映する。
- 初回合成では話者モデルのロードで時間がかかるため、Aivis への HTTP タイムアウトは 180 秒にする。
- 実行時ログは `app_data_dir/log/` にまとめる。Aivis の stdout/stderr は `aivis.log`、同梱 llama-server の stdout/stderr は `engine.log` に追記する。Aivis プロセスが起動直後に終了した場合や ready 待ちがタイムアウトした場合は、`aivis.log` の末尾をエラー文字列に含めてフロントへ返す。旧 `app_log_dir/aivis.log` がある場合は、新しい `log/aivis.log` が未作成のときだけ初回にコピーする。
- 状態は `aivis_status` command（`{ enabled, up }`）で取得できる。TTS 有効時、フロントは Aivis が `up` になるまで**キャラ・フキダシを含む全 UI の描画を保留**する（`.pet-app.is-booting` で `visibility:hidden`）。待機中に挨拶を（非表示のまま）描画しつつ音声を事前合成（`prepareSpeech`）しておき、準備完了で `is-booting` を外して**描画した瞬間に挨拶を再生**する（`playPreparedSpeech`）。起動失敗で固まらないよう約120秒で諦めて描画する。TTS 無効時（`enabled=false`）は待たずに即描画する。
- 実行ファイルは同梱配置 `src-tauri/resources/aivis/` だけを探す。同梱配置は `npm run dev` / `npm run build` の起動前に、未取得なら `scripts/fetch-aivis.mjs` が公式 GitHub Releases から既定 `1.2.0` を取得する（手動実行は `npm run setup:aivis`、更新時は `AIVIS_RELEASE=latest` またはタグを指定）。配布物では `tauri.conf.json` の `bundle.resources` で `resources/aivis/*` を同梱対象にする。
- speaker ID は選択中キャラクターの `PICOAGENT_TTS_CHARACTER_OVERRIDES[characterId].speaker` を優先し、未設定なら共通の `PICOAGENT_TTS_SPEAKER` を使う。
- アプリが起動した子プロセスだけを `RunEvent::Exit` で停止する。既にユーザーが手動起動していた Aivis には触らない。

---

## スキル（ツール）コネクタ

連結ファイルはプロジェクト直下の `skills/<スキル名>/mod.rs`、`Tool` トレイト・レジストリ本体は `src-tauri/src/tools/mod.rs`。スキル専用の補助モジュール、実行スクリプト、テンプレート、少量の静的アセットは同じ `skills/<スキル名>/` 配下へ同梱する。Rust の外部依存クレートは `src-tauri/Cargo.toml` に集約し、スキル配下には依存クレートそのものではなく、そのスキル専用のラッパーやスクリプトを置く。

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
- スキル実体、名前インデックス、function schema は `OnceLock` でキャッシュする。多数のスキルを導入しても会話ごとの schema 構築や dispatch 時の全スキル再生成を避ける。
- `tools::schemas_for_text(text)` がユーザー発話から必要そうな function schema を選び、通常会話で全スキル定義をLLMへ渡さない。`dispatch` は全スキルを呼べるため、これはモデルへ見せる候補の圧縮であり、内部スキルや明示呼び出しの実行能力は維持する。
- `build.rs` は `skills/<スキル名>/mod.rs` を自動登録し、さらに `skills/` 配下を再帰的に `cargo:rerun-if-changed` へ登録する。これにより `skills/chitchat/trends.rs` や `skills/ms_office/scripts/office.ps1` のような補助ファイル変更もビルドに反映される。

### 同梱ツール

| name | 機能 | 安全策 |
|---|---|---|
| `get_time` | 現在の日付・時刻（任意で IANA タイムゾーン、`chrono-tz`） | 副作用なし |
| `weather` | Open-Meteo による現在天気・日別予報。天気アイコン＋気温スパークライン（時間別/日別）でグラフィカルに表示。地名未指定時は IP から現在地を自動推定 | APIキー不要、地名検索、timeout、自動位置は無効化可 |
| `currency_rates` | Frankfurter API による為替レート取得・換算・推移チャート（スパークライン。`chart=true` / `days`） | APIキー不要、timeout |
| `market_indices` | Yahoo Finance chart API による日経平均・TOPIX・NYダウ・NASDAQ・S&P500 の取得と推移チャート（スパークライン） | APIキー不要、timeout、User-Agent 指定 |
| `rss_feed` | RSS/Atom フィードの最新記事取得 | フィードサイズ1MB上限、件数上限、timeout |
| `read_file` | ローカルのテキストファイル読込 | 許可ルート `PICOAGENT_TOOLS_ROOT`（既定はホーム）配下に限定、`..` 等のルート外を拒否、64KB で打ち切り |
| `web_search` | Web検索結果のタイトル・URL・概要を返す。すべての回答前に必ず実行 | 検索語500文字、1〜8件、待ち時間3〜60秒、応答32KBで制限 |
| `chitchat` | アイドル時の自発雑談の指示文を返す内部スキル（`agent_callable=false`、会話中はモデルへ提示しない）。アプリの待機タイマーが `pet_idle_chatter` 経由で利用 | `skills/chitchat/trends.rs` で X API / 急上昇RSS / ニュースRSSの話題メモを取得し、生成は人格込みで `provider.chat`（tools 無し）。会話履歴には積まない |
| `alarm` | 目覚まし時計。声でやさしく起こすアラームを add/list/remove/clear/enable/disable できる。時刻(HH:MM)・曜日・繰り返し・スヌーズ分数を指定 | 一覧は `crate::alarm` 経由で config `PICOAGENT_ALARMS` に保存。発火はフロントの時刻ポーリングが `pet_alarm_wake` を呼ぶ。一度きりは発火後に無効化し、`おきたよ！` が押されるまで指定分数おきに再通知 |
| `reminder` | 日常リマインダー。指定時刻または数分後に声で知らせる | `PICOAGENT_DAILY_EVENTS` に保存。フロントの `pet_daily_events` ポーリングで期限到来分を通知 |
| `focus_timer` | 集中タイマーと作業見守り。終了時または指定間隔ごとに声で知らせる | `PICOAGENT_DAILY_EVENTS` に保存。見守りは repeat_minutes で次回へ繰り返す |
| `praise_log` | できたこと・進捗・がんばったことを保存し、あとで振り返る | `PICOAGENT_PRAISE_LOG` に保存。最大100件 |
| `daily_briefing` | おでかけ前チェック。天気・控えている通知をまとめる | `weather` を内部呼び出し。APIキー不要 |
| `clipboard_text` | クリップボードのテキストを取得し、要約・返信文・TODO化の素材として返す | OS標準コマンドを3秒上限で実行。取得文字数は `PICOAGENT_CLIPBOARD_MAX_CHARS` で制限 |
| `workspace_files` | 許可ルート内のファイル一覧、テキスト読込、書込、追記、フォルダ作成、情報取得 | 許可ルート `PICOAGENT_WORKSPACE_ROOT` 配下に限定、読込/書込サイズ上限、削除操作なし |
| `run_command` | 許可ルート内でローカルコマンドを実行 | 既定無効、`PICOAGENT_COMMAND_ALLOWLIST` の prefix 一致のみ、shell 不使用、stdin 無効、timeout、出力32KBで制限 |
| `slack_send_message` | Slack Bot token または Incoming Webhook URL でメッセージ送信 | token/webhook は設定から都度取得、秘密値を出力しない、本文4000文字、timeout |
| `gmail_message` | Gmail API でメール送信または下書き作成 | OAuth access token は設定から都度取得、秘密値を出力しない、既定は下書き、本文/件名サイズ上限、timeout |
| `google_calendar` | Google Calendar の予定一覧取得・予定作成 | OAuth access token は設定から都度取得、秘密値を出力しない、件名/説明サイズ上限、timeout |
| `google_docs` | Google Docs の文書作成・本文抽出・末尾追記・文字列置換 | Google Docs API `batchUpdate` を使用、OAuth access token は設定から都度取得、秘密値を出力しない、timeout |
| `google_drive` | Google Drive のファイル検索・メタデータ取得・テキストファイル作成 | OAuth access token は設定から都度取得、秘密値を出力しない、作成サイズ上限、timeout |
| `google_sheets` | Google Sheets の範囲読込・更新・追記 | OAuth access token は設定から都度取得、秘密値を出力しない、行数上限、timeout |
| `google_tasks` | Google Tasks のタスクリスト取得・タスク一覧・作成・完了 | OAuth access token は設定から都度取得、秘密値を出力しない、timeout |
| `github_issue` | GitHub Issue の一覧取得・作成 | token は設定から都度取得、秘密値を出力しない、作成は token 必須、件名/本文サイズ上限、timeout |
| `notion` | Notion のページ検索・データベースページ作成 | integration token は設定から都度取得、秘密値を出力しない、タイトル/本文サイズ上限、timeout |
| `discord_send_message` | Discord Webhook でメッセージ送信 | webhook は設定から都度取得、秘密値を出力しない、本文2000文字、timeout |
| `x_twitter` | X API v2 のポスト投稿・最近のポスト検索・ユーザーポスト取得 | OAuth2 bearer token は設定から都度取得、秘密値を出力しない、本文280文字、timeout |
| `mastodon` | Mastodon の投稿・ホームタイムライン・検索 | instance URL/access token は設定から都度取得、秘密値を出力しない、timeout |
| `bluesky` | Bluesky AT Protocol の投稿・投稿検索・プロフィール取得 | identifier/app password でセッション作成、秘密値を出力しない、本文300文字、timeout |
| `reddit` | Reddit のテキスト投稿・subreddit hot取得・コメント投稿 | OAuth bearer token は設定から都度取得、User-Agent設定、秘密値を出力しない、timeout |
| `linkedin` | LinkedIn のプロフィール取得・UGCテキスト投稿 | access token/author URN は設定から都度取得、秘密値を出力しない、timeout |
| `misskey` | Misskey のノート投稿・タイムライン・ユーザー検索 | instance URL/API token は設定から都度取得、秘密値を出力しない、timeout |
| `microsoft_graph` | Outlook メール送信、予定一覧/作成、Teams チャンネル送信 | OAuth access token は設定から都度取得、秘密値を出力しない、timeout |
| `jira` | Jira Cloud の課題検索・作成 | base URL/email/API token は設定から都度取得、秘密値を出力しない、timeout |
| `linear` | Linear の Issue 一覧・作成 | API key は設定から都度取得、秘密値を出力しない、timeout |
| `trello` | Trello のカード一覧・作成 | API key/token は設定から都度取得、秘密値を出力しない、timeout |
| `todoist` | Todoist のタスク一覧・作成・完了 | API token は設定から都度取得、秘密値を出力しない、timeout |
| `http_api` | 個別スキルがないSaaSやWebhookへの汎用HTTP API呼び出し | 許可ドメイン必須、任意Bearer/API key、本文/応答サイズ上限、timeout |
| `web_fetch` | 指定URLの本文取得 | http/https のみ、許可ドメイン設定、応答64KBで制限、timeout。取得内容は信頼できない外部データとして扱う |
| `ms_office` | Microsoft Office COM による Word/Excel/PowerPoint の文字列置換・PDF出力 | `PICOAGENT_DOCUMENT_ROOT` 配下のみ、Windows + Microsoft Office 必須、PowerShell 経由、timeout |
| `document_convert` | LibreOffice headless による Office/PDF 変換 | `PICOAGENT_DOCUMENT_ROOT` 配下のみ、専用エンジン `PICOAGENT_LIBREOFFICE_PATH`、timeout |
| `pdf_tools` | qpdf による PDF 情報取得・ページ抽出・結合・回転・線形化 | `PICOAGENT_DOCUMENT_ROOT` 配下のみ、専用エンジン `PICOAGENT_QPDF_PATH`、timeout |

---

## 新しいコネクタの作り方

### 叩く AI を増やす（例: Anthropic Claude）

1. `src-tauri/src/providers/anthropic.rs` を作り、`Provider` を実装して `inventory::submit!` する。`chat()` 内で正規形 ⇄ Anthropic Messages API 形式を変換。
2. 再ビルドすると設定画面のプロバイダ一覧に自動で現れる（`build.rs` が `pub mod` を生成、`inventory` が登録）。
3. `SPEC.md` の同梱プロバイダ表と、必要な設定キーを `README.md` に追記。

### エージェント機能を増やす（例: Web 検索）

1. `skills/web_search/mod.rs` を作り、`Tool` を実装して `inventory::submit!` する。副作用があるなら境界（タイムアウト・件数上限）を設ける。
2. 再ビルドすると、ツールスキーマが自動でモデルへ渡る。
3. `SPEC.md` の同梱ツール表を更新。

どちらも中央の **`mod.rs` の編集は不要**。プロバイダは `src/providers/*.rs`、スキルは `skills/<スキル名>/mod.rs` を置くだけで検出される。設定値が要るなら `config_schema()` で申告すれば設定画面に項目が並ぶ。

---

## 設定（config と設定画面）

### 設定の正本（`config.rs`）

- 値の優先順位: **永続ファイル `app_config_dir/config.json` > 環境変数 > ハードコード既定値**。
- API: `load(path)` / `get(key, fallback)` / `get_bool(key, fallback)` / `get_all()` / `set_all(patch)`。
- 空文字は「未設定」とみなし env → 既定値へフォールバックする。

### 設定画面（右クリック →「設定…」）

- `ui/settings.{html,css,js}` の独立 Tauri ウィンドウ。`ui/picoagent.js` の `window.picoagent.settings.*` →
  `commands.rs` の `settings_load` / `settings_list_models` / `settings_save` command で構成。
- 画面は **基本** / **キャラクター** / **音声** / **叩くAI** / **スキル** のタブで構成し、導入済みスキル一覧・導入済みキャラクター一覧・各コネクタの `config_schema()` を動的に描画する。コネクタを足すと一覧も設定項目も自動で増える。
- スキル一覧は `settings_load` が返す各ツールの `name` / `label` / `description` をカードで表示する。`config_schema()` を持つスキルは ⚙ 付きのアコーディオンになり、カードをクリックするとそのスキルの設定項目だけがその場に開く（設定項目を末尾へずらずら並べない）。キャラクター一覧は `assets/characters/index.json` から表示し、使用中のキャラに「（使用中）」を付ける。
- 「叩くAI」タブのコネクタ設定は、選択中プロバイダの設定だけを表示する（プロバイダを切り替えると表示も切り替わる）。プロバイダ一覧は既定の `local` を先頭に表示し、その他をラベル順に並べる。基本タブの「モデル」欄は local では非表示にし（モデルはプリセットで選ぶ）、ollama/openai でのみ表示する。
- `Field` の項目: `{ key, label, type:"text"|"password"|"textarea"|"checkbox", placeholder }`。
- 保存は `config.set_all()` で永続化し、実行中の状態（プロバイダ・モデル・システムプロンプト・長期メモリ・常に手前）へ即反映する。コネクタ固有設定は `cfg.get()` 都度読みのため自動反映。Ollama 選択時は `settings_list_models` で `/api/tags` の候補を読み込み、保存時に選択モデルを `PICOAGENT_MODEL` と `OLLAMA_MODEL` の両方へ反映する。

---

## 設定キー一覧

| キー | 既定 | 区分 | 用途 |
|---|---|---|---|
| `PICOAGENT_PROVIDER` | `local` | 基本 | 選択中の叩く AI の id |
| `PICOAGENT_MODEL` | プロバイダ既定 | 基本 | 選択中モデル |
| `PICOAGENT_SYSTEM_PROMPT` | 既定プロンプト | 基本 | system メッセージ |
| `PICOAGENT_BUBBLE_FONT_PX` | `13` | 基本 | フキダシの文字サイズ(px)。`pet:ui` で即反映 |
| `PICOAGENT_BUBBLE_WIDTH_PX` | 自動 | 基本 | フキダシ右下のドラッグで保存する幅(px)。`pet:ui` で即反映 |
| `PICOAGENT_BUBBLE_HEIGHT_PX` | 自動 | 基本 | フキダシ右下のドラッグで保存する高さ(px)。`pet:ui` で即反映 |
| `PICOAGENT_ALWAYS_ON_TOP` | `true` | 基本 | 常に手前に表示 |
| `PICOAGENT_MEMORY_ENABLED` | `true` | 基本 | 会話から好み・自己情報・継続指示を暗黙的に長期メモリへ保存する |
| `PICOAGENT_MEMORY_MAX_ITEMS` | `80` | 基本 | 長期メモリに保持する最大件数（10〜200） |
| `PICOAGENT_CHARACTER` | `mugi` | 基本 | 選択中キャラクターのID |
| `PICOAGENT_CHARACTER_OVERRIDES` | `{}` | 基本 | キャラクターIDごとの人格設定上書き |
| `PICOAGENT_TTS_ENABLED` | `false` | 音声 | Aivis で回答を読み上げる |
| `PICOAGENT_TTS_DEVICE` | `auto` | 音声 | 同梱 Aivis の推論デバイス（`auto` は起動前に GPU 候補を判定 / `cpu` / `gpu` は明示指定） |
| `PICOAGENT_TTS_SPEAKER` | `888753760` | 音声 | Aivis の speaker/style ID |
| `PICOAGENT_TTS_CHARACTER_OVERRIDES` | `{}` | 音声 | キャラクターIDごとの Aivis speaker ID 上書き |
| `PICOAGENT_LOCAL_MODEL_PRESET` | `auto` | local | ローカルモデルプリセット（`small`: Gemma 4 E2B/約3.1GB、`balanced`: Gemma 4 E4B/約5.0GB、`large`: Gemma 4 12B/約7.1GB、`qwen3`: Qwen3 4B Instruct/約2.5GB、`custom`） |
| `PICOAGENT_MODEL_URL` | Gemma 4 E4B Q4_K_M GGUF | local | 同梱エンジンが取得するモデル URL |
| `PICOAGENT_LOCAL_PORT` | `11435` | local | 同梱エンジンのポート |
| `PICOAGENT_LOCAL_CTX` | `8192` | local | コンテキスト長 |
| `PICOAGENT_LOCAL_NGL` | `999` | local | GPU へオフロードするレイヤ数（`999`=全レイヤ優先 / `0`=CPU 固定） |
| `PICOAGENT_LOCAL_GPU_DEVICE` | （空=既定先頭） | local | 使用 GPU の指定（複数 GPU 環境向け、例 `Vulkan1`） |
| `OLLAMA_HOST` | `http://127.0.0.1:11434` | ollama | Ollama 接続先 |
| `OLLAMA_MODEL` | `qwen3:8b` | ollama | Ollama 既定モデル |
| `OPENAI_BASE_URL` | `https://api.openai.com/v1` | openai | OpenAI 互換エンドポイント |
| `OPENAI_API_KEY` | （なし） | openai | OpenAI 互換の API キー。未設定なら選択不可 |
| `OPENAI_MODEL` | `gpt-4o-mini` | openai | OpenAI 互換の既定モデル |
| `PICOAGENT_GEMINI_API_KEY` | （なし） | gemini | Gemini API キー。未設定なら選択不可 |
| `PICOAGENT_GEMINI_MODEL` | `gemini-2.5-flash` | gemini | Gemini の既定モデル |
| `PICOAGENT_GEMINI_BASE_URL` | `https://generativelanguage.googleapis.com/v1beta/openai` | gemini | Gemini の OpenAI 互換エンドポイント |
| `PICOAGENT_TOOLS_ROOT` | ユーザーホーム | read_file | `read_file` が読み取れるルート |
| `PICOAGENT_WEB_SEARCH_MAX_RESULTS` | `5` | web_search | Web検索で返す件数（1〜8） |
| `PICOAGENT_WEB_SEARCH_TIMEOUT_SECS` | `15` | web_search | Web検索の待ち時間（秒、3〜60） |
| `PICOAGENT_CHATTER_ENABLED` | `false` | chitchat | アイドル時に自発的な雑談をする |
| `PICOAGENT_CHATTER_MIN_SECS` | `90` | chitchat | 自発雑談の最短間隔（秒、30〜3600） |
| `PICOAGENT_CHATTER_MAX_SECS` | `240` | chitchat | 自発雑談の最長間隔（秒、30〜3600） |
| `PICOAGENT_CHATTER_TOPIC_MODE` | `auto` | chitchat | 自発雑談の話題ソース（`auto` / `x` / `trends` / `news` / `off`） |
| `PICOAGENT_CHATTER_X_QUERY` | `lang:ja -is:retweet min_faves:1000` | chitchat | X API 利用時の自発雑談向け検索クエリ |
| `PICOAGENT_CHATTER_TRENDS_RSS_URLS` | `https://trends.google.com/trending/rss?geo=JP` | chitchat | X API 未設定時にも使う急上昇RSS URL（カンマ区切り） |
| `PICOAGENT_CHATTER_NEWS_RSS_URLS` | `https://news.google.com/rss?hl=ja&gl=JP&ceid=JP:ja` | chitchat | 自発雑談向けニュースRSS URL（カンマ区切り） |
| `PICOAGENT_CHATTER_TOPIC_TIMEOUT_SECS` | `8` | chitchat | 自発雑談の話題取得待ち時間（秒、3〜30） |
| `PICOAGENT_ALARMS` | `[]` | alarm | 目覚まし一覧（JSON文字列。通常はチャットで設定。設定画面では専用フォームで編集） |
| `PICOAGENT_DAILY_EVENTS` | `[]` | reminder / focus_timer | リマインダー・集中タイマー・見守り通知（JSON文字列） |
| `PICOAGENT_PRAISE_LOG` | `[]` | praise_log | できたことメモ（JSON文字列） |
| `PICOAGENT_CLIPBOARD_MAX_CHARS` | `3000` | clipboard_text | クリップボード取得の最大文字数（500〜8000） |
| `PICOAGENT_WEATHER_DEFAULT_LOCATION` | `Tokyo` | weather | 天気の既定地点（自動取得失敗時のフォールバック） |
| `PICOAGENT_WEATHER_DISABLE_AUTO_LOCATION` | `false` | weather | 地名未指定時の現在地IP自動取得を無効化 |
| `PICOAGENT_WEATHER_TIMEOUT_SECS` | `15` | weather | 天気API待ち時間（秒、3〜60） |
| `PICOAGENT_CURRENCY_DEFAULT_BASE` | `USD` | currency_rates | 為替の既定換算元 |
| `PICOAGENT_CURRENCY_DEFAULT_SYMBOLS` | `JPY` | currency_rates | 為替の既定換算先 |
| `PICOAGENT_CURRENCY_TIMEOUT_SECS` | `15` | currency_rates | 為替API待ち時間（秒、3〜60） |
| `PICOAGENT_MARKET_TIMEOUT_SECS` | `15` | market_indices | 市場指数API待ち時間（秒、3〜60） |
| `PICOAGENT_RSS_DEFAULT_URL` | （なし） | rss_feed | RSS既定URL |
| `PICOAGENT_RSS_TIMEOUT_SECS` | `15` | rss_feed | RSS取得待ち時間（秒、3〜60） |
| `PICOAGENT_WORKSPACE_ROOT` | ユーザーホーム | workspace_files | ファイル一覧・読込・書込・追記・フォルダ作成の許可ルート |
| `PICOAGENT_COMMAND_ENABLED` | `false` | run_command | allowlist 制限付きコマンド実行を有効化 |
| `PICOAGENT_COMMAND_ROOT` | ユーザーホーム | run_command | コマンド実行の許可ルート |
| `PICOAGENT_COMMAND_ALLOWLIST` | `git status,git diff,npm test,cargo test` | run_command | 実行を許可するコマンド prefix（カンマ区切り） |
| `PICOAGENT_COMMAND_TIMEOUT_SECS` | `30` | run_command | コマンド実行の既定タイムアウト秒（1〜120） |
| `PICOAGENT_SLACK_BOT_TOKEN` | （なし） | slack_send_message | Slack Bot token（Webhook 未使用時） |
| `PICOAGENT_SLACK_CHANNEL` | （なし） | slack_send_message | Slack の既定送信先チャンネル |
| `PICOAGENT_SLACK_WEBHOOK_URL` | （なし） | slack_send_message | Slack Incoming Webhook URL（Bot token の代替） |
| `PICOAGENT_SLACK_TIMEOUT_SECS` | `15` | slack_send_message | Slack API の待ち時間（秒、3〜60） |
| `PICOAGENT_GMAIL_ACCESS_TOKEN` | （なし） | gmail_message | Gmail API の OAuth access token |
| `PICOAGENT_GMAIL_BASE_URL` | `https://gmail.googleapis.com/gmail/v1` | gmail_message | Gmail API の base URL |
| `PICOAGENT_GMAIL_TIMEOUT_SECS` | `20` | gmail_message | Gmail API の待ち時間（秒、3〜60） |
| `PICOAGENT_GOOGLE_CALENDAR_ACCESS_TOKEN` | （なし） | google_calendar | Google Calendar API の OAuth access token |
| `PICOAGENT_GOOGLE_CALENDAR_ID` | `primary` | google_calendar | 既定のカレンダーID |
| `PICOAGENT_GOOGLE_CALENDAR_TIMEOUT_SECS` | `20` | google_calendar | Google Calendar API の待ち時間（秒、3〜60） |
| `PICOAGENT_GOOGLE_DOCS_ACCESS_TOKEN` | （なし） | google_docs | Google Docs API の OAuth access token |
| `PICOAGENT_GOOGLE_DOCS_TIMEOUT_SECS` | `20` | google_docs | Google Docs API の待ち時間（秒、3〜60） |
| `PICOAGENT_GOOGLE_DRIVE_ACCESS_TOKEN` | （なし） | google_drive | Google Drive API の OAuth access token |
| `PICOAGENT_GOOGLE_DRIVE_TIMEOUT_SECS` | `20` | google_drive | Google Drive API の待ち時間（秒、3〜60） |
| `PICOAGENT_GOOGLE_SHEETS_ACCESS_TOKEN` | （なし） | google_sheets | Google Sheets API の OAuth access token |
| `PICOAGENT_GOOGLE_SHEETS_ID` | （なし） | google_sheets | 既定の spreadsheet ID |
| `PICOAGENT_GOOGLE_SHEETS_TIMEOUT_SECS` | `20` | google_sheets | Google Sheets API の待ち時間（秒、3〜60） |
| `PICOAGENT_GOOGLE_TASKS_ACCESS_TOKEN` | （なし） | google_tasks | Google Tasks API の OAuth access token |
| `PICOAGENT_GOOGLE_TASKS_LIST_ID` | `@default` | google_tasks | 既定の tasklist ID |
| `PICOAGENT_GOOGLE_TASKS_TIMEOUT_SECS` | `15` | google_tasks | Google Tasks API の待ち時間（秒、3〜60） |
| `PICOAGENT_GITHUB_TOKEN` | （なし） | github_issue | GitHub token（private repo や Issue 作成用） |
| `PICOAGENT_GITHUB_OWNER` | （なし） | github_issue | 既定の GitHub owner |
| `PICOAGENT_GITHUB_REPO` | （なし） | github_issue | 既定の GitHub repository |
| `PICOAGENT_GITHUB_TIMEOUT_SECS` | `20` | github_issue | GitHub API の待ち時間（秒、3〜60） |
| `PICOAGENT_NOTION_TOKEN` | （なし） | notion | Notion integration token |
| `PICOAGENT_NOTION_DATABASE_ID` | （なし） | notion | Notion の既定 database ID |
| `PICOAGENT_NOTION_TIMEOUT_SECS` | `20` | notion | Notion API の待ち時間（秒、3〜60） |
| `PICOAGENT_DISCORD_WEBHOOK_URL` | （なし） | discord_send_message | Discord Webhook URL |
| `PICOAGENT_DISCORD_TIMEOUT_SECS` | `15` | discord_send_message | Discord Webhook の待ち時間（秒、3〜60） |
| `PICOAGENT_X_BEARER_TOKEN` | （なし） | x_twitter | X OAuth2 bearer token |
| `PICOAGENT_X_USER_ID` | （なし） | x_twitter | X 既定 user ID |
| `PICOAGENT_X_TIMEOUT_SECS` | `20` | x_twitter | X API の待ち時間（秒、3〜60） |
| `PICOAGENT_MASTODON_BASE_URL` | `https://mastodon.social` | mastodon | Mastodon instance URL |
| `PICOAGENT_MASTODON_ACCESS_TOKEN` | （なし） | mastodon | Mastodon access token |
| `PICOAGENT_MASTODON_TIMEOUT_SECS` | `20` | mastodon | Mastodon API の待ち時間（秒、3〜60） |
| `PICOAGENT_BLUESKY_BASE_URL` | `https://bsky.social` | bluesky | Bluesky PDS URL |
| `PICOAGENT_BLUESKY_IDENTIFIER` | （なし） | bluesky | Bluesky identifier |
| `PICOAGENT_BLUESKY_APP_PASSWORD` | （なし） | bluesky | Bluesky app password |
| `PICOAGENT_BLUESKY_TIMEOUT_SECS` | `20` | bluesky | Bluesky API の待ち時間（秒、3〜60） |
| `PICOAGENT_REDDIT_BEARER_TOKEN` | （なし） | reddit | Reddit OAuth bearer token |
| `PICOAGENT_REDDIT_USER_AGENT` | `PicoAgent/1.0` | reddit | Reddit User-Agent |
| `PICOAGENT_REDDIT_SUBREDDIT` | （なし） | reddit | Reddit 既定 subreddit |
| `PICOAGENT_REDDIT_TIMEOUT_SECS` | `20` | reddit | Reddit API の待ち時間（秒、3〜60） |
| `PICOAGENT_LINKEDIN_ACCESS_TOKEN` | （なし） | linkedin | LinkedIn access token |
| `PICOAGENT_LINKEDIN_AUTHOR_URN` | （なし） | linkedin | LinkedIn author URN |
| `PICOAGENT_LINKEDIN_TIMEOUT_SECS` | `20` | linkedin | LinkedIn API の待ち時間（秒、3〜60） |
| `PICOAGENT_MISSKEY_BASE_URL` | `https://misskey.io` | misskey | Misskey instance URL |
| `PICOAGENT_MISSKEY_TOKEN` | （なし） | misskey | Misskey API token |
| `PICOAGENT_MISSKEY_TIMEOUT_SECS` | `20` | misskey | Misskey API の待ち時間（秒、3〜60） |
| `PICOAGENT_MS_GRAPH_ACCESS_TOKEN` | （なし） | microsoft_graph | Microsoft Graph OAuth access token |
| `PICOAGENT_MS_GRAPH_TEAM_ID` | （なし） | microsoft_graph | Microsoft Teams 既定 team ID |
| `PICOAGENT_MS_GRAPH_CHANNEL_ID` | （なし） | microsoft_graph | Microsoft Teams 既定 channel ID |
| `PICOAGENT_MS_GRAPH_TIMEOUT_SECS` | `20` | microsoft_graph | Microsoft Graph API の待ち時間（秒、3〜60） |
| `PICOAGENT_JIRA_BASE_URL` | （なし） | jira | Jira Cloud base URL |
| `PICOAGENT_JIRA_EMAIL` | （なし） | jira | Jira account email |
| `PICOAGENT_JIRA_API_TOKEN` | （なし） | jira | Jira API token |
| `PICOAGENT_JIRA_PROJECT_KEY` | （なし） | jira | Jira 既定 project key |
| `PICOAGENT_JIRA_TIMEOUT_SECS` | `20` | jira | Jira API の待ち時間（秒、3〜60） |
| `PICOAGENT_LINEAR_API_KEY` | （なし） | linear | Linear API key |
| `PICOAGENT_LINEAR_TEAM_ID` | （なし） | linear | Linear 既定 team ID |
| `PICOAGENT_LINEAR_TIMEOUT_SECS` | `20` | linear | Linear API の待ち時間（秒、3〜60） |
| `PICOAGENT_TRELLO_KEY` | （なし） | trello | Trello API key |
| `PICOAGENT_TRELLO_TOKEN` | （なし） | trello | Trello token |
| `PICOAGENT_TRELLO_LIST_ID` | （なし） | trello | Trello 既定 list ID |
| `PICOAGENT_TRELLO_TIMEOUT_SECS` | `15` | trello | Trello API の待ち時間（秒、3〜60） |
| `PICOAGENT_TODOIST_TOKEN` | （なし） | todoist | Todoist API token |
| `PICOAGENT_TODOIST_PROJECT_ID` | （なし） | todoist | Todoist 既定 project ID |
| `PICOAGENT_TODOIST_TIMEOUT_SECS` | `15` | todoist | Todoist API の待ち時間（秒、3〜60） |
| `PICOAGENT_HTTP_ALLOWED_DOMAINS` | （なし） | http_api | 汎用HTTPで呼べるドメイン（必須、カンマ区切り） |
| `PICOAGENT_HTTP_BEARER_TOKEN` | （なし） | http_api | 汎用HTTPに付与する Bearer token |
| `PICOAGENT_HTTP_API_KEY_HEADER` | `X-API-Key` | http_api | 汎用HTTPに付与する API key ヘッダー名 |
| `PICOAGENT_HTTP_API_KEY_VALUE` | （なし） | http_api | 汎用HTTPに付与する API key 値 |
| `PICOAGENT_HTTP_TIMEOUT_SECS` | `20` | http_api | 汎用HTTPの待ち時間（秒、3〜60） |
| `PICOAGENT_WEB_FETCH_ALLOWED_DOMAINS` | （空） | web_fetch | URL取得を許可するドメイン（カンマ区切り。空なら制限なし） |
| `PICOAGENT_WEB_FETCH_TIMEOUT_SECS` | `20` | web_fetch | URL取得の待ち時間（秒、3〜60） |
| `PICOAGENT_DOCUMENT_ROOT` | ユーザーホーム | ms_office / document_convert / pdf_tools | Office/PDF 操作の許可ルート |
| `PICOAGENT_POWERSHELL_PATH` | `powershell` | ms_office | PowerShell 実行ファイルパス |
| `PICOAGENT_MS_OFFICE_TIMEOUT_SECS` | `120` | ms_office | Microsoft Office 操作タイムアウト秒（5〜300） |
| `PICOAGENT_LIBREOFFICE_PATH` | `soffice` | document_convert | LibreOffice 実行ファイルパス |
| `PICOAGENT_DOCUMENT_CONVERT_TIMEOUT_SECS` | `120` | document_convert | 文書変換タイムアウト秒（5〜300） |
| `PICOAGENT_QPDF_PATH` | `qpdf` | pdf_tools | qpdf 実行ファイルパス |
| `PICOAGENT_PDF_TIMEOUT_SECS` | `120` | pdf_tools | PDF操作タイムアウト秒（5〜300） |

各キーは環境変数としても渡せる（既定値としての参照のみ。保存先は常に `config.json`）。

---

## フロント連携と製品化

### キャラクターパック

- `characters/index.json` に `{ id, name, manifest }` を追加し、各フォルダへ `character.json` と連番フレームを置く（`ui/assets/characters/` へは `sync-characters.mjs` が同期）。
- `character.json` は `name`、`description`、`greeting`、`persona`、`animations` を持つ。任意で `assetVersion` を持て、同名フレーム更新時の画像キャッシュ回避に使う。人格には性格・口調・一人称・ユーザーの呼び方・追加指示を定義できる。
- 各アニメーションは `path`、`frames`、`fps`、`motion` を持つ。任意で `frameDurations`（ミリ秒配列）を持て、まばたきや口パクなど均等 `fps` だと不自然になるフレームだけ表示時間を調整できる。フロントは画像フレームとCSS補間動作を組み合わせる。
- 選択中キャラの人格は `pet_set_character_profile` でシステムプロンプトへ追加する。既定プロンプトは挨拶・自己紹介でアプリ説明ではなくキャラクター本人として名乗ることを優先する。設定画面のキャラ別上書きは `PICOAGENT_CHARACTER_OVERRIDES` に保存する。
- 自発雑談（雑談スキル `chitchat`）: `PICOAGENT_CHATTER_ENABLED` が ON のとき、フロントは `[min, max]` 秒のランダム間隔の待機タイマーで `pet_idle_chatter` を呼ぶ。生成した独り言は**先に音声を合成（`prepareSpeech`）してから文字を表示し、表示と同時に読み上げる**（`playPreparedSpeech`。挨拶と同じ同期再生）。入力中・応答中・読み上げ中・全画面中は割り込まず次回へ送る。ユーザー操作（送信・入力欄を開く・会話リセット）や設定変更（`pet:ui`）のたびにタイマーを張り直す。`skills/chitchat/trends.rs` は `PICOAGENT_CHATTER_TOPIC_MODE=auto` で X API（`PICOAGENT_X_BEARER_TOKEN` がある場合）→ Google Trends RSS → ニュースRSSの順に話題メモを取る。`x` / `trends` / `news` / `off` でソースを固定または無効化できる。X API 未設定時は公式X投稿とは断定せず、急上昇ワードやトップニュースの候補として扱う。生成はWeb検索ツールを使わず、生成後は「直前の自発雑談: ...」として会話文脈へ1件積み、通常会話と同じ圧縮・履歴保存に通す。これにより、ユーザーが直後または再起動後に「今の何の話？」と聞いた場合に直前の雑談を参照できる。挨拶・名乗り・自己紹介は出さず、毎回ちがう話題になるよう話題ヒントを擬似ランダムで与え、さらに直近の独り言（`AppState.recent_chatter`、末尾6件）を「避ける文」として渡して繰り返しを防ぐ。雑談の生成は**人格(system)＋現在時刻＋指示だけ**を文脈にし、ユーザー発話（会話履歴）は含めない（小型モデルが入力をオウム返しするのを防ぐため）。`current_time_context()` で現在のローカル日時・時間帯（朝/昼/夕方/夜）を渡し、夜に「日差し」など時間に合わない描写をしないようにする。なお通常会話の返信がユーザー入力のオウム返しになった場合、フロントはそれを読み上げない。話題メモ（ニュース等）は `pet_idle_chatter` が `trends::topic_brief` で1件取得して `topic` 引数で skill へ渡し、出典（記事タイトル＋URL）も受け取る。生成後、返信が話題に触れていれば（返信と記事タイトルの内容バイグラム一致）最も関連する記事URLを末尾へ `参考:` として添える（URL 行は読み上げ対象外。例: 地震の独り言なら地震ニュースのURL）。
- 目覚まし時計（`alarm` スキル + `alarm.rs`）: フロントは約20秒間隔で `pet_alarms` を取得し、現在のローカル時刻（HH:MM）と曜日が一致する有効なアラームがあれば `pet_alarm_wake` を呼んで**声でやさしく起こす**（生成した起こし文を音声合成→表示→即時再生。生成失敗時は固定の起こし文）。同じ分での二重発火は済みキー集合で防ぐ。一度きり（`repeat=false`）は発火後に `enabled=false` へ。鳴った後はフキダシに `おきたよ！` ボタンを出し、押されるまでは `snooze_minutes`（既定5、1〜60分）おきに同じアラームを再通知する。アラームは時刻・曜日・繰り返し・スヌーズ分数を**チャット（`alarm` スキル）でも設定画面でも**設定できる。設定画面のスキル「目覚まし時計」を開くと、時刻ピッカー＋曜日チェック＋繰り返し/有効/スヌーズ分数/ラベル/削除/追加の専用フォーム（`settings.js` の `buildAlarmEditor`）で編集でき、保存時に `PICOAGENT_ALARMS`（JSON）へ組み立てる。保存先は config。
- 日常通知（`reminder` / `focus_timer` / `daily.rs`）: フロントは約20秒間隔で `pet_daily_events` を呼び、期限到来したリマインダー・集中終了・作業見守りを吹き出し表示と読み上げで通知する。単発イベントは通知後に無効化し、見守りのような `repeat_minutes` 付きイベントは次回時刻へ進める。起動時の挨拶には `pet_daily_greeting` で時間帯・通知件数・直近のできたことメモを添える。
- 起動待ち画面: `#petApp.is-booting` 中は通常のフキダシとキャラクターを隠し、`#bootScreen` に新規生成した横向き寝姿アセット `characters/mugi/boot/sleeping.png` を使った睡眠ループ風の待機画面を表示する。`renderer.js` の `setStatus()` がモデルDL、同梱エンジン起動、Aivis 起動待ちの文言を `#bootStatus` に同期し、準備完了時に `is-booting` を外して通常表示へ切り替える。

- フロント（`ui/`）は Vite 等のバンドラを使わず、Tauri の静的 `frontendDist` で配信。`withGlobalTauri:true` で
  `window.__TAURI__` を有効化し、`ui/picoagent.js` が旧 `window.picoagent`（init/send/menu/on/settings.*）を再現する。
  これにより `renderer.js` / `settings.js` は Electron 版とほぼ同じコードのまま動く。
- フキダシ右下のリサイズつまみは Pointer Events で幅・高さを変更し、確定時に `pet_set_bubble_size` で設定へ永続化する。
- キャラクターまたはフキダシのダブルクリックはウィンドウをフルスクリーン化（`getCurrentWindow().setFullscreen`）し、その中でフキダシを画面最大化のように広げる（`.bubble.fullscreen`）。最大化中はキャラを薄く背面化し、送信ボタンや入力欄と被らないようにする。解除は再度ダブルクリックまたは `Esc`。ドラッグ領域のダブルクリックによるネイティブのウィンドウ最大化は `maximizable:false` で無効化している。
- 拡大時はセッション内の会話履歴をフキダシ内に表示し、最新応答と入力フォームは下部に残す。`pet:reset` では履歴もクリアする。
- ウィンドウ: 透明・枠なし・常に手前・タスクバー表示・最大化不可（`tauri.conf.json`）。ドラッグは `data-tauri-drag-region`。
- アプリ終了時は `RunEvent::Exit` で `engine::stop` が同梱 llama-server の子プロセスを kill し、`aivis::stop` がアプリ自身で裏起動した AivisSpeech Engine だけを停止する。さらに `npm run dev` / `npm run build` は Tauri 起動前に `scripts/stop-stale.mjs` を同期実行し、前回の残存プロセスを停止して同梱 DLL ロックによるビルド失敗（Windows os error 32）を防ぐ。手動停止は `npm run stop`。起動ログは `app_data_dir/log/engine.log` と `app_data_dir/log/aivis.log` に集約する。
- 権限は `src-tauri/capabilities/default.json`（start-dragging / set-always-on-top / set-fullscreen / event / webview 生成）。
- 製品化: `productName=PicoAgent` / `identifier=com.picoagent.app`（旧 `com.coda.app` から改名。既存環境は `app_config_dir`/`app_data_dir` のフォルダ名を新識別子へリネーム移行する）。Windows バンドルは NSIS（`bundle.targets`）。
  アイコンは `src-tauri/icons/`（`npx tauri icon src-tauri/icon-source.png` で再生成）。
  署名（Authenticode）・自動更新（`tauri-plugin-updater`）はリリース時に鍵・証明書を用意して設定する（現状未設定）。
