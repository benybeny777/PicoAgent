# 変更履歴

このファイルには、現行アーキテクチャ以降の運用や品質に影響する大きめの変更だけを残します。

---

## 2026-06-27

### ツンデレキャラ「りん」を追加

- 新キャラクター `rin`（りん）を追加し、黒髪ツインテールのツンデレ作業相棒として人格設定・挨拶・アニメーション定義を登録した。
- `characters/index.json` に `りん` を追加し、設定画面のキャラクター選択から切り替えられるようにした。
- ローディング画面の画像と文言を固定のむぎ表示から選択中キャラクターに合わせるようにし、`りん` 選択時は起動待ち中もりんの画像を表示するようにした。
- `りん` のローディング画像を、むぎと同じ横向き寝姿の専用アセットへ差し替えた。
- `りん` の既定音声を AivisHub の「るな」に設定し、未導入環境では初回読み上げ時に AIVMX を AivisSpeech Engine へ追加するようにした。
- 設定保存時、ローカルモデル関連の値が実際に変わった場合だけ同梱エンジンを再起動するようにし、キャラクター変更だけで「モデルを準備中…」が走らないようにした。
- 同梱 Aivis の既定ポートを既存 Aivis と被りやすい `10102` から PicoAgent 用の `11436` へ変更し、旧既定値が保存済みの場合も起動時に `11436` へ移行するようにした。
- `りん` 選択時にローディング画面の冒頭で一瞬むぎが表示される不具合を修正した（選択中キャラクターの画像が決まるまで起動画像を出さないようにした）。
- 会話履歴をキャラクター別ファイル（`history-<キャラID>.json`）に分け、キャラ変更時は文脈を切替先キャラの履歴へ入れ替えるようにした。これにより旧キャラ（むぎ）の履歴に引きずられて新キャラがむぎとして自己紹介・応答する不具合を修正した。旧 `history.json` は起動時に既定キャラ `mugi` のファイルへ一度だけ移行する（旧履歴はキャラ別管理が無かった時代のもので当時の既定キャラ＝mugi のため。後から追加した rin 等へ割り当てると人格が混ざる）。
- 実行時ログ（`app` / `aivis` / `engine`）を 1 日ごとに日付付きファイル（`app.<YYYY-MM-DD>.log` など）へローテートし、1 週間より古いログを起動時と 1 日ごとに自動削除するようにした。会話履歴はログではないため対象外。
- `りん` の読み上げが出ない不具合を修正した。ハードコードしていた global style ID（`-1801897920`）が AivisSpeech の実 ID（`345585728`）と一致せず、モデル導入後も `/speakers` に見つからず合成へ進めていなかった。話者 UUID＋スタイル名から実 style ID を実行時に `/speakers` から解決する方式へ変更し、ID の取り違えを起こさないようにした。
- 重い設定変更の反映（再起動）を、プロセス再起動から「バックエンド再初期化＋メインウィンドウ webview 再読込」へ変更した。`app.restart()` は `tauri dev` の開発配信サーバが落ちて `127.0.0.1` 接続拒否になるため使わず、`restart_app` がエンジン/Aivis を停止→設定から再起動し、選択中キャラの履歴を読み直したうえで `pet:reload` で起動シーケンスを再実行する。dev / 配布版どちらでもキャラ変更後の反映が壊れないようにした。
- 新キャラクター `mai`（まい）を追加した。落ち着いた紫のドレスが似合う、包容力のあるやさしいおねえさん作業相棒。`characters/index.json` に追加し、設定画面のキャラクター選択から切り替えられる。音声は AivisHub の「まい」（未導入なら初回読み上げ時に AIVMX を AivisSpeech Engine へ追加）。提供画像は背景を透過処理し、現状は単一フレームの静止立ち絵（まばたき・口パク等のアニメーションは未対応）。
- ローディング画面の寝姿画像の「ZZZ」が白背景で見えにくかったため、むぎ・りん共通で見やすい藤色へ recolor した（ZZZ 部分のみ。キャラクター本体は変更なし）。
- 叩く AI・モデル・音声・キャラクターなど重い設定変更を「再起動して反映」する共通機構を追加した。`settings_save` が該当キーの変更で `requiresRestart` を返し、設定画面が確認画面を出して `restart_app`（`app.restart()`）で再起動、モデル取得・エンジン/Aivis 起動・キャラ読込を起動シーケンス（ローディング画面）でまとめて行う。これに伴い、これらの反映を実行中に行わなくしたため、フキダシに「モデルを準備中…」が出なくなった（進捗はローディング画面のみに表示。`local` 起動時はエンジン ready までローディング画面で待機）。
- 設定保存時のローカルモデル再起動判定で `PICOAGENT_LOCAL_CTX` の比較既定値が実際の既定 `8192` とずれて `4096` になっていたのを修正し、値を変えていないのに同梱エンジンが再起動するのを防いだ。

