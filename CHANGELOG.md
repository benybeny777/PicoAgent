# 変更履歴

このファイルには、現行アーキテクチャ以降の運用や品質に影響する大きめの変更だけを残します。

---

## 2026-06-29

### ローカル録音・Whisper文字起こし議事録を追加

- **録音から議事録までローカル完結** (`skills/meeting_recorder/`): マイク録音をWAV保存し、設定したローカルWhisper系コマンドで文字起こしし、議事録Markdownを生成するスキルを追加。録音・文字起こし・議事録ファイルは `PICOAGENT_MEETING_ROOT` 配下に限定する。
- **保存先を共通ワークスペースへ統一** (`skills/meeting_recorder/`): `PICOAGENT_MEETING_ROOT` が空なら共通 `PICOAGENT_WORKSPACE_ROOT` 配下の `meetings/YYYY-MM-DD/` に `YYYYMMDD-HHMMSS-meeting.wav` 形式で保存し、文字起こし・議事録も同じ録音名に揃える。パス未指定時は最新の録音/文字起こしを自動選択する。
- **話者分類オプション** (`skills/meeting_recorder/`): `diarize=true` のとき `PICOAGENT_TRANSCRIBE_DIARIZE_ARGS`（既定 `-tdrz`）を追加し、whisper.cpp の tinydiarize 等に対応できるようにした。話者分類の精度とマーカー形式は利用するローカル実装・モデルに依存する。
- **会議系依頼の直行ルート** (`agent.rs` / `tools/mod.rs`): 「録音」「文字起こし」「議事録」「話者」「Whisper」などの依頼で `meeting_recorder` を提示・直接実行できるようにした。

### Genspark風ワークフローを10系統追加

- **分析カテゴリを統合スキル化** (`skills/genspark_suite/`): Deep Research、AI Slides、AI Sheets、AI Docs、Sparkpage、Autopilot、AI Drive、Meeting Notes、Multi-Agent、AI Browser計画の10系統を同じフォルダに生成するスキルを追加。保存は `PICOAGENT_WORKSPACE_ROOT` 配下に限定し、まずは外部通信なしのローカル成果物生成として使える。
- **全部依頼の直行ルート** (`agent.rs` / `tools/mod.rs`): 「Genspark風」「Genspark全部」「AI Slides」「AI Sheets」「Sparkpage」「Autopilot」「Meeting Notes」などの依頼で `genspark_suite` を提示・直接実行できるようにした。

### 資料ワークスペース生成を追加

- **Genspark風の一括成果物生成** (`skills/spark_workspace/`): ひとつの依頼から `brief.md`、`deck.md`、`sheet.csv`、`page.html`、`agent-plan.md`、`prompts.md`、`checklist.md` を同じフォルダに生成するスキルを追加。保存は `PICOAGENT_WORKSPACE_ROOT` 配下に限定し、外部通信なしで使える。
- **資料一式依頼の直行ルート** (`agent.rs` / `tools/mod.rs`): 「Genspark」「資料一式」「ワークスペース」「サイトとスライド」などの依頼で `spark_workspace` を候補提示し、小型ローカルモデルの tool_calls 判断を待たずに直接実行できるようにした。

### ローカル実用ツールを30機能追加

- **設定不要の実用処理を拡充** (`skills/productivity_tools/`): 文字数分析、大小文字変換、JSON整形/圧縮、Base64、URL変換、単位換算、日付計算、日付差分、週番号、timestamp、行分析、反転、行ソート、重複行除去、行trim、折り返し、URL抽出、メール抽出、キーワード数、チェックサム、CSV→Markdown表、割合、BMI、HEX/RGB変換、スラッグ化など30種類のローカル実用機能を追加。外部通信なしで即使える。
- **スキル候補の自動選択を拡張** (`tools/mod.rs`): 「文字数」「JSON」「Base64」「URLエンコード」「単位換算」「日付計算」「CSV」「RGB」などの依頼で `productivity_tools` をモデルに提示するようにした。

### 出典付き調査レポート生成スキルを追加

