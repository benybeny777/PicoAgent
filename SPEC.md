# PicoAgent 仕様書

対象: PicoAgent（コネクタ駆動デスクトップペット / Tauri）
最終更新: 2026-08-18

---

## 概要

PicoAgent は透明な **Tauri** ウィンドウに常駐する、キャラクターとフキダシだけのデスクトップペット。
バックエンドは Rust（`src-tauri/`）、フロントは webview（`ui/`）。
「叩く AI（LLM バックエンド）」と「エージェント機能（ツール）」をコネクタとして拡張できる。

- **叩く AI** → プロバイダコネクタ（`src-tauri/src/providers/`）。**既定は `local`（同梱 llama-server、Ollama 不要）**、Ollama / OpenAI 互換も同梱。
- **エージェント機能** → スキル（ツール）コネクタ（連結ファイルはプロジェクト直下の `skills/<スキル名>/mod.rs`、トレイトは `src-tauri/src/tools/mod.rs`）。モデルが function calling で呼び出す。
- **キャラクター** → 正本はプロジェクト直下の `characters/index.json` と各 `character.json`。`cargo xtask sync-characters` が `ui/assets/characters/` へ差分同期し webview が配信。見た目・アニメーション・人格を一体で切り替える。制作専用の `expression-patches/`、`live2d-src/`、通常2Dの正本 `layers/*.png` は配信コピーへ含めず、実行用lossless WebPだけを残す。`runtimeLayers`を持たない追加パックのPNGレイヤーは後方互換で配信する。
- **関係性** → `relationship.rs` がキャラクター別の `app_data_dir/relationship-<キャラクターID>.json` に、会話・完了作業・ふれあい・共有した出来事を原子的に保存する。数値は内部更新だけに使い、UIとシステム文脈では段階名と最近の出来事だけを扱う。
- **両者をつなぐ中核** → エージェントループ（`src-tauri/src/agent.rs`）。
- **中断** → 停止要求は生成ストリーム、再試行待機、並列ツール実行と競合させて即時に反映する。中断したツール呼び出しにも同じ `tool_call_id` の中断結果を補い、履歴の対応関係を崩さない。
- **予約の追いつき** → `last_checked_minute` から現在分までを調べ、スリープ・再起動中の未処理分は直近24時間を上限に実行する。時計巻き戻り時は過去予定を二重実行しない。
- **復号不能シークレット** → 設定画面では暗号文を入力値として表示せず、空欄と理由を示す。利用者が再入力しないまま保存した場合は既存暗号文を保持する。

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
- [外部インターフェース（ローカルAPI / MCP）](#外部インターフェースローカルapi--mcp)
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
- 登録済みプロバイダ/スキルは `OnceLock` でプロセス内キャッシュされ、設定画面表示・ツール dispatch・プロバイダ取得のたびに `Box` を再生成しない。ツールスキーマも初回だけ生成して再利用する。会話時は全スキルを常にLLMへ渡さず、ユーザー発話のキーワードから候補スキーマを絞り込む（`get_time` / `web_search` / `web_fetch` は基本候補）。一致候補は `intent_priority`、一致語の長さ、スキル名の順で安定整列し、登録順に依存させない。
- `AppState`（`commands.rs`）が選択中プロバイダ・モデル・会話履歴・直近の自発雑談・**感情状態**（`emotion::EmotionState`）を保持。プロバイダとモデルは設定画面から切替。感情状態はユーザー発話と返答のキーワードから `emotion::update()` でルールベース更新し、`pet:emotion` イベントでフロントへ通知する（`renderer.js` はキャラの `dataset.emotion` / `dataset.energyLevel` へ反映）。
- `hi` / `こんにちは` などの短い挨拶は `agent::simple_greeting_answer()` がプロバイダを呼ばず直接返す。小型モデルへ「AIとして説明しない」等の内部ガイドをユーザー本文に追記すると、禁止事項そのものを返答として説明・読み上げることがあるため、挨拶経路へ内部指示を混ぜない。履歴へはユーザーが入力した原文と自然な短文だけを残す。返す一言は全キャラ共通の定型文にせず、`pet_set_character_profile` が受け取った `character.json` の `greeting`（人格そのままの挨拶）を使う。時間帯は「おはよう」「こんばんは」のように挨拶自体が指していればそれに合わせ、`hi` / `やっほー` のように時間帯を含まない挨拶は実時刻（5〜10時=朝 / 11〜16時=昼 / それ以外=夜）で決める。朝・夜は先頭の挨拶語だけを差し替えて二重挨拶を避け、キャラクターの挨拶が無い場合だけ既定文へ落とす。
- キャラの通常ドラッグはOS標準のウィンドウ移動とする。通常2Dは従来の上半身位置 `upper` を標準とし、長身・実写・Live2Dは長身でも腰まで入りすぎない胸上 `bust` を標準にする。Live2Dはモデル別の`live2d.scale` / `offsetY`で胸上へ調整済みなので、未保存時にPNG向け`bust`倍率を重ねず、中立の`upper=1.55`を基準倍率として扱う。立ち絵の構図は設定画面の「キャラごとの構図」で編集し、表示形式ごとにサイズスライダーとプレビュー上のドラッグ（数値入力も併設）で調整する。`PICOAGENT_CHARACTER_FRAMING` はキャラIDと表示形式ごとに `zoom` / `x` / `y` / `preset` を保存する。表示側は `#characterView` に `translate(--char-x, --char-y) scale(--char-zoom)` を当て、中の `img` が持つ待機モーションと干渉させない。
- 設定画面には表示モニターの手動指定を置かない。新規作成時と再表示時に、メインウィンドウの `current_monitor()`（未確定時だけ主画面）を基準として同じディスプレイの中央へ配置する。
- `crates/picoagent-core` は将来のOSS化に備えた内部crateで、Tauri・OS・製品設定に依存しない正規形メッセージとUTF-8安全処理を持つ。`src-tauri/src/{message,text}.rs` は既存のアプリ内参照を保つ互換層で、実装はコアを再公開する。誤公開を防ぐため現在は `publish = false` とし、ライセンスと公開APIが決まるまで外部配布しない。設計ガードは `guard_scan::all_sources()` を通してコアのRustソースも走査する。
- 表示形式（画面構成）は `layout.rs` が担当する。`PICOAGENT_LAYOUT` 1キーで `mascot` / `pet` / `chat` / `bar` / `workspace` / `stream` / `widget` の7形式を切り替え、フロントは `<main id="petApp" data-layout="…">` を見て `styles.css` が配置を変える。`chat` は青灰色の壁紙と左右の白／緑吹き出しを持つLINE風表示。旧 `talk` は `chat` へ統合し、選択値、ウィンドウ寸法、キャラ別構図を読み込み時に移行する。**HTML の要素はどの形式でも同じ**で、キャラクター描画・会話履歴・アクションセンター・読み上げ・音声入力・添付・許可判定はすべて共通のまま使う。バックエンドの仕事は「選択中の形式」と「ウィンドウ寸法」の2つだけ。切替は設定画面の「画面・操作」にある実表示プレビューカードと、右クリックメニュー「画面レイアウト」の両方から行える。右クリックの「画面レイアウト・位置…」は同ページを直接開く。右クリックメニューは `layout::select()`、設定保存は `remember_current_size()` → 設定保存 → `apply()` の順に処理する。どちらも切替前の寸法を `PICOAGENT_WINDOW_LAYOUTS` へ覚えてから、次の形式の寸法を当てる。大きい形式へ切り替えてウィンドウが画面外へ出た場合は `pull_back_on_screen()` が見える位置へ戻す。
- `beta-full`付きのアルファ配布中だけ全形式に小型の`ALPHA`バッジを表示する。**表示は二段で決める。** 実行時は起動初期化が返すビルド種別（`betaBuild`）で決め、既定は `hidden` なので通常`build`と正式な`build-distribution`では出ない。加えて配信UIの生成時に、`build-ui` がアルファ指定以外（`dev` / `build` / `build-distribution`）では `ui-dist/index.html` からバッジの記述自体を取り除くため、販売物にはマークアップも残らない。バッジの記述は `ui/index.html` の1箇所だけで、剥がし処理と実際の記述がずれないよう xtask のテストが記述と除去結果を双方向で検査する。バッジは`pointer-events:none`で入力・ドラッグ・リサイズを妨げず、通常形式は右上、配信形式はLIVE表示・キャラ・操作列を避けたステージ左上へ置く。
- 開発中アルファ版はCargo feature `beta-full`を付けた`cargo xtask build-beta`だけがPro対象スキルを試用可能にし、製品情報には`ALPHA`と全機能試用中であることを返す。通常`build`と正式な`build-distribution`はこのfeatureを付けず、署名済みライセンスによるFree／Pro判定を維持する。
- グローバルホットキーは `hotkey.rs` が担当する（`tauri-plugin-global-shortcut`）。起動時と設定保存時に `apply()` が既存登録を全解除してから設定どおりに張り直す。呼び出しキーはウィンドウを表示・最小化解除・前面化してから `pet:hotkey`（payload `"summon"`）を送り、`renderer.js` が居眠りを解除して入力欄を開く。隠すキーは表示中なら `hide()`、隠れているときは呼び出しと同じ動作をする。**呼び出しキーが登録できていない状態で隠れたままにはしない**（右クリックメニューにも手が届かず操作不能になるため、`apply()` が非表示のウィンドウを表示へ戻す）。終了時は `release()` が登録を全解除して OS へ返す。空欄は他の設定欄と同じく既定値の意味なので、片方だけ止めるときは `off` を指定する。
- `Tracer`（`tracer.rs`）は Tauri managed state として登録され、`agent.rs` がツール実行ごとにレイテンシを計測して直近 200 件を保持する。`pet_traces` コマンドでスナップショットとツール別集計（平均レイテンシ・呼び出し回数・エラー数）を取得できる。
- `usage.rs` はトークン使用量を `app_data_dir/usage.json` へ永続化する。`pet_usage` コマンドで今月のプロバイダ/モデル別集計と全期間累計を取得できる。90日より古いレコードは自動削除。`app_data_dir` を取得できない場合は一時領域へ降格せず、空集計で会話を継続しつつ診断ログへ記録する。
- `memory.rs` の長期メモリ検索が意味的類似度（バイグラムジャカード係数）に対応した。`context_for_query(app, cfg, query)` がユーザー発話に近いメモリ項目を優先して提示し、閾値（0.05）以下なら全件フォールバック。
- 設定の正本は `config.rs`（`app_config_dir/config.json`）。コネクタは `cfg.get()` で都度読むため、設定変更は再起動なしに反映。
- 会話履歴は `history.rs` がキャラクター別に `app_data_dir/history-<キャラクターID>.json` へ永続化する（対話データなので config とは分離）。`pet_send` のたびにコンテキストを圧縮して選択中キャラのファイルへ保存し、起動時に末尾 20 件を文脈へ復元する。保存先取得や既存ファイル読み取りに失敗した場合は空履歴として保存可能状態へ進まず、エラーを上位へ返す。JSON破損時は元ファイルの退避成功を確認してからだけ空履歴で継続し、退避できなければ上書きを止める。**保存を止めている状態は `pet_init` の `historySaveBlocked` でフロントへ返し、起動時に画面へ知らせる**（ログだけだと、利用者は保存されていると思ったまま会話を続け、再起動でその分がまるごと消える）。圧縮は2段階: ① `maybe_summarize_context()` がsystemを含むトークン推定量（日本語 ×2・ASCII ÷4）が予算の70%を超えた場合に古い40%のターンをLLMで要約し、systemメッセージ内の自動要約1ブロックを置換更新する（要約失敗時は元の会話を保持）。② `compacted_conversation()` で予算上限を超えた発話を古い順に削除する（最低 2 件は保持）。予算はlocalなら `PICOAGENT_LOCAL_CTX` の75%（下限1,500／上限24,000トークン）、コンテキスト長を取得できないプロバイダは6,000トークンで、system文の消費量も含める。これにより再起動をまたいで直近の会話を踏まえて話す。途中の tool 呼び出し/結果は順序破綻を避けるため文脈・履歴に残さない。`AppState.character_id` が文脈の持ち主キャラを保持し、`pet_set_character_profile` でキャラが変わったら現在の文脈を破棄して切替先キャラの履歴を復元する（旧キャラの一人称・自己紹介が混ざらない）。`AppState.conversation_epoch` はリセット・キャラ切替・設定再初期化で進み、応答中だった `pet_send` / `pet_send_image` / `pet_idle_chatter` / 承認待ちコマンドが後から完了しても、開始時のキャラIDまたは世代と異なる場合は履歴ファイル・画面履歴へ書き戻さない。旧仕様の単一 `history.json` は起動時に既定キャラ `mugi` のファイルへ一度だけ移行する（旧履歴はキャラ別管理が無かった時代のもので当時の既定キャラが mugi のため。選択中キャラへ割り当てると人格が混ざる）。さらに旧版が短い挨拶へ追記していた内部ガイドは、起動時に全 `history-*.json` を対象としてユーザー原文へ戻し、未選択キャラへ切り替えた後も文脈へ再混入させない。
- 暗黙的な長期メモリは `memory.rs` が `app_data_dir/memory.json` に保存する。ユーザーの自己情報・好み・継続指示らしい文だけを保守的に抽出し、起動時と設定変更時に system 文脈へ短く差し込む。会話履歴がキャラクター別なのに対し、長期メモリはキャラクターIDを持たない**全キャラクター共通**のデータである。設定画面の「基本設定」→「覚えたこと」は本文・`updated_at`・`hits`を一覧表示し、分類・本文・最終更新日時が一致する1件だけを`store::lock_state("memory.json")`の中で削除する。削除後は`AppState.memory_context`とsystemメッセージを即時更新し、削除前の世代で進行中だった応答の書き戻しも拒否する。人格やプロンプトの変更（`reset_conversation`）では会話の流れを保持し、右クリックメニュー「表示と会話をリセット」→「会話履歴と覚えたことを消す」→「消す（元に戻せません）」（`clear_conversation_and_history`）では、選択中キャラの履歴ファイルと全キャラクター共通の長期メモリの削除が両方成功してから文脈・直近雑談を消して `pet:reset` を通知する。削除に失敗した場合は画面状態を成功扱いで消さず、エラー通知と診断ログを残す。

---

## メッセージ正規形（コネクタ間の共通言語）

`agent.rs` はプロバイダ固有のワイヤ形式を一切知らない。正規形は `crates/picoagent-core/src/message.rs` が持ち、アプリ内では互換層 `src-tauri/src/message.rs` を通して参照する。各プロバイダが自分の API 形式へ相互変換する。

| role | 形（serde） |
|---|---|
| system / user | `{ role, content, images?: [base64 data URL, ...] }` |
| assistant | `{ role:"assistant", content, tool_calls?: [{ id, name, arguments(Value) }] }` |
| tool（結果） | `{ role:"tool", tool_call_id, name, content }` |

- `arguments` は **`serde_json::Value`（オブジェクト）** で持つ（OpenAI の JSON 文字列はプロバイダ内で相互変換）。
- Ollama は tool 結果を id で紐付けず位置・名前で扱うため `tool_call_id` は無視（正規形には残す）。
- `images` は **マルチモーダル入力**用のフィールド（省略可）。base64 data URL 文字列の配列。プロバイダが OpenAI 互換の場合、user メッセージの `content` を `[{type:"text",...}, {type:"image_url",...}]` 配列形式へ変換して送信する。`usage` フィールドは ephemeral（`#[serde(skip)]`）で、プロバイダから返ったトークン数を一時的に保持する。
- **手元の CLI エージェントを叩く AI にする（`providers/agent_cli.rs`）**: Claude Code / Codex CLI を非対話（`claude -p` / `codex exec`）で1往復だけ呼び、テキスト応答を受け取る。通常は `PICOAGENT_AGENT_CLI_KIND` を使い、マルチAIでは候補のモデル欄へ `claude` / `codex` を指定して呼び分けられる。Claude は safe mode かつ組み込みツールなし、Codex はユーザー設定なし・read-only sandbox・一時セッションで起動し、**CLI 自身のツールから PicoAgent の権限境界を迂回させない**。ツール呼び出しは OpenAI 形式を受け付けないため、**プロンプト側の約束事**で代替する——使えるスキルの一覧（名前・説明・引数キーだけに削ったもの）を渡し、「ツールを使うときは `{"tool":..,"args":..}` だけを返す」と指示して、その JSON を `tool_calls` へ変換する。コードフェンスや前後の文章は剥がして最初の JSON オブジェクトだけを見る。**JSON でない出力は通常の返答として扱い**、**一覧に無いツール名も採用しない**（幻覚をそのまま実行へ回さない）。約束が守られなくても壊れず、単にツールなしの応答になる。プロンプトは引数として渡し、シェルは経由しない。会話は役割ラベル付きで1本に畳み、長すぎる場合は**古い方から**落として直近の指示を残す。CLI の導入とログインは利用者側で行い、配布構成の既定にはしない。
- **スクリーンショット解析**: `pet_capture_screen` コマンドがデスクトップ画像を PNG base64 data URL で返す。取得は**外部コマンドを介さず** `xcap` で主モニタを直接キャプチャし、PNG へエンコードして返す（以前は Windows で PowerShell + GDI、macOS で `screencapture` を起動していた。起動が重く、実行ポリシーやプロファイルの影響を受け、失敗の切り分けもしづらかった）。**対応 OS は Windows / macOS / Linux**。LinuxではX11に加えてWayland／PipeWire経路を組み込み、OSの画面共有許可やポータルが拒否・未提供の場合は理由付きエラーを返す。対応対象外のOSではビルドを維持するスタブになる。取得と PNG 変換は同期処理なので `spawn_blocking` で UI スレッドから逃がす。フロントエンドは「画面を見て」「スクリーンショット」などの発話を検出し、自動で `pet_capture_screen` → `pet_send_image` へ切り替えてビジョン対応モデルへ送る。
- **インライン画像表示**: `appendLinkedText()` が `![alt](url)` Markdown 画像記法を `<img>` 要素としてフキダシ内にインライン表示する。DALL-E 3 生成画像や外部 URL の画像をそのまま確認できる。

---

## エージェントループ（`agent.rs`）

`run(cfg, provider, model, messages, max_steps = 5, app = None) -> Result<String>`

1. `provider.chat_stream()` を呼び、テキストチャンクを `on_chunk` コールバック経由で `pet:stream_chunk` イベント送信しつつ、完全な assistant を `messages` に積む。フロントは受信チャンクをフキダシへ逐次表示し、文末（句点・改行）まで確定した分から順に読み上げ合成を先行させる（生成完了を待たずに話し始める）。**ステップごとに `pet:stream_reset` を先に送る**ため、ツール実行を挟んだ再 chat では前ステップの途中テキスト（「検索しますね」等）が最終回答へ連結表示されず、読み上げも仕切り直される。履歴へ積むのは最終応答だけ。assistant の本文は `message::strip_reasoning()` で `<think>...</think>` の推論ブロックを除去してから最終回答・履歴へ渡す（ストリーミング中は除去前の生テキストが流れるため、フロント側の読み上げも同じ規則で推論ブロックを除いてから合成する）（DeepSeek-R1 / Qwen3 系などの reasoning モデルで英語/中国語の推論が混ざるのを防ぐ）。
2. `tool_calls` が無ければ、その `content` を最終応答として返す。
3. `tool_calls` があれば、全呼び出しを `futures::future::join_all()` で**並列実行**し、結果を `tool` メッセージとして積み 1 に戻る。
4. `max_steps` を超えたら打ち切り、最後の文面を返す（暴走防止）。
5. 各 chat 呼び出しは `chat_with_retry()` でラップし、失敗時に最大 3 回・指数バックオフ（1秒→2秒）でリトライする（4xx 認証エラーは即中断）。
6. chat 完了後、返ったトークン使用量（`Message.usage`）を `usage::record()` で `app_data_dir/usage.json` に永続化する。
7. ツール実行は `Tracer::start_span()` でレイテンシを計測し、`Tracer::record()` で直近 200 件を保持する。
8. ツール出力は `PICOAGENT_TOOL_OUTPUT_MAX_CHARS`（既定 `auto`）で打ち切り、コンテキスト超過を防ぐ。`auto` は local ならコンテキスト長の45%（1,000〜60,000文字）、クラウドなら24,000文字。また `tools::check_injection()` でプロンプトインジェクション試行パターンを検査し、検知時は結果を警告付きでラップしてからメッセージへ積む。**同じ検査を添付ファイルの抽出本文にも適用する**（`commands.rs` の `mark_untrusted_if_injection`）。添付本文はツール結果ではなくユーザー発話の一部として渡るため、ここで検査しないと「受け取った文書に指示を仕込む」経路だけが素通りしてしまう。ツール引数 JSON の解析失敗（`_raw` フィールド）を検出した場合はツールを呼ばずエラー文字列を返してモデルへ再試行を促す（`dispatch_or_error()`）。
9. `PICOAGENT_PLANNING_ENABLED` または `PICOAGENT_SUPER_AGENT` が有効、かつ質問が複雑（120文字超・疑問符2個以上・手順/ステップ系語を含む）な場合、`provider.chat_with_options(json_mode=true)` でサブタスクへ分解し（`planning_step()`）、分解結果を system 追記として付与してから通常の chat ループへ進む。
10. `PICOAGENT_SELF_EVAL_ENABLED` または `PICOAGENT_SUPER_AGENT` が有効な場合、最終応答が短すぎる（8文字未満）または明示的な拒否パターンを含む場合に `provider.chat()` で1回再生成する（`self_improve()`）。再生成に失敗した場合は元の応答をそのまま返す。
11. **スーパーエージェントモード（`PICOAGENT_SUPER_AGENT=true`）**: 上記の計画・自己評価をまとめて有効化し、トップレベルの `max_steps` を `PICOAGENT_SUPER_AGENT_MAX_STEPS`（既定 12、呼び出し値〜30 にクランプ）へ引き上げて多段のツール連鎖を回し切る。計画開始・計画内容・「ステップ k/N を実行中…」・「ツールを実行中: …」を `pet:agent_status` イベントでフロントへ送り、計画→実行を可視化する（フロントはステータス行に表示）。ツールの実行有無・対象・成否は表示文言から推測せず、`pet:agent_activity` の `started` / `completed` / `waiting` と `tools` / `failedTools` で構造化して通知する。
12. **ネスト深さガード**: `run()` は `tokio::task_local!` の `AGENT_DEPTH` を非同期タスクごとに増やす。`subagent` スキルはこの深さを見て `PICOAGENT_SUBAGENT_MAX_DEPTH`（既定 2）以上ならネストを拒否し、無限再帰を防ぐ。プロセス全体のカウンターにしないことで、並行する兄弟AIを相互にネスト扱いしない。計画・自己評価はトップレベル（深さ 1）でのみ行い、サブエージェント内（深さ 2 以降）ではスキップする。

通常の質問では、ローカルモデルの `tool_calls` 対応状況に依存させず、最初の `chat()` より前に
必要なスキルを直接実行する。**Web検索は質問が事実確認・最新情報・定義などを要するとき（`should_web_search`：明示依頼、または「最新/ニュース/価格/誰/いつ/とは…」等のマーカー）だけ実行する**。ただし「今日のニュース」「最新ニュースをまとめて」「ニュース教えて」「今日の出来事」などの一般ニュース要約依頼は、「ニューす」「にゅーす」などの軽い表記ゆれも含め、検索ポータルだけを返しやすい通常Web検索ではなく、Google News / Yahoo!ニュースのHTMLを対象にした `news_scrape` を先に実行して見出しと記事リンクを取得し、見出しベースで要約させる。リンクを出す場合は生URLではなくMarkdownリンクにする。雑談・感想・相談（「むぎかわいい」「ありがとう」「自己紹介して」等）では検索せず、キャラ本人として速く自然に返す（デスクトップペットとして毎回の検索遅延・ノイズを避ける）。天気の質問（`is_weather_query`）は `weather` スキルを、為替の質問（`currency_query` が通貨を読み取れたとき）は `currency_rates` スキルを、市場指数の質問（`market_query` が日経平均等を読み取れたとき）は `market_indices` スキルを**先に実行してその整形済み出力（天気アイコン・気温/為替・市場指数のスパークライン等）をそのまま返す**（小型モデルが tool_calls を出せず、かつチャートを再現できないため、モデルを介さず確実にグラフィカル表示する）。スキル直行応答も assistant 発話として履歴へ保存し、前回のユーザー発話だけが残って次の質問へ混ざらないようにする。代表的なリマインダー・集中タイマー・見守り・できたことメモ、クリップボード要約、おでかけ前チェック、出典付き調査レポート生成もスキルを先に実行する。その他のローカル操作系依頼では、明示的なWeb検索依頼がない限り事前Web検索を省き、モデルへスキル利用を優先する指示を追加する。為替は文中の通貨（ドル/円/ユーロ…）と「推移/チャート」「N日」を読み取って `base`/`symbols`/`chart`/`days` を組み立てる。短い挨拶は検索結果に引っ張られて表記解説へ逸れやすいため検索を省き、キャラクター本人として返す指示だけを追加する。検索結果は外部の信頼できない参考データとして区切ってユーザーメッセージへ
付加し、結果内の指示には従わないよう明記する。検索失敗もモデルへ渡し、ユーザーへ明示させる。
参照元はMarkdownハイパーリンクで回答させ、フキダシはMarkdownリンクと通常のURLを安全なDOMリンクへ変換する。さらに、事前検索で得た参照元URLのうち本文に未掲載のものを、最終応答の末尾へ `参考:` として確定的に添える（モデルがリンクを出し損ねても参照元が残る）。URL は場所を問わず読み上げ（`speech_text` とフロントの読み上げ前処理）で除外されるため音声には乗らない。`speech_text` はさらに、RSSダンプ行、スパークラインのブロック文字、絵文字（天気・為替チャート由来）も読み上げから除外する。

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
    // ストリーミング対応。デフォルト実装は chat() を呼んで完了後に on_chunk を1回だけ呼ぶ。
    // openai.rs / local.rs は OpenAI 互換 SSE（stream_chat を共用）、ollama.rs は
    // /api/chat の NDJSON をオーバーライドで受信し、on_chunk を逐次呼ぶ。
    async fn chat_stream(&self, cfg: &Config, model: &str, messages: &[Message], tools: &[Value], on_chunk: Arc<dyn Fn(String) + Send + Sync>) -> anyhow::Result<Message>;
    // JSON モードなどオプション付き chat。デフォルト実装は chat() へ委譲（json_mode は無視）。
    // openai.rs はオーバーライドして response_format: json_object を付与する。
    async fn chat_with_options(&self, cfg: &Config, model: &str, messages: &[Message], tools: &[Value], json_mode: bool) -> anyhow::Result<Message>;
}
inventory::submit! { ProviderReg { make: || Box::new(MyProvider) } }
```

> 接続先・APIキー・モデルは各メソッド内で `cfg.get()` から都度読む（フィールドに固定しない）。設定変更を再起動なしに反映するため。
> `on_chunk` は `Arc<dyn Fn(String) + Send + Sync>` で渡す（async_trait の Send 境界との相性のため参照ではなく Arc で共有する）。

### 同梱プロバイダ

| id | 叩く先 | 接続条件 |
|---|---|---|
| `local`（既定） | 同梱 llama-server `/v1/chat/completions`（OpenAI 互換） | エンジン同梱＋モデル取得済（`engine.rs` が管理）。Ollama 不要 |
| `ollama` | ローカル Ollama `/api/chat` | `ollama serve` 起動 |
| `openai` | OpenAI 互換 `/chat/completions` | 公式URLは `OPENAI_API_KEY`、互換URLは `OPENAI_COMPAT_API_KEY`。未設定なら `is_up()=false` |
| `anthropic` | Anthropic Messages API `/messages` | `PICOAGENT_ANTHROPIC_API_KEY` 設定（未設定なら `is_up()=false`） |
| `deepseek` | DeepSeek OpenAI 互換 `/chat/completions` | `PICOAGENT_DEEPSEEK_API_KEY` 設定（未設定なら `is_up()=false`） |
| `gemini` | Gemini の OpenAI 互換 `/v1beta/openai/chat/completions` | `PICOAGENT_GEMINI_API_KEY` 設定（未設定なら `is_up()=false`） |
| `xai` | xAI OpenAI 互換 `/chat/completions` | `PICOAGENT_XAI_API_KEY` 設定（未設定なら `is_up()=false`） |
| `mistral` | Mistral OpenAI 互換 `/chat/completions` | `PICOAGENT_MISTRAL_API_KEY` 設定（未設定なら `is_up()=false`） |

`openai` は `OPENAI_BASE_URL` を変えれば OpenAI 本体に限らず OpenAI 形式の任意ゲートウェイに接続できる。`OPENAI_API_KEY` は HTTPS の `api.openai.com:443` にだけ送り、互換先には別の `OPENAI_COMPAT_API_KEY` を使う。既定候補は `gpt-5.4-mini` で、設定画面では `gpt-5.5` / `gpt-5.4` / `gpt-5.4-mini` / `gpt-5.4-nano` を種類・サイズで選べる。
`anthropic` は正規形メッセージとツール呼び出しを Anthropic の `tool_use` / `tool_result` content block に相互変換し、`claude-opus-4-8` / `claude-sonnet-4-6` / `claude-haiku-4-5` を主要候補として表示する。
`deepseek` は `openai.rs` の `to_wire`/`from_wire` を再利用し、`deepseek-v4-flash` / `deepseek-v4-pro` を主要候補として表示する。`PICOAGENT_DEEPSEEK_THINKING` が有効な場合は V4 系リクエストに `thinking: {type:"enabled"}` を付ける。旧互換の `deepseek-chat` / `deepseek-reasoner` も設定画面から選べるが、新規設定は V4 系を推奨する。
`gemini` は Google の OpenAI 互換エンドポイントを使い、`openai.rs` の `to_wire`/`from_wire` をそのまま再利用する（裏は OpenAI 互換と同じ）。tools 付きで失敗したら tools 無しで一度だけ再試行する。
`xai` と `mistral` も OpenAI 互換変換を再利用する。設定画面では xAI は `grok-4.3` / `grok-build-0.1`、Mistral は Medium / Small / Large / Ministral 系の主要候補だけを表示する。
`local` は `openai.rs` の `to_wire`/`from_wire` を再利用し、tools 非対応テンプレートのモデルでも会話が成立するよう tools 付きで失敗したら tools 無しで再試行する。
Gemma 等のチャットテンプレートは user/assistant の厳密な交互（かつ user 始まり）を要求し、崩れると 400「Conversation roles must alternate」になる。自発雑談（assistant 発話の差し込み）や履歴復元で assistant が連続・先頭になり得るため、`normalize_for_local` で送信直前に正規化する（system は先頭へ集約、先頭の純 assistant は除去、連続する同 role の user/assistant は内容を連結して1つに畳む。tool 呼び出しを含むメッセージはそのまま通す）。

### 同梱推論エンジン（`engine.rs`）

`local` プロバイダのために、Ollama の代わりに llama.cpp の `llama-server` をアプリに同梱して使う。

アプリがプリセットとして提供するモデルは一般PCでの容量・メモリ・速度を優先し、原則として量子化済みGGUFに限定する。現在はGemma 4／Qwen3.5／Phi-4 miniがQ4_K_M、gpt-oss-20bがMXFP4である。利用者が明示する`custom` URLにはこの制限を適用しない。

- エンジン本体（`llama-server` + ggml DLL）はリポジトリに含めず、`xtask`（`bundled::fetch_engine`）（`cargo xtask setup engine`）で
  `src-tauri/resources/bundled/engine/` に取得。llama.cpp安定版v0.2.0が配布用に対応付けた`b10566`を正本とし、WindowsはVulkan、Ubuntu LinuxはVulkan、macOSはMetal対応の対象OS別資産名と固定SHA-256をGitHub digest・ダウンロード実体の両方へ照合する。配置後は`picoagent-engine.json`へタグ・資産名・SHA-256を残し、3項目が一致しない既存配置は再取得する。`LLAMACPP_ASSET`で資産を明示変更する場合も対応する`LLAMACPP_SHA256`を必須にする。`tauri.conf.json` の `bundle.resources` で配布物に同梱する。
- 起動時（既定が `local` のとき）`engine::ensure_and_start` を非同期実行:
  1. `PICOAGENT_LOCAL_MODEL_PRESET` が未設定または `auto` の場合、Windows では `Win32_VideoController.AdapterRAM`、共通でRAM総量を見て Gemma 4 系の `small` / `balanced` / `large` を初回自動選定し、`config.json` へ保存する。手動選択では Qwen3.5 4B／9B、OpenAI gpt-oss-20b、Phi-4 mini、LLM-jp-4 8B Thinking の GGUF も選べる。旧Qwen／Llama／Mistral／PhiプリセットIDは同等負荷帯の現行候補へ移行し、`custom` の場合だけ `PICOAGENT_MODEL_URL` を直接使う。
  2. 選定モデル GGUF がユーザーデータ配下の `PicoAgent/bundled/models/` に無ければプリセットURLまたは `PICOAGENT_MODEL_URL` からストリーミング DL（進捗を `engine:status {phase:"download", percent}` で UI へ）。旧 `app_data_dir/models/` または旧 `PicoAgent/models/` に取得済みモデルがある場合は初回に新保存先へ移行する。
  3. Windowsは実GPUを判定し、Turing以降かつドライバー580以上のNVIDIAは`CUDA 13.3 → Vulkan → CPU`、AMD公式HIP Runtime対応SKUは`ROCm 7.14 → Vulkan → CPU`、Intel Arc／Iris Xe／Core Ultra系の`Intel Graphics`／Data Center Flex・Max／Server GPUは`SYCL → Vulkan → CPU`の順に起動する。Intel SYCLは対応済みだが、想定利用者に占めるIntel GPUのシェアを踏まえて主要な訴求・優先対応の対象には置かず、推奨構成の案内はNVIDIA CUDAまたはAMD ROCmを主軸にする。旧Intel UHD／HD Graphics、公式表にないAMD、Maxwell／Pascal／Voltaまたは古いドライバーのNVIDIAは専用方式を取得せずVulkanから始める。追加配布物は対象GPUの確認後だけ取得し、固定サイズ・SHA-256・安全な展開・`--list-devices`の実認識を通ったステージングだけを原子的に有効化する。追加方式の準備または起動に失敗しても同梱Vulkanへ戻る。`PICOAGENT_LOCAL_NGL=0`は追加取得を行わずCPU固定を優先する。AMD／Intel対応境界はllama.cpp固定版または配布版の更新時に各社公式対応表を再確認する。
     移行判定と旧ファイル削除前の照合は数GBのSHA-256になるため、照合対象が実在するときだけ `engine:status {phase:"verify"}` を出す（無表示だと起動が固まったように見える）。
     旧保存名の削除対象は、廃止済みプリセット（`RETIRED_MODEL_FILES`）と**選択中プリセットの旧保存名だけ**に限る。全プリセット分を消すと、利用者が別プリセットとして持っている実体まで巻き込んで再取得になる。
  4. `llama-server` は実行時配置の `PicoAgent/bundled/engine/`、同梱配置 `resources/bundled/engine/`、旧同梱配置 `resources/engine/`、開発時の `src-tauri/resources/bundled/engine/` / `src-tauri/resources/engine/` の順に探し、`cargo xtask dev` / `cargo xtask build` は事前に `xtask`（`bundled::fetch_engine`） で同梱配置を確認・取得してから Tauri を起動する。起動時は `llama-server -m <model> --host 127.0.0.1 --port <PICOAGENT_LOCAL_PORT> -c <PICOAGENT_LOCAL_CTX> -ngl <PICOAGENT_LOCAL_NGL> --jinja`
     を spawn（`PICOAGENT_LOCAL_GPU_DEVICE` 指定時は `--device <値>` を追加。Windows は CREATE_NO_WINDOW）。
     Qwen3.5だけは、内部推論をキャラクター発話へ混ぜずツール解析を安定させるため、`--chat-template-kwargs {"enable_thinking":false} --reasoning-budget 0` も追加する。
     既定の同梱ビルドは Vulkan(GPU) 版で、`-ngl 999` により全レイヤを GPU へオフロードする（GPU 非対応環境では自動 CPU フォールバック）。
     `/health` が 200 になったら `engine:status {phase:"ready"}`。前回クラッシュなどで既存 `llama-server` が同じポートに残っている場合は、`/v1/models` のモデルIDが要求中の GGUF と一致する時だけ再利用する。別モデルなら誤用せずエラーにし、残存プロセス停止を促す。
- 子プロセスは `RunEvent::Exit` で停止（`engine::stop`）。状態は `engine_status` command でも取得可能。
- 設定画面の「モデル」欄（`PICOAGENT_MODEL`）でローカルプリセット（Gemma / Qwen / OpenAI / Phi / LLM-jp / custom）を選ぶと、`local::model_url` がその選択を取得URLへ反映する。`settings_save` は `PICOAGENT_MODEL` / `PICOAGENT_LOCAL_MODEL_PRESET` / `PICOAGENT_MODEL_URL` などローカル関連の変更を再起動対象とし、再起動後の起動シーケンスで `ensure_and_start`（未取得モデルは自動ダウンロードしてから起動）を行う。
- 同梱 Aivis の状態は `aivis_status` command で取得する（フロントの起動時キャラ描画ゲートに使用）。選択中キャラクターの AivisHub 音声モデル導入は `pet_prepare_voice_model` command で起動シーケンス中に明示実行する。

### AivisSpeech Engine 連携（`aivis.rs` / `pet_speak`）

`PICOAGENT_TTS_ENABLED` が有効な場合、回答テキストを AivisSpeech Engine の `/audio_query` → `/synthesis` へ渡して WAV を生成し、フロントで再生する。音声再生中のみ会話アニメーションを維持する。起動時・キャラクター変更時に吹き出しへ出る挨拶（`character.json` の `greeting`）も同じ経路で読み上げる。

- 読み上げにはPicoAgent管理下の Aivis だけを使う。通常開発版はアプリ資源、軽量販売版は初回／設定画面で選択導入した利用者データ側を解決する。接続先は `http://127.0.0.1:<PICOAGENT_TTS_PORT>`（既定 `11436`）とする。旧固定ポート `10102` が保存されている場合は起動時に旧既定扱いで `11436` へ移行する。`PICOAGENT_TTS_ENABLED` が有効な場合はアプリ起動時に AivisSpeech Engine を即 spawn する。設定画面で表示する音声項目は ON/OFF と音量だけにし、デバイス・ポート・speaker ID は通常UIでは非表示の内部設定として扱う。`pet_speak` 時点で `/version` が未応答なら ready まで待つ。
- フロントは `pet_speak` が返す WAV バイト列を `Blob` / `Audio` で再生するため、Tauri の CSP は `media-src 'self' blob: data:` を許可する。再生時は `PICOAGENT_TTS_VOLUME`（0〜100）を `Audio.volume` に反映し、設定保存後の `pet:ui` で再生中の音量も即時変更する。再生失敗時はエラー内容をステータスへ表示する。
- 推論デバイスは内部設定 `PICOAGENT_TTS_DEVICE` で `auto` / `cpu` / `gpu` を選ぶ。`gpu` は `--use_gpu`、`cpu` は `--no-use_gpu` で起動する。`auto` は起動前に GPU 候補を確認し、Windows では NVIDIA / Radeon・Vega・Graphics 表記のある AMD APU/表示アダプタ / Intel 系の表示アダプタを候補として、GPU 候補があれば `--use_gpu`、なければ `--no-use_gpu` で起動する。AMD CPU だけの環境は GPU 候補にしない。CPU/GPU の明示指定は自動判定より優先する。
- 初回合成では話者モデルのロードで時間がかかるため、Aivis への HTTP タイムアウトは 180 秒にする。キャラクター固有の AivisHub モデル導入は `pet_prepare_voice_model` で事前に行う。呼び出しは画面起動後のバックグラウンド準備だけなので、長い AIVMX ダウンロードの待ち時間・失敗は状態表示へ出さず `app` ログへ残す（すでに利用者が使っている画面の状態表示を上書きしないため）。
- 実行時ログはユーザーデータ配下の `PicoAgent/log/` にまとめる（Windows 例: `%APPDATA%\PicoAgent\log\`）。バックエンドのパニックは `std::panic::set_hook` で、フロントは大域 `error`/`unhandledrejection` ハンドラ、`window.picoagent` の `invoke` ラッパ、Tauriイベントの購読・同期／非同期コールバック・解除、WebAudio等の非同期解放失敗で捕捉し、いずれも `app` ログへ文脈付きで記録する。アプリ本体とフロントのエラーは `app`、Aivis の stdout/stderr は `aivis`、同梱 llama-server の stdout/stderr は `engine` のログに追記する。起動時更新確認はオフライン・タイムアウトだけを正常な見送りとし、通信異常・フィード解析失敗は画面を妨げず `app` ログへ残す。ログ保存先自体を取得できない場合に限り、失敗を記録する最後の手段として一時領域へログを退避し、stderrにも理由を出す。ログは 1 日ごとに日付付きファイル（`app.<YYYY-MM-DD>.log` など。`log_paths::dated_name` が当日日付を挟む）へローテートし、保持期間は 1 週間（`LOG_RETAIN_DAYS=7`）。起動時に `prune_old_logs` で古い `.log` を削除し、長時間連続起動でも 1 日ごとのタイマーで掃除する。**会話履歴（`history-<キャラID>.json`）はログではないためローテーション・削除の対象に含めない。** Aivis プロセスが起動直後に終了した場合や ready 待ちがタイムアウトした場合は、当日の `aivis` ログの末尾をエラー文字列に含めてフロントへ返す。旧 `app_log_dir/aivis.log` または旧 `app_data_dir/log/aivis.log` がある場合は、新しい当日ファイルが未作成のときだけ初回に移行（コピー後に旧ファイルを削除）する。
- 状態は `aivis_status` command（`{ enabled, up }`）で取得できる。**音声エンジン・音声モデル・ローカルAIの準備を起動画面で待たない。** 起動画面が待つのは選択中キャラクターの描画と会話履歴の復元だけで、そこまで済んだら `pet_finish_boot` が保存中の表示形式と寸法へ戻して `is-booting` を外す。音声モデルの導入（`pet_prepare_voice_model`）とローカルAIの準備待ちは、画面が使える状態になってから `startBackgroundStartupServices()` がバックグラウンドで進める。TTS 有効かつ音声モデルの準備が済んだ時点で起動時の挨拶を合成して再生する（`prepareSpeech` → `playPreparedSpeech`）が、**待っている間に会話が進んでいたら（会話リセット・履歴の増減・応答中・再生中）挨拶は割り込ませずに捨てる**。TTS 無効時（`enabled=false`）は合成も再生もしない。
- 実行ファイルはユーザーデータ配下の `PicoAgent/bundled/aivis/` から起動する。同梱配置 `resources/bundled/aivis/` や旧同梱配置 `resources/aivis/` は起動元にせず、初回にユーザーデータ配下へコピーしてから使う。同梱配置の `run.exe` がユーザーデータ側より新しい場合は起動前に上書きコピーする。Aivisの配布対象版は `component-manifest.json` の `targetVersion` を正本とし、実行時取得と `xtask`（`office::fetch_aivis`）の既定取得版が同じ値を読む。手動実行は `cargo xtask setup aivis`、一時的な取得確認だけは `AIVIS_RELEASE=latest` またはタグで上書きできる。macOS で `.7z.001` 展開用の `7z` / `7za` が無い場合、事前取得スクリプトは音声同梱だけを警告扱いにして起動を続行する。音声同梱を必須にする場合は `AIVIS_REQUIRED=1` を指定する。配布物では `tauri.conf.json` の `bundle.resources` で `resources/bundled/**/*` を同梱対象にする。同梱 Aivis が見つからない場合、実行時にも同じ公式リリースを `PicoAgent/bundled/aivis/` へ自動取得して展開する（展開には `7z` / `7za` / `tar` のいずれかを使う。macOS の `.7z.001` は p7zip を必要とする）。大容量取得は接続30秒・受信無通信120秒の上限を持つが、取得全体の時間では打ち切らない。再導入と同梱版更新は固有のステージング先へ展開・検査し、旧版を退避してから原子的に入れ替え、失敗時は旧版を復元する。設定保存または再導入のために管理中プロセスを停止した場合は終了防止フラグを解除し、読み上げ有効時だけ再起動する。旧 `app_data_dir/aivis/` または旧 `PicoAgent/aivis/` に取得済みの Aivis がある場合は、新しい `PicoAgent/bundled/aivis/` へ初回移行する。非 Windows ではコピー・展開後と起動直前に `run` の実行権限を補正する。`.app` 配下など chmod できない同梱リソースは補正失敗だけで止めず、実行時の spawn 結果で判定する。
- PicoAgent が起動する Aivis 子プロセスには Windows で `APPDATA` / `LOCALAPPDATA` を `PicoAgent/aivis-userdata/` へ向ける環境変数を渡し、Aivis のモデル・辞書・キャッシュを PicoAgent のユーザーデータ配下に閉じ込める。
- speaker ID は内部設定として、選択中キャラクターの `PICOAGENT_TTS_CHARACTER_OVERRIDES[characterId].speaker` を優先し、未設定なら `default_character_tts_voice` のキャラ別既定声、対象外キャラだけ共通の `PICOAGENT_TTS_SPEAKER` を使う。組み込みキャラは、むぎ／Live2Dむぎ=`まお・あまあま`、りん／THA3りん=`るな・ノーマル`、まい=`まい・ノーマル`、ひより=`まお・ふつー`、はる=`花音・ノーマル`、イプシロン=`コハク・せつなめ`、まお=`まお・おちつき`、レン=`天深シノ・ささやき`、ライス=`阿井田 茂・Calm`、わんこ=`TANAKA・Happy` とする。通常の設定画面では speaker ID 入力欄を表示しない。`pet_prepare_voice_model` と `pet_speak` は、話者 UUID が `/speakers` に無ければモデル UUID の AIVMX を `/aivm_models/install` へ渡して導入し、その後 `/speakers` から話者 UUID＋スタイル名で**実 style ID を実行時に解決**して合成する。global style ID はエンジンが導入時に算出するためハードコードせず、既存の `styleId` 定数は解決失敗時のフォールバックに留める。ユーザーが上書き設定で明示した speaker は実行時解決せずそのまま使う。
- アプリが起動した子プロセスだけを `RunEvent::Exit` で停止する。既にユーザーが手動起動していた Aivis には触らない。

### 同梱Whisper CLI（`meeting_recorder`）

録音文字起こしには whisper.cpp の `whisper-cli` をCLI実行で使う。通常開発版はアプリ資源、軽量販売版は選択導入した利用者データ側を解決する。常駐サーバは立てず、ポートも使わないため、ユーザーが手動で起動している別Whisperサーバや別ツールとはポート競合しない。

- 実行ファイルとモデルはリポジトリに含めない。通常開発版は `cargo xtask setup whisper` で `src-tauri/resources/bundled/whisper/` に取得し、軽量販売版はwhisper.cpp公式Releaseと公式モデル配布から利用者データ側へ直接導入する。既存の手動インストール先、グローバルPATH、常駐サーバ用ポートには既定で触れない。
- `PICOAGENT_TRANSCRIBE_PROGRAM` / `PICOAGENT_TRANSCRIBE_MODEL` が設定されている場合は、手動インストール済みの実行ファイル・モデルを最優先する。空の場合だけ同梱 `whisper-cli` と同梱モデルを使う。
- 文字起こし時は `whisper-cli` を子プロセスとして1回だけ起動し、完了・失敗・タイムアウトで終了する。標準入力は使わず、`PICOAGENT_TRANSCRIBE_TIMEOUT_SECS` で上限時間を設ける。
- 設定画面から削除できるのは利用者データ側のPicoAgent管理領域だけとする。外部に手動導入した `whisper-cli` やモデル、アプリ資源側の開発用同梱物は削除しない。

### 初回追加機能ウィザード（`components.rs`）

初回セットアップは「用途 → 追加機能 → Live2D追加案内」の三段階とする。追加機能では Aivis／Whisper／THA3／LibreOffice／PDF・画像・文書ツールを個別表示し、選ばなくても本体、ローカルAIエンジン、Live2D表示は使える。「使い方」から初回セットアップ全体を再表示しても履歴・設定・追加済みキャラは削除しない。Aivisを導入しても `PICOAGENT_TTS_ENABLED` は変更せず、読み上げはミュートのままとする。Aivisは導入先の `engine_manifest.json` にある版と `component-manifest.json` の `targetVersion` を比較し、古ければ「更新あり」、新しければ「推奨版より新しい」、読取不能または比較不能なら「確認が必要」と表示する。版確認の失敗は詳細をログへ残し、他の追加機能の状態表示は継続する。更新・推奨版への復帰・修復・削除は、維持または削除されるデータを画面内に表示する二段階操作とする。二度目の操作を待つ状態は10秒経過またはフォーカス移動で解除し、別の操作を後から誤実行しない。`PicoAgent/aivis-userdata/` の音声モデル・辞書・設定には触れない。更新・確認件数は設定タブ、基本設定の入口、初回案内へ表示する。配布対象版と公式Releaseリンクが一致しなければ状態取得を失敗させ、配布設定のずれを成功扱いにしない。

- 第三者ソフトウェアをPicoAgentのReleaseへ再アップロードしない。`component-manifest.json` は `version=2` / `policy=direct-upstream`、公式取得元表示、管理対象で固定する `targetVersion` だけを持ち、自前パッケージURL・分割ファイル・公開済みフラグを持たない。
- `installMode=managed` のAivisとWhisperだけは、各公式配布元から利用者データ `PicoAgent/bundled/<id>/` へ直接取得する。GitHub Release APIのサイズとSHA-256 digest、Whisperモデルの固定SHA-256、HTTPS、上限サイズ、zip slip、展開容量を検査し、ステージングから原子的に入れ替える。削除はPicoAgent管理領域だけを対象とする。
- 追加機能の導入・更新・修復・削除はバックエンドの共通ロックで直列化し、初回案内と設定画面、複数カードから同時に実行されても管理領域を競合更新しない。
- `installMode=external` のTHA3、LibreOffice、Pandoc、FFmpeg、qpdf、Poppler、ImageMagick、ripgrepは公式ページを表示する。利用者が提供元の条件を確認して導入し、必要なら設定で実行ファイルやTHA3フォルダを指定する。PicoAgentは外部導入物を削除しない。
- ネットワーク未接続・タイムアウトは起動時確認を黙って見送る。初回／設定画面の明示操作では同じ画面内に「オフラインのため見送り」と表示し、エラー画面・モーダル・通常画面のフキダシを出さない。
- `diarize=true` のときだけ `PICOAGENT_TRANSCRIBE_DIARIZE_ARGS` を追加する。既定は whisper.cpp の tinydiarize 用 `-tdrz` だが、話者分類の精度とマーカー形式は利用するローカル実装・モデルに依存する。

---

## スキル（ツール）コネクタ

連結ファイルはプロジェクト直下の `skills/<スキル名>/mod.rs`、`Tool` トレイト・レジストリ本体は `src-tauri/src/tools/mod.rs`。スキル専用の補助モジュール、実行スクリプト、テンプレート、少量の静的アセットは同じ `skills/<スキル名>/` 配下へ同梱する。アプリとスキルの外部依存クレートは `src-tauri/Cargo.toml` に集約し、スキル配下には依存クレートそのものではなく、そのスキル専用のラッパーやスクリプトを置く。独立した `crates/picoagent-core` だけは自身の最小依存を持つ。

`Tool` トレイトを実装し、`inventory::submit!` で登録する。

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn label(&self) -> &'static str;
    fn description(&self) -> String;
    fn parameters(&self) -> Value;            // JSON Schema
    fn examples(&self) -> Vec<&'static str> { Vec::new() }
    fn intent_keywords(&self) -> &'static [&'static str] { &[] }
    fn intent_priority(&self) -> i32 { 0 }
    fn intent_invocation(&self, text: &str) -> Option<IntentInvocation> { None }
    fn config_schema(&self) -> Vec<Field> { Vec::new() }
    async fn run(&self, cfg: &Config, args: Value) -> String;  // 失敗もエラー文字列で返す
}
inventory::submit! { ToolReg { make: || Box::new(MyTool) } }
```

