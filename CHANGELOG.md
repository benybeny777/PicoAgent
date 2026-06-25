# 変更履歴

このファイルには、現行アーキテクチャ以降の運用や品質に影響する大きめの変更だけを残します。

---

## 2026-06-25

### アプリ名を PicoAgent に変更・キャラをむぎのみに整理

- アプリの表示名を **Coda → PicoAgent** に変更した（`productName` / ウィンドウタイトル / 設定ウィンドウ / 右クリックメニュー「終了」/ 入力欄プレースホルダ / `package.json` / `Cargo.toml` の説明 / ドキュメント）。バンドル識別子（`com.coda.app`）と内部 JS API（`window.coda`）・Rust クレート名（`coda` / `coda_lib`）は既存データ互換と影響範囲の都合で据え置き。
- キャラクターを **むぎのみ** に整理し、`coda`（旧）/ `koharu` / `lab_mate` を削除（`characters/` 実体と `index.json`）。`default` は `mugi`。

### キャラ・スキルをプロジェクト直下へ移動（扱いやすい配置）

- スキル（ツール）の連結ファイルを `src-tauri/src/tools/` から**プロジェクト直下の `skills/`** へ移動した。`Tool` トレイト・レジストリ本体（`tools/mod.rs`）は src に残し、`build.rs` の走査先を `../skills` に変更した。`.rs` を1枚置けば自動登録される動作は維持。
- キャラクターの正本を `ui/assets/characters/` から**プロジェクト直下の `characters/`** へ移動した。webview は `ui/` 配下しか配信できないため、`scripts/sync-characters.mjs` が `ui/assets/characters/` へ同期し、`tauri.conf.json` の `beforeDevCommand` / `beforeBuildCommand` で dev/build 前に自動実行する。配信コピー（`ui/assets/characters/`）は `.gitignore` 対象。
- `npm run sync:characters` で手動同期も可能。

### 設定画面に導入済みスキル・キャラクター一覧を表示

- 設定画面に「導入済みスキル（エージェント機能）」と「導入済みキャラクター」の読み取り専用一覧を追加した。設定項目を持たないツールも含め登録済みのものが一覧表示される。
- `settings_load` のツール情報に `description` を追加し、各スキルの説明を表示できるようにした。

### ダブルクリックでフキダシを全画面表示

- キャラクターのダブルクリックで透明ウィンドウがネイティブ最大化され画面が崩れる問題を修正した。`tauri.conf.json` に `maximizable:false` を加えてドラッグ領域ダブルクリックの最大化を無効化した。
- ダブルクリックの挙動を、ウィンドウをフルスクリーン化しその中でフキダシを大きく広げるトグルに変更した（`getCurrentWindow().setFullscreen` + `.bubble.fullscreen`）。右側にキャラ分の余白を残して全身と送信ボタンに被らないようにし、しっぽは口元へ向ける。解除はキャラ／フキダシのダブルクリックまたは `Esc`。`capabilities/default.json` に `core:window:allow-set-fullscreen` を追加。

### 新キャラ「むぎ」を既定化

- 添付イラストを透過切り抜きし、`mugi` キャラクターパックとして追加した。
- 待機・思考・会話・落胆の連番フレームを作成し、既存のLive2D風モーションで揺れるようにした。
- `CODA_CHARACTER` の既定値と初期表示を `mugi` に変更した。設定画面のキャラクター選択には `むぎ` / `ラボメイト` / `こはる` / `コーダ（旧）` を残し、保存後すぐ切り替えられる。

### Live2D風の新キャラ「ラボメイト」を既定化

- 添付イラストから中央の全身キャラを透過切り抜きし、`lab_mate` キャラクターパックとして追加した。
- 待機・思考・会話・落胆の連番フレームと `live2d-*` CSSモーションを組み合わせ、静止絵でもLive2D風に揺れて見えるようにした。
- `CODA_CHARACTER` の既定値と初期表示を `lab_mate` に変更し、設定画面から従来の `こはる` / `コーダ（旧）` へ戻せる構成は維持した。

---

## 2026-06-24

### キャラクターパックと新キャラ「こはる」を追加