- **調査からMarkdownレポート化までを一括化** (`skills/research_report/`): 複数観点のWeb検索を実行し、要約・観点別メモ・比較表・次の深掘り・出典一覧を含む Markdown 調査レポートを生成するスキルを追加。`save=true` または `save_path` で `PICOAGENT_WORKSPACE_ROOT` 配下へ `.md` 保存できる。検索件数・待ち時間・保存先に境界を設け、危険な任意パス書き込みは拒否する。
- **成果物セット化** (`skills/research_report/`): 保存時は既定で本文レポートに加え、1ページ要約 `.brief.md`、HTMLダッシュボード `.dashboard.html`、スライド下書き `.slides.md`、出典CSV `.sources.csv`、出典JSON `.sources.json` も同時生成するようにした。`artifact_bundle=false` で単一レポート保存にも戻せる。
- **調査依頼の直行ルート** (`agent.rs`): 「調査」「レポート」「出典」「競合比較」などを含む依頼は、小型ローカルモデルの tool_calls 判断を待たず `research_report` を直接実行するようにした。

### 設定画面の未保存変更を戻せるように修正

- **変更途中の安心感を改善** (`settings.html` / `settings.js` / `settings.css`): 未保存変更がある時だけ「変更を戻す」を有効化し、変更済みの設定行を強調表示するようにした。ボタンは最後に読み込んだ、または保存に成功したスナップショットからフォームを再描画し、キャラ上書き・音声上書き・目覚まし設定も含めて未保存変更だけを破棄する。

### 設定画面に診断タブを追加

- **配布後の切り分けを容易化** (`settings.js` / `settings.css`): 設定画面に「診断」タブを追加し、選択中プロバイダ、同梱エンジン、Aivis、トークン使用量、スキル実行状況を1画面で確認できるようにした。各状態取得が失敗しても設定画面全体は落とさず、該当カードだけエラー表示する。

### 全画面フキダシの履歴検索と再利用を追加

- **過去会話を使いやすく改善** (`index.html` / `renderer.js` / `styles.css`): 全画面フキダシに履歴検索を追加し、会話履歴を絞り込めるようにした。各履歴にはコピー操作を追加し、ユーザー発話は「再利用」で入力欄へ戻せるようにした。

### 配布物の解析しやすさを下げるビルド経路を追加

- **配布用フロントを `ui-dist/` へ分離** (`build-ui-dist.mjs` / `tauri.conf.json`): `ui/` は編集用正本として残し、Tauri の `frontendDist` は生成物 `ui-dist/` を参照するようにした。`npm run dev` は通常コピー、`npm run build` は HTML/CSS のコメント・余分な空白と JS の空行・行頭行末空白を削った release コピーを作ってからバンドルする。
- **release バイナリの情報露出を削減** (`Cargo.toml`): 既存の LTO・strip・panic abort に加え、`debug=false` / `incremental=false` を明示し、配布バイナリにデバッグ情報が残りにくいようにした。

### 会話入力のクイック操作と入力履歴を追加

- **会話操作の摩擦を削減** (`index.html` / `renderer.js` / `styles.css`): 入力欄上部に「画面を見る」「クリップボード要約」「今日の予定」「できること」のクイック操作を追加し、よく使う依頼をワンクリックで送信できるようにした。ユーザー発話の直近入力履歴を保持し、入力欄の `↑` / `↓` で過去入力と編集中ドラフトを行き来できる。`Ctrl+Enter` / `Cmd+Enter` 送信にも対応。

### 設定画面の横断検索を追加

- **設定を探しやすく改善** (`settings.html` / `settings.js` / `settings.css`): 設定画面ヘッダーに検索入力を追加し、設定キー・ラベル・プロバイダ・スキル名・キャラクター名/説明でタブ横断フィルタできるようにした。検索中は一致するタブ/セクション/カードだけを表示し、入力値や保存対象は変えない。

### ローカルエンジン再利用判定を安全化

- **残存 `llama-server` の誤用を防止** (`engine.rs`): 前回クラッシュ等で同じポートのローカルエンジンが残っている場合、`/health` だけでなく `/v1/models` のモデルIDが現在要求中の GGUF と一致する時だけ再利用するようにした。モデル変更後に旧モデルのサーバーを黙って使う状態を避け、一致しない場合は残存プロセス停止を促すエラーにする。