- `tools::build_schemas()` が `{ type:"function", function:{name,description,parameters} }` 形式の配列を生成しプロバイダへ渡す。
- `tools::dispatch(cfg, name, args)` が実行を仲介する。未登録ツールもエラー文字列で返す（エージェントが回復できるように）。出力が `PICOAGENT_TOOL_OUTPUT_MAX_CHARS` の上限を超える場合は末尾を切り捨て、コンテキスト超過を防ぐ。
- スキル実体、名前インデックス、function schema は `OnceLock` でキャッシュする。多数のスキルを導入しても会話ごとの schema 構築や dispatch 時の全スキル再生成を避ける。
- `tools::schemas_for_text(text)` がユーザー発話から必要そうな function schema を選び、通常会話で全スキル定義をLLMへ渡さない。`dispatch` は全スキルを呼べるため、これはモデルへ見せる候補の圧縮であり、内部スキルや明示呼び出しの実行能力は維持する。
- 候補判定語彙は各`Tool::intent_keywords()`、候補・先回り実行の優先度は`intent_priority()`を正本とする。`tools::all()`はスキル名順、候補が上限を超える場合は優先度の降順、一致語の長さの降順、スキル名の昇順で安定選択し、リンク順へ依存させない。
- 小型モデルのfunction callingを待たずに動かすスキルは、各`Tool::intent_invocation()`が引数と`IntentHandling`を返す。`Direct`は整形済み出力をそのまま利用者へ返し、`Material`はスキル結果を内部資料としてモデルへ差し込む。`agent.rs`は登録済みスキルを同じ優先順で走査して最初の一致を実行するだけで、スキル名・引数組み立て・分岐順を持たない。`run_command`／`ask_user`／`view_image`の制御マーカー解釈だけはループ本体の責務として残す。
- 資料差し込みの長文プロンプトは`skills/<スキル名>/prompts/*.md`を`include_str!`で同梱する。現在はニュース要約、クリップボード、Web検索、時刻未指定リマインダーのローカル操作ガイドがこの経路を使う。中央ルート表が空であること、`agent.rs`へスキル名と長文資料プロンプトを戻していないこと、代表発話の割当先・引数・直接返却／資料差し込みの別を設計ガードで検査する。
- 標準的な外向きHTTPは `tools::http::client()` の接続プールを共有し、待ち時間設定は `tools::http::timeout()` で範囲を統一する。User-Agent、Cookie、プロキシ、DNS固定・リダイレクト制御、長時間ストリーム等の専用構築が必要なコネクタは個別builderを使ってよい。個別builderは要求全体timeout、または長時間処理向けの接続timeout＋無通信timeoutを必須とし、設計ガードが全スキル・バックエンドを検査する。
- 共有クライアントは接続確立に 10 秒の上限を持つ（応答本文には掛からないので、モデルDLや生成のストリーミングは妨げない）。モデル一覧のような短い問い合わせは `tools::http::metadata_timeout()`（`PICOAGENT_HTTP_METADATA_TIMEOUT_SECS`）でリクエスト全体にも上限を付け、この値は基本設定の「詳細設定」から変更できる。
- `build.rs` は `skills/<スキル名>/mod.rs` を自動登録し、さらに `skills/` 配下を再帰的に `cargo:rerun-if-changed` へ登録する。これにより `skills/chitchat/trends.rs` や `skills/run_command/jobs.rs` のような補助ファイル変更もビルドに反映される。

