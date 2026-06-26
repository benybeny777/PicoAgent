# 変更履歴

このファイルには、現行アーキテクチャ以降の運用や品質に影響する大きめの変更だけを残します。

---

## 2026-06-26

### ローカルモデル選択と初回自動選定

- `PICOAGENT_LOCAL_MODEL_PRESET` を追加し、設定画面から `auto` / `small` / `balanced` / `large` / `custom` のローカルモデルを選べるようにした。プリセット変更時は同梱エンジンを再起動し、未取得モデルは自動ダウンロードする。
- 初回起動時は `auto` が PC のVRAM/RAM目安からローカルモデルを選び、選定結果を `config.json` へ保存する。ユーザーがプリセットやカスタムURLを明示した場合は自動選定で上書きしない。

### 実行時の設定ガイドと静的検査対応

- 外部サービス系スキルで必要な token / Webhook URL / OAuth access token / 既定ID が未設定だった場合、単に未設定エラーを返すのではなく、設定画面で入力すべきキーと再実行手順を案内するようにした。
- Clippy 指摘（iterator、連続 `replace`、不要 borrow）を修正し、スキル先頭コメントの旧 `tools/*.rs` 表記を現在の `skills/<スキル名>/mod.rs` に合わせた。

### スキル補助ファイルの同梱ルール

- スキル専用の補助モジュール・スクリプト・テンプレートは `skills/<スキル名>/` 配下へ同梱するルールを明文化し、`build.rs` が `skills/` 配下を再帰的に変更監視するようにした。
- `ms_office` のPowerShell実行スクリプトを `mod.rs` の巨大文字列から `skills/ms_office/scripts/office.ps1` へ移し、`include_str!` でスキル本体へ同梱する構成に変更した。

### 自発雑談の話題取得と起動待ち画面

- `chitchat` スキルに `skills/chitchat/trends.rs` を同梱し、自発雑談の素材として X API（Bearer Token 設定時）→ Google Trends RSS → ニュースRSSの順に話題メモを取得するようにした。X API 未設定時も急上昇ワードやトップニュースから話題を拾い、低精度なSNSスクレイピングでX由来と断定しない。
- 設定画面に `PICOAGENT_CHATTER_TOPIC_MODE` / `PICOAGENT_CHATTER_X_QUERY` / `PICOAGENT_CHATTER_TRENDS_RSS_URLS` / `PICOAGENT_CHATTER_NEWS_RSS_URLS` / `PICOAGENT_CHATTER_TOPIC_TIMEOUT_SECS` を追加し、話題ソース・RSS・待ち時間を変更できるようにした。
- 初期起動や音声エンジン待ちの間、通常のフキダシとキャラクターを隠す代わりに、新規生成した横向き寝姿アセット `characters/mugi/boot/sleeping.png` を使った睡眠ループ風ローディング画面を表示するようにした。

### コネクタレジストリの効率化

- プロバイダ/スキルのレジストリを `OnceLock` で静的キャッシュ化し、設定画面表示・会話開始・ツール dispatch のたびに全コネクタを `Box` 再生成しないようにした。スキル名/プロバイダIDのインデックスもキャッシュし、dispatch/get の線形生成探索を避ける。
- `tools::build_schemas()` の結果も初回生成後に再利用し、多数のスキル導入時でも会話ごとの function schema 再構築コストを抑えた。
- 会話時に全スキルの function schema を常時LLMへ渡すのをやめ、`tools::schemas_for_text()` でユーザー発話に合う候補だけを渡すようにした。通常会話のトークン量とツール選択負荷を抑えつつ、`dispatch` 側の全スキル実行能力は維持。

### スキル配置をディレクトリ階層へ変更

- スキルコネクタの配置を `skills/<スキル名>.rs` から `skills/<スキル名>/mod.rs` に変更。`build.rs` は `skills/<スキル名>/mod.rs` を走査して `#[path] pub mod` を生成するため、中央の `mod.rs` 編集不要の自動登録は維持。

### 外部サービス連携とワークスペース操作スキル