### 読み上げ(TTS)のパイプライン化と「できること」回答の改善

- **読み上げの体感待ち時間を短縮** (`renderer.js`): 返答を文単位のチャンクに分割し、先頭チャンクを再生しながら次チャンクを裏で先読み合成するパイプライン方式にした。長い返答でも全文合成を待たずに早く話し始める。`stopVoice` は世代カウンタで進行中の連続再生を確実に打ち切る。
- **「何ができる？」にスキルを踏まえて回答** (`tools::capability_summary` / `effective_system_prompt`): エージェントが使えるスキル一覧を system プロンプトへ添え、能力を聞かれたら具体的に挙げて答えるよう指示を追加。

### ファイルドロップ・ペースト対応を追加

- **Rust コマンド** (`pet_read_file_content`): ドロップ/ペーストされたファイルをテキストへ変換する新コマンドを追加。xlsx/xlsm は calamine でシート別 Markdown テーブル化、docx/docm は ZIP 展開 + XML ストリップ、pptx/pptm はスライドごとにテキスト抽出、csv/tsv は Markdown テーブル化、テキスト系・コードファイルはそのままテキスト、PDF は pdf_tools スキルへ誘導メッセージを返す（最大 12,000 文字）。
- **フロントエンド** (`renderer.js`): `attachedFiles` 状態とファイルチップ UI を追加。composer エリアへのドラッグ&ドロップで画像・ドキュメントを選別して添付（最大各 4 件）。送信時にファイル内容を抽出してメッセージの前に付加し、テキストなしでもファイルのみで送信可能。
- **UI** (`styles.css`, `index.html`): `.file-chip` スタイルを追加。ファイル種別ごとにアイコン（📊 Excel、📝 Word、📕 PDF 等）を表示。
- **Cargo.toml**: `zip = "0.6"` と `calamine = "0.24"` を追加。

### ComfyUI ローカル画像生成スキルを追加

- **ComfyUI スキル** (`skills/comfyui/`): APIキー不要のローカル画像（および動画）生成スキルを追加。ComfyUI を起動しておくだけで「猫の絵を描いて」などの発話で `comfyui_generate` ツールが呼ばれる。設定キーは `COMFYUI_BASE_URL`（既定 `http://127.0.0.1:8188`）と `COMFYUI_CHECKPOINT`（モデルファイル名）のみ。KSampler ワークフローをデフォルトで生成し、プロンプト・ネガティブプロンプト・サイズ・ステップ数・CFG・シードを引数で制御可能。AnimateDiff 等の動画出力（GIF/MP4/WebM）もリンクで返す。
- **ローカルURL画像表示** (`renderer.js`): `appendLinkedText()` の画像レンダリングを拡張し、`http://localhost` および `http://127.0.0.1` のURLも `<img>` としてインライン表示できるようにした（外部 `http://` は除外）。ComfyUI の `/view` エンドポイントから返る画像をフキダシ内で直接確認できる。
- **TOOL_ROUTES 更新** (`tools/mod.rs`): 「画像」「絵」「描いて」等のキーワードを `comfyui_generate` へルーティング。DALL-E 3 の `image_generate` は `dall-e` / `openai画像` キーワードのみ残す。

### 画像系機能を追加

- **画像生成スキル** (`skills/image_generate/`): DALL-E 3 を利用してテキストプロンプトから画像を生成する新スキルを追加。OPENAI_API_KEY が設定されていれば「猫の絵を描いて」などの発話でモデルが `image_generate` ツールを呼び出して URL を返す。サイズ（1024×1024 / 1792×1024 / 1024×1792）と画質（standard / hd）を指定可能。
- **スクリーンショット解析** (`pet_capture_screen` Tauri コマンド): デスクトップのスクリーンショットを PNG base64 data URL で返す新コマンドを追加。Windows は PowerShell + GDI、macOS は `screencapture`、Linux は grim → scrot → ImageMagick の順に試みる。フロントエンドは「画面を見て」「スクリーンショット」などの発話を検出し、自動でスクリーンショットを撮影して `pet_send_image` 経由でビジョン対応モデルへ送る。
- **インライン画像レンダリング** (`renderer.js`): `appendLinkedText()` を拡張し、`![alt](url)` 形式の Markdown 画像記法を `<img>` 要素としてフキダシ内にインライン表示できるようにした。画像生成スキルの返答や他のスキルが返す画像 URL をそのままフキダシで確認できる。