### スキルの分類（エージェント設計としての位置づけ）

スキルの正体は **function calling のツール定義**（`name` / `description` / `parameters` の JSON Schema / `run()`）であり、MCP のツール定義と同じ層にある。README のスキル一覧はユーザー向けの**用途別**分類なので、設計上の位置づけはこちらで見る。

分類は2軸で捉える。**役割**（何をする道具か）と**副作用の強さ**（読むだけか、外部状態を変えるか）で、後者は MCP の `readOnlyHint` / `destructiveHint` に対応する概念である。副作用は「そのスキルが持つ最も強い操作」で判定する。

| 役割 | 副作用 | 本数 | スキル |
|---|---|---|---|
| **Retrieval**（外部知識の検索・取得） | read-only | 10 | `web_search` `web_fetch` `browser_control` `doc_search` `rss_feed` `news_scrape` `weather` `currency_rates` `market_indices` `time` |
| **Read**（手元の状態の読み取り） | read-only | 5 | `read_file` `clipboard_text` `glob_files` `grep_search` `view_image` |
| **Action**（メッセージ送信） | write | 9 | `gmail` `slack` `discord` `x_twitter` `mastodon` `misskey` `bluesky` `reddit` `linkedin` |
| **Action**（SaaS の読み書き） | write | 12 | `github` `jira` `linear` `notion` `todoist` `trello` `google_calendar` `google_docs` `google_sheets` `google_drive` `google_tasks` `microsoft_graph` |
| **Action**（ファイル生成・変換） | write | 9 | `ms_office` `document_convert` `pdf_tools` `image_tools` `image_generate` `comfyui` `meeting_recorder` `whisper_transcribe` `notebook_edit` |
| **Action**（資料一式の生成） | write | 2 | `genspark_suite` `spark_workspace` |
| **Action**（汎用の出入り口） | write | 3 | `http_api` `workspace_files` `edit_file` |
| **Destructive / 高リスク** | destructive | 6 | `trash_file` `run_command` `command_job` `code_runner` `app_control` `desktop_control` |
| **State**（アプリ内の永続状態） | write | 7 | `alarm` `reminder` `focus_timer` `task_scheduler` `praise_log` `todo_plan` `project` |
| **Computation**（純粋計算） | read-only | 1 | `productivity_tools` |
| **Orchestration**（多段実行・組み立て） | 呼び先に従う | 6 | `subagent` `workflow` `daily_briefing` `research_report` `chitchat` `ask_user` |

補足:

- **Destructive** の5本は安全のしくみ側で別扱いになる。`run_command` は allowlist 一致のみ、`command_job` は `run_command` が起動したジョブの様子見と停止のみ（起動はできない）、`code_runner` は最終手段、`trash_file` はゴミ箱への移動（完全削除はしない）、`app_control` はアプリ自身の終了。
- **Orchestration** は他スキルを呼ぶため、副作用は呼び先で決まる。`workflow` は手順を保存する点で State も兼ね、`research_report` は検索（Retrieval）と Markdown 書き出し（write）を兼ねる。`chitchat` は `agent_callable=false` の内部スキルで、モデルには提示せずアプリ側が直接 `dispatch` する。
- **Action（SaaS の読み書き）** は一覧取得と作成の両方を持つものが大半で、`gmail` のアーカイブ・既読化のように既存状態を変える操作も含む。
- **コーディングエージェント系**（`read_file` `glob_files` `grep_search` `edit_file` `notebook_edit` `todo_plan` `run_command` の `background` と `command_job`）は、Claude Code / Codex が持つ Read / Glob / Grep / Edit（apply_patch）/ NotebookEdit / TodoWrite（update_plan）/ Bash のバックグラウンド実行（BashOutput・KillShell）に相当する。読む → 探す（名前・内容）→ 部分的に直す → 走らせて様子を見る → 進捗を進める、という作業の流れを分担するため、単体ではなく組で使う。`edit_file` / `notebook_edit` は `workspace_files` と同じロック・取り消し履歴を共有するので、編集は `workspace_files` の `undo` で戻せる。
- **`view_image` と `ask_user` はツール結果（文字列）だけでは成立しないため、マーカー方式でエージェントループと連携する。** ツールは内部プロトコルのマーカー文字列を返し、`agent.rs` がそれを解釈する。`view_image` は画像付き user メッセージを会話へ差し込んでループを続け、`ask_user` は `run_command` の承認カードと同じくループを止めてユーザーへ質問を返す。マーカーは設定キーではないので `doc_sync` の `NOT_CONFIG_KEYS` に理由付きで登録してある。
- **プロジェクト管理は `project` 1本に統合してある。** 以前は「ワークスペース（環境の名前付きセット）」と「課題・記録」が別スキルだったが、ユーザーから見ればどちらも同じ「いま取り組んでいる案件」で、名前も概念も二重だった。1つの `Project` レコードが環境（許可ルート・追加許可パス・AI・モデル・キャラクター）と課題・記録を持ち、`switch` すれば全部が一緒に切り替わる。`todo_plan`（1本の作業計画）とは別物で、あちらは「いま取りかかっている手順」、`project` の課題は「残っている案件のやること」を持つ。
- `ask_user` は「聞かないとやり直しになる」場面専用で、既定値で進める・調べれば分かることには使わない。候補ツールの圧縮（最大12件）に載るのはキーワードルーティング経由なので、曖昧な依頼で必ず提示されるとは限らない。

### 多重起動の防止

**PicoAgent は同時に1つしか動かさない**（`tauri-plugin-single-instance`）。2つ目の起動は、既に動いているウィンドウを「表示 → 最小化解除 → 前面化」して自分は終了する（`window_surface::surface_main_window`。ホットキー呼び出しと同じ手順）。