### 配布識別子と旧データ移行を整理

- Tauri の macOS バンドル拡張子と衝突し得る `.app` 終端を避けるため、アプリ識別子を `com.picoagent.app` から `com.picoagent.desktop` へ変更した。
- 初回起動時に旧 `com.picoagent.app` / `com.coda.app` 配下の `config.json` / `history.json` / `memory.json` を現行識別子へコピー移行し、識別子変更で設定・会話履歴・長期メモリを見失わないようにした。
- NSIS アンインストール対象に現行識別子 `com.picoagent.desktop` の設定・キャッシュも追加した。

### 製品レベルの会話・読み上げ品質を改善

- 雑談・感想・相談まで毎回 Web 検索していたのをやめ、`should_web_search` で事実確認・最新情報・定義などを要する質問のときだけ検索するようにした。雑談はキャラ本人として検索せず速く自然に返す（毎回の検索遅延・ノイズ・小型モデルの脱線を解消）。明示的な検索依頼や「最新/ニュース/価格/誰/いつ/とは」等のマーカーがある質問は従来どおり検索する。
- 読み上げ（`speech_text`）からスパークラインのブロック文字や絵文字を除外し、天気・為替チャートの読み上げが記号の羅列にならないようにした。

### エラーの全捕捉ログと拡大フキダシのキャラ被り解消

- バックエンドのパニックを `std::panic::set_hook` で捕捉し `PicoAgent/log/app.log` に記録するようにした（release では stderr が見えないため、異常終了も必ずログに残る）。
- フロントの `window.picoagent` 経由の `invoke` をラップし、呼び出し側で握られたコマンドエラーも含めてすべて app.log へ記録してから再スローするようにした（`pet_log` 自身は再帰回避で除外）。フロントの大域 `error` / `unhandledrejection` ハンドラと合わせ、フロント・バックエンド双方のエラーを取りこぼさない。
- フキダシを拡大（全画面/ドラッグ）したときにキャラクターへ被る問題を解消。全画面表示は右側にキャラ分の幅を空けた左ペインにし、ドラッグ拡大もキャラクター矩形へ侵入しないよう幅・高さを再計算してクランプするようにした（`renderer.js` / `styles.css`）。キャラは薄くせず右側にそのまま表示する。

### NSIS アンインストールと配布前チェックを強化

- NSIS の `installerHooks` を追加し、アンインストール前に PicoAgent / llama-server / PicoAgent 配下の Aivis `run.exe` を停止し、アンインストール後に `%APPDATA%\PicoAgent`、`%LOCALAPPDATA%\PicoAgent`、現行識別子 `com.picoagent.desktop` と旧識別子 `com.picoagent.app` / `com.coda.app`、PicoAgent 用一時ディレクトリを削除するようにした。
- PicoAgent が起動する Aivis 子プロセスの `APPDATA` / `LOCALAPPDATA` を `PicoAgent/aivis-userdata/` へ向け、Aivis のモデル・辞書・キャッシュも PicoAgent のアンインストール対象に含めた。外部の AivisSpeech インストールが使う通常の `%APPDATA%\AivisSpeech-Engine` は削除対象にしない。
- `npm run dev` / `npm run build` で Aivis だけでなく同梱 `llama-server` も事前確認・取得するようにし、クリーン環境の配布ビルドで同梱 LLM が欠ける穴を塞いだ。
- 同梱 Aivis 更新時は配布リソース側の `run.exe` がユーザーデータ側より新しければ `PicoAgent/bundled/aivis/` を上書き更新するようにした。
- アプリ本体・フロント・設定画面・音声再生の主要エラーを `PicoAgent/log/app.log` に記録するようにした。Aivis と llama-server の専用ログは従来どおり `aivis.log` / `engine.log` に分離する。