### AIエージェント品質向上: 2機能を追加

- **会話要約圧縮**: `commands.rs` に `maybe_summarize_context()` を追加。会話のトークン推定量（日本語 ×2・ASCII ÷4）が予算の70%（約 4,200 トークン）を超えた場合、古い40%のターンを LLM で 2〜3 文に要約し system メッセージへ付加する。要約失敗時は元の会話を返す（会話を止めない）。これにより長い会話でも重要な文脈を維持しやすくなる。
- **ツール引数修復リトライ**: `agent.rs` に `dispatch_or_error()` を追加。モデルが返したツール呼び出しの引数 JSON が解析できない場合（`_raw` フィールドで検出）、ツールを呼ばずエラー文字列を返してモデルへ「正しい JSON で再呼び出し」を促す。次の chat でモデルが修正した引数で再試行できるようになる。

### AIエージェント品質向上: 8機能を追加

- **セマンティック記憶のリクエスト別接続**: `commands.rs` の `pet_send()` でユーザー発話ごとに `context_for_query(app, cfg, text)` を呼び、その発話に最も関連する長期メモリを system 文脈へ差し込む。従来は起動時の静的コンテキストのみだったが、これにより発話の内容に応じたメモリが毎回選択される。
- **ツール出力長制限**: `tools::dispatch()` に `MAX_TOOL_OUTPUT_CHARS = 8,000` による打ち切りを追加。出力が上限を超えた場合は末尾を省略して「〔N文字で打ち切りました〕」を付与し、コンテキストウィンドウの超過を防ぐ。
- **プロンプトインジェクション検知**: `tools::check_injection()` を追加。ツール出力に「ignore previous instructions」「前の指示を無視」等の既知パターンが含まれる場合、警告テキストでラップしてから assistant メッセージへ積む。外部データ経由の乗っ取りを軽減する。
- **トークンベースのコンテキスト圧縮**: `commands.rs` のコンテキスト圧縮を「最大 24 件」からトークン予算方式へ変更。日本語文字 ×2・ASCII ÷4 で概算し `MAX_CONTEXT_TOKENS = 6,000` を超えた場合は古い発話から削除する。日本語比率の高い会話でのコンテキスト超過を抑制する。
- **プランニングステップ**: `agent.rs` に `planning_step()` を追加（`PICOAGENT_PLANNING_ENABLED` で有効化。既定 OFF）。質問が複雑（120文字超、疑問符2個以上、手順/ステップ系語を含む）な場合、`chat_with_options(json_mode=true)` でサブタスクへ分解し分解結果を system 補足として追加してから通常の chat ループへ渡す。
- **自己評価ステップ**: `agent.rs` に `self_improve()` を追加（`PICOAGENT_SELF_EVAL_ENABLED` で有効化。既定 OFF）。最終応答が 8 文字未満または明示的な拒否パターン（「申し訳ありません、できません」等）を含む場合に1回再生成する。再生成失敗時は元の応答を返す。
- **構造化出力 / JSON モード**: `Provider` トレイトに `chat_with_options(json_mode: bool)` メソッドをデフォルト実装付きで追加。`openai.rs` はオーバーライドして `response_format: {type:"json_object"}` を付与（temperature 0.3）。プランニングステップの JSON 解析精度が向上する。
- **マルチエージェント Lite（プランニング統合）**: プランニングステップがサブタスク分解を担うことで、単一の会話ループ内でタスクの計画と実行を段階的に行える設計とした（独立プロセスを立てない軽量実装）。

### AIエージェントとして不足していた8機能を整備