理由は利用者データの保護。`store::lock_state` は**プロセス内 Mutex** なのでプロセスをまたぐと効かず、`config.json` / `memory.json` / `usage.json` / `schedule-runs.json` / `history-<キャラID>.json` はいずれも「全体を読んで全体を書き戻す」ため、片方の保存がもう片方の古いスナップショットで丸ごと上書きされる。原子的書き込み・破損退避・ロックで守ってきたものが、プロセス境界で崩れる。あわせてスケジュールの二重実行、同梱エンジンとローカルAPIのポート衝突も防ぐ。

### キャラクターの追加（ユーザー導入）

同梱キャラクターは `ui-dist/assets/characters/` としてビルド時に実行ファイルへ焼き込まれるため、実行時には増減できない。**自分で追加した Live2D モデル等は `app_data_dir/characters/<id>/` に置く**（`user_characters.rs`）。webview へは asset プロトコル（`convertFileSrc`）で読ませるので、`tauri.conf.json` の `app.security.assetProtocol.scope` を `$APPDATA/characters/**` に限定し、CSP へ `asset:` / `http://asset.localhost` を足してある（`tauri` の `protocol-asset` フィーチャが必要）。
保存先を取得できない場合はカレントディレクトリへ降格せず、一覧・導入・削除をエラーとして止める。これにより開発リポジトリや任意の起動フォルダへ利用者データを書き込まない。

CSP は `tauri.conf.json` の値を正本とする。`default-src 'self' asset: http://asset.localhost` は同梱UIと許可範囲内の利用者アセット、`script-src 'self'` は同梱スクリプトだけを許可する。`script-src 'unsafe-eval'` はLive2D描画層のPIXI 6.5.10（`ui/vendor/live2d/pixi.min.js`）がWebGLのバッファ・uniform同期関数を3箇所の`new Function`で生成するために限って残す。これを外したCSPではPIXIの実描画が動的コード実行不可エラーで停止し、同じ描画は許可時に成功することをヘッドレス比較で確認済み。`'wasm-unsafe-eval'` はCubism CoreのWASM実体化に必要なので分離して維持する。PicoAgent自身とGit管理するベンダのJavaScriptは`eval()`／`Function`コンストラクタを使わず、`bundle_scope::guard`が追加を拒否する。`style-src 'self' 'unsafe-inline'`は既存のインライン・動的スタイル、`img-src 'self' data: asset: http://asset.localhost`は同梱画像・添付・利用者キャラ、`media-src 'self' blob: data: asset: http://asset.localhost`は音声Blobと許可アセットのために維持する。`'unsafe-inline'`は`script-src`に無く、インライン`<script>`は動かない。

- 導入経路は設定画面の「キャラ・音声」タブ（zip またはフォルダを選ぶ）。選択は `settings_character_pick` が Rust 側の `tauri-plugin-dialog` で行い、フロントへはパスだけを返す（プラグインの JS バインディングを配信物へ足さずに済ませるため）。完成済みPicoAgentパックは `settings_character_install`、利用者が配布元から自分で取得した生のLive2Dデータは `settings_character_install_live2d` を使う。
- 生のLive2D導入は `.model3.json` を再帰的に探し、1個だけならその親フォルダを組み込み用データとしてコピーして `character.json` を自動生成する。0個または複数なら曖昧に選ばず拒否する。公式サンプルを含むモデル実体をアプリが代理取得・再配布しない。ランタイム取得は `cargo xtask setup live2d` と製品 `build` に限定し、最小の `cargo xtask dev` はネットワーク取得を行わない。開発者向けサンプル取得は `cargo xtask setup live2d-samples` の明示操作に限定する。製品ビルドは `distribution/release.json` の配布許可リストだけを同期する。初回案内は用途選択と追加機能選択の次にLive2D追加／公式配布ページ／後回しを選ばせ、追加時は `character-add` 経路でキャラタブの導入欄を開く。`pet_show_onboarding` は設定ウィンドウを隠してメインへ `pet:show-onboarding` を送り、保存済みデータを消さず同じ案内を再表示する。
- THA3生成経路は同じタブの「1枚絵からキャラクターを作成」。`settings_tha3_status` / `settings_tha3_pick_source` / `settings_tha3_pick_root` / `settings_tha3_generate` / `settings_tha3_cancel` を使い、512x512透過PNGから17キーポーズ・42フレームの一時パックを生成する。THA3モデル出力をPillow等で引き伸ばさず、実効解像度で配置枠を満たせない場合はパックを確定せず理由を返す。高解像度で直接生成する経路またはPhotoshop「ディテールを保持 2.0」の仕上げが用意されるまで、その素材を配布品質として扱わない。入力画像と手動選択したTHA3フォルダは共通permissionsで検証する。画像・フォルダ選択と選択解除は同時に走らせず、生成中も変更できない。中断要求に失敗した場合は理由を表示して再試行可能にする。画像プレビューはasset protocolの許可範囲を広げず、20MB以下の選択済みPNGだけをdata URLとして返す。生成パックは通常の `character_pack::validate` と `user_characters::install` を通った場合だけ確定する。
- キャラクターIDは**英数字・ハイフン・アンダースコアだけの64文字以内**とし、判定は `character_pack::is_valid_id()` の1箇所で行う。IDはそのまま `app_data_dir/characters/<id>/` のフォルダ名と設定キーの一部になるため、区切り文字や `.` を含む値を受け付けない。導入・削除・構図保存・配信設定・関係性・THA3生成はすべて同じ判定を通す。
- 選択したTHA3フォルダは `PICOAGENT_THA3_ROOT` に保存し、次回設定画面と生成時に再利用する。「選択解除」は空文字を保存し、開発用の自動検出へ戻す。販売用軽量版はTHA3本体・モデル・Python／PyTorchを同梱せず、利用者が作者配布元から用意したフォルダだけを使う。通常の開発環境では `cargo xtask setup tha3` がGit管理外の開発用領域へ準備する。
- THA3のPython子プロセスは `tha3_creator.rs` が `tools::process::command()` で起動し、標準出力・標準エラーを `tha3:progress` で設定画面へ送る。選択・状態確認・生成・中断の失敗は操作名付きで `app.log` に残す。長時間生成なので全体timeoutは設けないが、GUIキャンセルではkill後にwaitして回収する。設定画面を閉じる際はキャンセルを要求し、アプリ終了時はバックエンドもキャンセル後の回収完了を待つ。生成途中のスクリプト・画像・パックは `app_cache_dir/tha3-creator/` に限定し、成功・失敗・中断の全経路で掃除する。
- 導入は「一時フォルダへ展開/複製 → 検証 → `rename` で確定」の順。検証を通らなかったものが導入先へ残らない。zip は `enclosed_name()` で zip slip を拒否し、件数・総量・1ファイルサイズを展開しながら検査する。
- GUI生成・zip/フォルダ導入・削除は同じ `user-characters` 状態ロックで直列化し、一時導入フォルダや同じIDの確定先を同時に更新しない。
- 「zip を開くとモデル名のフォルダが1つ」という一般的な配布形は、1階層だけ潜って `character.json` を探して吸収する。複数見つかった場合は1体に決められないので拒否する。
- 削除は OS のゴミ箱へ移動する（ユーザーが持ち込んだデータのため物理削除しない）。保存中の選択キャラを削除するときは先に `PICOAGENT_CHARACTER` を既定へ戻し、ゴミ箱移動が失敗したら自分が設定した既定値のままである場合だけ元の選択へ復元する。成功応答は削除前の画面選択にかかわらず実際の保存中キャラを返す。設定画面は削除中のフォーム変更と保存を止め、キャラ切替キューの完了を待って一覧・選択欄・人格欄・保存スナップショットを再表示なしで更新する。設定・会話履歴は削除しない。
- 一覧はフロントが「同梱の `index.json`」と「`pet_user_characters` が返す追加分」を突き合わせて作る。同じ id は同梱側を優先し、追加分が同梱キャラを乗っ取らないようにする。

**検証規則の正本は `src-tauri/src/character_pack.rs` 1本**。`cargo xtask character-pack` は同じファイルを `#[path]` で取り込む（規則が分かれると「CLI では通るのにアプリが弾く」ズレが生まれる）。`user_characters::guard` のテストが、CLI 側に規則のコピーが復活していないかを検出する。Live2D は `model3.json` の `FileReferences` が参照するファイルの実在まで確かめる（テクスチャ欠けのまま導入すると、そのキャラを選んだ瞬間に描画が崩れるため）。参照は「拡張子を持つ文字列」で拾うので、`Moc` / `Textures` / `Motions` / `Expressions` のような形の違いに影響されない。

### RAG との関係

**スキルの仕組み自体は RAG ではなく tool use** である。RAG が「検索した文書をプロンプトへ足して生成する」読み取り専用の手法であるのに対し、スキルはモデルが呼ぶ関数であり、送信・変換・削除といった副作用を持つ。

そのうえで、上表の **Retrieval 群（9本）は RAG のリトリーバに相当する**。特にエージェントループの「原則 Web 検索」（通常の質問はローカルモデルの知識に頼らず `web_search` してから出典付きで答える）は、運用としては RAG である。既定が小型のローカルモデルであるため、知識不足を検索で補う狙いも RAG と同じ。

長期メモリ（`memory.rs`）も構造は RAG に近い。クエリとの関連度でメモリを絞り、system 文脈へ差し込む。ただし**埋め込みベクトルは使わず、文字バイグラムの Jaccard 係数＋言及回数のボーナス**でスコアリングする。埋め込みモデルを同梱せずに済ませる割り切りであり、日本語の短文には効くが言い換えには弱い。

### 同梱ツール

同梱 OSS CLI（Pandoc / ripgrep / ffmpeg / qpdf / Poppler / ImageMagick）は、`xtask`（`tools::fetch_all`）を `cargo xtask setup tools` または通常の製品 `build` 前処理から実行し、PicoAgent 管理下の `src-tauri/resources/bundled/tools/` へ取得する。最小の `cargo xtask dev` は自動取得しない。再取得は `TOOLS_FORCE=1`、対象を絞る場合は `PICOAGENT_SETUP_TOOLS=pandoc,ripgrep` のようにカンマ区切りで指定する。各スキルは `PICOAGENT_*_PATH` の明示設定 → 同梱 → PATH の順に実行ファイルを解決する。


| name | 機能 | 安全策 |
|---|---|---|
| `app_control` | PicoAgentアプリ自身を操作する。チャットで「アプリ終了して」「PicoAgent を閉じて」のように頼むと `quit` を実行する | スキル結果の `PICOAGENT_APP_QUIT_REQUESTED` をバックエンドが検出し、フロントが短く表示してから `pet_quit` で `app.exit(0)` を呼ぶ。単なる「終了」はタイマー等の終了と混ざるため、アプリ名またはアプリ文脈を含む依頼だけに使う |
| `get_time` | 現在の日付・時刻（任意で IANA タイムゾーン、`chrono-tz`） | 副作用なし |
| `weather` | Open-Meteo による現在天気・日別予報。天気アイコン＋気温スパークライン（時間別/日別）でグラフィカルに表示。地名未指定時は IP から現在地を自動推定 | APIキー不要、地名検索、timeout、自動位置は無効化可 |
| `currency_rates` | Frankfurter API による為替レート取得・換算・推移チャート（スパークライン。`chart=true` / `days`） | APIキー不要、timeout |
| `market_indices` | Yahoo Finance chart API による日経平均・TOPIX・NYダウ・NASDAQ・S&P500 の取得と推移チャート（スパークライン） | APIキー不要、timeout、User-Agent 指定 |
| `news_scrape` | Google News / Yahoo!ニュースのHTMLから最新ニュース見出し候補を取得。今日のニュース要約ではこのスキルを先に使う | 対象ドメインを Google News / Yahoo!ニュースに限定、HTML 1MB上限、件数上限、timeout、User-Agent 指定 |
| `rss_feed` | RSS/Atom フィードの最新記事取得 | 各リダイレクト先を含む公開HTTP(S)宛先検査、DNS解決先固定、最大5回、フィードサイズ1MB上限、件数上限、timeout |
| `read_file` | ローカルのテキストファイルを行番号つきで読込。`offset` / `limit` で範囲指定できる | 共通許可ルート（`PICOAGENT_ALLOWED_ROOT` / `PICOAGENT_ALLOWED_PATHS`）配下に限定、`..` やリンク経由のルート外を拒否。ファイル4MB・既定800行（最大2000）・1行800文字・出力128KB で打ち切り、続きの `offset` を案内する |
| `view_image` | 許可ルート配下の画像をモデルへ見せる（png / jpg / gif / webp / bmp、最大3枚） | 共通許可ルート配下に限定、1枚4MB上限。ツール結果には画像を載せず、`agent.rs` が画像付き user メッセージを会話へ差し込む。実際に読めるかはモデルがマルチモーダルかどうかに依るため、「非対応モデルでは無視される」旨をツール結果へ明記する。画像を運べる形式へは各プロバイダが変換する（OpenAI 互換は `image_url`、Ollama は `images` 配列、Anthropic は image ブロック） |
| `ask_user` | 判断が分かれる点を2〜4個の選択肢つきでユーザーへ確認する | 外部送信なし。マーカーを返してエージェントループを止め、質問文と選択肢をチャットへ返す（フロントはクイック操作欄へ選択肢ボタンを出す）。次のユーザー発話は番号・全角番号・ラベル一致なら選択肢名へ寄せ、それ以外は自由記述としてそのまま渡す。質問300文字・ラベル40文字・説明120文字で切り詰める |
| `glob_files` | 許可ルート配下のファイルを glob パターン（`**/*.rs` など）で探し、パス一覧を返す。既定は更新の新しい順 | 共通許可ルート配下に限定、シンボリックリンク／ジャンクションは辿らない、深さ12・走査20,000件・表示既定100件（最大500）で打ち切り、`node_modules` / `target` / `.git` など生成物と隠しフォルダは除外 |
| `grep_search` | 許可ルート配下のファイル内容を正規表現で検索し、行番号付きの一致行・一致ファイル・件数を返す（`output_mode`） | 共通許可ルート配下に限定、検索本体は同梱 ripgrep → PATH の順に解決。ripgrep が無い場合は内蔵走査（固定文字列一致）へ落ち、その旨を結果の先頭に明記する。1ファイル2MB・深さ12・走査5,000件・1行400文字・表示既定60行（最大300）で打ち切り、timeout 30秒 |
| `web_search` | Web検索結果のタイトル・URL・概要を返す。すべての回答前に必ず実行 | 検索語500文字、1〜8件、待ち時間3〜60秒、応答32KBで制限 |
| `research_report` | 複数観点のWeb検索を束ね、出典付きMarkdown調査レポートを生成する。`save=true` または `save_path` 指定で保存し、既定で1ページ要約 `.brief.md`、HTMLダッシュボード `.dashboard.html`、スライド下書き `.slides.md`、出典CSV `.sources.csv`、出典JSON `.sources.json` も生成する | 検索待ち時間3〜60秒、観点最大6件、各観点最大5件、レポート96KB上限、保存は共通許可ルート配下の `.md` のみ |
| `spark_workspace` | brief、deck、sheet、共有HTMLページ、エージェント実行計画、プロンプト集、チェックリストを含む資料ワークスペースを一括生成する | 保存は共通許可ルート配下のみ、入力テキスト24,000文字上限、外部通信なし |
| `genspark_suite` | Deep Research、AI Slides、AI Sheets、AI Docs、Sparkpage、Autopilot、AI Drive、Meeting Notes、Multi-Agent、AI Browser計画の10系統を同じフォルダに生成する | 保存は共通許可ルート配下のみ、入力テキスト32,000文字上限、外部通信なし |
| `subagent` | 単独委譲に加え、複数AIへ同じ質問を送る `consensus`、まとめ役が作業を割り振って統合する `collaborate`、分担結果を個別に返す `divide` を実行する | `agents` ごとに provider / model / 任意の focus を指定（2〜8件）。まとめ役 judge も別指定できる。local候補は同梱モデルの競合を避けて直列、その他は最大同時数まで並行実行。AppHandle は渡さず、タスク単位の `AGENT_DEPTH` と最大深さ（既定2）で再帰を制限。合議は候補数＋統合、分担＋統合は計画＋候補数＋統合だけ各AIを呼ぶ |
| `chitchat` | アイドル時の自発雑談の指示文を返す内部スキル（`agent_callable=false`、会話中はモデルへ提示しない）。アプリの待機タイマーが `pet_idle_chatter` 経由で利用 | `skills/chitchat/trends.rs` で X API / 急上昇RSS / ニュースRSSの話題メモを取得し、生成は人格込みで `provider.chat`（tools 無し）。生成後は「直前の自発雑談」として会話文脈へ短く残す |
| `alarm` | 目覚まし時計。声でやさしく起こすアラームを add/list/remove/clear/enable/disable できる。時刻(HH:MM)・曜日・繰り返し・スヌーズ分数を指定 | 一覧は `crate::alarm` 経由で config `PICOAGENT_ALARMS` に保存。発火はフロントの時刻ポーリングが `pet_alarm_wake` を呼ぶ。一度きりは発火後に無効化し、`おきたよ！` が押されるまで指定分数おきに再通知 |
| `task_scheduler` | 指定時刻または cron 風5フィールドで、許可済みコマンドまたは別スキルを定期実行する。add/list/remove/clear/enable/disable に対応 | 一覧は config `PICOAGENT_SCHEDULED_TASKS` に保存。バックエンド常駐タスクが30秒間隔で確認し、実行結果をログ、`app_data_dir/schedule-runs.json`、`pet:notice` へ送る |
| `reminder` | 日常リマインダー。指定時刻または数分後に声で知らせる | `PICOAGENT_DAILY_EVENTS` に保存。フロントの `pet_daily_events` ポーリングで期限到来分を通知 |
| `focus_timer` | 集中タイマーと作業見守り。終了時または指定間隔ごとに声で知らせる | `PICOAGENT_DAILY_EVENTS` に保存。見守りは repeat_minutes で次回へ繰り返す |
| `praise_log` | できたこと・進捗・がんばったことを保存し、あとで振り返る | `PICOAGENT_PRAISE_LOG` に保存。最大100件 |
| `daily_briefing` | おでかけ前チェック。天気・控えている通知をまとめる | `weather` を内部呼び出し。APIキー不要 |
| `meeting_recorder` | 会議向けの複合スキル。マイク録音、既存音声の文字起こし、文字起こしからの議事録Markdown生成を `action` で扱う。`diarize=true` で話者分類用の追加引数を渡す | 既定保存先は共通許可ルート配下の `meetings/YYYY-MM-DD/`。別の場所は `PICOAGENT_ALLOWED_PATHS` へ追加して絶対パスで指定する。`PICOAGENT_TRANSCRIBE_PROGRAM` / `PICOAGENT_TRANSCRIBE_MODEL` の手動設定が最優先で、空なら PicoAgent 管理下の `resources/bundled/whisper/` に置いた同梱Whisper CLIを使う。同梱Whisperは1回実行のCLIで、常駐サーバやポートを使わない。外部APIなし |
| `whisper_transcribe` | 既存音声をローカルで文字起こしする単体スキル。スキル一覧の表示タイトルは「音声文字起こし」 | `PICOAGENT_TRANSCRIBE_PROGRAM` / `PICOAGENT_TRANSCRIBE_MODEL` の手動設定が最優先。空なら PicoAgent 管理下の同梱Whisper CLIと同梱モデルを使う。常駐サーバやポートを使わない |
| `clipboard_text` | クリップボードのテキストを取得し、要約・返信文・TODO化の素材として返す | `arboard` で OS API を直接利用。取得文字数は `PICOAGENT_CLIPBOARD_MAX_CHARS` で制限 |
| `productivity_tools` | 文字数分析、大小文字変換、JSON整形/圧縮、Base64、URL変換、単位換算、日付計算、日付差分、週番号、timestamp、行分析、反転、行ソート、重複行除去、行trim、折り返し、URL抽出、メール抽出、キーワード数、チェックサム、CSV→Markdown表、割合、BMI、HEX/RGB変換、スラッグ化など30種類のローカル実用処理 | 設定不要、外部通信なし、入力テキスト20,000文字上限 |
| `workspace_files` | 許可ルート内のファイル一覧、テキスト読込、書込、追記、フォルダ作成、情報取得、直前変更の取り消し | 共通許可ルートと `permissions::fs` に限定、読込/書込サイズ上限。同一プロセス内の変更は直列化する。`write` / `append` は `preview=true` で無変更確認でき、成功した直近20件を1件1MBまでメモリ内保持する。`undo` は実行時点の許可を再検証してから復元し、新規ファイルはOSのゴミ箱へ移動する。アプリ再起動後は取り消せない |
| `edit_file` | 既存ファイルの一部を、元の文字列と新しい文字列の厳密一致で置換する。`edits` 配列で複数箇所をまとめて編集できる | 共通許可ルートと `permissions::fs` に限定、既存ファイルのみ（新規作成は不可）、512KB 上限、1回20件まで。一致0件と（`replace_all` なしの）複数一致はエラーにして書き込まない。途中の1件でも失敗したら全件書き込まない。`preview=true` で無変更確認。書き込みロックと取り消し履歴は `workspace_files` と共有し、`workspace_files` の `undo` で戻せる |
| `notebook_edit` | Jupyter ノートブック（.ipynb）をセル単位で読み・差し替え・挿入・削除する | 共通許可ルートと `permissions::fs` に限定、拡張子は `.ipynb` のみ、ファイル8MB・1セル128KB・500セル上限。書き換えたコードセルの `outputs` / `execution_count` は消す。`delete` は `confirm=true` が必要。ノートブックの実行はしない。取り消し履歴は `workspace_files` と共有する |
| `todo_plan` | 多段タスクの作業計画を登録し、1手ずつ進捗を進める（set / list / complete / add / clear）。完了すると次の手順が自動で着手中になる | `PICOAGENT_TODO_PLAN` に保存し、一覧更新は名前付きロックで直列化する。手順は20件・1件200文字まで。読み取りに失敗したときは空一覧で上書きせずエラーを返す |
| `project` | 案件ごとの作業環境（許可ルート・追加許可パス・AI・モデル・キャラクター）の登録・切替・登録解除に加え、課題台帳（登録・一覧・更新・完了・削除）と記録（メモ・決定事項・進捗）を同じレコードで扱う | `PICOAGENT_PROJECTS` に保存し、選択IDは `PICOAGENT_ACTIVE_PROJECT`。一覧更新は名前付きロックで直列化する。プロジェクト50件、1プロジェクトあたり課題200件・記録200件、タイトル120文字・記録300文字・タグ5個まで。`due` は `YYYY-MM-DD`（または `due_in_days` の相対指定）以外を拒否する。`forget` / `task_remove` / `note_remove` は `confirm=true` が必要で、`forget` のプレビューには一緒に消える課題・記録の件数を出す。記録は上限に達しても古いものを自動削除せず整理を促す（決定事項が黙って消えないように）。登録解除でファイルは削除しない。使用中IDを消したら選択状態も外す。許可ルートは次のスキル実行から、AI・モデル・キャラクターは再起動後に反映する。統合前の `PICOAGENT_WORKSPACES` / `PICOAGENT_ACTIVE_WORKSPACE` は引き継がず、読まない（統合前の環境設定だけのレコードが課題ゼロのプロジェクトとして復活しないように）。保存データは `id` 以外を `#[serde(default)]` にして、項目を足しても既存の一覧が読めなくならないようにする |
| `workflow` | 最大8件のスキル手順を名前付きで保存し、順番に再実行 | `PICOAGENT_WORKFLOWS` に保存し、一覧更新は名前付きロックで直列化する。登録時にツールの存在と引数形式を検証し、自身の再帰登録は禁止する。エラー結果で後続を停止し、各スキルの許可・確認・上限はそのまま適用する |
| `trash_file` | 許可ルート内の既存ファイル/フォルダをOSのゴミ箱へ移動 | 物理削除なし、共通許可ルート配下に限定、許可ルートそのもの・ルート外・Windows/Program Files 配下は拒否、`confirm=true` がない場合は preview のみ |
| `run_command` | 許可ルート内でローカルコマンドを実行。`background=true` で完了を待たずに起動し job_id を返す | 既定無効、`PICOAGENT_COMMAND_ALLOWLIST` の完全一致のみ、shell 不使用、stdin 無効、timeout、出力32KBで制限。許可ルート・保護パスは共通 permissions を参照。`rm` / `del` / `Remove-Item` / `git clean` などスキル経由の物理削除コマンドは拒否する。`background` も同じ許可判定を通ってから起動し、承認カード経由の実行は同期実行のみ |
| `command_job` | `run_command` が `background=true` で起動したジョブの一覧・途中出力取得・停止 | 起動はできない（様子見と停止のみ）。`PICOAGENT_COMMAND_ENABLED` が必要。同時実行4件、1ジョブの保持出力64KB（超過分は古い側から破棄）、終了済み履歴20件。`output` は前回の続きだけを返す。停止は `kill` 後に `wait` でハンドルまで回収し、アプリ終了時（`RunEvent::Exit`）に全ジョブを停止する |
| `code_runner` | 安全な隔離実行を準備中の内部機能 | OSレベルのサンドボックスが無いため `agent_callable=false`。設定を有効にしてもモデルのツール一覧には公開しない |
| `slack_send_message` | Slack Bot token または Incoming Webhook URL でメッセージ送信。Bot token では認証テストとチャンネル履歴取得もできる | token/webhook は設定から都度取得、秘密値を出力しない、本文4000文字、履歴20件上限、timeout |
| `gmail_message` | Gmail API でメール一覧取得・本文取得・ラベル一覧・ラベル付け/解除・アーカイブ・既読/未読化・ゴミ箱移動・送信・下書き作成 | OAuth access token は設定から都度取得、秘密値を出力しない、既定は下書き、検索条件ベースの cleanup は preview 後に `confirm=true` で実行、ゴミメール掃除は完全削除ではなく Gmail のゴミ箱移動、cleanup 件数上限は `PICOAGENT_GMAIL_CLEANUP_MAX_RESULTS`（0で無制限）、検索クエリ/本文/件名サイズ上限、timeout |
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
| `desktop_control` | デスクトップのマウス・キーボード操作（クリック・ドラッグ・文字入力・キー送出・スクロール）とウィンドウ一覧・前面化 | 既定無効、**シェルを介さない固定 action のみ**、座標の画面内チェック、入力文字数 2,000 上限、1分あたりの操作回数上限。ウィンドウ一覧・前面化は現在Windowsのみ。Linuxの入力操作はX11またはデスクトップ環境の許可範囲で動作し、Waylandが拒否した場合はエラーを返す |
| `browser_control` | 実ブラウザ（Chrome / Edge）を CDP で操作してページを読む。JS 描画ページ・ログインが要るページ向け | 既定無効、CDP は127.0.0.1限定かつ自分が起動した子だけを操作、http/https のみ、許可ドメインは入力時・リダイレクト後・再読込時に検査、既定は専用プロファイル、読み取り専用（open / read / pages / close）、timeout。取得内容は信頼できない外部データとして `check_injection` を通す |
| `comfyui_generate` | ComfyUI ローカルサーバーで画像・動画を生成（APIキー不要）。KSampler ワークフローを組み立て `/prompt` → `/history` ポーリング → `/view` URL を返す。AnimateDiff 等の動画出力にも対応 | `COMFYUI_BASE_URL`（既定 `http://127.0.0.1:8188`）・`COMFYUI_CHECKPOINT`（モデルファイル名）のみ。120秒 timeout |
| `image_generate` | OpenAI DALL-E 3 による画像生成。テキストプロンプトから画像 URL を返し、フキダシへ `![alt](url)` でインライン表示する | `OPENAI_API_KEY` 必須、90秒 timeout、サイズ/画質パラメータ対応 |
| `ms_office` | OfficeCLI と OOXML 直接編集による Word/Excel/PowerPoint の文字列置換・PDF出力 | 共通許可ルート配下のみ、同梱または明示設定した OfficeCLI、timeout |
| `document_convert` | OfficeCLI / Pandoc による Office・PDF・文書変換 | 共通許可ルート配下のみ、同梱または明示設定した変換エンジン、timeout |
| `pdf_tools` | qpdf / Poppler による PDF 情報取得・本文抽出・ページ抽出・結合・回転・線形化 | 共通許可ルート配下のみ、同梱または明示設定したCLI、timeout |

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