### release 版の Aivis 読み上げ再生を修正

- Tauri の CSP に `media-src 'self' blob: data:` を追加し、`pet_speak` が返した WAV を `Blob` / `Audio` で再生できるようにした。release / インストーラ版で Aivis 側の合成は成功しているのに音が出ない問題への対応。
- フロントの音声再生失敗を握りつぶさず、ステータスへ理由を表示するようにした。
- 同梱 Aivis は配布リソースから直接起動せず、ユーザー書き込み可能な `PicoAgent/bundled/aivis/` へコピーしてから起動するようにした。旧 `resources/aivis/` / `resources/engine/` に実体がある開発環境では、setup スクリプトが新しい `resources/bundled/` 配下へコピーする。

### 同梱系ランタイム保存先を PicoAgent bundled ディレクトリへ移動

- 実行時に自動取得する Aivis の保存先を旧 `app_data_dir/aivis/` / 旧 `PicoAgent/aivis/` からユーザーデータ配下の `PicoAgent/bundled/aivis/` へ変更した。旧保存先に取得済みの Aivis がある場合は初回に新保存先へ移行する。
- 同梱 local 用モデル GGUF の保存先も `PicoAgent/bundled/models/` へ変更した。旧 `app_data_dir/models/` / 旧 `PicoAgent/models/` に取得済みモデルがある場合は初回に新保存先へ移行する。`llama-server` 本体は実行時配置の `PicoAgent/bundled/engine/` を最優先で探す。
- 開発・配布用の同梱物も `src-tauri/resources/bundled/engine/` と `src-tauri/resources/bundled/aivis/` にまとめ、`tauri.conf.json` の同梱対象を `resources/bundled/**/*` に統一した。旧 `resources/engine/` / `resources/aivis/` は互換候補としてのみ探す。
- 実行時ログも `app_data_dir/log/` ではなく `PicoAgent/log/` へ集約するようにした。旧 `app_log_dir` / `app_data_dir/log` の Aivis ログは新ログ未作成時にコピーする。

### Aivis の実行時自動取得を追加

- 配布後に同梱 Aivis が見つからない場合、初回の読み上げ起動時に公式 GitHub Releases から既定 `1.2.0` を `PicoAgent/bundled/aivis/` へ自動取得して使うようにした。
- 既に同梱または取得済みの Aivis がある場合は追加ダウンロードせず、それを優先して起動する。

### 実行時ログを log ディレクトリへ集約

- Aivis のログ保存先を旧 `app_log_dir/aivis.log` から `PicoAgent/log/aivis.log` へ移し、旧ログは新ログ未作成時にコピーするようにした。
- 同梱 llama-server の stdout/stderr も `PicoAgent/log/engine.log` に保存し、Aivis と推論エンジンのログを同じ `log` ディレクトリで確認できるようにした。

### 天気・為替・市場指数のスキル直行を安定化

- `weather` / `currency_rates` などのスキル直行応答も assistant 発話として会話履歴へ保存し、前回のユーザー発話だけが残って次の質問へ混ざる問題を修正した。
- `market_indices` スキルを追加し、日経平均（「日系平均」表記ゆれ含む）/ TOPIX / NYダウ / NASDAQ / S&P500 はモデル判断を待たずに専用スキルで取得するようにした。

### ローカルモデルを Gemma 4 系へ更新

- `local` プロバイダの `small` / `balanced` / `large` プリセットを Gemma 4 E2B / E4B / 12B の GGUF へ差し替えた。
- 追加プリセット `qwen3` として Qwen3 4B Instruct Q4_K_M を選べるようにした。

### Aivis の推論デバイス判定を改善

