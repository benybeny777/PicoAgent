<div align="center">
  <h1>🐾 PicoAgent</h1>
  <p><strong>叩く AI とエージェント機能をコネクタで拡張できる、キャラクター + フキダシだけの透明デスクトップペット</strong></p>

  ![Tauri](https://img.shields.io/badge/Tauri-2-FFC131?style=for-the-badge&logo=tauri)
  ![Rust](https://img.shields.io/badge/Rust-backend-000000?style=for-the-badge&logo=rust)
  ![Ollama](https://img.shields.io/badge/Ollama-Local_LLM-black?style=for-the-badge&logo=ollama)
</div>

<br />

> 透明な Tauri ウィンドウに常駐するデスクトップペット。バックエンドは Rust、フロントは webview。**Ollama 不要** — 推論エンジンを同梱し、初回起動時に PC のVRAM/RAM目安からローカルモデルを自動選定して取得します。**叩く AI（LLM バックエンド）** と **エージェント機能（ツール）** を、決まったフォルダへコネクタを足すだけで差し替え・追加できます。

---

## 📑 目次

- [🚀 クイックスタート](#-クイックスタート)
- [✨ 特徴](#-特徴)
- [🧩 コネクタの仕組み](#-コネクタの仕組み)
- [🖱 使い方](#-使い方)
- [⚙️ 設定](#️-設定)
- [📦 製品ビルド（配布）](#-製品ビルド配布)
- [📁 フォルダ構成](#-フォルダ構成)
- [🛟 トラブルシューティング](#-トラブルシューティング)

> [!NOTE]
> アーキテクチャ・コネクタ仕様の詳細は **[SPEC.md](SPEC.md)**、AI エージェント向け作業ルールは **[AGENTS.md](AGENTS.md)** を参照してください。

---

## 🚀 クイックスタート

前提: **Rust（cargo）** と **Node.js ≥ 20**、Windows は WebView2 ランタイム。**Ollama は不要**です。

```powershell
npm install
npm run setup:engine   # 同梱推論エンジン(llama-server、Windows は Vulkan/GPU 版)を取得
npm run dev
```

- 既定の叩く AI は **`local`（同梱エンジン）**。初回起動時に `PICOAGENT_LOCAL_MODEL_PRESET=auto` が PC のVRAM/RAM目安から Gemma 4 系の `small` / `balanced` / `large` を選び、対応する GGUF を自動ダウンロードします。以後はオフラインで動作します。追加プリセットとして `qwen3` も選べます。ダウンロード中はキャラクターの吹き出し下に「モデルを準備中… NN%」と表示されます。設定画面からローカルモデルを変更すると、必要なモデルを取得して同梱エンジンを再起動します。
- Ollama を使いたい場合は `ollama serve` 後に右クリック →「設定…」でプロバイダを切り替えます（任意）。Ollama 選択時はローカルのモデル一覧を読み込み、モデル欄から選べます。

> [!NOTE]
> 初回の `npm run dev` / `npm run build` は Rust の crate 取得・コンパイルで時間がかかります（ネットワーク必須）。モデル DL も初回のみネットが必要です。

OpenAI 互換の叩く AI を使う場合（費用が発生します）は、起動後に右クリック →「設定…」で API キーとプロバイダを設定します。

---

## ✨ 特徴

- **Ollama 不要・ローカル完結** — 推論エンジン同梱、初回 DL の gemma4 でそのまま動く（既定 `local`）。
- **叩く AI を差し替え可能** — local / Ollama / OpenAI 互換を同梱。コネクタを足せば他社 AI にも切替できる。
- **エージェント機能を追加可能** — ツールコネクタを足すと、モデルが function calling で呼び出せる。
- **原則Web検索** — 通常の質問ではローカルモデルの判断に頼らずWeb検索してから参照元のハイパーリンク付きで回答する。短い挨拶は検索に引っ張られないよう例外扱いし、キャラクター本人として返す。
- **設定画面ですべて設定** — 右クリック →「設定…」で叩く AI・モデル・接続先・API キー・ツール設定などを GUI 編集。保存すると即反映。
- **軽量な Tauri ネイティブ** — システム WebView を使い、Rust バックエンドで動作。
- キャラクターパックごとの連続フレームと状態別の滑らかな補間動作（待機 / 考え中 / 返答 / エラー）。
- キャラクターごとに名前・挨拶・性格・口調・一人称を定義でき、設定画面でキャラ選択と人格の上書きが可能。
- 会話は選択中の叩く AI にのみ送られる。ローカル Ollama を使えば外部送信なし。

---

## 🧩 コネクタの仕組み

```mermaid
graph LR
    A[agent.rs] -->|chat| P[providers/*<br/>叩く AI]
    A -->|dispatch| T[tools/*<br/>エージェント機能]
    P -. inventory 自動登録 .- PI[build.rs + mod.rs]
    T -. inventory 自動登録 .- TI[build.rs + mod.rs]
```

- **叩く AI を増やす**: `src-tauri/src/providers/` に `Provider` トレイトを実装した `.rs` を置き、`inventory::submit!` で登録。
- **エージェント機能を増やす**: プロジェクト直下の `skills/<スキル名>/mod.rs` に `Tool` トレイトを実装し、`inventory::submit!` で登録。

`build.rs` が `.rs` を自動で `mod` 取り込みし、再ビルドで**自動登録**されます（`mod.rs` 編集不要）。書き方は [SPEC.md](SPEC.md#新しいコネクタの作り方) を参照。

---

## 🖱 使い方

- フキダシをクリック: 入力欄を開く
- フキダシ右下のつまみをドラッグ: フキダシの幅・高さを変更（自動保存）
- キャラクターをダブルクリック: フキダシを全画面表示／元に戻す（`Esc` でも解除）
- `Enter`: 送信 / `Shift + Enter`: 改行 / `Esc`: 入力欄を閉じる
- キャラクターをドラッグ: 移動
- 右クリック: **設定…**・**常に手前に表示**・**PicoAgent を終了**

---

## ⚙️ 設定

右クリック →「**設定…**」で設定ウィンドウを開き、すべての設定を編集できます。保存するとアプリ再起動なしで反映されます。設定は `app_config_dir/config.json`（個人 PC ローカル）に保存されます。

会話履歴は `app_data_dir/history.json`（個人 PC ローカル）に保存され、起動時に直近の会話を文脈へ復元します（再起動をまたいで直近の話を踏まえて話します）。さらに、好み・自己情報・継続指示らしい発話だけを暗黙的な長期メモリとして `app_data_dir/memory.json` に保存し、次回以降の会話で参照します。履歴とメモリを消したいときは右クリック →「**会話をリセット**」を選びます。

目覚まし時計は、チャットで「**平日の7時に起こして**」「土日の朝8時、繰り返しで」のように頼むと設定できます（声でやさしく起こします）。一覧・削除も「アラーム一覧」「その目覚ましを消して」で操作でき、設定画面の「スキル」タブからも編集できます。鳴った後は `おきたよ！` を押すまで既定5分おきに再通知します。アプリ起動中のみ鳴ります。

日常補助として、「**15分後に洗濯物を取り込むって教えて**」のようなリマインダー、「**25分集中タイマー**」「**1時間おきに作業を見守って**」、おでかけ前チェック、できたことメモ、クリップボード要約も使えます。操作系の依頼はWeb検索よりローカルスキルを優先します。

設定画面の項目は「基本」「キャラクター」「音声」「叩くAI」「スキル」のタブで構成されます。スキル一覧は日常・検索/情報・連携などのカテゴリタブで整理され、各コネクタが申告した設定も自動で増えます。下表は区分（基本／音声／叩くAI／スキル名）ごとにまとめています。使うスキルの欄だけ見れば十分です。

#### 基本（アプリ全体）

| キー | 既定 | 区分 | 用途 |
|---|---|---|---|
| `PICOAGENT_PROVIDER` | `local` | 基本 | 選択中の叩く AI |
| `PICOAGENT_MODEL` | プロバイダ既定 | 基本 | 選択中モデル |
| `PICOAGENT_SYSTEM_PROMPT` | 既定プロンプト | 基本 | システムプロンプト |
| `PICOAGENT_BUBBLE_FONT_PX` | `13` | 基本 | フキダシの文字サイズ(px) |
| `PICOAGENT_BUBBLE_WIDTH_PX` | 自動 | 基本 | ドラッグで保存したフキダシの幅(px) |
| `PICOAGENT_BUBBLE_HEIGHT_PX` | 自動 | 基本 | ドラッグで保存したフキダシの高さ(px) |
| `PICOAGENT_ALWAYS_ON_TOP` | `true` | 基本 | 常に手前に表示 |
| `PICOAGENT_MEMORY_ENABLED` | `true` | 基本 | 会話から好み・自己情報・継続指示を暗黙的に覚える |
| `PICOAGENT_MEMORY_MAX_ITEMS` | `80` | 基本 | 長期メモリの最大件数（10〜200） |
| `PICOAGENT_CHARACTER` | `mugi` | 基本 | 選択中のキャラクターID |
| `PICOAGENT_CHARACTER_OVERRIDES` | `{}` | 基本 | キャラクターIDごとの人格設定上書き |

#### 音声（Aivis 読み上げ）

| キー | 既定 | 区分 | 用途 |
|---|---|---|---|
| `PICOAGENT_TTS_ENABLED` | `false` | 音声 | Aivis で回答を読み上げる |
| `PICOAGENT_TTS_DEVICE` | `auto` | 音声 | 同梱 Aivis の推論デバイス（`auto` は起動前に GPU 候補を判定 / `cpu` / `gpu` は明示指定） |
| `PICOAGENT_TTS_PORT` | `11436` | 音声 | 同梱 Aivis のポート（旧既定 `10102` は起動時に `11436` へ移行） |
| `PICOAGENT_TTS_SPEAKER` | `888753760` | 音声 | Aivis の speaker/style ID |
| `PICOAGENT_TTS_CHARACTER_OVERRIDES` | `{}` | 音声 | キャラクターIDごとの Aivis speaker ID 上書き（任意で `modelUuid` / `modelName` / `sourceUrl` も保持） |

#### 叩く AI（プロバイダ）

| キー | 既定 | 区分 | 用途 |
|---|---|---|---|
| `PICOAGENT_LOCAL_MODEL_PRESET` | `auto` | local | ローカルモデルプリセット（`small`: Gemma 4 E2B/約3.1GB、`balanced`: Gemma 4 E4B/約5.0GB、`large`: Gemma 4 12B/約7.1GB、`qwen3`: Qwen3 4B Instruct/約2.5GB、`custom`） |
| `PICOAGENT_MODEL_URL` | Gemma 4 E4B Q4_K_M | local | 同梱エンジンが取得するモデル GGUF の URL |
| `PICOAGENT_LOCAL_PORT` | `11435` | local | 同梱エンジンのポート |
| `PICOAGENT_LOCAL_CTX` | `8192` | local | コンテキスト長 |
| `PICOAGENT_LOCAL_NGL` | `999` | local | GPU へオフロードするレイヤ数（`999`=全レイヤ優先 / `0`=CPU 固定） |
| `PICOAGENT_LOCAL_GPU_DEVICE` | （空=既定先頭） | local | 使用 GPU の指定。複数 GPU 環境で discrete GPU を選ぶ場合に `Vulkan1` 等を指定 |
| `OLLAMA_HOST` | `http://127.0.0.1:11434` | Ollama | Ollama 接続先 |
| `OLLAMA_MODEL` | `qwen3:8b` | Ollama | Ollama 既定モデル |
| `OPENAI_BASE_URL` | `https://api.openai.com/v1` | OpenAI | OpenAI 互換エンドポイント |
| `OPENAI_API_KEY` | （なし） | OpenAI | OpenAI 互換の API キー |
| `OPENAI_MODEL` | `gpt-4o-mini` | OpenAI | OpenAI 互換の既定モデル |
| `PICOAGENT_GEMINI_API_KEY` | （なし） | Gemini | Gemini API キー |
| `PICOAGENT_GEMINI_MODEL` | `gemini-2.5-flash` | Gemini | Gemini の既定モデル |
| `PICOAGENT_GEMINI_BASE_URL` | `.../v1beta/openai` | Gemini | Gemini の OpenAI 互換エンドポイント |

#### スキル：情報・日常（検索 / 雑談 / 目覚まし / 天気 / 為替 / 市場指数 / RSS）

| キー | 既定 | 区分 | 用途 |
|---|---|---|---|
| `PICOAGENT_TOOLS_ROOT` | ユーザーホーム | read_file | `read_file` ツールが読めるルート |
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
| `PICOAGENT_ALARMS` | `[]` | alarm | 目覚まし一覧（JSON文字列。通常はチャットや設定画面の専用フォームで設定） |
| `PICOAGENT_DAILY_EVENTS` | `[]` | reminder / focus_timer | リマインダー・集中タイマー・見守り通知（JSON文字列。通常はチャットで設定） |
| `PICOAGENT_PRAISE_LOG` | `[]` | praise_log | できたことメモ（JSON文字列。通常はチャットで記録） |
| `PICOAGENT_CLIPBOARD_MAX_CHARS` | `3000` | clipboard_text | クリップボード取得の最大文字数（500〜8000） |
| `PICOAGENT_WEATHER_DEFAULT_LOCATION` | `Tokyo` | weather | 天気の既定地点（自動取得に失敗した時に使用） |
| `PICOAGENT_WEATHER_DISABLE_AUTO_LOCATION` | `false` | weather | 地名未指定時の現在地IP自動取得を無効化（既定は自動取得ON） |
| `PICOAGENT_WEATHER_TIMEOUT_SECS` | `15` | weather | 天気API待ち時間（秒、3〜60） |
| `PICOAGENT_CURRENCY_DEFAULT_BASE` | `USD` | currency_rates | 為替の既定換算元 |
| `PICOAGENT_CURRENCY_DEFAULT_SYMBOLS` | `JPY` | currency_rates | 為替の既定換算先 |
| `PICOAGENT_CURRENCY_TIMEOUT_SECS` | `15` | currency_rates | 為替API待ち時間（秒、3〜60） |
| `PICOAGENT_MARKET_TIMEOUT_SECS` | `15` | market_indices | 市場指数API待ち時間（秒、3〜60） |
| `PICOAGENT_RSS_DEFAULT_URL` | （なし） | rss_feed | RSS既定URL |
| `PICOAGENT_RSS_TIMEOUT_SECS` | `15` | rss_feed | RSS取得待ち時間（秒、3〜60） |

#### スキル：ファイル操作・コマンド実行

| キー | 既定 | 区分 | 用途 |
|---|---|---|---|
| `PICOAGENT_WORKSPACE_ROOT` | ユーザーホーム | workspace_files | ファイル一覧・読込・書込・追記・フォルダ作成の許可ルート |
| `PICOAGENT_COMMAND_ENABLED` | `false` | run_command | allowlist 制限付きコマンド実行を有効化 |
| `PICOAGENT_COMMAND_ROOT` | ユーザーホーム | run_command | コマンド実行の許可ルート |
| `PICOAGENT_COMMAND_ALLOWLIST` | `git status,git diff,npm test,cargo test` | run_command | 実行を許可するコマンド prefix（カンマ区切り） |
| `PICOAGENT_COMMAND_TIMEOUT_SECS` | `30` | run_command | コマンド実行の既定タイムアウト秒（1〜120） |

#### スキル：外部サービス・SNS 連携（使うサービスの欄だけ設定）

| キー | 既定 | 区分 | 用途 |
|---|---|---|---|
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

#### スキル：ドキュメント変換（Office / PDF）

| キー | 既定 | 区分 | 用途 |
|---|---|---|---|
| `PICOAGENT_DOCUMENT_ROOT` | ユーザーホーム | ms_office / document_convert / pdf_tools | Office/PDF 操作の許可ルート |
| `PICOAGENT_POWERSHELL_PATH` | `powershell` | ms_office | PowerShell 実行ファイルパス |
| `PICOAGENT_MS_OFFICE_TIMEOUT_SECS` | `120` | ms_office | Microsoft Office 操作タイムアウト秒（5〜300） |
| `PICOAGENT_LIBREOFFICE_PATH` | `soffice` | document_convert | LibreOffice 実行ファイルパス |
| `PICOAGENT_DOCUMENT_CONVERT_TIMEOUT_SECS` | `120` | document_convert | 文書変換タイムアウト秒（5〜300） |
| `PICOAGENT_QPDF_PATH` | `qpdf` | pdf_tools | qpdf 実行ファイルパス |
| `PICOAGENT_PDF_TIMEOUT_SECS` | `120` | pdf_tools | PDF操作タイムアウト秒（5〜300） |

> [!TIP]
> 各キーは環境変数としても渡せます（既定値としての参照のみ。保存先は常に `config.json`）。

---

## 📦 製品ビルド（配布）

```powershell
npm run setup:engine   # 同梱エンジンを resources/bundled/engine へ取得（未取得の場合）
npm run build
```

`src-tauri/target/release/bundle/` に Windows インストーラ（NSIS）が生成されます。`npm run dev` / `npm run build` は起動前に同梱エンジンと Aivis を確認し、未取得なら `src-tauri/resources/bundled/` へ取得してから Tauri を起動します。

> [!NOTE]
> 以下のパスはいずれも Windows 表記です。macOS / Linux では同等のユーザーデータ配下（`PicoAgent/...`）に読み替えてください。

#### 同梱推論エンジン（llama-server）

- 実行ファイルは配布物へ同梱しますが、モデル GGUF（数 GB）は含めず、初回起動時に `%APPDATA%\PicoAgent\bundled\models\` へ取得します。
- `llama-server` を実行時配置で差し替える場合は `%APPDATA%\PicoAgent\bundled\engine\` を優先して探します。

#### 同梱 Aivis（読み上げ）

- **取得**: 公式 GitHub Releases から既定 `1.2.0` の実行ファイル一式を `src-tauri/resources/bundled/aivis/` へ展開します（手動実行は `npm run setup:aivis`、更新時は `AIVIS_RELEASE=latest` またはタグを指定）。
- **配布**: このフォルダは `tauri.conf.json` の `bundle.resources` 対象なので配布物に含まれます（巨大ファイルやライセンス確認が必要な実体は Git には含めません）。配布後に見つからない場合は、初回の読み上げ起動時に同じリリースを `%APPDATA%\PicoAgent\bundled\aivis\` へ自動取得・展開します（展開には `7z` / `7za` / `tar` のいずれかを使用）。
- **起動**: `PICOAGENT_TTS_ENABLED` が有効ならアプリ起動時に `127.0.0.1:<PICOAGENT_TTS_PORT>`（既定 `11436`）で裏起動します。旧既定の `10102` が保存済みなら起動時に `11436` へ移行します。アプリが起動した子プロセスは終了時に停止します。
- **推論デバイス**: `PICOAGENT_TTS_DEVICE` で `auto` / `cpu` / `gpu` を選べます。`auto` は起動前に GPU 候補を確認し、Windows では NVIDIA / Radeon・Vega・Graphics 表記の AMD APU・表示アダプタ / Intel 系の表示アダプタを候補とし、候補があれば `--use_gpu`、なければ `--no-use_gpu` で起動します（AMD CPU だけの環境は GPU 候補にしません）。固定したい場合は設定画面で明示指定します。
- **話者（speaker ID）**: キャラクターごとに設定画面の「キャラクター」タブで指定でき、未指定なら共通の `PICOAGENT_TTS_SPEAKER` を使います。ただし `りん` は既定で AivisHub の「るな」（モデル UUID `4f281e78-eba6-495a-8e50-5c322d02b5b1`、style ID `-1801897920`）を使い、未導入なら初回読み上げ時に AIVMX を AivisSpeech Engine へ追加します。

> [!NOTE]
> 同梱 Aivis の初回起動はモデル確認や BERT 読み込みで数十秒、初回の読み上げは話者モデルのロードで時間がかかり、音声が出るまで「読み上げ準備中…」になることがあります。

#### ログ

`%APPDATA%\PicoAgent\log\` にまとめて保存します。

| ファイル | 内容 |
|---|---|
| `app.log` | アプリ本体とフロントのエラー |
| `aivis.log` | Aivis の起動ログ（起動失敗時は末尾がエラー表示に含まれる） |
| `engine.log` | 同梱 llama-server のログ |

#### 識別子・データ・署名

- アプリ識別子は `com.picoagent.desktop`（旧 `com.picoagent.app` / `com.coda.app`。初回起動時に旧識別子配下の `config.json` / `history.json` / `memory.json` を現行へコピー移行）。設定・履歴・メモリは `app_config_dir` / `app_data_dir` 配下、同梱系の実行時取得物は `PicoAgent/bundled/`、ログは `PicoAgent/log/` に保存します。アイコンは `src-tauri/icons/`（`npx tauri icon src-tauri/icon-source.png` で再生成）。
- NSIS アンインストーラは関連プロセスを停止後、`%APPDATA%\PicoAgent\` / `%LOCALAPPDATA%\PicoAgent\`、現行・旧識別子の設定・キャッシュ、専用一時ディレクトリを削除します。Aivis 子プロセスの `APPDATA` / `LOCALAPPDATA` は `%APPDATA%\PicoAgent\aivis-userdata\` へ向けるため、Aivis のモデル・辞書・キャッシュもアンインストール対象に含まれます。
- コード署名（Authenticode）・自動更新（`tauri-plugin-updater`）はリリース時に鍵・証明書を用意して有効化します（現状は未設定）。

---

## 📁 フォルダ構成

```text
20_PicoAgent/
├── characters/          ← キャラクター正本（index.json・人格定義・連番フレーム）
├── skills/              ← エージェント機能（スキル）コネクタ（<スキル名>/mod.rs と補助ファイルを同梱）
├── ui/                  ← webview フロント（picoagent.js シム・ペット UI・設定画面）
│                          ※ assets/characters/ は characters/ の配信コピー（自動生成）
├── src-tauri/
│   ├── Cargo.toml / build.rs / tauri.conf.json / capabilities / icons
│   └── src/
│       ├── main.rs / lib.rs / commands.rs / menu.rs
│       ├── config.rs / agent.rs / message.rs
│       ├── providers/   ← 叩く AI コネクタ（.rs を足すと自動登録）
│       └── tools/mod.rs ← スキルの Tool トレイト・レジストリ本体
├── scripts/             ← fetch-engine.mjs / sync-characters.mjs
├── README.md / SPEC.md / CHANGELOG.md / AGENTS.md / CLAUDE.md
```

---

## 🛟 トラブルシューティング

- **「モデルを準備中…」のまま** → 初回のモデル DL 中（数 GB）。ネット接続を確認。`PICOAGENT_MODEL_URL` を変えると別モデルを取得。
- **「準備に失敗」と出る** → `npm run setup:engine` でエンジンが入っているか、DL 元 URL が有効か確認。
- **オンライン表示が赤い / 応答しない（Ollama 利用時）** → `ollama serve` を起動。設定の `OLLAMA_HOST` も確認。
- **AIプロバイダで OpenAI が「未接続」** → `OPENAI_API_KEY` 未設定。設定画面で入力して保存。
- **ツールが呼ばれない** → モデルが function calling 非対応の可能性。`qwen3:8b` などツール対応モデルを使う。
- **`read_file` が拒否される** → 許可ルート（既定はホーム）外。`PICOAGENT_TOOLS_ROOT` を設定するか、ルート配下のパスを指定。
- **Web検索に失敗する** → インターネット接続を確認。短時間の連続検索で拒否された場合は、時間を置いて再試行。
- **天気・為替・RSSに失敗する** → インターネット接続、地点名、通貨コード、RSS URL を確認。
- **外部サービス連携に失敗する** → Slack / Gmail / Google Calendar / Google Docs / Google Drive / Google Sheets / GitHub / Notion / Discord / Microsoft Graph / Jira / Linear / Trello / Todoist の各 token、Webhook URL、OAuth scope、対象IDを確認。秘密値は設定画面に保存し、ログやドキュメントへ値を書かない。
- **SNS連携に失敗する** → X / Mastodon / Bluesky / Reddit / LinkedIn / Misskey の token、app password、投稿権限、APIプラン、インスタンスURL、User-Agent を確認。
- **Office/PDF 操作に失敗する** → `PICOAGENT_DOCUMENT_ROOT` の許可ルート、Microsoft Office + PowerShell、LibreOffice（`soffice`）、qpdf の実行ファイルパスを確認。Google Docs/Sheets はローカルファイルではなく専用APIスキルを使う。
- **`run_command` が拒否される** → 既定では無効。設定で有効化し、`PICOAGENT_COMMAND_ALLOWLIST` に実行したい `program args...` の prefix を追加する。shell は使わず `program` と `args` を分けて実行する。
- **`npm run dev` が遅い / 失敗する** → 初回は Rust のコンパイルに時間がかかります。Rust（cargo）と WebView2 が入っているか確認。
- **ビルドが `os error 32`（別のプロセスが使用中）で失敗する** → 前回の `llama-server` やアプリ本体が残って同梱 DLL を掴んでいる状態。`npm run dev` / `npm run build` は Tauri 起動前に `scripts/stop-stale.mjs` を同期実行して残存プロセスを停止する。手動で止めたいときは `npm run stop`。
- **`npm run dev` 起動直後に `picoagent.exe (exit code: 1)` で落ちる** → 前回起動の `picoagent.exe`（とその WebView2 子プロセス）が残ったまま新インスタンスが起動し、WebView の生成が一時的に競合した状態。`npm run dev` は残存プロセスを停止後、実際に消えるまで待ってから Tauri を起動する。手動停止は `npm run stop`。