- OpenClaw 系の実用スキルに近い外部連携として、Slack 送信スキル `slack_send_message`、Gmail 送信/下書きスキル `gmail_message`、Google Calendar 予定操作 `google_calendar`、Google Docs 文書操作 `google_docs`、Google Drive ファイル操作 `google_drive`、Google Sheets 範囲操作 `google_sheets`、GitHub Issue 操作 `github_issue`、Notion 検索/ページ作成 `notion`、Discord 送信 `discord_send_message`、Microsoft Graph 連携 `microsoft_graph`、Jira `jira`、Linear `linear`、Trello `trello`、Todoist `todoist`、URL本文取得 `web_fetch`、汎用HTTP/API呼び出し `http_api` を追加。各サービスの token / Webhook URL / OAuth access token は設定から都度読み、秘密値をエラーやログへ出さない。
- ワークスペース操作として `workspace_files`（許可ルート内の一覧・読込・書込・追記・フォルダ作成・情報取得）と `run_command`（既定無効、allowlist prefix 一致のみ、shell 不使用、timeout 付き）を追加。危険な削除操作や任意 shell 実行は入れず、設定画面から許可範囲を明示する構成にした。
- Office/PDF 操作として `ms_office`（Microsoft Office COM による Word/Excel/PowerPoint の文字列置換・PDF出力）、`document_convert`（LibreOffice headless による高精度変換）、`pdf_tools`（qpdf によるページ抽出・結合・回転・線形化）を追加。低精度なOOXML直接編集は採用せず、専用API/専用エンジンを使うルートに寄せた。
- 普段使い向けに `weather`（Open-Meteo 天気）、`currency_rates`（Frankfurter 為替換算）、`rss_feed`（RSS/Atom 更新取得）、`google_tasks`（Google Tasks の一覧・作成・完了）を追加。天気・為替・RSSはAPIキーなしで使える。
- SNS系スキルとして `x_twitter`（X API v2 投稿・検索）、`mastodon`（投稿・タイムライン・検索）、`bluesky`（AT Protocol 投稿・検索・プロフィール）、`reddit`（投稿・hot取得・コメント）、`linkedin`（プロフィール取得・UGC投稿）、`misskey`（ノート投稿・タイムライン・ユーザー検索）を追加。各サービスの token / app password は設定から都度読み、秘密値をエラーやログへ出さない。

### 会話履歴の永続化と直近会話の文脈復元

- 会話履歴を `app_data_dir/history.json` に永続化する `history.rs` を追加。`pet_send` のたびに「system + クリーンな直近発話（最大24件）」へ圧縮して保存し、起動時に末尾20件を文脈へ復元する。再起動をまたいで直近の会話を踏まえて話せるようにした（対話データなので config.json とは分離）。
- `reset_conversation` を会話保持型に変更（人格・プロンプト変更で直近文脈を失わない）。右クリックメニューに「会話をリセット」を追加し、文脈・履歴ファイル・直近雑談を消して `pet:reset` を通知する。
- 自発雑談の「毎回同じことを言う」対策として、直近の独り言（最大6件）を「避ける文」として生成プロンプトへ渡し、繰り返しを抑制（`AppState.recent_chatter`）。

### アイドル時の自発雑談スキル（chitchat）

- アイドル（待機）中にキャラクターが自発的にひとこと雑談する機能を、スキルコネクタ `skills/chitchat/mod.rs` として追加。設定画面の「スキル」タブで ON/OFF と発話間隔（最短/最長秒）を変更でき、設定値はスキルの `config_schema` で自動表示される（`PICOAGENT_CHATTER_ENABLED` / `PICOAGENT_CHATTER_MIN_SECS` / `PICOAGENT_CHATTER_MAX_SECS`）。
- 生成は `pet_idle_chatter` command が選択中キャラの人格込みで `provider.chat`（tools 無し＝Web検索なし）に依頼し、会話履歴には積まない。フロントは `[min,max]` 秒のランダム間隔で発火し、入力・応答・読み上げ・全画面中は割り込まず、ユーザー操作や設定変更でタイマーを張り直す。
- `Tool` トレイトに `agent_callable`（既定 true）を追加し、`chitchat` のように会話中にモデルへ提示しない内部スキルを `build_schemas` から除外できるようにした。`Field` に `checkbox` 型を明記。