- `PICOAGENT_TTS_DEVICE=auto` で同梱 Aivis を起動する際、起動前に GPU 候補を確認し、GPU 候補がなければ最初から CPU 起動（`--no-use_gpu`）を選ぶようにした。Windows では NVIDIA / Radeon・Vega・Graphics 表記のある AMD APU/表示アダプタ / Intel 系の表示アダプタを GPU 候補として扱い、AMD CPU だけの環境は GPU 候補にしない。`cpu` / `gpu` の明示指定は自動判定より優先する。
- 設定画面の「叩くAI」プロバイダ一覧は、まぎらわしさを減らすため既定の `local` を常に先頭に表示するようにした。

### 日常補助スキルと操作系スキル優先ルートを追加

- `reminder` / `focus_timer` / `praise_log` / `daily_briefing` / `clipboard_text` を追加。リマインダー、集中タイマー、作業見守り、できたことメモ、おでかけ前チェック、クリップボード要約に対応した。
- リマインダー・集中タイマー・作業見守りはフロントが期限到来をポーリングし、吹き出し表示と読み上げで通知する。起動時あいさつには時間帯、通知件数、直近のできたことメモを添える。
- 操作系の依頼では事前Web検索を省き、ローカルスキルを優先するようにした。代表的なリマインダー・集中タイマー・見守り・できたことメモ、クリップボード要約、おでかけ前チェックはスキル直結で実行する。
- 設定画面のスキル一覧を「日常」「検索・情報」「ファイル/実行」「連携」「SNS/投稿」「文書」などのカテゴリタブで整理した。

### 目覚ましに起床確認スヌーズを追加

- 目覚まし発火後、フキダシに `おきたよ！` ボタンを表示し、押されるまで既定5分おきに再度起こすスヌーズ動作を追加。スヌーズ分数はアラームごとの `snooze_minutes`（1〜60分）として保存し、設定画面の目覚まし専用フォームでも編集できる。

## 2026-06-26

### 天気・為替をスキル直結でグラフィカル表示

- 小型ローカルモデルが tool_calls を出せず、天気・為替スキルが使われずグラフィカル表示にならなかった問題を修正。`agent.rs` で天気質問（`is_weather_query`）は `weather`、為替質問（`currency_query`）は `currency_rates` を先に実行し、整形済み出力（アイコン・スパークライン）をモデルを介さずそのまま返すようにした。
- 為替は文中の通貨（ドル/円/ユーロ等）と「推移/チャート」「N日」を読み取って `base`/`symbols`/`chart`/`days` を組み立てる。為替の通常レートは生JSONではなく整形して表示。
- 天気は agent が地名を抽出して渡し、解決失敗時は現在地（IP）→既定地点へフォールバックするようにした。

### 為替チャート・天気のグラフィカル表示・現在地自動取得

- 為替（`currency_rates`）に推移チャートを追加。`chart=true` または `days` 指定で Frankfurter の時系列を取得し、スパークライン（▁▂▃▅▇）＋始値/終値/最安/最高/変化率を返す。
- 天気（`weather`）をグラフィカル化。天気アイコン（☀️🌧️❄️等）＋今日の気温の時間別スパークライン＋日別予報（アイコン/最高最低/降水確率）＋最高気温の推移スパークラインで表示。
- 天気の地名未指定時、IP から現在地（緯度経度・市名）を自動推定するようにした（`ip-api.com`）。失敗時は既定地点へフォールバック。`PICOAGENT_WEATHER_DISABLE_AUTO_LOCATION` で無効化可能。

### Gemini プロバイダを追加

- 叩く AI に `gemini` プロバイダ（`providers/gemini.rs`）を追加。Google の OpenAI 互換エンドポイント（`/v1beta/openai`）を使い、`openai.rs` の `to_wire`/`from_wire` を再利用する（裏は OpenAI 互換と同じ）。設定キー `PICOAGENT_GEMINI_API_KEY` / `PICOAGENT_GEMINI_MODEL`（既定 `gemini-2.5-flash`）/ `PICOAGENT_GEMINI_BASE_URL` を追加。API キー設定時のみ接続する。

### 自発雑談に出典URLを添える

- 雑談がニュース等の話題に触れたとき、出典の記事URLを末尾へ `参考:` として添えるようにした（例: 地震の独り言なら地震ニュースのURL）。`trends.rs` が記事タイトルとURLを返すようにし、`pet_idle_chatter` が話題メモを取得して skill へ `topic` 引数で渡し、返信と記事タイトルの内容バイグラム一致で最も関連するURLを選ぶ。URL 行は読み上げ対象外。