- **並列ツール実行**: `agent.rs` で `futures::future::join_all()` を使い、複数の `tool_calls` を逐次ではなく並列実行するようにした。`Cargo.toml` に `futures = "0.3"` を追加。
- **ストリーミングレスポンス**: `Provider` トレイトに `chat_stream()` メソッドを追加（デフォルト実装は従来の `chat()` に委譲）。`openai.rs` はオーバーライドして SSE ストリームを受信し、テキストチャンクを `on_chunk` コールバック経由で逐次フロントへ `pet:stream_chunk` イベント送信する。`renderer.js` はチャンクを受け取るたびにフキダシを即時更新し、最初のチャンク到着で「思考中」から「会話中」アニメーションへ切り替える。SSE の `tool_calls` は `PartialToolCall` バッファで `index` ごとに積み、`[DONE]` 到達時に組み立てる。
- **トークン使用量追跡**: `usage.rs` を新規追加。chat 呼び出しのトークン数（prompt/completion/total）を `app_data_dir/usage.json` に記録し、90日を超えたレコードは自動削除する。`pet_usage` コマンドで今月のプロバイダ/モデル別集計と全期間累計を取得できる。`openai.rs` は `extract_usage()` でレスポンスの `usage` フィールドを取り出して `Message.usage` に格納する。
- **感情状態管理**: `emotion.rs` を新規追加。ユーザー発話と返答から喜び・悲しみ・興奮・好奇心・怒りのキーワードを検出してキャラの感情（mood 種別 + energy 0〜1）を推定する。慣性係数 0.35 で急激な感情変化を防ぐ。`AppState` に `emotion: Mutex<EmotionState>` を追加し、`pet_send` / `pet_send_image` の応答後に更新して `pet:emotion` イベントでフロントへ通知する。`pet_emotion` コマンドで現在状態を取得できる。`renderer.js` はキャラの `dataset.emotion` と `dataset.energyLevel` を更新する。
- **高度なリトライ戦略**: `chat_with_retry()` を新設。最大3回・指数バックオフ（失敗後1秒→2秒待機）でリトライする。4xx 認証エラー（401/403/400/404）は即中断してリトライしない。
- **セマンティック記憶検索**: `memory.rs` に `context_for_query()` を追加。ユーザー発話と記憶項目のバイグラム（2文字連続ペア）ジャカード係数で関連度を算出し、閾値 0.05 以上の項目をヒット回数ボーナス付きで降順に最大 24 件提示する。関連項目がゼロのときはヒット数順の全件フォールバック。`agent.rs` から `context_for_query()` を呼ぶよう変更。
- **マルチモーダル入力**: `Message` 構造体に `images: Option<Vec<String>>`（base64 data URL 配列）を追加。`openai.rs` は user メッセージに画像が含まれる場合、`content` を `[{type:"text",...},{type:"image_url",...}]` 配列形式へ変換して送信する。`pet_send_image(text, images)` コマンドを追加（1枚10MB制限、最大10枚）。`window.picoagent.sendImage(text, images)` から呼べる。
- **分散トレーシング**: `tracer.rs` を新規追加。`Tracer`（`SpanGuard`）で各ツール実行のレイテンシを計測し、直近200件を Tauri managed state として保持する。`pet_traces` コマンドでスナップショットとツール別集計（`avgMs` / `callCount` / `errorCount`）を取得できる。

---

## 2026-06-28

### 主要LLMプロバイダとモデル選択を追加

- 叩く AI に `anthropic`（Claude）/ `deepseek`（DeepSeek）/ `xai`（Grok）/ `mistral` プロバイダを追加した。Claude は Anthropic Messages API の `tool_use` / `tool_result` へ正規形ツール呼び出しを変換し、それ以外の追加プロバイダは OpenAI 互換 API を使う。
- OpenAI の既定候補を `gpt-5.4-mini` に更新し、OpenAI / Claude / Gemini / DeepSeek / xAI / Mistral の主要モデルを設定画面で「種類 → サイズ」から選べるようにした。
- ローカルモデルも `auto` / Gemma / Qwen / Llama / Mistral / Phi / DeepSeek / custom の種類からサイズを選ぶ UI に変更し、選択中プロバイダのモデルを保存時に `PICOAGENT_MODEL` へ同期するようにした。
- ローカルモデルプリセットに `qwen25_7b` / `llama32_3b` / `llama31_8b` / `mistral7b` / `phi35mini` / `deepseek_r1_qwen7b` を追加し、各候補に推奨VRAM・推奨RAM・ダウンロード目安を表示するようにした。