### 設定画面（右クリック →「設定を開く…」）

- `ui/settings.{html,css,js}` の独立 Tauri ウィンドウ。`ui/picoagent.js` の `window.picoagent.settings.*` →
  `commands.rs` の `settings_load` / `settings_list_models` / `settings_save` command で構成。
- 画面は **基本設定**（`basic`） / **画面・操作**（`display`） / **キャラ・音声**（`character`） / **追加機能・更新**（`components`） / **AIモデル・連携**（`providers`） / **スキル**（`tools`） / **安全・許可**（`permissions`） / **使い方**（`skillguide`） / **診断・修復**（`diagnostics`） の9タブで構成し、導入済みスキル一覧・導入済みキャラクター一覧・各コネクタの `config_schema()` を動的に描画する。読み上げは「キャラ・音声」へ、ローカルAPIとMCPは「AIモデル・連携」へ統合し、旧「音声」タブ（`voice`）を指す入口は `settings_tab()` と `activateSettingsTab()` の両方が `character` へ読み替える。変更頻度の低い項目は `<details class="advanced-group">` へ畳み、検索中は一致した中身だけを開いて見せる（畳んだまま一致させると見つからないのと同じになるため）。「画面・操作」タブはREADME撮影で検証した7形式のWebPを `build-ui` が配信用アセットへコピーし、用途・既定寸法・選択中状態と一緒にカード表示する。カードのIDと既定寸法は `layout::tests` のガードが `LAYOUTS` と突き合わせる。コネクタを足すと一覧も設定項目も自動で増える。
- 設定画面の画像選択・モニター移動・THA3操作は同じ操作群を排他化し、完了・取消・失敗のどの経路でもボタンを再利用可能な状態へ戻す。狭幅では状態表示をフッターの独立行にし、保存・取り消しボタンの可読幅を維持する。
- キャラの人格欄はマニフェスト既定値と異なる場合だけ `PICOAGENT_CHARACTER_OVERRIDES` へ保存する。既定人格を表示・切替しただけでは上書き設定や未保存状態を作らない。
- `ui/picoagent.js` はTauriイベント購読の解除関数を一元管理する。呼び出し側が明示解除した場合は登録から外し、画面破棄時は残りをすべて解除する。設定画面で同じ進捗イベントを再購読するときはキーごとに1本だけを有効にし、古い購読のPromiseが遅れて完了しても新しい解除関数を上書きさせない。
- ヘッダーの検索入力で、設定キー・ラベル・プロバイダ・スキル名・キャラクター名/説明をタブ横断で絞り込める。検索中は一致するタブ/セクション/カードだけを表示し、入力値や保存対象は変えない。
- 未保存変更がある場合はフッターの「変更を戻す」を有効化し、変更済みの設定行を薄く強調する。「変更を戻す」は最後に読み込んだ、または保存に成功した設定スナップショットからフォームを再描画し、キャラ上書き・音声上書き・目覚ましJSONを含めて未保存状態だけを破棄する。
- スキル（`tools`）タブには、`settings_load` が返す各ツールの `name` / `label` / `description` / `configSchema` を使い、カテゴリ見出し付きで全スキルを縦に並べる。各カードには `configSchema` と保存値から「設定不要」「未設定」「一部設定」「設定済み」の状態バッジを表示し、`settings.js` の `approvalPolicy()` が送信前確認・実行前確認・書込/変更・外部読取・低リスクの承認ポリシーを表示する。Gmail / Slack / Googleカレンダーは `settings_test_tool` で保存済み設定の接続テストを実行でき、`diagnose_tool_test()` の診断文で token 未設定、401、403、404、timeout、接続失敗を原因候補として示す。設定を持つカードはクリックで展開し、`alarm` は `buildAlarmEditor`、`task_scheduler` は `buildScheduleEditor` の専用フォームで JSON 保存値を組み立てる。
- 使い方（`skillguide`）タブには `settings_load` が返す各ツールの `name` / `label` / `description` / `examples` を使い、区分・できること・頼み方例を横断表示する。個別カードだけでは機能全体を把握しづらいため、まず一覧資料として全スキルを見通せるようにする。
- スキル一覧は `settings_load` が返す各ツールの `name` / `label` / `description` をカードで表示する。`config_schema()` を持つスキルは ⚙ 付きのアコーディオンになり、カードをクリックするとそのスキルの設定項目だけがその場に開く（設定項目を末尾へずらずら並べない）。APIキー・OAuth token・Webhook URL が必要なスキル/プロバイダには、設定画面内で取得手順・貼り付け先・公式リンクをまとめた折りたたみ式の設定ガイドを表示する。チャットで設定方法を尋ねた場合も `agent.rs` の固定ガイドで同等の手順とリンクを返す。キャラクター一覧は `assets/characters/index.json` と各 `character.json` から表示し、表示名・内部ID・説明・描画形式バッジ（`PNGアニメ` / `Live2D` / `自作THA3` / `追加PNG`）と、idle 先頭フレーム（無ければ `boot.sleeping`、どちらも無ければ `_preview-placeholder.png`）を丸い小アイコンとして添える。専用検索は表示名・内部ID・説明を対象とし、`Live2D` / `PNGアニメ` / `自作THA3` / `追加キャラ` のフィルターと組み合わせて候補件数を表示する。使用中のカードは枠を強調して選択ボタンを「選択中」の無効表示にし、他のカードの「このキャラを選ぶ」でプルダウンと同じ `PICOAGENT_CHARACTER` と人格・口調の編集対象を切り替える（反映は「保存」時。同じ表示名のキャラが並んでも内部IDと描画形式で区別できる）。
- 「AIモデル・連携」タブのコネクタ設定は、選択中プロバイダの設定だけを表示する（プロバイダを切り替えると表示も切り替わる）。未使用プロバイダのAPIキー欄・モデル欄はDOMにも作らず、プロバイダ切替時にフォームを作り直す。保存時は既存設定を土台に表示中入力だけを上書きし、未表示プロバイダの保存済み設定を消さない。プロバイダ一覧は既定の `local` を先頭に表示し、その他をラベル順に並べる。基本設定タブの「モデル」欄は Ollama のローカルモデル一覧専用とし、local / OpenAI / Claude / DeepSeek / Gemini / xAI / Mistral は「AIモデル・連携」タブ内でモデルを種類・サイズの2段セレクトから選ぶ。保存時は選択プロバイダのモデル設定を `PICOAGENT_MODEL` にも同期する。「AIモデル・連携」タブには「ダウンロード済みモデル」セクションがあり、`pet_list_downloaded_models`（`app_data_dir/bundled/models` 配下の `.gguf` をファイル名・サイズ・対応プリセット名・使用中フラグ付きで列挙）と `pet_delete_downloaded_model`（指定 GGUF を物理削除）でモデルを管理できる。削除はモデルディレクトリ配下の `.gguf` のみを対象とし、パストラバーサルと使用中モデルの削除を拒否する。同梱物（PicoAgent 管理下）のためゴミ箱ではなく削除でディスクを解放する。削除前は再ダウンロードが必要になることを `aria-live` の画面内ステータスへ表示し、同じボタンの二度目の操作だけで実行する。ブラウザ標準の確認／警告ダイアログは使わない。
- 「診断・修復」タブは `engine_status` / `aivis_status` / `pet_usage` / `pet_traces` を読み、選択中プロバイダ・同梱エンジン状態・Aivis状態・今月/累計トークン使用量・直近スキル実行の件数/平均遅延/エラー数をカードで表示する。`settings_load.product` の製品名・Cargoパッケージ版・識別子に加え、ローカルAIと外部連携の通信境界、保存データ、第三者ソフトウェアの扱いも製品情報として表示する。設定値は変更せず、各取得に失敗した場合も診断カード単位でエラー表示に留める。
- `Field` の項目: `{ key, label, type:"text"|"password"|"textarea"|"checkbox", placeholder }`。
- 保存は `config.set_all()` で永続化する。反映は「軽い設定は実行中に即反映」「重い設定は再起動して起動シーケンスで反映」に分ける。
  - **即反映**: システムプロンプト・長期メモリ・表示テーマ・常に手前・フキダシの見た目・自発雑談・キャラ人格の上書き（同一キャラ）・グローバルホットキー・コネクタ固有設定（`cfg.get()` 都度読み）。`pet:ui` イベントで実行中へ反映する。ホットキーは `hotkey::apply()` が張り直し、登録できなかったキーがあれば `settings_save` が `hotkeyWarning` を返して設定画面のステータスへ出す（保存できたのに効かない状態を黙って残さない）。
  - **再起動して反映（共通機構）**: 以下のキーが実際に変わったら `settings_save` は `{ requiresRestart: true }` を返し、実行中のエンジン/Aivis/キャラ再読込は行わない。設定画面は確認画面を出し、ユーザーが承諾したら `restart_app` command を呼ぶ。`restart_app` は **プロセス再起動（`app.restart()`）はしない**（`tauri dev` の開発配信サーバが落ちて `127.0.0.1` 接続拒否になるため）。代わりにバックエンドを起動シーケンス相当に再初期化（選択中キャラの履歴を文脈へ読み直し、`settings_save` が返す `restartEngine` / `restartAivis` に応じて変わった部分だけ停止→再起動。例: キャラ変更だけなら LLM エンジンは再起動しない）し、設定ウィンドウを閉じてからメインウィンドウへ `pet:reload` を送って webview を再読込させる。再読込でフロントの起動シーケンス（ローディング画面）が再実行され、キャラと履歴を表示可能にしてからモデル取得・エンジン/Aivis準備をバックグラウンドで続ける。再起動対象キー: `PICOAGENT_PROVIDER` / `PICOAGENT_MODEL` / `PICOAGENT_MODEL_URL` / `PICOAGENT_LOCAL_MODEL_PRESET` / `PICOAGENT_LOCAL_PORT` / `PICOAGENT_LOCAL_CTX` / `PICOAGENT_LOCAL_NGL` / `PICOAGENT_LOCAL_GPU_DEVICE` / `PICOAGENT_TTS_ENABLED` / `PICOAGENT_TTS_DEVICE` / `PICOAGENT_TTS_PORT` / `PICOAGENT_CHARACTER`。ユーザーが「あとで」を選んだ場合も設定は保存済みで、次回起動時に反映される（実行中状態は触らないので旧エンジンと不整合にならない）。
  - 起動シーケンスはキャラクターと履歴の表示だけを待ち、ローカルAI・Aivis・音声モデルの準備は画面起動後に続ける。選択プロバイダの疎通確認は3秒、キャラクター一覧・定義・PNGレイヤー・Live2Dランタイムは4秒、Live2Dモデルは候補ごとに8秒を上限とする。選択中の任意キャラが失敗した場合は同梱2Dキャラ `mugi` へ一度だけ切り替え、同じ失敗を繰り返して起動画面を占有しない。`waitForEngineReady` は起動後もバックグラウンド監視を続けるが、取得・起動中の進捗で通常利用中の状態表示を上書きしない。準備失敗も本体操作を止めない。
  - 起動完了後の「準備できた」系メッセージは小さいステータス専用表示にせず、必要な場合は通常のフキダシ本文として表示し、TTS 有効時は読み上げ対象にする。キャラクター挨拶が表示される場合は挨拶を優先し、完了メッセージを重ねない。
  - Ollama 選択時は `settings_list_models` で `/api/tags` の候補を読み込み、保存時に選択モデルを `PICOAGENT_MODEL` と `OLLAMA_MODEL` の両方へ反映する。