### 会話・フキダシ・起動描画の改善

- むぎの人格から「小さく分けて」「一つずつ」など作業分解を促す重複表現を整理し、同じ言い回しを毎回繰り返さないよう明記（`characters/mugi/character.json`）。同型の表現を多用する応答を抑える。
- フキダシのテキストを選択中はクリックで入力欄へ切り替えないようにし、ドラッグ選択＋コピーをできるようにした（`renderer.js`）。
- Web検索を伴う回答では、事前検索で得た参照元URLのうち本文に未掲載のものを、最終応答の末尾へ `参考:` として確定的に添えるようにした（`agent.rs`）。URL 行は読み上げ対象から除外済み。
- TTS 有効時は同梱 Aivis が応答可能になるまでキャラ・フキダシを含む全 UI の描画を保留し、待機中に挨拶を事前合成しておき、描画した瞬間に挨拶を再生するようにした（`aivis_status` command 追加 / `prepareSpeech`・`playPreparedSpeech` / `renderer.js` / `styles.css`）。TTS 無効時は従来どおり即描画。
- リソースの明示的解放をルール化（`AGENTS.md`）し、合わせて修正: 音声 Object URL を再生失敗時にも解放、同梱エンジン停止時に `kill` 後 `wait` でプロセスハンドルを回収（`engine.rs`）。

### 起動時の挨拶も読み上げ

- 起動時・キャラクター変更時に吹き出しへ出る挨拶（`greeting`）も Aivis で読み上げるようにした（`renderer.js`）。従来は通常回答のみ読み上げていた。

### Aivis 同梱の取りこぼし修正と LLM の GPU 実行

- `tauri.conf.json` の `bundle.resources` を `resources/{engine,aivis}/*` から `**/*` に変更。`*` ではサブフォルダ（Aivis の `engine_internal/` 等）が同梱・dev コピーされず、`run.exe` が `python311.dll` をロードできずに即終了していた不具合を修正した。
- 同梱 llama.cpp を Windows で **Vulkan(GPU) ビルド**取得に変更（`fetch-engine.mjs`）。従来は CPU 専用ビルドのため GPU が一切使われていなかった。
- `engine.rs` の `llama-server` 起動に `-ngl`（既定 `999`=全レイヤ GPU オフロード）と、複数 GPU 環境向けの `--device`（`PICOAGENT_LOCAL_GPU_DEVICE`）を追加。設定キー `PICOAGENT_LOCAL_NGL` / `PICOAGENT_LOCAL_GPU_DEVICE` を `local` プロバイダの設定スキーマに追加した。

### コネクタ自動生成パスを相対化

- `build.rs` が生成するコネクタ `#[path]` を、ビルド生成先から各 `.rs` への相対パスに変更した。作業フォルダ名を変更した後でも、古い絶対パスを含む生成物に引きずられにくくした。

### Aivis 起動状態と GPU 優先を改善

- `PICOAGENT_TTS_DEVICE=auto` を GPU 優先に変更し、CPU 固定を選んだ場合だけ `--no-use_gpu` で Aivis を起動するようにした。
- Aivis の stdout/stderr を `aivis.log` へ保存し、起動直後の終了や ready 待ちタイムアウト時にログ末尾をエラーへ含めるようにした。読み上げ準備中の UI には経過秒を表示する。

---

## 2026-06-25

### 命名を picoagent に統一

- クレート名・バンドル識別子・設定キー接頭辞・内部 JS API・互換シム名を `picoagent` 系へ統一した（dev 実行ファイル `picoagent.exe`、識別子 `com.picoagent.app`、設定キー `PICOAGENT_*`、`window.picoagent`、`ui/picoagent.js`）。`OPENAI_*` / `OLLAMA_*` キーは変更なし。
- 既存環境は `app_config_dir`/`app_data_dir` のフォルダ名と `config.json` のキー接頭辞を新名へ移行して設定・DL 済みモデルを引き継ぐ（再ダウンロード回避）。`README.md` / `SPEC.md` の設定キー一覧も更新。