### コマンド実行のチャット確認と保護パスを追加

- `run_command` で未登録コマンドまたは許可ルート外 cwd が必要になった場合、即エラーではなくチャット上で「許可」/「永続許可」/「キャンセル」を求めるようにした。「許可」は保留中の1件だけに効き、「永続許可」はコマンド prefix を allowlist、必要なら cwd を追加許可パスへ保存してから実行する。
- `C:\Windows` と `C:\Program Files*` 配下は編集不可にし、コマンド実行先、スケジュール実行先、`workspace_files` の書き込み・追記・フォルダ作成で拒否するようにした。
- ローディング画面の状態文は、実際に実行中の処理だけを表示するようにし、準備完了直前の「起こしているよ」は従来どおりプログレスバー下に表示する。
- フキダシのリサイズつまみを左上へ移し、拡大時の画面内上限とキャラクター回避による自動縮小を外した。さらに左上方向の拡大に合わせて Tauri ウィンドウ自体を左上へ移動しながら広げるようにした。
- 透明ウィンドウのサイズと縦位置をアプリ終了時に保存し、起動時に復元するようにした（タイマーによる定期保存は廃止）。横位置は以前指定した右端基準で固定し、縦位置は画面下端からの相対オフセットで持つことで解像度が変わっても右下基準を保つ。ドラッグで上端に近づくと上端へスナップする。既定のフキダシはウィンドウ下端付近まで縦に伸ばし、右のキャラクターに被らない幅に調整した（キャラクター回避の自動縮小は撤去し、位置移動のみにした）。吹き出しの尻尾を縦長時でもキャラの方を向くよう下寄りにした。フキダシを大きくしたときは右側にキャラクター分の幅を確保するようウィンドウを左上へ自動拡大し（右下は固定）、フキダシとキャラが重ならないようにした。右クリックメニューに「フキダシ位置とサイズをリセット」を追加した。
- ローカルモデルのサイズ候補に、推奨VRAM・推奨RAM・ダウンロード目安を再表示した。
- まいの既定人格を「おっとりして優しいお姉さん」として明示した。

### cron風タスクスケジューラを追加

- `task_scheduler` スキルを追加し、チャットから指定時刻・曜日・cron風5フィールドでタスクを追加・一覧・削除・有効/無効化できるようにした。
- `PICOAGENT_SCHEDULED_TASKS` を設定キーとして追加し、設定画面の「スキル」タブからもJSONで編集できるようにした。
- バックエンドに常駐スケジューラを追加し、アプリ起動中に30秒間隔で現在分とタスクを照合して実行するようにした。同じ分の二重実行は `last_run_minute` で防ぎ、一度だけのタスクは実行後に無効化する。
- `kind=command` は `run_command` と同じ `PICOAGENT_COMMAND_ENABLED` / `PICOAGENT_COMMAND_ROOT` / `PICOAGENT_COMMAND_ALLOWLIST` / timeout を使い、shell を使わず `program + args` だけを実行する。
- `kind=tool` を追加し、指定時刻に `tool_name` / `tool_args` で既存スキルを実行できるようにした。実行結果はアプリログへ記録する。

---

## 2026-06-27

### ツンデレキャラ「りん」を追加