---

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
| `PICOAGENT_WINDOW_WIDTH_PX` | 自動 | 内部UI | 透明ウィンドウの幅(px)。アプリ終了時に保存し、起動時に復元する |
| `PICOAGENT_WINDOW_HEIGHT_PX` | 自動 | 内部UI | 透明ウィンドウの高さ(px)。アプリ終了時に保存し、起動時に復元する |
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
| `PICOAGENT_BOOT_TIMEOUT_MS` | `12000` | 内部UI | 起動監視1区間の長さ。フロントの読込開始（`pet_frontend_ready`）をこのミリ秒だけ待ち、届いたらさらに同じ長さだけ起動完了（`pet_finish_boot`）を待つ。どちらかが届かなければキャラクター付きの案内画面へ切り替える（案内までの最長は2区間＝既定24秒）。設定画面には出さない（`3000`〜`120000` に丸める） |
| `PICOAGENT_NAP_MINUTES` | `15` | 基本 | 無操作でキャラが居眠りするまでの分数（`0` で居眠りしない） |
| `PICOAGENT_QUICK_ACTIONS` | 空 | 基本 | 入力欄上のクイック操作を上書きする（1行1件「ラベル\|送る文章」。空なら既定の8件） |
| `PICOAGENT_UPDATE_CHECK_ENABLED` | `true` | 基本 | 起動時に公開配布Gitの `latest.json` を確認し、新しければ通知する。オフライン時は通知しない |
| `PICOAGENT_MEMORY_ENABLED` | `true` | 基本 | 会話から好み・自己情報・継続指示を暗黙的に長期メモリへ保存する |
| `PICOAGENT_PLANNING_ENABLED` | `false` | 基本 | 複雑な質問（120文字超・疑問符2個以上・手順系語）をサブタスクへ自動分解するプランニングステップを有効化する |
| `PICOAGENT_SELF_EVAL_ENABLED` | `false` | 基本 | 回答が短すぎる・拒否パターンを含む場合に1回再生成する自己評価ステップを有効化する |
| `PICOAGENT_SUPER_AGENT` | `false` | 基本 | スーパーエージェントモード。計画ステップ・自己評価を一括有効化し、ループ上限を引き上げ、進捗を `pet:agent_status` で可視化する |
| `PICOAGENT_SUPER_AGENT_MAX_STEPS` | `12` | 基本 | スーパーエージェントモードのトップレベル最大ループ数（呼び出し値〜30 にクランプ） |
| `PICOAGENT_MULTI_AI_MODE` | `single` | 複数AI | `single` / `consensus` / `collaborate` / `divide`。既定は従来どおり単発 |
| `PICOAGENT_MULTI_AI_AGENTS` | `[]` | 複数AI | 参加AI（name / provider / model / focus）のJSON配列。複数モードでは2〜8件 |
| `PICOAGENT_MULTI_AI_JUDGE_PROVIDER` | 現在のプロバイダ | 複数AI | 分担計画と回答統合を行うまとめ役のプロバイダ |
| `PICOAGENT_MULTI_AI_JUDGE_MODEL` | 現在のモデル | 複数AI | まとめ役のモデル |
| `PICOAGENT_MULTI_AGENT_MAX_CONCURRENCY` | `3` | 複数AI | 外部AI候補の最大同時数（1〜8）。内蔵local候補は常に直列 |
| `PICOAGENT_SUBAGENT_MAX_DEPTH` | `2` | スキル(subagent) | サブエージェントの最大ネスト深さ。これ以上は再帰委譲を拒否する |
| `PICOAGENT_MEMORY_MAX_ITEMS` | `80` | 基本 | 長期メモリに保持する最大件数（10〜200） |
| `PICOAGENT_RELATIONSHIP` | `assistant` | 基本 | ユーザーとの関係（`assistant` / `friend` / `partner` / `sister_younger` / `sister_older` / `senpai` / `custom`）。人格の後ろへ差分として重ねる |
| `PICOAGENT_RELATIONSHIP_OVERRIDES` | `{}` | 基本 | キャラクターIDごとの関係の上書き（`{"mai":"sister_older"}`）。全体設定より優先する |
| `PICOAGENT_RELATIONSHIP_CUSTOM` | （空） | 基本 | `PICOAGENT_RELATIONSHIP=custom` のときに人格へ足す自由記述 |
| `PICOAGENT_CHARACTER` | `mugi` | 基本 | 選択中キャラクターのID |
| `PICOAGENT_CHARACTER_OVERRIDES` | `{}` | 基本 | キャラクターIDごとの人格設定上書き |
| `PICOAGENT_LIVE2D_SDK_OVERRIDES` | `{}` | 基本 | 複数世代のモデルを持つLive2Dキャラクターについて、キャラクターIDごとに `"4"` / `"5"` を固定する。未指定は `auto` |
| `PICOAGENT_COMPANION_MODE` | `both` | 基本 | 初回案内で選ぶ相棒の用途（`life` / `work` / `both`） |
| `PICOAGENT_RELATIONSHIP_MEMORY_ENABLED` | `true` | 基本 | キャラクター別の関係性と共有作業記録を保存する。OFFでは既存データを消さず、更新とAI文脈への差し込みを止める |
| `PICOAGENT_ONBOARDING_DONE` | `false` | 基本 | 初回3択案内の完了状態 |
| `PICOAGENT_TTS_ENABLED` | `false` | 音声 | Aivis で回答を読み上げる |
| `PICOAGENT_TTS_DEVICE` | `auto` | 音声 | 同梱 Aivis の推論デバイス（内部設定。通常UIでは非表示） |
| `PICOAGENT_TTS_PORT` | `11436` | 音声 | 同梱 Aivis のポート（内部設定。通常UIでは非表示。旧既定 `10102` は起動時に `11436` へ移行） |
| `PICOAGENT_TTS_SPEAKER` | `888753760` | 音声 | Aivis の speaker/style ID（内部設定。通常UIでは非表示） |
| `PICOAGENT_TTS_VOLUME` | `50` | 音声 | キャラボイス再生音量（0〜100）。フロントで即反映。ミュートまたは0では全キャラの口パクも停止 |
| `PICOAGENT_TTS_CHARACTER_OVERRIDES` | `{}` | 音声 | キャラクターIDごとの Aivis speaker ID 上書き（内部設定。通常UIでは非表示。任意で `modelUuid` / `modelName` / `speakerUuid` / `styleName` / `sourceUrl` も保持） |
| `PICOAGENT_LOCAL_MODEL_PRESET` | `auto` | local | ローカルモデルプリセット（`small`: Gemma 4 E2B/約3.1GB、`balanced`: Gemma 4 E4B/約5.0GB、`large`: Gemma 4 12B/約7.1GB、`qwen35_4b`: Qwen3.5 4B/約2.7GB、`qwen35_9b`: Qwen3.5 9B/約5.7GB、`gpt_oss_20b`: OpenAI gpt-oss-20b/約12.1GB、`phi4mini`: Phi-4 mini/約2.5GB、`llmjp4_8b`: LLM-jp-4 8B Thinking/約5.3GB、`custom`）。旧プリセットIDは現行候補へ自動移行する |
| `PICOAGENT_MODEL_URL` | Gemma 4 E4B Q4_K_M GGUF | local | 同梱エンジンが取得するモデル URL |
| `PICOAGENT_LOCAL_PORT` | `11435` | local | 同梱エンジンのポート |
| `PICOAGENT_LOCAL_CTX` | `8192` | local | コンテキスト長 |
| `PICOAGENT_LOCAL_NGL` | `999` | local | GPU へオフロードするレイヤ数（`999`=全レイヤ優先 / `0`=CPU 固定） |
| `PICOAGENT_LOCAL_GPU_DEVICE` | （空=既定先頭） | local | 使用 GPU の指定（複数 GPU 環境向け、例 `Vulkan1`） |
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
| `PICOAGENT_SCHEDULED_TASKS` | `[]` | task_scheduler | cron 風のスケジュールタスク一覧（JSON文字列。`kind=command` / `kind=tool`。通常はチャットで設定。設定画面では専用フォームで編集） |
| `PICOAGENT_SCHEDULED_TOOL_ALLOWLIST` | `time,weather,web_fetch` | task_scheduler | 無人スケジュール実行を明示的に許可するスキル名（カンマ区切り） |
| `PICOAGENT_CLIPBOARD_MAX_CHARS` | `3000` | clipboard_text | クリップボード取得の最大文字数（500〜8000） |
| `PICOAGENT_MEETING_ROOT` | （旧キー） | meeting_recorder | 旧キー。設定画面からは廃止。設定済みなら既存利用者向けに追加許可ルートとしてだけ引き継ぐ |
| `PICOAGENT_TRANSCRIBE_PROGRAM` | 空 | meeting_recorder / whisper_transcribe | ローカル文字起こし実行ファイル。手動設定が最優先、空なら同梱 `whisper-cli`。同梱CLIはポートを開かない |
| `PICOAGENT_TRANSCRIBE_MODEL` | 空 | meeting_recorder / whisper_transcribe | ローカルWhisperモデルのパス。手動設定が最優先、空なら同梱モデル |
| `PICOAGENT_TRANSCRIBE_ARGS` | `-m {model} -f {audio} -otxt -of {output_stem}` | meeting_recorder / whisper_transcribe | 文字起こし実行時の引数テンプレート |
| `PICOAGENT_TRANSCRIBE_DIARIZE_ARGS` | `-tdrz` | meeting_recorder / whisper_transcribe | `diarize=true` 時に追加するローカル話者分類引数 |
| `PICOAGENT_TRANSCRIBE_TIMEOUT_SECS` | `1800` | meeting_recorder / whisper_transcribe | ローカル文字起こしタイムアウト秒（10〜7200） |
| `PICOAGENT_WEATHER_DEFAULT_LOCATION` | `Tokyo` | weather | 天気の既定地点（自動取得失敗時のフォールバック） |
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
| `PICOAGENT_API_ENABLED` | `false` | ローカルAPI | ローカル HTTP API を有効化（127.0.0.1 のみ。変更は再起動後に反映） |
| `PICOAGENT_API_PORT` | `11500` | ローカルAPI | ローカル API のポート |
| `PICOAGENT_API_KEY` | 空 | ローカルAPI | 設定すると `Authorization: Bearer` を必須にする（`password` 型。キー名に `API_KEY` を含むためシークレットとして扱われ、Windows では DPAPI 暗号化して保存される） |
| `PICOAGENT_LICENSE_TOKEN` | 空 | 製品ライセンス | `PA1.<payload>.<Ed25519署名>` 形式のPro／Businessライセンス。`password` 型でWindowsではDPAPI暗号化保存。無効・期限切れ・未設定は無料版として動作し、PC操作・ファイル編集・Office・外部サービス・MCP等の高度なスキルを共通dispatch境界で拒否する |
| `PICOAGENT_API_ALLOWED_ORIGINS` | 空 | ローカルAPI | ブラウザからの横断リクエストを許可する Origin（カンマ区切り。空＝Origin 付きは全て拒否） |
| `PICOAGENT_MCP_SERVERS` | 空 | MCP | 外部 MCP サーバ（1行1件「名前\|コマンド 引数…」。stdio 接続。`#` でコメント。変更は再起動後に反映） |
| `PICOAGENT_PDF_TIMEOUT_SECS` | `120` | pdf_tools | PDF操作タイムアウト秒（5〜300） |

各キーは環境変数としても渡せる（既定値としての参照のみ。保存先は常に `config.json`）。

---

## 外部インターフェース（ローカルAPI / MCP）

アーキテクチャは **agent（ループ）/ skills（内蔵ツール）/ mcp（外部相互運用）** の3層。`mcp` は外部との境界だけを担い、エージェントループとスキル実装からは独立している。

### ローカル HTTP API（`api_server.rs`）

- 既定で無効（`PICOAGENT_API_ENABLED`）。有効時のみ `127.0.0.1:<PICOAGENT_API_PORT>` にバインドし、LAN へは公開しない。`PICOAGENT_API_KEY` を設定すると全エンドポイントで `Authorization: Bearer` を必須にする。**127.0.0.1 限定だけではブラウザ経由の攻撃を防げない**（ユーザーが開いた悪意あるページが fetch でこの API を叩ける。`Content-Type: text/plain` なら CORS プリフライトも回避でき、応答は読めなくても「エージェントを実行させる」副作用だけを起こせる）。そのため **`Origin` ヘッダ付きのリクエストは既定で 403**（ブラウザは横断 fetch に必ず Origin を付けるが、CLI / MCP クライアントは付けない。MCP 仕様も Origin 検証をサーバ要件としている）、**ボディ付きは `Content-Type: application/json` 限定で 415**（単純リクエストによる迂回を防ぐ）。例外が要る場合だけ `PICOAGENT_API_ALLOWED_ORIGINS` に列挙する。
- 依存追加なしの最小 HTTP/1.1 実装（tokio `TcpListener`）。ボディ 1MB 上限・リクエストタイムアウト 300 秒・ヘッダ 16KB 上限。不正 JSON は接続を閉じずに HTTP 400（`/mcp` は JSON-RPC parse error `-32700`）を返す。
- エンドポイント: `GET /api/version`・`GET /api/tags`（Ollama 風）、`POST /api/chat`・`POST /api/generate`、`POST /v1/chat/completions`（OpenAI Chat Completions 互換。`stream` 要求には全文単一チャンクの SSE）、`POST /mcp`（下記）。
- 会話はリクエスト内で完結する**ステートレス**設計で、デスクトップの会話履歴・キャラ文脈とは混ざらない。スキルは通常どおり使える。
- 付属 CLI: `xtask`（`cli`）（`cargo xtask cli status/models/chat/ask/repl`）。接続先は `PICOAGENT_API_PORT` / `PICOAGENT_API_KEY` 環境変数。

### MCP（`mcp/{mod,client,server}.rs`）

JSON-RPC 2.0。プロトコルバージョンは `2024-11-05`。

- **サーバ側（スキルの公開）**: `POST /mcp` が `initialize` / `tools/list` / `tools/call` / `ping` に応答し、`agent_callable` なスキルを MCP ツールとして公開する。**公開していないスキル（アプリが内部で使うもの）は `tools/call` でも実行を拒否する**（一覧に出す面と実行できる面をずらさない）。スキルは失敗も文字列で返す設計のため、「エラー」始まりの結果を `isError: true` へ写像する。通知（id なし）には本文なしの 202 を返す。Claude Code からは `claude mcp add --transport http picoagent http://127.0.0.1:11500/mcp` で登録できる。
- **クライアント側（外部ツールの取り込み）**: `PICOAGENT_MCP_SERVERS` の stdio サーバを起動時に spawn し、`initialize` → `tools/list` の結果を `mcp__<サーバ名>__<ツール名>` として動的登録する。`tools::schemas_for_text` が候補へ加え、`tools::dispatch` が `mcp__` 接頭辞をクライアントへフォールバックして `tools/call` へ中継する。応答の読み取りは**1行8MBで打ち切る**（`read_line` に上限が無く、改行を出さないサーバに付き合ってメモリを伸ばし続けるため。他の外部入力と同じく必ず上限を持たせる）。tokio の `read_line` はキャンセル安全でないため、タイムアウト・I/O 失敗後はその接続を切断して再利用しない。終了時（`RunEvent::Exit`）に全子プロセスを停止する。

---

## フロント連携と製品化

### キャラクターパック

- **Live2D ランタイムの互換固定**: `pixi-live2d-display 0.4.0` / PIXI 6.5.10 と、公式の世代別Coreを組み合わせる。SDK 5モデルはCore 6（Cubism 5.3形式）、SDK 4モデルはCore 5（互換用）で読み込む。`cargo xtask setup live2d` は版付きURLから両方を取得してSHA-256を照合し、旧版・破損ファイルを置換する。Cubism 5.3用Coreではひよりの口ArtMeshが消えることを実画像で確認しているため、旧モデルを同じCoreへ無理に統一しない。
- **Live2D の表示倍率**: 各モデルの `scale` / `offsetY` はすでに上半身表示へ調整済みなので、共通の `PICOAGENT_CHARACTER_ZOOM` は既定 `1.55` を Live2D の基準1倍として差分だけを重ねる。PNGアニメは従来どおり `1.55` をそのまま適用する。
- **Live2D の描画基準寸法**: Live2D は表示形式にかかわらず 285×315 で描画する。chat / bar の62px／42pxの枠を初期化寸法に使うと、モデル固有の `offsetY` で全描画がキャンバス外へ出るため。アイコン形式は同じキャンバスを CSS で縮小し、顔を円形に切り出す。