### dev/build 前に残存プロセスを自動停止（DLL ロック対策）

- 前回の `llama-server` / アプリ本体が同梱 DLL を掴んでビルドが `os error 32` で失敗する問題に対し、`scripts/stop-stale.mjs` を追加し `beforeDevCommand` / `beforeBuildCommand` で dev/build 前に自動停止するようにした。手動停止は `npm run stop`。
- Windows で `beforeDevCommand` の停止処理が新しく起動した dev 用 `picoagent.exe` を巻き込むレースを避けるため、`npm run dev` / `npm run build` の npm スクリプト側で `stop-stale` とキャラクター同期を同期実行してから Tauri を起動する形に変更した。
- アプリ正常終了時は従来どおり `RunEvent::Exit` → `engine::stop` で子プロセスを kill する。

### アプリ名を PicoAgent に変更・キャラをむぎのみに整理

- アプリの表示名を **PicoAgent** に統一した（`productName` / ウィンドウタイトル / 設定ウィンドウ / 右クリックメニュー「終了」/ `package.json` / `Cargo.toml` の説明 / ドキュメント）。入力欄プレースホルダは、既定キャラに話しかける文言として「むぎに話しかける…」にした。
- キャラクターを **むぎのみ** に整理し、`koharu` / `lab_mate` などの旧キャラを削除（`characters/` 実体と `index.json`）。`default` は `mugi`。

### キャラ・スキルをプロジェクト直下へ移動（扱いやすい配置）

- スキル（ツール）の連結ファイルを `src-tauri/src/tools/` から**プロジェクト直下の `skills/`** へ移動した。当時は `skills/<スキル名>.rs` を走査していたが、現在は `skills/<スキル名>/mod.rs` を走査する。`Tool` トレイト・レジストリ本体（`tools/mod.rs`）は src に残し、中央レジストリ編集不要の自動登録は維持。
- キャラクターの正本を `ui/assets/characters/` から**プロジェクト直下の `characters/`** へ移動した。webview は `ui/` 配下しか配信できないため、`scripts/sync-characters.mjs` が `ui/assets/characters/` へ同期し、`npm run dev` / `npm run build` で Tauri 起動前に自動実行する。配信コピー（`ui/assets/characters/`）は `.gitignore` 対象。
- `npm run sync:characters` で手動同期も可能。

### 設定画面に導入済みスキル・キャラクター一覧を表示