### 自発雑談の時間整合とオウム返し防止

- 自発雑談の生成を「人格(system)＋現在時刻＋指示」だけで行い、ユーザー発話（会話履歴）を文脈に含めないようにした。小型モデルがユーザー入力をオウム返しして読み上げる挙動を解消（`pet_idle_chatter`）。
- `current_time_context()` で現在のローカル日時・時間帯（朝/昼/夕方/夜）を雑談へ渡し、夜なのに「日差し」など時間に合わない独り言を出さないようにした。
- 通常会話でも、返信がユーザー入力のオウム返しになった場合は読み上げないようにした（`renderer.js`）。

### Gemma の「ロール交互」400 エラーを修正

- 自発雑談が `assistant` 発話を履歴へ積むことや履歴復元により、会話が「assistant 連続」「先頭 assistant」になり、Gemma のチャットテンプレートが 400「Conversation roles must alternate user/assistant」を返してチャットが失敗していた不具合を修正。`local` プロバイダ送信直前に `normalize_for_local` で user/assistant の交互・user 始まりへ正規化する（system を先頭集約、先頭の純 assistant 除去、連続同 role を連結）。実機で「連続assistant→400 / 正規化後→200」を確認。

### コンテキスト拡大と叩くAI/モデル選択のUI整理

- ローカルエンジンの既定コンテキスト長を `4096` → `8192` に拡大（`PICOAGENT_LOCAL_CTX` 既定とプリセット ctx、`engine.rs`/`local.rs`）。Web検索結果を含む長めの文脈でも切り詰めにくくした。
- 設定画面の「叩くAI」コネクタ設定を、選択中プロバイダの設定だけ表示するようにした（切替で表示も追従）。
- 基本タブの「モデル」欄を local では非表示にした（モデルはプリセットで選ぶため。候補にエンジン報告の生IDなど不要項目が混ざるのを解消）。ollama/openai では従来どおり表示。

### 目覚ましの専用UIとアプリアイコン更新

- 目覚まし時計の設定を、設定画面で時刻ピッカー＋曜日チェック＋繰り返し/有効/ラベル/追加/削除の専用フォームにした（`settings.js` の `buildAlarmEditor`、`settings.css`）。JSON直書きは不要になり、保存時に `PICOAGENT_ALARMS` を組み立てる。
- アプリ／タスクバーのアイコンをむぎの立ち絵に更新（`icon-source.png` を差し替え、`npx tauri icon` で各サイズ・`icon.ico` を再生成）。

### ローカルモデル変更時の自動DLと設定画面のスキル設定整理

- 設定画面の「モデル」欄（`PICOAGENT_MODEL`）でローカルのプリセット（small/balanced/large）を切り替えたとき、取得URLにその選択を反映し（`local::model_url`）、`settings_save` の再起動トリガーに `PICOAGENT_MODEL` を追加。これにより未取得モデルへ切り替えると自動でダウンロード→エンジン再起動するようにした（従来はプリセット欄/URL変更時のみ）。
- 設定画面のスキル設定を、下部の長い一覧から「スキル一覧をクリックでその場に展開」するアコーディオン方式へ変更（`settings.js` / `settings.css`）。設定を持つスキルには ⚙ を表示し、クリックでそのスキルの設定だけを開ける。

### 目覚まし時計スキル（声でやさしく起こす）

- 目覚まし時計を追加。中核 `src-tauri/src/alarm.rs`（config `PICOAGENT_ALARMS` に保存、曜日/時刻の正規化）＋チャット用スキル `skills/alarm/mod.rs`（add/list/remove/clear/enable/disable）で構成。「平日の7時に起こして」のようにチャットで設定でき、設定画面の「スキル」タブから JSON でも編集できる。
- フロントは約20秒間隔で `pet_alarms` を照合し、現在のローカル時刻・曜日が一致したら `pet_alarm_wake` を呼んで、選択中キャラの口調の起こし文を音声合成→表示→即時再生で**声でやさしく起こす**（生成失敗時は固定の起こし文）。同じ分の二重発火を防ぎ、一度きり（`repeat=false`）は発火後に無効化する。
- `schemas_for_text` に alarm のキーワードルート（アラーム/目覚まし/起こして/何時に 等）を追加。