- キャラクター固定パスを廃止し、一覧JSONとキャラ別 `character.json` から名前・挨拶・人格・アニメーションを読み込む構成へ変更した。フォルダと一覧項目の追加だけでキャラクターを増やせる。
- 設定画面へキャラクター選択とキャラ別の性格・口調上書きを追加した。切替時は見た目とシステムプロンプトの人格設定を一緒に反映する。
- 桜色の髪と紫の瞳を継承した新しいちびキャラ「こはる」を制作し、状態別フレームと呼吸・思考・会話・落胆の滑らかな補間動作を組み合わせた。

### 右クリックメニューを簡素化

- 右クリックメニューを「設定…」「常に手前に表示」「コーダを終了」の3項目だけにした。プロバイダとモデルの設定は設定画面へ集約した。

### Web検索ツールを追加

- APIキーなしでDuckDuckGoの軽量HTML検索を利用する `web_search` ツールを追加した。モデルは最新情報・ニュース・現在の仕様・出典が必要な質問で検索し、タイトル・URL・概要を材料に回答できる。
- 検索語500文字、結果1〜8件、待ち時間3〜60秒、応答32KBの境界を設けた。結果件数と待ち時間は設定画面から変更でき、保存後すぐに反映する。
- すべてのユーザー入力で、ローカルモデルの `tool_calls` 対応に依存せず回答前に検索を強制実行する。検索結果は信頼できない外部データとして区切り、結果内の指示を採用しない。検索失敗も隠さず回答へ反映する。
- 検索回答の参照元をMarkdownハイパーリンク形式に統一し、フキダシ上で安全にクリックできるリンクとして描画するようにした。

### フキダシのドラッグリサイズに対応

- フキダシ右下のつまみをドラッグして幅と高さを変更できるようにした。確定したサイズは `config.json` に保存し、次回起動時にも復元する。

### Ollama 非依存化 — gemma4(Gemma 3n E2B) を同梱エンジンで既定化

- 推論エンジン llama.cpp（`llama-server`）をアプリに同梱し、起動時に spawn して OpenAI 互換 API をローカルに立てる新プロバイダ **`local`** を追加し、これを既定（`CODA_PROVIDER=local`）にした。**Ollama なしで動作**する。
- エンジン本体はリポジトリに含めず `scripts/fetch-engine.mjs`（`npm run setup:engine`）で取得し、`src-tauri/resources/engine/` に配置して `tauri.conf.json` の `bundle.resources` で同梱する。
- モデル GGUF（既定: Gemma 3n E2B の Q4_K_M、約3〜4GB）は配布物に含めず、**初回起動時に `app_data_dir/models/` へダウンロード**する（以後オフライン）。進捗・状態は `engine:status` イベントで UI に表示する（`engine.rs`）。
- `local` プロバイダは OpenAI 互換変換（`openai.rs` の `to_wire`/`from_wire`）を再利用。tools 非対応テンプレートのモデルでも会話が成立するよう、tools 付きで失敗したら tools 無しで再試行する。
- 設定キー追加: `CODA_MODEL_URL` / `CODA_LOCAL_PORT` / `CODA_LOCAL_CTX`。Ollama プロバイダは任意利用として存続。

### Electron を Tauri へ全面移行（Rust バックエンド・製品化対応）

- ランタイムを Electron から **Tauri 2** へ置換し、`electron/` と Node の `src/*.js`、`start.cjs`、electron 依存を削除した。フロントは `ui/` の静的 webview として配信する。
- コネクタ/エージェント/設定/ツールのロジックを **すべて Rust（`src-tauri/`）で再実装**した。`message.rs`（正規形）/ `config.rs`（設定の正本）/ `agent.rs`（ループ）/ `providers/`（ollama・openai）/ `tools/`（time・read_file）/ `commands.rs` / `menu.rs`。
- コネクタの自動登録を `inventory` crate ＋ `build.rs` のモジュール自動生成で実現し、「`.rs` を1枚置くだけ」でコネクタを追加できる思想を Rust でも維持した。
- 旧 `window.coda`（Electron preload）を Tauri 上で再現する互換シム `ui/coda.js` を追加し、`renderer.js` / `settings.js` をほぼ無改変で流用した。ウィンドウのドラッグは `data-tauri-drag-region` に置換。
- 設定の正本を `app_config_dir/config.json` に移し、優先順位は永続ファイル > 環境変数 > 既定値を踏襲した。
- 製品化対応: `productName=Coda` / `identifier=com.coda.app`、アイコン一式（`src-tauri/icons/`）、Windows NSIS インストーラのバンドル設定を追加。署名・自動更新はリリース時設定の道筋として記載。

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