- 設定画面を **基本** / **キャラクター** / **音声** / **叩くAI** / **スキル** のタブ構成にし、ジャンルごとに設定項目を探しやすくした。
- AivisSpeech Engine 連携の読み上げ設定を追加した。`PICOAGENT_TTS_ENABLED` が有効な場合、回答を `/audio_query` → `/synthesis` で WAV 化して再生する。接続先と speaker ID は設定画面から変更できる。
- Aivis 読み上げが有効な場合は、音声再生中も会話アニメーションを維持し、再生終了時に待機へ戻すようにした。
- Aivis 未起動時に同梱配置から AivisSpeech Engine を裏起動できるようにした。アプリが起動した Aivis 子プロセスは終了時に停止する。
- `src-tauri/resources/aivis/` を配布リソース対象に追加した。実体バイナリは巨大ファイルとライセンス確認のため Git 管理外にし、置き場だけを管理する。
- 読み上げは同梱版 Aivis だけを使う方針にし、接続先・自動起動・実行ファイルパス・同梱ポートの設定項目を削除した。同梱版は内部固定の `127.0.0.1:10102` で起動する。
- `PICOAGENT_TTS_CHARACTER_OVERRIDES` を追加し、設定画面の「キャラクター」タブでキャラごとの Aivis speaker ID を指定できるようにした。未指定時は共通の `PICOAGENT_TTS_SPEAKER` を使う。
- `npm run setup:aivis` を追加し、AivisSpeech Engine 公式 GitHub Releases から同梱用実行ファイル一式を `src-tauri/resources/aivis/` へ取得できるようにした。`npm run dev` / `npm run build` では起動前に同梱 Aivis がなければ自動取得する。再配布ビルドの再現性のため既定取得版は `1.2.0` に固定し、更新時だけ `AIVIS_RELEASE` で差し替える。
- Aivis 初回起動が数十秒かかる場合に備え、起動待ちを 180 秒へ延長した。
- Aivis 初回合成で話者モデルロードが 30 秒を超える場合があるため、音声クエリ・合成の HTTP タイムアウトも 180 秒へ延長し、フロントに読み上げ準備中/読み上げ中の状態を表示するようにした。
- `PICOAGENT_TTS_ENABLED` が有効な場合はアプリ起動時に同梱 Aivis を即 spawn し、設定画面で ON にした場合も保存後すぐ起動するようにした。
- `PICOAGENT_TTS_DEVICE` を追加し、同梱 Aivis の推論デバイスを自動 / CPU固定 / GPU固定から選べるようにした。自動は安定優先で CPU を使う。
- 読み上げ ON/OFF と Aivis 推論デバイスの変更は、保存時に同梱 Aivis を停止/再起動して即時反映するようにした。
- 設定画面に「導入済みスキル（エージェント機能）」と「導入済みキャラクター」の読み取り専用一覧を追加した。設定項目を持たないツールも含め登録済みのものが一覧表示される。
- `settings_load` のツール情報に `description` を追加し、各スキルの説明を表示できるようにした。
- Ollama を選んだ時はローカルのモデル一覧を読み込み、モデル欄に候補を反映するようにした。保存時は選択モデルを `OLLAMA_MODEL` にも保存する。

### ダブルクリックでフキダシを全画面表示

- キャラクターのダブルクリックで透明ウィンドウがネイティブ最大化され画面が崩れる問題を修正した。`tauri.conf.json` に `maximizable:false` を加えてドラッグ領域ダブルクリックの最大化を無効化した。
- ダブルクリックの挙動を、ウィンドウをフルスクリーン化しその中でフキダシを画面最大化のように広げるトグルに変更した（`getCurrentWindow().setFullscreen` + `.bubble.fullscreen`）。キャラまたはフキダシのダブルクリックで切り替え、最大化中はキャラを薄く背面化する。解除は再度ダブルクリックまたは `Esc`。`capabilities/default.json` に `core:window:allow-set-fullscreen` を追加。
- 拡大時はセッション内の会話履歴をスクロール表示し、最新応答と入力欄は下部に残すようにした。
- 透明・枠なしのペットウィンドウは維持したまま、タスクバーには表示するようにした。

### 新キャラ「むぎ」を既定化

- 添付イラストを透過切り抜きし、`mugi` キャラクターパックとして追加した。
- 待機・思考・会話・落胆の連番フレームを作成し、既存のLive2D風モーションで揺れるようにした。
- 待機・思考・落胆にまばたき、会話中に口パクの表情フレームを追加した。横揺れは抑え、上下の小さな動きで状態差を出す。表情差分は作業用PSD（`characters/mugi/mugi_expressions.psd`）にもまとめ、生成元の確認用シート（`characters/mugi/source-expression-sheet.png`）も保存した。
- むぎの人格設定を、話し方・根拠の扱い・Web検索結果の使い方・作業支援の距離感まで含めて詳細化した。
- キャラクター定義に `assetVersion` を追加し、同名PNGの更新後もWebViewが古いフレームをキャッシュし続けないようにした。`index.json` と `character.json` もキャッシュ無効化付きで取得する。
- キャラクター定義に任意の `frameDurations` を追加し、まばたきだけ短く抜けるようにした。回答は文字を表示している間だけ口パクし、全文表示後に待機へ戻す。
- 考え中・Web検索中のフレームは口を固定し、口パクは最終回答を返している間だけ出るようにした。
- 既定システムプロンプトから「デスクトップペット」としての自己紹介誘導を外し、挨拶・自己紹介では選択中キャラクター本人として名乗るようにした。保存済みの旧既定プロンプトは実行時と設定表示時に新既定へ正規化する。
- `PICOAGENT_CHARACTER` の既定値と初期表示を `mugi` に変更した。