### ローカルモデル選択と初回自動選定

- `PICOAGENT_LOCAL_MODEL_PRESET` を追加し、設定画面から `auto` / `small` / `balanced` / `large` / `custom` のローカルモデルを選べるようにした。プリセット変更時は同梱エンジンを再起動し、未取得モデルは自動ダウンロードする。
- 初回起動時は `auto` が PC のVRAM/RAM目安からローカルモデルを選び、選定結果を `config.json` へ保存する。ユーザーがプリセットやカスタムURLを明示した場合は自動選定で上書きしない。

### 実行時の設定ガイドと静的検査対応

- 外部サービス系スキルで必要な token / Webhook URL / OAuth access token / 既定ID が未設定だった場合、単に未設定エラーを返すのではなく、設定画面で入力すべきキーと再実行手順を案内するようにした。
- Clippy 指摘（iterator、連続 `replace`、不要 borrow）を修正し、スキル先頭コメントの旧 `tools/*.rs` 表記を現在の `skills/<スキル名>/mod.rs` に合わせた。
- 回答末尾の `参考:` / `参考：` ブロックは表示には残しつつ、読み上げ用テキストからは除外するようにした。
- 自発雑談を生成したあと、直前の独り言を会話文脈へ短く残すようにした。ユーザーが雑談内容について質問したとき、モデルが直前の雑談を参照して回答できる。
- `app_data_dir/memory.json` に暗黙的な長期メモリを追加し、ユーザーの好み・自己情報・継続指示らしい発話だけを保守的に保存して system 文脈へ反映するようにした。設定画面で有効/無効と最大件数を変更でき、会話リセットで履歴と一緒に削除する。

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
- 既存環境は `app_config_dir`/`app_data_dir` のフォルダ名と `config.json` のキー接頭辞を新名へ移行して設定を引き継ぐ。DL 済みモデルは現行では `PicoAgent/bundled/models/` へ移行して再ダウンロードを避ける。`README.md` / `SPEC.md` の設定キー一覧も更新。

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
- `src-tauri/resources/bundled/aivis/` を配布リソース対象に追加した。実体バイナリは巨大ファイルとライセンス確認のため Git 管理外にし、置き場だけを管理する。
- 読み上げは同梱版 Aivis だけを使う方針にし、接続先・自動起動・実行ファイルパス・同梱ポートの設定項目を削除した。同梱版は内部固定の `127.0.0.1:10102` で起動する。
- `PICOAGENT_TTS_CHARACTER_OVERRIDES` を追加し、設定画面の「キャラクター」タブでキャラごとの Aivis speaker ID を指定できるようにした。未指定時は共通の `PICOAGENT_TTS_SPEAKER` を使う。
- `npm run setup:aivis` を追加し、AivisSpeech Engine 公式 GitHub Releases から同梱用実行ファイル一式を `src-tauri/resources/bundled/aivis/` へ取得できるようにした。`npm run dev` / `npm run build` では起動前に同梱 Aivis がなければ自動取得する。再配布ビルドの再現性のため既定取得版は `1.2.0` に固定し、更新時だけ `AIVIS_RELEASE` で差し替える。
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

### Ollama 非依存化 — 同梱エンジンを既定化

- 推論エンジン llama.cpp（`llama-server`）をアプリに同梱し、起動時に spawn して OpenAI 互換 API をローカルに立てる新プロバイダ **`local`** を追加し、これを既定（`PICOAGENT_PROVIDER=local`）にした。**Ollama なしで動作**する。
- エンジン本体はリポジトリに含めず `scripts/fetch-engine.mjs`（`npm run setup:engine`）で取得し、`src-tauri/resources/bundled/engine/` に配置して `tauri.conf.json` の `bundle.resources` で同梱する。
- モデル GGUF は配布物に含めず、**初回起動時にユーザーデータ配下の `PicoAgent/bundled/models/` へダウンロード**する（以後オフライン）。進捗・状態は `engine:status` イベントで UI に表示する（`engine.rs`）。
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