- 新キャラクター `rin`（りん）を追加し、黒髪ツインテールのツンデレ作業相棒として人格設定・挨拶・アニメーション定義を登録した。
- `characters/index.json` に `りん` を追加し、設定画面のキャラクター選択から切り替えられるようにした。
- ローディング画面の画像と文言を固定のむぎ表示から選択中キャラクターに合わせるようにし、`りん` 選択時は起動待ち中もりんの画像を表示するようにした。
- `りん` のローディング画像を、むぎと同じ横向き寝姿の専用アセットへ差し替えた。
- `りん` の既定音声を AivisHub の「るな」に設定し、未導入環境では初回読み上げ時に AIVMX を AivisSpeech Engine へ追加するようにした。
- キャラクター固有の AivisHub 音声モデル導入を、初回読み上げ時ではなく起動シーケンスのローディング画面で行う `pet_prepare_voice_model` へ分離した。未導入時は「声を準備中…」として待ち、挨拶の事前合成は導入後に行う。
- 設定保存時、ローカルモデル関連の値が実際に変わった場合だけ同梱エンジンを再起動するようにし、キャラクター変更だけで「モデルを準備中…」が走らないようにした。
- 同梱 Aivis の既定ポートを既存 Aivis と被りやすい `10102` から PicoAgent 用の `11436` へ変更し、旧既定値が保存済みの場合も起動時に `11436` へ移行するようにした。
- `りん` 選択時にローディング画面の冒頭で一瞬むぎが表示される不具合を修正した（選択中キャラクターの画像が決まるまで起動画像を出さないようにした）。
- 会話履歴をキャラクター別ファイル（`history-<キャラID>.json`）に分け、キャラ変更時は文脈を切替先キャラの履歴へ入れ替えるようにした。これにより旧キャラ（むぎ）の履歴に引きずられて新キャラがむぎとして自己紹介・応答する不具合を修正した。旧 `history.json` は起動時に既定キャラ `mugi` のファイルへ一度だけ移行する（旧履歴はキャラ別管理が無かった時代のもので当時の既定キャラ＝mugi のため。後から追加した rin 等へ割り当てると人格が混ざる）。
- 実行時ログ（`app` / `aivis` / `engine`）を 1 日ごとに日付付きファイル（`app.<YYYY-MM-DD>.log` など）へローテートし、1 週間より古いログを起動時と 1 日ごとに自動削除するようにした。会話履歴はログではないため対象外。
- `りん` の読み上げが出ない不具合を修正した。ハードコードしていた global style ID（`-1801897920`）が AivisSpeech の実 ID（`345585728`）と一致せず、モデル導入後も `/speakers` に見つからず合成へ進めていなかった。話者 UUID＋スタイル名から実 style ID を実行時に `/speakers` から解決する方式へ変更し、ID の取り違えを起こさないようにした。
- 重い設定変更の反映（再起動）を、プロセス再起動から「バックエンド再初期化＋メインウィンドウ webview 再読込」へ変更した。`app.restart()` は `tauri dev` の開発配信サーバが落ちて `127.0.0.1` 接続拒否になるため使わず、`restart_app` がエンジン/Aivis を停止→設定から再起動し、選択中キャラの履歴を読み直したうえで `pet:reload` で起動シーケンスを再実行する。dev / 配布版どちらでもキャラ変更後の反映が壊れないようにした。
- 新キャラクター `mai`（まい）を追加した。落ち着いた紫のドレスが似合う、包容力のあるやさしいおねえさん作業相棒。`characters/index.json` に追加し、設定画面のキャラクター選択から切り替えられる。音声は AivisHub の「まい」（未導入なら AIVMX を AivisSpeech Engine へ追加）。提供画像は背景を透過処理し、まばたき・口パク用の表情フレームを追加した。
- `りん` と `まい` に表情差分レイヤーを追加し、待機・思考・落胆にまばたき、会話中に元の口位置へ小さく開き口を重ねる口パクが入るようにした。`assetVersion` も更新して古い PNG キャッシュを避ける。
- `むぎ` / `りん` / `まい` の連番アニメーションを 12 枚中心へ増やし、表情差分と微小な髪・上半身揺れを組み合わせて動きを滑らかにした。まいにはむぎ・りんと同じローディング用の横向き寝姿 `boot/sleeping.png` を追加した。
- キャラクター画像準備を `characters/README.md` と `scripts/build-character-animations.py` で標準化した。`layers/` の表情差分と `boot/sleeping.png` を正本にし、連番フレームと `character.json` の `animations` / `assetVersion` はスクリプトで再生成する。
- ローディング画面の寝姿画像の「ZZZ」が白背景で見えにくかったため、むぎ・りん共通で見やすい藤色へ recolor した（ZZZ 部分のみ。キャラクター本体は変更なし）。
- 叩く AI・モデル・音声・キャラクターなど重い設定変更を「再起動して反映」する共通機構を追加した。`settings_save` が該当キーの変更で `requiresRestart` を返し、設定画面が確認画面を出して `restart_app`（`app.restart()`）で再起動、モデル取得・エンジン/Aivis 起動・キャラ読込を起動シーケンス（ローディング画面）でまとめて行う。これに伴い、これらの反映を実行中に行わなくしたため、フキダシに「モデルを準備中…」が出なくなった（進捗はローディング画面のみに表示。`local` 起動時はエンジン ready までローディング画面で待機）。
- 設定保存時のローカルモデル再起動判定で `PICOAGENT_LOCAL_CTX` の比較既定値が実際の既定 `8192` とずれて `4096` になっていたのを修正し、値を変えていないのに同梱エンジンが再起動するのを防いだ。
- Aivis の `/speakers` 応答形式異常やモデル一覧取得エラーを空結果として隠さず、設定画面やログへ原因が出るようにした。README の古い `りん` style ID 説明も、話者 UUID＋スタイル名から実 style ID を解決する現行方式へ合わせた。
- Aivis 同梱により NSIS の入力データが約 1GB になり、既定の LZMA 圧縮で `makensis` が不安定に終了することがあったため、配布ビルドの安定性を優先して NSIS 圧縮を `zlib` に変更した。
- 会話中の口パク頻度を下げ、表情差分の緑フリンジ除去を強めて髪まわりの透過残りを削った。
- フキダシ右下ドラッグの拡大は、キャラクター衝突をサイズ上限にするのではなく座標を逃がす方式に変更した。確定直前にも再配置し、再配置後の座標を自動保存する。全画面拡大時も固定余白ではなくキャラクターの実測矩形からフキダシ領域を再計算し、ウィンドウサイズ変更・キャラ画像ロード・設定反映でも再計算する。設定画面からフキダシの幅・高さ入力欄は削除した。
- 入力欄の体験を改善した。空入力・送信中は送信ボタンを無効化し、入力内容に合わせてテキストエリアを自動伸縮する。送信コマンドが例外で失敗しても `busy` が戻らず固まる状態を防ぎ、エラー表示と履歴追加まで一貫して処理するようにした。
- フキダシ本文・履歴・入力欄・リンク上の右クリックでは通常のコピー/リンク操作を優先し、それ以外の場所で PicoAgent メニューを開くようにした。
- フキダシのドラッグリサイズ後に、幅・高さだけでなく再配置後の座標も `PICOAGENT_BUBBLE_LEFT_PX` / `PICOAGENT_BUBBLE_TOP_PX` として自動保存し、次回起動時に復元するようにした。
- ローディング画面に `#bootLog` を追加し、モデル準備・エンジン起動・音声準備の状態を寝姿画像の領域へ時刻なしの横流しテロップとして表示するようにした。TTS 無効時は音声モデル準備表示を出さない。
- 起動時にバックエンドへ復元済みの直近会話をフロントにも渡し、全画面フキダシの履歴表示へ反映するようにした。読み上げ中にキャラクターをクリックすると現在の音声を停止する。
- 設定画面のキャラクター定義キャッシュキーを現行アニメーション版へ合わせ、古いキャラ定義や画像が残りにくいようにした。
- フキダシ右上へ回答操作ボタン（コピー / 再読み上げ / 停止 / 拡大）を追加し、通知は入力中・読み上げ中・全画面中ならキューへ退避して邪魔しないタイミングで表示するようにした。
- 音声設定に `PICOAGENT_TTS_VOLUME`（0〜100）を追加し、キャラボイスの再生音量を保存後すぐ反映できるようにした。再生中の音声にも即時適用する。

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
- フキダシを拡大（全画面/ドラッグ）したときにキャラクターへ被る問題を解消。全画面表示は右側にキャラ分の幅を空けた左ペインにし、ドラッグ拡大はサイズをなるべく維持したままフキダシ座標を左上へ再配置するようにした（`renderer.js` / `styles.css`）。キャラは薄くせず右側にそのまま表示する。

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
- 待機・思考・落胆にまばたき、会話中に口パクの表情フレームを追加した。横揺れは抑え、上下の小さな動きで状態差を出す。以前保存していた作業用PSDと確認用シートは、実行時に使わない制作素材として後に削除した。
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