### Live2D風の新キャラ「ラボメイト」を既定化

- 添付イラストから中央の全身キャラを透過切り抜きし、`lab_mate` キャラクターパックとして追加した。
- 待機・思考・会話・落胆の連番フレームと `live2d-*` CSSモーションを組み合わせ、静止絵でもLive2D風に揺れて見えるようにした。
- `PICOAGENT_CHARACTER` の既定値と初期表示を `lab_mate` に変更し、設定画面から従来の `こはる` へ戻せる構成は維持した。

---

## 2026-06-24

### キャラクターパックと新キャラ「こはる」を追加

- キャラクター固定パスを廃止し、一覧JSONとキャラ別 `character.json` から名前・挨拶・人格・アニメーションを読み込む構成へ変更した。フォルダと一覧項目の追加だけでキャラクターを増やせる。
- 設定画面へキャラクター選択とキャラ別の性格・口調上書きを追加した。切替時は見た目とシステムプロンプトの人格設定を一緒に反映する。
- 桜色の髪と紫の瞳を継承した新しいちびキャラ「こはる」を制作し、状態別フレームと呼吸・思考・会話・落胆の滑らかな補間動作を組み合わせた。

### 右クリックメニューを簡素化

- 右クリックメニューを「設定…」「常に手前に表示」「PicoAgent を終了」の3項目だけにした。プロバイダとモデルの設定は設定画面へ集約した。

### Web検索ツールを追加

- APIキーなしでDuckDuckGoの軽量HTML検索を利用する `web_search` ツールを追加した。モデルは最新情報・ニュース・現在の仕様・出典が必要な質問で検索し、タイトル・URL・概要を材料に回答できる。
- 検索語500文字、結果1〜8件、待ち時間3〜60秒、応答32KBの境界を設けた。結果件数と待ち時間は設定画面から変更でき、保存後すぐに反映する。
- 通常の質問では、ローカルモデルの `tool_calls` 対応に依存せず回答前に検索を実行する。検索結果は信頼できない外部データとして区切り、結果内の指示を採用しない。検索失敗も隠さず回答へ反映する。
- 短い挨拶は検索結果に引っ張られて表記解説へ逸れやすいため、検索を省いてキャラクター本人として返す特例にした。
- 検索回答の参照元をMarkdownハイパーリンク形式に統一し、フキダシ上で安全にクリックできるリンクとして描画するようにした。

### フキダシのドラッグリサイズに対応

- フキダシ右下のつまみをドラッグして幅と高さを変更できるようにした。確定したサイズは `config.json` に保存し、次回起動時にも復元する。

### Ollama 非依存化 — gemma4(Gemma 3n E2B) を同梱エンジンで既定化

- 推論エンジン llama.cpp（`llama-server`）をアプリに同梱し、起動時に spawn して OpenAI 互換 API をローカルに立てる新プロバイダ **`local`** を追加し、これを既定（`PICOAGENT_PROVIDER=local`）にした。**Ollama なしで動作**する。
- エンジン本体はリポジトリに含めず `scripts/fetch-engine.mjs`（`npm run setup:engine`）で取得し、`src-tauri/resources/engine/` に配置して `tauri.conf.json` の `bundle.resources` で同梱する。
- モデル GGUF（既定: Gemma 3n E2B の Q4_K_M、約3〜4GB）は配布物に含めず、**初回起動時に `app_data_dir/models/` へダウンロード**する（以後オフライン）。進捗・状態は `engine:status` イベントで UI に表示する（`engine.rs`）。
- `local` プロバイダは OpenAI 互換変換（`openai.rs` の `to_wire`/`from_wire`）を再利用。tools 非対応テンプレートのモデルでも会話が成立するよう、tools 付きで失敗したら tools 無しで再試行する。
- 設定キー追加: `PICOAGENT_MODEL_URL` / `PICOAGENT_LOCAL_PORT` / `PICOAGENT_LOCAL_CTX`。Ollama プロバイダは任意利用として存続。

