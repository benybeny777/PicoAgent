# PicoAgent 設定キーリファレンス

この文書は、永続設定と環境変数で使う `PICOAGENT_*` キーの唯一の一覧です。設定機構、優先順位、設定画面の動作仕様は [SPEC.md](../SPEC.md#設定config-と設定画面)、日常的に使う項目は [README.md](../README.md#️-設定) と [MANUAL.md](../MANUAL.md#14-主要設定リファレンス) を参照してください。

設定キーを追加・変更・削除したときは、この一覧を同じPRで更新します。`doc_sync::guard` が実装との追加漏れ・削除漏れを双方向で検出します。

## 設定キー一覧

| キー | 既定 | 区分 | 用途 |
|---|---|---|---|
| `PICOAGENT_PROVIDER` | `local` | 基本 | 選択中の叩く AI の id |
| `PICOAGENT_MODEL` | プロバイダ既定 | 基本 | 選択中モデル |
| `PICOAGENT_SYSTEM_PROMPT` | 既定プロンプト | 基本 | system メッセージ |
| `PICOAGENT_THEME` | `system` | 基本 | 表示テーマ。`system` はOS/システム設定に追従、`light` は通常、`dark` はダーク。`pet:ui` で即反映 |
| `PICOAGENT_BUBBLE_FONT_PX` | `13` | 基本 | フキダシの文字サイズ(px)。`pet:ui` で即反映 |
| `PICOAGENT_BUBBLE_WIDTH_PX` | 自動 | 内部UI | フキダシ左上つまみのドラッグで自動保存する幅(px)。設定画面には出さない |
| `PICOAGENT_BUBBLE_HEIGHT_PX` | 自動 | 内部UI | フキダシ左上つまみのドラッグで自動保存する高さ(px)。設定画面には出さない |
| `PICOAGENT_BUBBLE_LEFT_PX` | 自動 | 内部UI | フキダシ左上つまみのドラッグで自動保存する left(px)。設定画面には出さない |
| `PICOAGENT_BUBBLE_TOP_PX` | 自動 | 内部UI | フキダシ左上つまみのドラッグで自動保存する top(px)。設定画面には出さない |
| `PICOAGENT_WINDOW_WIDTH_PX` | （旧キー） | 内部UI | 旧版の共通ウィンドウ幅。現在は表示形式別の `PICOAGENT_WINDOW_LAYOUTS` を正本にする |
| `PICOAGENT_WINDOW_HEIGHT_PX` | （旧キー） | 内部UI | 旧版の共通ウィンドウ高さ。現在は表示形式別の `PICOAGENT_WINDOW_LAYOUTS` を正本にする |
| `PICOAGENT_WINDOW_LEFT_PX` / `PICOAGENT_WINDOW_TOP_PX` | 自動 | 内部UI | ウィンドウ左上の**仮想デスクトップ上の物理座標**(px)。アプリ終了時に保存し、起動時に復元する。マルチモニタでも同じディスプレイの同じ位置へ戻る。現在のディスプレイ構成のどこにも載らない場合は既定位置へ戻す |
| `PICOAGENT_WINDOW_RIGHT_PX` / `PICOAGENT_WINDOW_BOTTOM_PX` | （旧キー） | 内部UI | 旧版が保存していた画面右端・下端からの相対オフセット。モニタ原点を持たないためマルチモニタでどのディスプレイに居たかを復元できず、絶対座標へ置き換えた。現在は書き込まず、「レイアウトを初期化」で他のレイアウトキーと一緒に掃除するだけ |
| `PICOAGENT_ALWAYS_ON_TOP` | `true` | 基本 | 常に手前に表示 |
| `PICOAGENT_GAZE_TRACKING_ENABLED` | `false` | キャラクター | 視線追従。OFFではPNGの自動よそ見・マウス方向の顔差分と傾き・Live2Dのマウス追従を止め、常に正面へ戻す。ONでは顔付近のカーソルだけへ控えめに追従し、画面下など顔から離れた位置では追従しない。`pet:ui` で即反映 |
| `PICOAGENT_CHARACTER_ZOOM` | `1.55` | 内部UI | 構図未設定時に使う互換用の共通拡大率。新しい調整は `PICOAGENT_CHARACTER_FRAMING` を使う |
| `PICOAGENT_CHARACTER_FRAMING` | `{}` | 画面 | キャラクターID×表示形式ごとの構図。`preset`（`bust` / `upper` / `full` / `custom`）、`zoom`、`x`、`y`を保持する。未設定時は通常2Dが従来の上半身 `upper`、長身・実写・Live2Dが胸上 `bust` を使う。旧 `talk` の保存値は `chat` へ移す |
| `PICOAGENT_CHARACTER_OFFSET_X` | `0` | 内部UI | 旧版の立ち絵横移動量（px）。通常ドラッグはウィンドウ移動へ戻したため新規には書き込まず、既存値の互換表示と位置リセットだけに使う |
| `PICOAGENT_CHARACTER_OFFSET_Y` | `0` | 内部UI | 旧版の立ち絵縦移動量（px）。同上 |
| `PICOAGENT_LAYOUT` | `pet` | 画面 | 表示形式（画面構成）。`mascot` / `pet` / `chat` / `bar` / `workspace` / `stream` / `widget`。旧 `talk` はLINE風の `chat` へ移行し、未知の値は `pet` へ丸める。設定画面のプレビューカードと右クリックメニュー「画面レイアウト」の両方から変更でき、再起動なしで反映する |
| `PICOAGENT_STREAM_BACKGROUND` | `pastel` | 画面 | 配信形式の背景。`pastel` / `gaming` / `simple` / `custom` |
| `PICOAGENT_STREAM_BACKGROUND_IMAGE` | 空 | 画面 | `custom` 配信背景としてアプリ管理領域へ取り込んだ画像パス |
| `PICOAGENT_STREAM_DISPLAY` | `dashboard` | 画面 | 配信背景内ディスプレイ。`dashboard` / `game` / `clock` / `image` / `none` |
| `PICOAGENT_STREAM_DISPLAY_IMAGE` | 空 | 画面 | `image` ディスプレイへ映す、アプリ管理領域へ取り込んだ画像パス |
| `PICOAGENT_STREAM_PROFILES` | `{}` | キャラクター | キャラクターID別の配信表示上書き。背景、背景画像、画面内ディスプレイ、表示画像、字幕色を保持し、未設定キャラは共通の配信設定を使う。不正なID・選択肢・色は保存時に除去または安全な既定値へ正規化する |
| `PICOAGENT_WINDOW_LAYOUTS` | `{}` | 内部UI | 表示形式ごとに覚えたウィンドウ寸法（`{"chat":{"w":520,"h":760}}`）。既定はミニ360×560／ペット900×600／チャット520×760／バー900×320／作業1400×850／配信1920×1080（フルHD）／ウィジェット440×600。旧 `talk` の保存寸法は `chat` の値がない場合に引き継ぐ。旧既定値と完全一致する保存値だけ新既定へ移し、手動調整値は保持する。位置は形式をまたいで共有する。設定画面には出さない |
| `PICOAGENT_HOTKEY_ENABLED` | `true` | 基本 | グローバルホットキーを使う。OFF ですべてのホットキーを解除する |
| `PICOAGENT_HOTKEY_SUMMON` | `CommandOrControl+Alt+P` | 基本 | 呼び出しキー。表示・前面化してから `pet:hotkey` を送り、フロントが居眠り解除と入力欄オープンを行う。空欄は他の欄と同じく既定値、`off` でこのキーだけ無効 |
| `PICOAGENT_HOTKEY_HIDE` | `CommandOrControl+Alt+H` | 基本 | 隠す／戻すキー。表示中は `hide()`、隠れているときは呼び出しと同じ動作。空欄は他の欄と同じく既定値、`off` でこのキーだけ無効 |
| `PICOAGENT_BOOT_TIMEOUT_MS` | `120000` | 内部UI | 開始後の起動進捗通知がこの時間だけ途絶えた場合に、キャラクター付きの案内画面へ切り替える。モデル・音声のダウンロード中も10秒ごとに進捗通知するため、準備の総時間が2分を超えても停止扱いにしない。読込開始（`pet_frontend_ready`）だけは12秒（この値が小さい場合はその値）で判定し、画面を出せない障害の案内を待たせない。設定画面には出さない（`3000`〜`120000` に丸める） |
| `PICOAGENT_NAP_MINUTES` | `15` | 基本 | 無操作でキャラが居眠りするまでの分数（`0` で居眠りしない） |
| `PICOAGENT_QUICK_ACTIONS` | 空 | 基本 | 入力欄上のクイック操作を上書きする（1行1件「ラベル\|送る文章」。空なら既定の8件） |
| `PICOAGENT_UPDATE_CHECK_ENABLED` | `true` | 基本 | 起動時に公開配布Gitの `latest.json` を確認し、新しければ通知する。オフライン時は通知しない |
| `PICOAGENT_MEMORY_ENABLED` | `true` | 基本 | 会話から好み・自己情報・継続指示を暗黙的に長期メモリへ保存する |
| `PICOAGENT_PROJECT_RULES_AUTO_CAPTURE` | `false` | 基本 | 強く言い渡された作法を作業フォルダの `AGENTS.md` へ自動で追記する（既定は無効。利用者のフォルダにあるファイルを書き換えるため） |
| `PICOAGENT_PLANNING_ENABLED` | `false` | 基本 | 複雑な質問（120文字超・疑問符2個以上・手順系語）をサブタスクへ自動分解するプランニングステップを有効化する |
| `PICOAGENT_SELF_EVAL_ENABLED` | `false` | 基本 | 回答が短すぎる・拒否パターンを含む場合に1回再生成する自己評価ステップを有効化する |
| `PICOAGENT_SUPER_AGENT` | `false` | 基本 | スーパーエージェントモード。計画ステップ・自己評価を一括有効化し、ループ上限を引き上げ、進捗を `pet:agent_status` で可視化する |
| `PICOAGENT_SUPER_AGENT_MAX_STEPS` | `12` | 基本 | スーパーエージェントモードのトップレベル最大ループ数（呼び出し値〜30 にクランプ） |
| `PICOAGENT_MULTI_AI_MODE` | `single` | 複数AI | `single` / `consensus` / `collaborate` / `divide`。既定は従来どおり単発。単発以外は Pro 対象の `subagent` を通るため、Pro でなければ保存を拒否し、実行時にライセンスが切れていれば理由を画面へ返す。参加AI設定が壊れていても`single`へ戻す保存は受け付ける |
| `PICOAGENT_MULTI_AI_AGENTS` | `[]` | 複数AI | 参加AI（name / provider / model / focus）のJSON配列。複数モードでは2〜8件 |
| `PICOAGENT_MULTI_AI_JUDGE_PROVIDER` | 現在のプロバイダ | 複数AI | 分担計画と回答統合を行うまとめ役のプロバイダ |
| `PICOAGENT_MULTI_AI_JUDGE_MODEL` | 空 | 複数AI | まとめ役のモデル。空欄のときは、まとめ役が選択中と同じプロバイダなら現在のモデル、違うプロバイダならそのプロバイダの既定モデルを使う（別プロバイダへ存在しないモデル名を渡さない） |
| `PICOAGENT_MULTI_AGENT_MAX_CONCURRENCY` | `3` | 複数AI | 外部AI候補の最大同時数（1〜8）。内蔵local候補は常に直列 |
| `PICOAGENT_SUBAGENT_MAX_DEPTH` | `2` | スキル(subagent) | サブエージェントの最大ネスト深さ。これ以上は再帰委譲を拒否する |
| `PICOAGENT_MEMORY_MAX_ITEMS` | `80` | 基本 | 長期メモリに保持する最大件数（10〜200） |
| `PICOAGENT_RELATIONSHIP` | `assistant` | 基本 | ユーザーとの関係（`assistant` / `friend` / `partner` / `sister_younger` / `sister_older` / `senpai` / `custom`）。人格の後ろへ差分として重ねる |
| `PICOAGENT_RELATIONSHIP_OVERRIDES` | `{}` | 基本 | キャラクターIDごとの関係の上書き（`{"mai":"sister_older"}`）。全体設定より優先する |
| `PICOAGENT_RELATIONSHIP_CUSTOM` | （空） | 基本 | `PICOAGENT_RELATIONSHIP=custom` のときに人格へ足す自由記述 |
| `PICOAGENT_CHARACTER` | `mugi` | 基本 | 選択中キャラクターのID |
| `PICOAGENT_CHARACTER_OVERRIDES` | `{}` | 基本 | キャラクターIDごとの人格設定上書き |
| `PICOAGENT_LIVE2D_SDK_OVERRIDES` | `{}` | 基本 | 複数世代のモデルを持つLive2Dキャラクターについて、キャラクターIDごとに `"4"` / `"5"` を固定する。未指定は `auto` |
| `PICOAGENT_COMPANION_MODE` | `both` | 基本 | 相棒の用途（`life` / `work` / `both`）。初回案内では質問せず`both`を保存し、設定画面で変更する |
| `PICOAGENT_RELATIONSHIP_MEMORY_ENABLED` | `true` | 基本 | キャラクター別の関係性と共有作業記録を保存する。OFFでは既存データを消さず、更新とAI文脈への差し込みを止める |
| `PICOAGENT_ONBOARDING_DONE` | `false` | 基本 | 初回3択案内の完了状態 |
| `PICOAGENT_TTS_ENABLED` | `false` | 音声 | Aivis で回答を読み上げる |
| `PICOAGENT_TTS_DEVICE` | `auto` | 音声 | 同梱 Aivis の推論デバイス（内部設定。通常UIでは非表示） |
| `PICOAGENT_TTS_PORT` | `11436` | 音声 | 同梱 Aivis のポート（内部設定。通常UIでは非表示。旧既定 `10102` は起動時に `11436` へ移行） |
| `PICOAGENT_TTS_SPEAKER` | `888753760` | 音声 | Aivis の speaker/style ID（内部設定。通常UIでは非表示） |
| `PICOAGENT_TTS_VOLUME` | `50` | 音声 | キャラボイス再生音量（0〜100）。フロントで即反映。ミュートまたは0では全キャラの口パクも停止 |
| `PICOAGENT_TTS_CHARACTER_OVERRIDES` | `{}` | 音声 | キャラクターIDごとの Aivis speaker/style ID 上書き（「キャラの音声」で編集。任意で `modelUuid` / `modelName` / `speakerUuid` / `styleName` / `sourceUrl` も保持） |
| `PICOAGENT_LOCAL_MODEL_PRESET` | `auto`（初回に `lfm25_2_6b` を保存） | local | ローカルモデルプリセット（`lfm25_2_6b`: LFM2.5 2.6B QAD/約1.59GB、`small`: Gemma 4 E2B/約3.1GB、`balanced`: Gemma 4 E4B/約5.0GB、`large`: Gemma 4 12B/約7.1GB、`qwen35_4b`: Qwen3.5 4B/約2.7GB、`qwen35_9b`: Qwen3.5 9B/約5.7GB、`gpt_oss_20b`: OpenAI gpt-oss-20b/約12.1GB、`phi4mini`: Phi-4 mini/約2.5GB、互換用`lfm25_1_2b_jp`／`llmjp4_8b`、`custom`）。既存の明示選択は変更せず、旧プリセットIDは現行候補へ自動移行する |
| `PICOAGENT_MODEL_URL` | LFM2.5 2.6B QAD Q4_0 GGUF | local | 同梱エンジンが取得するモデル URL |
| `PICOAGENT_LOCAL_PORT` | `11435` | local | 同梱エンジンのポート |
| `PICOAGENT_LOCAL_CTX` | `8192` | local | コンテキスト長 |
| `PICOAGENT_LOCAL_NGL` | `999` | local | GPU へオフロードするレイヤ数（`999`=全レイヤ優先 / `0`=CPU 固定） |
| `PICOAGENT_LOCAL_GPU_DEVICE` | （空=既定先頭） | local | 使用 GPU の指定（複数 GPU 環境向け、例 `Vulkan1`）。保存名は同じバックエンド候補だけへ渡し、CUDA等の別候補は各方式の既定先頭デバイスで起動する |
| `OLLAMA_HOST` | `http://127.0.0.1:11434` | ollama | Ollama 接続先 |
| `OLLAMA_MODEL` | `qwen3:8b` | ollama | Ollama 既定モデル |
| `OPENAI_BASE_URL` | `https://api.openai.com/v1` | openai | OpenAI 互換エンドポイント |
| `OPENAI_API_KEY` | （なし） | openai | OpenAI 互換の API キー。未設定なら選択不可 |
| `OPENAI_COMPAT_API_KEY` | （なし） | openai | `OPENAI_BASE_URL` がOpenAI公式以外の場合だけ使う互換先専用APIキー |
| `OPENAI_MODEL` | `gpt-5.4-mini` | openai | OpenAI 互換の既定モデル |
| `PICOAGENT_ANTHROPIC_API_KEY` | （なし） | anthropic | Anthropic API キー。未設定なら選択不可 |
| `PICOAGENT_ANTHROPIC_MODEL` | `claude-sonnet-4-6` | anthropic | Claude の既定モデル |
| `PICOAGENT_ANTHROPIC_BASE_URL` | `https://api.anthropic.com/v1` | anthropic | Anthropic Messages API のエンドポイント |
| `PICOAGENT_DEEPSEEK_API_KEY` | （なし） | deepseek | DeepSeek API キー。未設定なら選択不可 |
| `PICOAGENT_DEEPSEEK_MODEL` | `deepseek-v4-flash` | deepseek | DeepSeek の既定モデル |
| `PICOAGENT_DEEPSEEK_THINKING` | `true` | deepseek | DeepSeek V4 系で thinking mode を有効にする |
| `PICOAGENT_DEEPSEEK_BASE_URL` | `https://api.deepseek.com` | deepseek | DeepSeek OpenAI 互換エンドポイント |
| `PICOAGENT_GEMINI_API_KEY` | （なし） | gemini | Gemini API キー。未設定なら選択不可 |
| `PICOAGENT_GEMINI_MODEL` | `gemini-2.5-flash` | gemini | Gemini の既定モデル |
| `PICOAGENT_GEMINI_BASE_URL` | `https://generativelanguage.googleapis.com/v1beta/openai` | gemini | Gemini の OpenAI 互換エンドポイント |
| `PICOAGENT_XAI_API_KEY` | （なし） | xai | xAI API キー。未設定なら選択不可 |
| `PICOAGENT_XAI_MODEL` | `grok-4.3` | xai | xAI Grok の既定モデル |
| `PICOAGENT_XAI_BASE_URL` | `https://api.x.ai/v1` | xai | xAI OpenAI 互換エンドポイント |
| `PICOAGENT_MISTRAL_API_KEY` | （なし） | mistral | Mistral API キー。未設定なら選択不可 |
| `PICOAGENT_MISTRAL_MODEL` | `mistral-medium-latest` | mistral | Mistral の既定モデル |
| `PICOAGENT_MISTRAL_BASE_URL` | `https://api.mistral.ai/v1` | mistral | Mistral OpenAI 互換エンドポイント |
| `PICOAGENT_ALLOWED_ROOT` | ユーザーホーム | 許可（共通） | 全スキル共通の主たる許可ルート（相対パスの基準）。ファイル/ディレクトリ/コマンド実行はここを参照する |
| `PICOAGENT_WORKFLOWS` | `[]` | workflow | 名前付き保存ワークフロー一覧（JSON文字列、1件最大8手順） |
| `PICOAGENT_ALLOWED_PATHS` | （空） | 許可（共通） | 追加の許可ルート（改行または `;` 区切り）。run_command の「永続許可」でも追記される |
| `PICOAGENT_DENIED_PATHS` | （空） | 許可（共通） | 追加の拒否パス（改行または `;` 区切り）。この配下は常に操作を拒否する。`C:\Windows` / `Program Files` は設定に関わらず常に拒否 |
| `PICOAGENT_CODE_RUNNER_ENABLED` | `false` | code_runner | 自力実行（PowerShell/Python のコード実行）を有効化する。任意コード実行のため既定は無効 |
| `PICOAGENT_CODE_RUNNER_PYTHON` | `python` | code_runner | code_runner が使う Python 実行ファイル |
| `PICOAGENT_CODE_RUNNER_TIMEOUT_SECS` | `60` | code_runner | code_runner の既定タイムアウト秒（1〜300） |
| `PICOAGENT_TOOLS_ROOT` | （旧キー） | read_file | 旧キー。`PICOAGENT_ALLOWED_ROOT` へ統合。設定済みなら許可ルートとして引き継ぐ |
| `PICOAGENT_WEB_SEARCH_MAX_RESULTS` | `5` | web_search | Web検索で返す件数（1〜8） |
| `PICOAGENT_WEB_SEARCH_TIMEOUT_SECS` | `15` | web_search | Web検索の待ち時間（秒、3〜60） |
| `PICOAGENT_RESEARCH_TIMEOUT_SECS` | `15` | research_report | 調査レポートの各Web検索待ち時間（秒、3〜60） |
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
| `PICOAGENT_TODO_PLAN` | `[]` | todo_plan | 多段タスクの作業計画（JSON文字列、最大20手順） |
| `PICOAGENT_PROJECTS` | `[]` | project | プロジェクト一覧（環境・課題・記録を含む JSON 文字列、最大50件） |
| `PICOAGENT_ACTIVE_PROJECT` | （空） | project | 現在選択中のプロジェクトID |
| `PICOAGENT_WORKSPACES` | （旧キー） | project | 旧ワークスペース一覧。プロジェクト統合時に廃止し、既存値は読み込まない |
| `PICOAGENT_ACTIVE_WORKSPACE` | （旧キー） | project | 旧ワークスペース選択ID。プロジェクト統合時に廃止し、既存値は読み込まない |
| `PICOAGENT_SCHEDULED_TASKS` | `[]` | task_scheduler | cron 風のスケジュールタスク一覧（JSON文字列。`kind=command` / `kind=tool`。通常はチャットで設定。設定画面では専用フォームで編集） |
| `PICOAGENT_SCHEDULED_TOOL_ALLOWLIST` | `time,weather,web_fetch` | task_scheduler | 無人スケジュール実行を明示的に許可するスキル名（カンマ区切り） |
| `PICOAGENT_CLIPBOARD_MAX_CHARS` | `3000` | clipboard_text | クリップボード取得の最大文字数（500〜8000） |
| `PICOAGENT_MEETING_ROOT` | （旧キー） | meeting_recorder | 旧キー。設定画面からは廃止。設定済みなら既存利用者向けに追加許可ルートとしてだけ引き継ぐ |
| `PICOAGENT_TRANSCRIBE_PROGRAM` | 空 | meeting_recorder / whisper_transcribe | ローカル文字起こし実行ファイル。手動設定が最優先、空なら同梱 `whisper-cli`。同梱CLIはポートを開かない |
| `PICOAGENT_TRANSCRIBE_MODEL` | 空 | meeting_recorder / whisper_transcribe | ローカルWhisperモデルのパス。手動設定が最優先、空なら同梱モデル |
| `PICOAGENT_TRANSCRIBE_ARGS` | `-m {model} -f {audio} -otxt -of {output_stem}` | meeting_recorder / whisper_transcribe | 文字起こし実行時の引数テンプレート |
| `PICOAGENT_TRANSCRIBE_DIARIZE_ARGS` | `-tdrz` | meeting_recorder / whisper_transcribe | `diarize=true` 時に追加するローカル話者分類引数 |
| `PICOAGENT_TRANSCRIBE_TIMEOUT_SECS` | `1800` | meeting_recorder / whisper_transcribe | ローカル文字起こしタイムアウト秒（10〜7200） |
| `PICOAGENT_WEATHER_DEFAULT_LOCATION` | （なし） | weather | 天気の地点。保存済み地点を正本とし、空欄の場合だけHTTPSのIP位置情報から現在地を概算する。旧版が自動保存した `Tokyo` は未設定へ移行し、東京を固定する場合は「東京都」等を保存する |
| `PICOAGENT_WEATHER_DISABLE_AUTO_LOCATION` | `false` | weather | 地名未指定時の現在地IP自動取得を無効化 |
| `PICOAGENT_WEATHER_TIMEOUT_SECS` | `15` | weather | 天気API待ち時間（秒、3〜60） |
| `PICOAGENT_CURRENCY_DEFAULT_BASE` | `USD` | currency_rates | 為替の既定換算元 |
| `PICOAGENT_CURRENCY_DEFAULT_SYMBOLS` | `JPY` | currency_rates | 為替の既定換算先 |
| `PICOAGENT_CURRENCY_TIMEOUT_SECS` | `15` | currency_rates | 為替API待ち時間（秒、3〜60） |
| `PICOAGENT_MARKET_TIMEOUT_SECS` | `15` | market_indices | 市場指数API待ち時間（秒、3〜60） |
| `PICOAGENT_NEWS_SCRAPE_MAX_ITEMS` | `8` | news_scrape | Google News / Yahoo!ニュースから各サイト取得する最大件数（1〜12） |
| `PICOAGENT_NEWS_SCRAPE_TIMEOUT_SECS` | `15` | news_scrape | ニュースHTML取得待ち時間（秒、3〜60） |
| `PICOAGENT_RSS_DEFAULT_URL` | （なし） | rss_feed | RSS既定URL |
| `PICOAGENT_RSS_TIMEOUT_SECS` | `15` | rss_feed | RSS取得待ち時間（秒、3〜60） |
| `PICOAGENT_WORKSPACE_ROOT` | （旧キー） | workspace_files ほか | 旧キー。`PICOAGENT_ALLOWED_ROOT` へ統合。設定済みなら許可ルートとして引き継ぐ |
| `PICOAGENT_COMMAND_ENABLED` | `false` | run_command | allowlist 制限付きコマンド実行を有効化 |
| `PICOAGENT_COMMAND_ROOT` | （旧キー） | run_command / scheduler | 旧キー。`PICOAGENT_ALLOWED_ROOT` へ統合。設定済みなら許可ルートとして引き継ぐ |
| `PICOAGENT_COMMAND_ALLOWED_PATHS` | （旧キー） | run_command / scheduler | 旧キー。`PICOAGENT_ALLOWED_PATHS` へ統合。設定済みなら許可ルートとして引き継ぐ |
| `PICOAGENT_COMMAND_ALLOWLIST` | `git status,git diff,npm test,cargo test` | run_command | 実行を許可するコマンド全体（カンマ区切り・完全一致） |
| `PICOAGENT_COMMAND_TIMEOUT_SECS` | `30` | run_command | コマンド実行の既定タイムアウト秒（1〜120） |
| `PICOAGENT_SLACK_BOT_TOKEN` | （なし） | slack_send_message | Slack Bot token（Webhook 未使用時） |
| `PICOAGENT_SLACK_CHANNEL` | （なし） | slack_send_message | Slack の既定送信先チャンネル |
| `PICOAGENT_SLACK_WEBHOOK_URL` | （なし） | slack_send_message | Slack Incoming Webhook URL（Bot token の代替） |
| `PICOAGENT_SLACK_TIMEOUT_SECS` | `15` | slack_send_message | Slack API の待ち時間（秒、3〜60） |
| `PICOAGENT_GMAIL_ACCESS_TOKEN` | （なし） | gmail_message | Gmail API の OAuth access token |
| `PICOAGENT_GMAIL_BASE_URL` | `https://gmail.googleapis.com/gmail/v1` | gmail_message | Gmail API の base URL |
| `PICOAGENT_GMAIL_TIMEOUT_SECS` | `20` | gmail_message | Gmail API の待ち時間（秒、3〜60） |
| `PICOAGENT_GMAIL_CLEANUP_MAX_RESULTS` | `50` | gmail_message | Gmail の検索条件ベース整理/掃除で一度に扱う既定上限。`0` で無制限 |
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
| `PICOAGENT_HTTP_METADATA_TIMEOUT_SECS` | `15` | 全プロバイダ | モデル一覧取得など短い問い合わせの待ち時間（秒、3〜60） |
| `PICOAGENT_WEB_FETCH_ALLOWED_DOMAINS` | （空） | web_fetch | URL取得を許可するドメイン（カンマ区切り。空なら制限なし） |
| `PICOAGENT_WEB_FETCH_TIMEOUT_SECS` | `20` | web_fetch | URL取得の待ち時間（秒、3〜60） |
| `PICOAGENT_AGENT_CLI_KIND` | `claude` | agent_cli | 手元の CLI エージェントの種類（`claude` / `codex` / `custom`） |
| `PICOAGENT_AGENT_CLI_PATH` | （空=種類から自動） | agent_cli | CLI の実行パス |
| `PICOAGENT_AGENT_CLI_ARGS` | （空） | agent_cli | CLI へ追加する引数。Claude / Codex の安全な非対話引数は常に別途付加 |
| `PICOAGENT_AGENT_CLI_TIMEOUT_SECS` | `180` | agent_cli | 1回の応答を待つ秒数（10〜600） |
| `PICOAGENT_THA3_ROOT` | 空 | THA3キャラクター制作 | 作者配布から利用者が用意したTHA3本体・モデル・Python環境のフォルダ。設定画面で選択して保存し、共通許可ルートで検証する |
| `PICOAGENT_DESKTOP_ENABLED` | `false` | desktop_control | デスクトップ操作（マウス・キーボード）の有効化（既定は無効） |
| `PICOAGENT_DESKTOP_MAX_OPS_PER_MIN` | `60` | desktop_control | 1分あたりの操作回数の上限（1〜600）。暴走時の歯止め |
| `PICOAGENT_BROWSER_ENABLED` | `false` | browser_control | ブラウザ操作の有効化（既定は無効） |
| `PICOAGENT_BROWSER_PATH` | （空=自動検出） | browser_control | Chrome / Edge の実行ファイルパス |
| `PICOAGENT_BROWSER_ALLOWED_DOMAINS` | （空） | browser_control | 開いてよいドメイン（カンマ区切り。空なら全拒否） |
| `PICOAGENT_BROWSER_PROFILE` | `isolated` | browser_control | `isolated`=専用プロファイル / `default`=ブラウザの既定プロファイル（ログイン済みセッションを使う） |
| `PICOAGENT_BROWSER_HEADLESS` | `false` | browser_control | 画面を出さずに操作する |
| `PICOAGENT_BROWSER_PORT` | `11437` | browser_control | CDP のデバッグポート |
| `PICOAGENT_BROWSER_TIMEOUT_SECS` | `30` | browser_control | 1操作の待ち時間（秒、5〜180） |
| `PICOAGENT_BROWSER_MAX_CHARS` | `auto` | browser_control | 1回に読む本文の最大文字数（`auto`=`PICOAGENT_TOOL_OUTPUT_MAX_CHARS` に従う） |
| `PICOAGENT_TOOL_OUTPUT_MAX_CHARS` | `auto` | 基本 | ツール出力の最大文字数。`auto` は使用中モデルのコンテキスト長から算出（local は `PICOAGENT_LOCAL_CTX` の45%、下限1,000／上限60,000。クラウドは24,000） |
| `PICOAGENT_DOCUMENT_ROOT` | （旧キー） | ms_office / document_convert / pdf_tools | 旧キー。`PICOAGENT_ALLOWED_ROOT` へ統合。設定済みなら許可ルートとして引き継ぐ |
| `PICOAGENT_MS_OFFICE_TIMEOUT_SECS` | `120` | ms_office | Microsoft Office 操作タイムアウト秒（5〜300） |
| `PICOAGENT_LIBREOFFICE_PATH` | `soffice` | document_convert | LibreOffice 実行ファイルパス |
| `PICOAGENT_DOCUMENT_CONVERT_TIMEOUT_SECS` | `120` | document_convert | 文書変換タイムアウト秒（5〜300） |
| `PICOAGENT_QPDF_PATH` | `qpdf` | pdf_tools | qpdf 実行ファイルパス |
| `PICOAGENT_PDFTOTEXT_PATH` | `pdftotext` | pdf_tools | Poppler pdftotext 実行ファイルパス（空なら同梱→PATH） |
| `PICOAGENT_PANDOC_PATH` | `pandoc` | document_convert | Pandoc 実行ファイルパス（空ならPicoAgent内→PATH） |
| `PICOAGENT_MAGICK_PATH` | 空 | image_tools | ImageMagick 実行ファイルパス（空ならPicoAgent内→PATH。v7 `magick` が無ければ v6 `convert`/`identify`） |
| `PICOAGENT_IMAGE_TOOLS_TIMEOUT_SECS` | `120` | image_tools | 画像操作タイムアウト秒（5〜300） |
| `PICOAGENT_FFMPEG_PATH` | 空 | whisper_transcribe | ffmpeg 実行ファイルパス（非wav音声の16kHz WAV前処理用。空ならPicoAgent内→PATH） |
| `PICOAGENT_RIPGREP_PATH` | 空 | doc_search / grep_search | ripgrep 実行ファイルパス（空ならPicoAgent内→PATH。無ければ内蔵走査） |
| `PICOAGENT_SETUP_TOOLS` | （空） | xtask | `cargo xtask setup tools` で取得対象を絞る開発用環境変数（カンマ区切り） |
| `PICOAGENT_API_ENABLED` | `false` | ローカルAPI | ローカル HTTP API を有効化（127.0.0.1 のみ。変更は再起動後に反映） |
| `PICOAGENT_API_PORT` | `11500` | ローカルAPI | ローカル API のポート |
| `PICOAGENT_API_KEY` | 空 | ローカルAPI | 設定すると `Authorization: Bearer` を必須にする（`password` 型。キー名に `API_KEY` を含むためシークレットとして扱われ、Windows は DPAPI 暗号化、macOS はログインキーチェーンへ預けて保存される） |
| `PICOAGENT_LICENSE_TOKEN` | 空 | 製品ライセンス | `PA1.<payload>.<Ed25519署名>` 形式のPro／Businessライセンス。`password` 型で、WindowsはDPAPI暗号化、macOSはログインキーチェーンへ預けて保存。無効・期限切れ・未設定は無料版として動作し、PC操作・ファイル編集・Office・外部サービス・MCP等の高度なスキルを共通dispatch境界で拒否する |
| `PICOAGENT_API_ALLOWED_ORIGINS` | 空 | ローカルAPI | ブラウザからの横断リクエストを許可する Origin（カンマ区切り。空＝Origin 付きは全て拒否） |
| `PICOAGENT_MCP_SERVERS` | 空 | MCP | 外部 MCP サーバ（1行1件「名前\|コマンド 引数…」。stdio 接続。`#` でコメント。変更は再起動後に反映） |
| `PICOAGENT_PDF_TIMEOUT_SECS` | `120` | pdf_tools | PDF操作タイムアウト秒（5〜300） |

各キーは環境変数としても渡せる（既定値としての参照のみ。保存先は常に `config.json`）。

---