- `characters/index.json` に `{ id, name, manifest }` を追加し、各フォルダへ `character.json` と連番フレームを置く（`ui/assets/characters/` へは `cargo xtask sync-characters` が同期）。
- `character.json` は `name`、`description`、`greeting`、`persona`、`animations` を持つ。任意で `assetVersion` と `boot.sleeping` を持てる。正本側の `assetVersion` は制作上の版として保持するが、配信先への同期時はキャラ素材全体の内容ハッシュから自動生成した版へ置き換え、画像差し替え後の旧キャッシュを防ぐ。高精細な立ち絵と顔差分を優先する通常2Dは `animationQualityMode=sharp-expression` を指定する。同梱通常2Dの共通フル仕様は `motionProfile=full` とし、基本、半目／閉じ目、閉口／開口、左右視線、喜び／悲しみの9レイヤーを必須にする。実行レイヤーは `runtimeLayers` の名前→相対パス写像で指定し、同梱パックは `runtimeLayerEncoding=webp-lossless-v1`、実行フレームは `frameEncoding=webp-lossless-v1` を宣言する。同期時に不足を拒否するため、一部キャラだけ機能が抜けた状態を出荷できない。人格には性格・口調・一人称・ユーザーの呼び方・追加指示を定義できる。通常2Dの一部分だけへ遅れを伴う二次動作を付ける場合は `secondaryMotion` を使い、`type=bust`、中心座標 `centerX` / `centerY`、楕円範囲 `radiusX` / `radiusY`（すべて画像全体に対する百分率）、強さ `strength`、周期秒 `duration` を指定する。対象外を複製しない羽根付きマスクで合成し、Live2D・小型顔アイコン・動きを減らすOS設定では適用しない。
- 各アニメーションは `frames`、`fps`、`motion` と、従来の連番フォルダ `path` または明示的な画像配列 `framePaths` のどちらかを持つ。`framePaths` は同一画像を複数の状態・コマから共有し、42コマ分を重複配置せずに配布するために使う。任意で `frameDurations`（ミリ秒配列）を持て、まばたきや口パクなど均等 `fps` だと不自然になるフレームだけ表示時間を調整できる。フロントは起動直後に待機フレームを全件先読みし、残りの状態もアイドル時間に全件キャッシュする速度優先方式で、画像フレームとCSS補間動作を組み合わせる。キャラ切替時は旧キャラの先読み参照を解放する。`runtimeLayers`がない利用者追加パックは従来の `layers/<名前>.png` を読む。
- 同梱通常2Dの実行用フレームは `frameGeneration=<復号画素の内容ハッシュ>` と `frames/<frameGeneration>/*.webp` の単一世代へ封入する。全 `framePaths` はその世代だけを参照し、`runtimeLayers`は `runtime-layers/*.webp` を参照する。生成時は各WebPを再読込して変換元PNGと寸法・RGBAの完全一致を検査し、新世代を完成させてmanifestを原子的に保存してから旧世代を消す。`cargo xtask sync-characters` は配信先へ触る前に全同梱通常2Dの複数世代・世代外参照・WebP実体と制作正本を検査する。同期はファイルのサイズと更新日時が同じならコピーを省き、正本にない配信先ファイルだけを削除し、変更キャラだけ内容版を再計算する。`animationQualityMode=sharp-expression`の実行画像が縦1800pxを超える場合は、1780×3600以上の制作正本を変更せず、配信コピーだけを縦1800px上限・Lanczos3・lossless WebPで派生する。配信manifestの`deliveryImages`へ方式、上限、正本SHA-256を記録し、形式・寸法・ハッシュが一致する画像は次回同期で再生成しない。変換は最大4並列とし、各画像を検証済み一時ファイルから原子的に置き換える。`--force`を明示した場合だけ検証後に全再構築する。制作正本 `layers/*.png` は配信対象から外すため、新旧画像や制作PNGを同じ実行パックへ混在できない。Live2Dは通常2D世代の対象外だが、定義した各SDKモデルの `.model3.json` 一式が存在することを同期前に検証し、選択したSDKモデル内だけでテクスチャとモーションを解決する。ただし `characters/*/live2d/` は再配布条件のためGit管理外で、`cargo xtask setup live2d-samples` を実行していない作業環境には存在しない。宣言したモデルが**1つも無い**キャラは「未取得」として同期対象と生成先 `index.json` から外し（既定キャラだった場合は残った先頭へ寄せる）、同期は成功させる。clone直後でも `dev` / `build` が `ui-dist/` を作れる状態を保つため。宣言のうち**一部だけ**が欠けている場合は取得・書き出しが壊れている状態なので、外さず従来どおり失敗させる。
- **Live2D キャラクター**: `character.json` に `live2d` を持つキャラは連番フレームの代わりに Cubism ランタイムで WebGL 描画する（`animations` は省略可。`live2d.model` か `animations.idle` のどちらかが必須）。`live2d` は `model`（単一モデルのキャラフォルダ相対 `.model3.json` パス）、`models`（複数世代を持つ場合の `{ "sdk":"4"|"5", "path":"...model3.json" }` 配列。自動時は定義順に試す）、`motions`（状態名 idle / thinking / talking / sad → モデルのモーショングループ名の写像）、`lipSyncParam`（省略時はモデル定義の `Groups: LipSync` の先頭IDを自動検出）、`lipSync`（`false` で口パクを無効化）、`scale`（表示枠フィット後の倍率、既定1）、`offsetY`（下端オフセットpx、既定0）を持つ。複数世代があるキャラは `PICOAGENT_LIVE2D_SDK_OVERRIDES` で自動／Cubism 4固定／Cubism 5固定を切り替える。ランタイム（Cubism Core / PIXI 6 / pixi-live2d-display）は `bundled::fetch_live2d_runtime` が `ui/vendor/live2d/` へ取得し、モデル選択時だけ遅延読み込みする。公式サンプルモデルは自動取得・製品同梱しない。開発者が条件確認後に `cargo xtask setup live2d-samples` を明示した場合だけGit管理外の開発素材として取得する。利用者は配布元から自分で取得した1体分のzip／フォルダを設定画面から追加する。モデル同梱音声はTTSと重ねず、`PIXI.live2d.config.sound = false` で再生しない。`build-character-animations.py` はLive2Dキャラをスキップする。
- キャラクター画像の標準構成は `characters/README.md` に従う。制作正本として `layers/base.png`、`blink_half.png`、`blink_closed.png`、`mouth_closed.png`、`mouth_open.png` と `boot/sleeping.png` を用意し、`scripts/build-character-animations.py` で12枚構成の待機・思考・会話フレームと6枚構成の落胆フレームを生成する。THA3の追加キーポーズが17枚そろう場合は、呼吸・左右揺れ・思考・会話・落胆の専用系列を使う。生成後は内容ハッシュ単位のlossless WebP `frames/` と `runtime-layers/` を実行時に共有し、一時的な `animations/` 連番は残さない。フレームは手で直接編集せず、レイヤー差分から再生成する。同梱通常2Dは従来の上半身を既定構図とし、チャット／バーの小型枠は共通CSSで上端基準に切り出す。
- **関係性（`relationship.rs`）**: ユーザーとの関係（相棒 / 友達 / 恋人 / 妹 / 姉 / 先輩 / 自由設定）を、キャラ人格の**上に重ねる短い差分**として持つ。関係ごとに `character.json` を書き下ろすとキャラ数 × 関係数の掛け算になるため、差分は数文に抑え、呼び方は文章ではなく `user_name` の置き換えで表現する（同梱の既定モデルは system が長いほど指示追従が落ちる）。`reset_conversation` / `clear_conversation` が人格の直後へ `profile_suffix()` を足すので、設定変更は次のメッセージから効く（`settings_save` は関係キーの変更を人格変更と同じ「即反映」経路で扱う）。既定 `assistant` では差分が空になり、従来と同一の system になる。関係にかかわらず外さない安全の線（性的・年齢を想起させる表現をしない／作業の正確さを落とさない）はプリセット文面ではなく `SAFETY_NOTE` として一括で付与する。キャラ別の上書きは `PICOAGENT_RELATIONSHIP_OVERRIDES`、一覧は `settings_load` の `relationships` で設定画面へ渡す。
- **感情の見せかた**: `emotion.rs` が推定した mood / energy を、フロントは `data-emotion` / `data-energy-level` として `.character` に付ける。CSS 側はオーラ色と `--pet-tone`（顔色）を切り替える。常駐アプリとして目障りにならないよう、全身を揺らし続ける状態別モーションは持たず（`--pet-motion-scale` も廃止）、動きはまばたき・口パク・Live2D のモデルモーションと、成功時の1px級の反応だけで見せる。
- 選択中キャラの人格と挨拶は `pet_set_character_profile`（`characterId` / `profile` / `greeting`）で受け取り、人格はシステムプロンプトへ追加、挨拶は短い挨拶への直接応答に使う。既定プロンプトは挨拶・自己紹介でアプリ説明ではなくキャラクター本人として名乗ることを優先する。設定画面のキャラ別上書きは `PICOAGENT_CHARACTER_OVERRIDES` に保存する。
- **人格プロンプトの混入は表示と読み上げの両方で落とす。** 小型ローカルモデルは system プロンプトを復唱することがあり、そのままだとフキダシ・履歴へ設定文が出て、読み上げも設定文を喋る。`agent::remove_prompt_leak` が「system プロンプトに実在する行と中身が同一の行」を取り除く。照合は `normalize_leak_line`（空白除去・先頭の箇条書き記号除去・コロンの全角統一）を通すため、`- 名前：むぎ` のように整形し直された復唱も落ちる。改行を潰して1文へ流し込まれた場合は、12文字以上の行を原文どおりの並びで文中からも除去する。
  - **適用箇所はモデル出力が外へ出る4経路すべて。** 通常会話の最終回答2経路（`tool_calls` が無い応答、上限打ち切り）は**`self_improve` の後**に通す（自己評価は回答を作り直すため、先に消すと作り直した側が素通りする）。エージェント経路を通らない自発雑談 `pet_idle_chatter` と目覚まし `generate_wake_message` は、それぞれの生成直後に通す（雑談は「人格(system)＋指示」だけを文脈にするぶん復唱が起きやすい）。
  - **フロント側は音声合成の唯一の入口 `prepareSpeech` で落とす**（`stripPersonaLeak`）。省略できない理由は2つ。ストリーム読み上げは最終回答が整う前の途中テキストを声にすること、挨拶・自発雑談・目覚ましは `sanitizeSpeechText` を通らず `prepareSpeech` へ直接来ること。除去して何も残らない文は合成せず、無音の再生を挟まない。
  - 偶然の一致で普通の返答を消さないよう8文字以上の行だけを対象にするが、`名前:` `性格:` `口調:` `一人称:` `ユーザーの呼び方:` の人格項目行（`personaText` が組み立てる形）は短くても設定文として落とす。照合元は現在の system プロンプトと選択中キャラの人格なので、他キャラの設定文らしき行や、プロンプトに無い普通の返答は削らない。
- **関係性の system 文脈は「めったに変わらない内容」に保つ**（`relationship::context`）。system はプロンプトの先頭にあたるため、1文字でも変わると同梱エンジンのプロンプトキャッシュが丸ごと無効化され、毎ターン全文の再評価になる。載せるのは段階名・相棒モードと、**ユーザーが自分でピン留めした記録だけ**（ピン留めは明示操作なので通常の作業では文面が変わらない）。ピン留めしていない記録は記録カード（`pet_relationship_state`）で見せ、会話へ効かせる内容の選別は `memory.rs`（長期メモリ）の役目とすみ分ける。文脈の差し替えは `AppState::set_relationship_context()` が担当し、**中身が変わったときだけ** `reset_conversation()` を呼ぶ。記録の保存自体は `PICOAGENT_RELATIONSHIP_MEMORY_ENABLED` で止められる。
- ふれあい（`touch_*`）はフロント側で 5 秒に 1 回へ間引いてから記録する（連打のたびに関係性ファイルを読み書きしないため）。反応の台詞とモーションは間引かず毎回返す。
- `character.json` は任意の `interactions.reactions`（`head` / `cheek` / `hand` の反応台詞）と `variants`（`id` / `name` / 任意の `root` / `animations` / `live2d`）を持てる。`root` も `animations` も `live2d` も無い項目は、オーラ色だけが変わる「スタイル」の切替になる。**`variants[].live2d` を持たせると、同じキャラの「絵版」と「Live2D 版」を1つの `character.json` で切り替えられる**（`live2d: false` を指定すると、キャラ本体が Live2D でもその variant だけフレーム表示に戻す）。表示中の定義は `resolveLive2dConfig()` が variant → キャラ本体の順で解決し、モーション写像・口パク設定・`alwaysAnimate` もその定義から引く。フロントは意味状態（傾聴・検索・作業・成功・照れ・心配）を、専用素材が無ければ既存の待機・思考・返答・落胆素材へフォールバックして2D演出する。
- `cargo xtask character-pack validate <フォルダ>` は `character.json` のID・必須表示定義・variants・ふれあい台詞・パック内相対アセットに加え、シンボリックリンク、許可拡張子、ファイル数、総容量、単体容量、画像寸法を検証する。`install` は仮フォルダでコピー後検証し、同名IDを上書きせず `characters/<id>/` と `characters/index.json` へ原子的に追加する。配信用同期までのどこかで失敗した場合は一覧・正本・配信用コピーを導入前へ戻す。
- 関係性コマンドは `pet_relationship_state` / `pet_relationship_event` / `pet_relationship_pin_moment` / `pet_relationship_delete_moment` / `pet_relationship_clear` / `pet_complete_onboarding`。状態更新は `store::lock_state` で直列化し、`store::write_json_atomic` で保存する。破損ファイルは退避後に新規状態から再開する。会話本文は共有記録へ保存せず、作業記録の本文は構造化されたツール名からバックエンドで生成する。「続きをやる」用の元依頼は別フィールドに最大240文字で保存し、AIのsystem文脈には差し込まない。AI文脈へ渡すのはバックエンド生成の作業要約とユーザーが固定した記録だけで、記録内の命令に従わないよう明示する。
- 記録1件の削除と全消去は、対象を画面内で案内し、同じボタンをもう一度押したときだけ実行する。ボタンからフォーカスが外れた場合と失敗時は確認状態を解除し、ブラウザ標準の確認ダイアログは使わない。
- 自発雑談（雑談スキル `chitchat`）: `PICOAGENT_CHATTER_ENABLED` が ON のとき、フロントは `[min, max]` 秒のランダム間隔の待機タイマーで `pet_idle_chatter` を呼ぶ。生成した独り言は**先に音声を合成（`prepareSpeech`）してから文字を表示し、表示と同時に読み上げる**（`playPreparedSpeech`。挨拶と同じ同期再生）。入力中・応答中・読み上げ中・全画面中は割り込まず次回へ送る。ユーザー操作（送信・入力欄を開く・会話リセット）や設定変更（`pet:ui`）のたびにタイマーを張り直す。`skills/chitchat/trends.rs` は `PICOAGENT_CHATTER_TOPIC_MODE=auto` で X API（`PICOAGENT_X_BEARER_TOKEN` がある場合）→ Google Trends RSS → ニュースRSSの順に話題メモを取る。`x` / `trends` / `news` / `off` でソースを固定または無効化できる。X API 未設定時は公式X投稿とは断定せず、急上昇ワードやトップニュースの候補として扱う。生成はWeb検索ツールを使わず、生成後は「直前の自発雑談: ...」として会話文脈へ1件積み、通常会話と同じ圧縮・履歴保存に通す。これにより、ユーザーが直後または再起動後に「今の何の話？」と聞いた場合に直前の雑談を参照できる。挨拶・名乗り・自己紹介は出さず、毎回ちがう話題になるよう話題ヒントを擬似ランダムで与え、さらに直近の独り言（`AppState.recent_chatter`、末尾6件）を「避ける文」として渡して繰り返しを防ぐ。雑談の生成は**人格(system)＋現在時刻＋指示だけ**を文脈にし、ユーザー発話（会話履歴）は含めない（小型モデルが入力をオウム返しするのを防ぐため）。`current_time_context()` で現在のローカル日時・時間帯（朝/昼/夕方/夜）を渡し、夜に「日差し」など時間に合わない描写をしないようにする。なお通常会話の返信がユーザー入力のオウム返しになった場合、フロントはそれを読み上げない。話題メモ（ニュース等）は `pet_idle_chatter` が `trends::topic_brief` で1件取得して `topic` 引数で skill へ渡し、出典（記事タイトル＋URL）も受け取る。生成後、返信が話題に触れていれば（返信と記事タイトルの内容バイグラム一致）最も関連する記事URLを末尾へ `参考:` として添える（URL 行は読み上げ対象外。例: 地震の独り言なら地震ニュースのURL）。
- 目覚まし時計（`alarm` スキル + `alarm.rs`）: フロントは約20秒間隔で `pet_alarms` を取得し、現在のローカル時刻（HH:MM）と曜日が一致する有効なアラームがあれば `pet_alarm_wake` を呼んで**声でやさしく起こす**（生成した起こし文を音声合成→表示→即時再生。生成失敗時は固定の起こし文）。同じ分での二重発火は済みキー集合で防ぐ。一度きり（`repeat=false`）は発火後に `enabled=false` へ。鳴った後はフキダシに `おきたよ！` ボタンを出し、押されるまでは `snooze_minutes`（既定5、1〜60分）おきに同じアラームを再通知する。アラームは時刻・曜日・繰り返し・スヌーズ分数を**チャット（`alarm` スキル）でも設定画面でも**設定できる。設定画面のスキル「目覚まし時計」を開くと、時刻ピッカー＋曜日チェック＋繰り返し/有効/スヌーズ分数/ラベル/削除/追加の専用フォーム（`settings.js` の `buildAlarmEditor`）で編集でき、保存時に `PICOAGENT_ALARMS`（JSON）へ組み立てる。保存先は config。**一覧の取得に失敗した場合は、鳴らない状態が続くことを画面へ知らせる**（`notifyBackgroundFailure`。原因ごとに1度だけ出し、取得できた回に印を落として復旧後の再発はまた知らせる）。日常通知の取得失敗も同じ扱いにする。
- 日常通知（`reminder` / `focus_timer` / `daily.rs`）: フロントは約20秒間隔で `pet_daily_events` を呼び、期限到来したリマインダー・集中終了・作業見守りを吹き出し表示と読み上げで通知する。単発イベントは通知後に無効化し、見守りのような `repeat_minutes` 付きイベントは次回時刻へ進める。**`due_at` を解析できないイベントは永久に発火しないため、黙って飛ばさず `pet_daily_events` の `broken` として返す**（`daily::DuePoll`）。バックエンドは同じイベントをこの起動中1度だけ `broken` へ載せ、`app.log` へ理由を書き、フロントはエラー通知を1回出す。起動時の挨拶には `pet_daily_greeting` で時間帯・通知件数・直近のできたことメモを添える。
- タスクスケジューラ（`task_scheduler` スキル + `scheduler.rs`）: バックエンドは起動時に30秒間隔の常駐タスクを開始し、`PICOAGENT_SCHEDULED_TASKS` を毎回読み直して現在分に一致するタスクを実行する。同じ分の二重実行は `last_run_minute` で防ぎ、`repeat=false` のタスクは初回実行時に無効化する。スケジュールは `time`（HH:MM）+ `days`（mon..sun / 平日 / 週末 / 毎日。解釈できない曜日はエラーで断る——黙って捨てると空指定と区別が付かず「毎日」へ吸い込まれるため）または cron 風5フィールド（分 時 日 月 曜日、`*` / 数値 / カンマ区切り / `*/N`）で指定する。日と曜日を両方`*`以外にした場合は標準cronと同じOR条件で、`0 9 1 * 1`は毎月1日または毎週月曜の9時に一致する。片方が`*`ならもう片方だけを条件にする。**`1-5` のような範囲指定には対応しない。**保存時に `validate_cron` が照合器と同じ文法で検査し、解釈できない書式は断る。`kind=command` は `program` + `args` を shell なしで実行し、`PICOAGENT_COMMAND_ENABLED=true`、共通許可ルート配下、`PICOAGENT_COMMAND_ALLOWLIST` 完全一致を必須にする。`kind=tool` は `PICOAGENT_SCHEDULED_TOOL_ALLOWLIST` に完全一致する `tool_name` と `tool_args` だけを `tools::dispatch` に渡す。スケジュール実行は無人実行のためチャット確認を挟まず、未許可ならログへエラーを残して実行しない。登録・一覧・削除・有効/無効化はチャットまたは設定画面から行える。実行結果は `app.<日付>.log` と `app_data_dir/schedule-runs.json`（直近100件）へ記録し、メイン画面へ `pet:notice` を emit して通知一覧に表示する。
- アクションセンター（`renderer.js`）: メインフキダシの通知ボタンは `pet:notice`、`pet_schedule_runs`、会話タスクの状態を同じ一覧へ表示し、未読件数をバッジ表示する。直近50件は `localStorage` に保持する。実行中会話には停止、完了会話には再入力、スケジュール通知には再実行と設定の操作を出す。失敗時はエラー色、成功時は通常色で表示する。
- 共通プロセス実行（`tools/process.rs`）: スキルからの外部コマンド起動を1箇所へ集約する。`process::command()` がWindows のコンソール窓抑止（`CREATE_NO_WINDOW`）・`kill_on_drop`（中断時の子プロセス回収）・stdio の既定を適用した `Command` を返し、`process::output()` / `output_text()` がタイムアウト付きで実行する。**スキルが `Command::new` を直接使うことは禁止**で、`tools::process::guard` のテストが直接起動と自前 `hide_window` / `CREATE_NO_WINDOW` を検出して失敗させる。
- 共通許可（`permissions.rs`）: ファイル・ディレクトリ・コマンド実行の許可ルールを1箇所へ集約する。許可判定を通過したパスは専用型 **`SafePath`** で表し、生成経路を `resolve()` / `SafePath::join()` / `SafePath::sibling()` に限定する（内部 `PathBuf` は非公開。派生パスも再検証するので `..` で脱出できない）。ファイル操作は `SafePath` だけを受け取る `permissions::fs` を使う。アプリ内部が決めたパス（一時ファイル等）のみ `SafePath::app_owned` で扱う。**スキル側での許可判定の再実装は禁止**で、`permissions::guard` のテストが `skills/**/mod.rs` を走査し、自前判定（`fn bounded_path(base:` 等）とパス引数を持つのに `resolve` を通していないスキルを検出して失敗させる（例外は `run_command` / `code_runner` / `trash_file` のみ。いずれも独自の追加制約を持つ実行・削除系）。**正本キーは `PICOAGENT_ALLOWED_ROOT`（主ルート、既定はホーム）／`PICOAGENT_ALLOWED_PATHS`（追加許可ルート）／`PICOAGENT_DENIED_PATHS`（追加拒否）**。`C:\Windows` / `C:\Program Files` / `C:\Program Files (x86)` は設定に関わらず常に拒否する（`is_system_protected`）。許可判定では既存の最深祖先を実体化し、シンボリックリンクやWindowsジャンクション経由のルート外到達も拒否する。全ファイル/実行スキル（`read_file` / `workspace_files` / `trash_file` / `ms_office` / `document_convert` / `pdf_tools` / `image_tools` / `research_report` / `spark_workspace` / `genspark_suite` / `meeting_recorder` / `whisper_transcribe` / `run_command` / `code_runner` / `scheduler`）とドロップファイル読込はこの共通ルールを参照する。旧キー（`PICOAGENT_TOOLS_ROOT` / `PICOAGENT_WORKSPACE_ROOT` / `PICOAGENT_DOCUMENT_ROOT` / `PICOAGENT_COMMAND_ROOT` / `PICOAGENT_COMMAND_ALLOWED_PATHS` / `PICOAGENT_MEETING_ROOT`）は設定済みなら許可ルートとして引き継ぐ（移行互換）。設定画面の「安全・許可」タブで編集する。個別スキルの許可ルート設定フィールドは廃止した。
- プロジェクト文脈の差し込み: `pet_send` のたびに `project::project_context()` が選択中プロジェクトの未完了課題（上位5件）と直近の記録（4件）を1,200文字までにまとめ、`apply_project_context()` が system メッセージの `[プロジェクト文脈]`〜`[/プロジェクト文脈]` ブロックを**入れ替える**（追記だと課題を足すたびに古いまとめが積み重なる）。台帳が読めないときは差し込まずに会話を続ける（読み取り専用の付加情報で、保存はしないためデータは失われない）。
- ユーザーへの確認（`ask_user`）: モデルが判断に迷う点を2〜4個の選択肢つきで尋ねる。`run_command` の承認カードと同じマーカー方式で、`agent.rs` がエージェントループを止め、`commands.rs` が質問文と `choices` をフロントへ返す（クイック操作欄が選択肢ボタンに切り替わる）。保留中の質問は `AppState.pending_question` が1件だけ保持し、次のユーザー発話で必ず解除する。返答は番号・全角番号・ラベル一致なら選択肢名へ寄せ、それ以外は自由記述としてそのまま会話へ入る。外部送信も設定変更もしない。
- 自力実行（`code_runner`）: cwd検査と文字列denylistではスクリプト内部の許可ルート外アクセスや難読化された物理削除を保証できないため、現在は `agent_callable=false` としてモデルへ公開しない。OSレベルの隔離実行を用意するまで設定を有効にしても会話中のツール一覧へは出ない。
- コマンド実行（`run_command`）: `PICOAGENT_COMMAND_ENABLED=true` が必要。コマンドが `PICOAGENT_COMMAND_ALLOWLIST` に未登録、または `cwd` が共通の許可ルート外の場合、チャットへ確認文を返す。ユーザーが「許可」と送った場合は保留中の1件だけ実行する。「永続許可」と送った場合はコマンド prefix を `PICOAGENT_COMMAND_ALLOWLIST` へ、必要なら cwd を共通の `PICOAGENT_ALLOWED_PATHS` へ保存してから実行する。「キャンセル」で破棄する。`C:\Windows` と `C:\Program Files*` 配下は編集不可で、チャット許可があってもコマンド実行先やファイル編集先にしない。`workspace_files` の `write` / `append` / `mkdir` も同じ保護パスを拒否する。
- 起動待ち画面: `#petApp.is-booting` 中は保存形式に関係なく `data-layout="stream"` を適用し、通常の配信背景と同じレイヤーの上へ `#bootScreen` を重ねる。680×430の左ステージにはキャラ、右の配信チャット列には固定準備ログ・プログレス・現在処理を置き、プログレスがキャラの下へ回り込まないようにする。起動画面全体は `data-tauri-drag-region` で移動でき、新規環境のウィンドウ初期位置は既存の右下配置規則を使う。専用寝姿を起動時にも使うのはmanifestで `boot.useSleepingOnStartup=true` を持つ、むぎ・りん・まいの3キャラだけで、他の2Dは通常表示と同じ `idle` 先頭フレーム、Live2Dは紹介用 `preview.jpg` を出さず実モデルを使う。バックエンドの `layout::apply_boot()` は起動中だけ680×430へ広げ、この間は形式ごとの寸法記憶を更新しない。準備完了時は `pet_finish_boot` が保存形式と寸法を復元してから `is-booting` を外す。
- 通常の配信形式: 既定1920×1080のフルHD左ステージで、**2D／Live2Dを共通の胸上構図へ置く**。`cover` はキャラごとの縦横比で顔だけまで拡大するため使わない。画面内ディスプレイはステージ左、キャラは中央より右を既定位置とし、互いを覆わない。Live2Dの内部描画寸法は基準285×315pxのまま維持し、`:not(.is-booting)` のときだけキャンバスのCSS表示寸法を `--stream-canvas-scale`（2.75）倍にする。基準寸法は `renderer.js` の `LIVE2D_RENDER_WIDTH` / `HEIGHT` が正本で、起動時に CSS 変数 `--char-frame-width` / `--char-frame-height` へ渡すため、拡大後の実寸をCSSへ直書きしない。視線追従の `transform` を持つ立ち絵枠を配置基準にすると上端で切れるため、配信時はその変形を外して描画レイヤーを左ステージへ直接固定する。キャラ描画領域は画面下端まで延ばし、その手前へテロップだけをステージ最下部に重ねることで、余白の上端で身体が不自然に切れたりテロップ下へ身体がはみ出したりしないようにする。右側チャット列は `clamp(320px, 36vw, 560px)` で拡大し、履歴と入力欄を上下につながる一体パネルにする。字幕は28pxの濃いローズ色太字を左寄せし、複数行は孤立した短い二行目を作らない均等折り返しにする。「字幕」ラベルと枠は白〜薄桃半透明のパステル調にする。背景はキャラクター画像へ合成せず、原寸1920×1080の `assets/backgrounds/vtuber-room.webp` を独立した配信ステージ背景として使う。この指定は `data-layout="stream"` のみに閉じ、他の7形式と起動画面へ影響させない。将来キャラ設定から背景を選べるよう、背景は独立アセットとして管理する。