### Electron を Tauri へ全面移行（Rust バックエンド・製品化対応）

- ランタイムを Electron から **Tauri 2** へ置換し、`electron/` と Node の `src/*.js`、`start.cjs`、electron 依存を削除した。フロントは `ui/` の静的 webview として配信する。
- コネクタ/エージェント/設定/ツールのロジックを **すべて Rust（`src-tauri/`）で再実装**した。`message.rs`（正規形）/ `config.rs`（設定の正本）/ `agent.rs`（ループ）/ `providers/`（ollama・openai）/ `tools/`（time・read_file）/ `commands.rs` / `menu.rs`。
- コネクタの自動登録を `inventory` crate ＋ `build.rs` のモジュール自動生成で実現し、「`.rs` を1枚置くだけ」でコネクタを追加できる思想を Rust でも維持した。
- 旧 `window.picoagent`（Electron preload）を Tauri 上で再現する互換シム `ui/picoagent.js` を追加し、`renderer.js` / `settings.js` をほぼ無改変で流用した。ウィンドウのドラッグは `data-tauri-drag-region` に置換。
- 設定の正本を `app_config_dir/config.json` に移し、優先順位は永続ファイル > 環境変数 > 既定値を踏襲した。
- 製品化対応: `productName=PicoAgent` / `identifier=com.picoagent.app`、アイコン一式（`src-tauri/icons/`）、Windows NSIS インストーラのバンドル設定を追加。署名・自動更新はリリース時設定の道筋として記載。

### 設定ウィンドウを追加し、全設定を GUI 化

- 右クリック →「設定…」で開く設定ウィンドウ（`electron/renderer/settings.*`）を追加し、叩く AI・モデル・システムプロンプト・常に手前・各コネクタの接続先/API キー/許可ルートを GUI で編集できるようにした。保存はアプリ再起動なしで反映する。
- 設定の正本を `src/config.js`（永続ファイル `userData/config.json`）に集約した。優先順位は永続ファイル > 環境変数 > 既定値。環境変数は既定値としての参照のみで、保存先にはしない。
- 各コネクタが自分の設定項目を `configSchema` として申告し、設定画面が動的に描画する仕組みにした。コネクタを足すと設定項目も自動で増える。
- コネクタは設定値を import 時ではなく `config.get()` で都度読むよう変更し、設定変更の即時反映を可能にした。

### 叩く AI とエージェント機能をコネクタ化

- LLM バックエンドを `src/providers/` のプロバイダコネクタとして抽象化し、Ollama 直結をやめた。`src/providers/index.js` がフォルダ内の `*.js` を自動検出して登録する。叩く AI の追加はファイル1枚で済む。
- OpenAI 互換プロバイダ（`src/providers/openai.js`）を同梱した。`OPENAI_BASE_URL` / `OPENAI_API_KEY` / `OPENAI_MODEL` で OpenAI 形式の任意エンドポイントに接続できる。API キー未設定時はメニューに「未接続」と表示し選択不可にする。
- エージェント機能をツールコネクタ（`src/tools/`）として追加できるようにした。`src/tools/index.js` が自動検出し、`buildSchemas()` でモデルへスキーマを渡し、`dispatch()` で実行する。例として `get_time`（現在時刻）と `read_file`（許可ルート制限つきファイル読込）を同梱した。
- エージェントループ（`src/agent.js`）を追加し、`tool_calls` の実行と再問い合わせを `maxSteps` 上限つきで回すようにした。
- コネクタ間の共通言語として正規形メッセージを定義し、プロバイダ固有のワイヤ形式変換を各コネクタ内に閉じ込めた。
- 右クリックメニューに「AIプロバイダ」サブメニューを追加し、叩く AI を切り替えられるようにした。
- ドキュメント体系を整備した（`AGENTS.md` ルール正本 / `CLAUDE.md` リダイレクト / `SPEC.md` 仕様正本 / `CHANGELOG.md` / `README.md`）。