- フロント正本は `ui/`。`cargo xtask build-ui [--release] [--beta]` が `ui/` を `ui-dist/` へコピーし、Tauri の静的 `frontendDist` は `ui-dist/` を配信する。`cargo xtask dev` は読みやすいコピー、`cargo xtask build` は HTML/CSS のコメント・余分な空白と JS の空行・行頭行末空白を削った release コピーを生成する。`--beta`（`build-beta` が自動で渡す）を付けたときだけ `ALPHA` バッジを残す。コピー（と release の最小化）が終わったあと、HTML が読み込む同梱 CSS / JS の `href` / `src` へ内容ハッシュの `?v=…` を打ち直す。版を手書きしないので付け忘れ・食い違いが起きず、変更したファイルだけが webview のキャッシュを外れて読み直される。`withGlobalTauri:true` で
  `window.__TAURI__` を有効化し、`ui/picoagent.js` が旧 `window.picoagent`（init/send/menu/on/settings.*）を再現する。
  これにより `renderer.js` / `settings.js` は Electron 版とほぼ同じコードのまま動く。
- フキダシ左上のリサイズつまみは Pointer Events で幅・高さを変更し、確定時に `pet_set_bubble_layout` で幅・高さ・座標を設定へ永続化する。拡大時は最小サイズ以外の上限を設けない。保存済みのフキダシ幅・高さ・座標は起動時に復元し、座標はキャラクター回避の優先位置として扱う（重ならなければそのまま、重なる場合だけ位置を動かす。サイズの自動縮小は行わない）。既定（未保存時）のフキダシは CSS で「ウィンドウ下端付近まで縦に伸ばし、右側のキャラクターに被らない幅」にする。フキダシが透明ウィンドウ外へ出る場合は、ドラッグ中に Tauri ウィンドウ自体も `setPosition` / `setSize` で左上方向へ追従させる。さらにリサイズ確定時とサイズ復元時に `ensureWindowFitsBubble` で「フキダシ右側にキャラクター分の幅（と下側の高さ）」を確保するようウィンドウを左上方向へ拡大し（右下は固定）、大きくしたフキダシが右端固定のキャラクターと重ならないようにする。
- 会話入力欄は全形式で列幅を100%使い、送信・音声・連続ボタンは入力欄内へ重ねず `.composer-actions` の独立した下段へ並べる。狭い chat / widget / bar でも本文の実入力幅を確保し、添付プレビューとクイック操作は従来どおり入力欄の上へ置く。回答操作は記号だけにせず「通知・コピー・再生成・読上・停止・拡大」の可視ラベルを持ち、全形式で折り返さず1段に収める。段組みが増えると狭い形式で本文が押し出されるため、`.reply-actions` は `flex-wrap:nowrap` とし、文字・余白・間隔をフキダシ幅のコンテナクエリ（`cqi`）で縮める。ペット形式はキャラクターの取り分も `min(430px,48%)` と比率で持ち、窓を狭めてもフキダシが窓幅の45%を下回らないようにする。2D立ち絵は `data-render-kind="2d"` の形式別補正で透明余白ぶんを拡大し、Live2Dのモデル別倍率には重ねない。
- ウィンドウのレイアウトは、`RunEvent::Exit` ではウィンドウ座標を取得できないため、生存中の `WindowEvent::Moved`/`Resized` で**サイズと絶対座標を `WindowLayoutCache` に控え**、終了時にそのキャッシュ値だけを設定へ保存する。枠なし透明ウィンドウでもリサイズできるよう、画面の4辺と4隅に透明な操作領域を置き、`startResizeDragging` でOS標準リサイズを開始する。辺は幅または高さ、四隅は縦横を同時に変更する。**位置は仮想デスクトップ上の物理座標**（`PICOAGENT_WINDOW_LEFT_PX` / `PICOAGENT_WINDOW_TOP_PX`）で保存・復元するため、マルチモニタでも**同じディスプレイの同じ位置**へ戻る。**サイズ**も保存・復元する（全画面中はキャッシュせず、復元値はモニタ内へ収める）。
  - **モニタ内の相対オフセットでは「どのディスプレイに居たか」を復元できない。** 旧実装は画面右端・下端からのオフセットだけを保存し、配置計算でも `monitor.position()`（モニタ原点）を加えていなかったため、左側に並ぶモニタ（負の X を持つ）環境では常に主モニタ側の座標が算出されていた。位置計算は必ずモニタ原点を加える（`position_from_offsets`）。
  - **ユーザーはモニタの主/副・解像度・本数をいつでも変える。** Windows の仮想デスクトップは**主モニタ左上が原点**なので、主副を入れ替えるだけで同じ物理配置でも全モニタの座標が変わり、保存済みの絶対座標が別の場所（あるいは画面外）を指す。起動時と実行中の両方で検証する。
  - 復元時は保存座標と最も重なるモニタを選び、ウィンドウ全体がそのモニタ内へ収まる座標に補正する。ディスプレイを外した・主副を入れ替えた・解像度を下げた後に画面外へ復元して**起動したのに何も見えない**状態になるのを防ぐ。どのモニタにも載らない場合は認識中のモニタ一覧と一緒にログへ残し、既定位置へ戻す。既定位置はモニタ右端から12px内側とし、表示形式の切替時も `pull_back_on_screen()` が現在のモニタ内へウィンドウ全体を収める。
  - 実行中にディスプレイ構成が変わると Windows がウィンドウを動かし直して `Moved` が飛ぶ。このとき `is_on_screen()`（1px でも見えているか）で完全に画面外へ取り残されていないか確認し、取り残されていればその場で既定位置へ引き戻す。**実行中の判定は保存時より緩くする**（80px 基準で引き戻すと、端へ寄せる通常のドラッグ操作まで勝手に戻されるため）。画面外の座標をキャッシュ＝保存させない役目も兼ねる。
  - ドラッグで**そのモニタの上端**から `SNAP_THRESHOLD`(24px) 以内に来たら上端へスナップする（横はスナップしない）。仮想デスクトップの `y=0` ではなくモニタ原点を基準にする（上下に並ぶ配置では `y=0` が別の画面を指すため）。
  - 右クリックメニューの「表示と会話をリセット」→「今の画面位置・サイズを戻す」と、設定画面（画面・操作）の「画面位置を初期化」ボタン（`pet_reset_layout`）が同じ `menu::reset_bubble_and_window_layout()` を呼び、フキダシ保存値・ウィンドウサイズ・絶対座標を初期化して既定位置へ戻す。**ウィンドウを画面外で見失うと右クリックメニュー自体に辿り着けない**ため、設定画面にも復帰口を置いている。旧互換のため `pet_set_bubble_size` も残す。
- フキダシの吹き出しの尻尾（`.bubble::after`）は、縦長フキダシでも右下のキャラクターを向くよう下寄り（`top:64%`）に置く。
- キャラクターまたはフキダシのダブルクリックはウィンドウをフルスクリーン化（`getCurrentWindow().setFullscreen`）し、その中でフキダシを左上起点に大きく広げる（`.bubble.fullscreen`）。拡大時はキャラクターの実測矩形から右側の空け幅を再計算し、キャラ表示用の右ペインを残す。ドラッグでの拡大はまずサイズを維持したままフキダシ座標を再配置してキャラクター矩形を避ける。座標移動だけでは避けられない大きさの場合だけ、左ペインまたは上ペインのうち表示面積が大きい方へ最小限サイズを補正し、設定保存直前にも再配置後の座標とサイズを永続化する。再計算は全画面切替後、ウィンドウサイズ変更、キャラ画像ロード、設定反映、ドラッグ確定後に行う。解除は再度ダブルクリックまたは `Esc`。ドラッグ領域のダブルクリックによるネイティブのウィンドウ最大化は `maximizable:false` で無効化している。
- 入力欄は `Enter` / `Ctrl+Enter` / `Cmd+Enter` で送信、`Shift+Enter` で改行、`Esc` で閉じる。送信中や空入力では送信ボタンを無効化し、入力欄は内容に合わせて最大 180px まで自動伸縮する。ユーザー発話は直近40件まで入力履歴として保持し、入力欄の先頭/末尾で `↑` / `↓` を押すと過去入力と編集中ドラフトを行き来できる。入力欄上部の `quick-actions` は、画面確認・クリップボード要約・今日の予定・できること確認を既存の通常送信経路へ流す。通常の右クリックメニューはフキダシ本文・履歴・入力欄・リンク上では妨げず、コピーやリンク操作を優先する。フキダシ上部の `reply-actions` は回答コピー・再読み上げ・停止・全画面切替を提供する。読み上げ準備中・読み上げ中はフキダシ下部に `読み上げを中止` ボタンを表示し、押すと進行中の音声合成待ち・連続再生を打ち切る。
- 拡大時はセッション内の会話履歴をフキダシ内に表示し、最新応答と入力フォームは下部に残す。履歴上部の検索欄で会話履歴を絞り込み、各履歴の「コピー」と、ユーザー発話の「再利用」（入力欄へ戻す）を実行できる。`pet:reset` では履歴・履歴検索・入力履歴もクリアする。
- `pet_init` はバックエンド文脈に復元済みの直近会話を `history` として返し、フロントは全画面フキダシの履歴表示へ反映する。読み上げ中にキャラクターをクリックすると現在の音声を停止する。
- ウィンドウ: 透明・枠なし・常に手前・タスクバー表示・最大化不可（`tauri.conf.json`）。ドラッグは `data-tauri-drag-region`。`pet_open_settings(tab)` は省略可能なタブIDを受け取り、新規設定ウィンドウではクエリ、既存ウィンドウでは `settings:navigate` イベントで同じタブへ遷移する。設定画面は保存・再描画後も選択中タブを維持する。
- アプリ終了時は `RunEvent::Exit` で `engine::stop` が同梱 llama-server の子プロセスを kill し、`aivis::stop` がアプリ自身で裏起動した AivisSpeech Engine だけを停止する。右クリックメニューの「PicoAgent を終了」に加え、チャットでは `app_control` スキルの `quit` で終了できる。さらに `cargo xtask dev` / `cargo xtask build` は Tauri 起動前に残存プロセスの停止を実行し、前回の残存プロセスを停止して同梱 DLL ロックによるビルド失敗（Windows os error 32）を防ぐ。Unix 系では `pgrep` / `pkill` の完全一致で `llama-server` / `PicoAgent` / `picoagent` だけを止め、Aivis は PicoAgent 管理下の `resources/bundled/aivis/run` / `resources/aivis/run` / `PicoAgent/bundled/aivis/run` に一致する `run` だけを止める。撮影・検査ツールが残した `node` は、コマンドラインの区切りをスラッシュへ揃えたうえで `tools/screenshots/` を含むものだけをプロセスツリーごと止める（`node` は Codex の MCP サーバなど他アプリも使うため、実行ファイル名だけでは止めない）。`browser_control` が起動したブラウザは、アプリ終了時の `browser_control::stop` と `cargo xtask stop` の両方が、`--user-data-dir` に PicoAgent 専用プロファイル（`<データ領域>/PicoAgent/browser-profile`）を持つものだけを止める。Chrome / Edge は起動直後に本体を立て直して最初のプロセスが終了するため、保持している子ハンドルへの `kill` / `taskkill /T` は本体へ届かない。利用者が普段使っているブラウザは別プロファイルなので対象にならない。手動停止は `cargo xtask stop`。起動・アプリ・フロント・音声・エンジンログは `PicoAgent/log/` に日付付き（`app.<日付>.log` / `engine.<日付>.log` / `aivis.<日付>.log`）で集約し、1 日ごとにローテート・1 週間保持する（履歴は対象外）。
- Windows の NSIS は導入前（`NSIS_HOOK_PREINSTALL`）と削除前（`NSIS_HOOK_PREUNINSTALL`）の両方で、同じ `PICOAGENT_STOP_MANAGED_PROCESSES` を使い、`$INSTDIR` またはユーザーデータ配下 `%APPDATA%\PicoAgent\bundled\` の実体パスに一致する本体・`llama-server`・Aivis `run`・`whisper-cli`・THA3の`python`・THA3導入用`uv`だけを停止する。同梱プロセスの条件は `PICOAGENT_STOP_BUNDLED_PROCESS <実行ファイル名> <同梱ディレクトリ名>` の1か所にまとめる。名前一致では止めず、利用者が単独で起動した同名ローカルAIや設定で選んだ外部THA3は巻き込まない。導入前に止めるのは、上書き導入中に子プロセスが実行ファイル・DLLを掴んだままだと配置が失敗し、新旧が混ざった導入結果になって起動できなくなるため。
- 権限は `src-tauri/capabilities/default.json`（start-dragging / set-always-on-top / set-fullscreen / set-position / set-size / event / webview 生成）。
- 製品化: `productName=PicoAgent` / `identifier=com.picoagent.desktop`（旧 `com.picoagent.app` / `com.coda.app` から移行）。設定・履歴・メモリは `app_config_dir`/`app_data_dir` を使い、初回起動時に旧識別子配下の `config.json` / `history.json` / `memory.json` を現行識別子へコピー移行する。同梱系の実行時取得物はユーザーデータ配下の `PicoAgent/bundled/`、ログは `PicoAgent/log/` にまとめる。`bundle.targets=all` で各 OS の標準バンドルを生成し、Windows は NSIS 設定を保持する。`cargo xtask build`と販売用ビルドは基本エンジンと`installer-hooks-lite.nsh`だけの軽量構成にし、任意の大容量追加機能を暗黙取得しない。Aivis／Whisper／THA3／Office／外部CLIを含む従来構成は明示した`build-full`だけが`installer-hooks.nsh`で生成する。通常版のTHA3インストール後処理も、CUDA対応NVIDIA GPUを実行確認できた場合だけCUDA版PyTorchを取得し、AMD／Intel／GPUなし環境では取得前に正常スキップする。軽量版は公式取得元マニフェストを同梱し、管理対象のAivis／Whisperだけを利用者の明示操作後に検証付きで直接取得する。Rust release profile は LTO・strip・panic abort・debug 情報無効化を有効にして、配布バイナリのシンボル情報を減らす。アンインストール前は PicoAgent / llama-server / PicoAgent 配下の Aivis `run.exe` を停止する。専用一時ファイルは削除するが、設定・会話履歴・追加キャラ・モデル等のユーザーデータは対話時に削除確認し、既定の「いいえ」とサイレントアンインストールでは残す。
  アイコンは `src-tauri/icons/`（`npx tauri icon src-tauri/icon-source.png` で再生成）。
  `cargo xtask build`、`build-beta`、`build-distribution` はいずれも `distribution/release.json` の許可キャラだけを同期し、開発用サンプルが手元にあっても製品へ混ぜない。`build-beta` は販売条件を完了扱いにせず、販売版と同じ軽量同梱範囲で試用インストーラを作る。`build-distribution` だけはEULA／プライバシー確認、第三者同梱物台帳、Live2D審査参照、公開URL、3箇所の版一致、販売対象キャラの存在を検査し、未完了なら取得・同期より前に停止する。初期販売は `signing.required=false` / `updates.required=false` とし、OSコード署名・公証・自動更新をブロッカーにはしないが、警告表示と手動更新案内を残す。将来各 `required` を有効にした場合だけ、更新署名・Windows／macOS署名を再び必須検査する。秘密鍵・証明書は販売設定へ含めない。
- 更新確認の正本は公開配布Gitの `latest.json`（`schemaVersion=1`、`version`、`notes`、`releasePage`、OS別 `url` / `size` / `sha256`）。起動時は新版通知だけを行い、設定の「更新を確認」後に利用者が二段階の「アップデート」を完了した場合だけWindows x64インストーラを取得する。更新対象はPicoAgent本体で、設定・会話履歴・追加キャラ・ユーザーデータ配下のダウンロード済みLLMモデルと追加機能は維持し、再ダウンロードしない。追加機能の版変更は「追加機能・更新」で個別に行う。フィードは256KB、インストーラは512MBを上限とし、URLを公開配布RepoのReleaseに限定してサイズ・SHA-256・PE形式を検証してから起動し、PicoAgent本体を終了する。macOSはアプリ内更新未対応なので更新ボタンを出さない。オフライン・タイムアウトは正常な見送りとして扱う。HTTP異常・不正JSON・未対応schemaは、起動時確認なら画面を妨げず診断ログへ、設定からの明示確認なら操作元へ返す。
- 製品版番号はSemVerとし、通常の修正・依存／モデル／同梱物更新・文書／LP・既存アルファ再配布では同じ`major.minor`系列の`patch`を自動加算する。`minor`と`major`は製品管理者の明示指示がある場合だけ変更する。`cargo xtask bump-version <major|minor|patch>`は公開中の`latest.json`と現行ソース版の一致を確認してから次版を計算し、`src-tauri/Cargo.toml`、`xtask/Cargo.toml`、`src-tauri/tauri.conf.json`を同期する。不一致時は二重採番を避けるため停止する。同じ版の公開資産は差し替えず、次版のプレリリースを作成する。patch番号も公開画面と成果物へ表示する。起動時の自動確認は`major.minor`だけを比較してpatchを通知しないが、設定画面からの手動確認と手動インストールはSemVer全体を比較してpatchも対象にする。アルファ期間中は製品表示、LP、更新説明、Release名へ`Alpha`を付け、比較用SemVerとタグは数値だけにする。
