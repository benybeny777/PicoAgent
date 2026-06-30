# AGENTS.md — Claude Code / AI エージェント向け作業ガイド（ルール正本）

このファイルが唯一のルール正本です。`CLAUDE.md` はこのファイルへのリダイレクトのみです。ルールを追加・変更する場合はこのファイルだけを編集してください。

## 最重要運用ルール

- `AGENTS.md` はすべて日本語で記述する。英語セクション・英語コメントは追加しない。既存の英語記述を見つけたら日本語に置き換える。
- ルールに反する記述・命名・コメント・dead code を発見した場合は、その場で修正して作業完了後に事後報告する。都度確認を求めない。
- ルール正本は `AGENTS.md` 1本。`CLAUDE.md` はリダイレクトのみなので編集しない。恒久ルールはすべてこのファイルに追記する。
- **ドキュメントは常に最新を保つ。コード・スクリプト・設定・依存関係・フォルダ構成・コネクタのインターフェースなど、README や SPEC.md に記載のある内容を変更したら、必ず同じ作業内でドキュメントも更新する。ユーザーから指摘されてから直すのは禁止。**
- 対象ドキュメントの目安: アーキテクチャ・コネクタ仕様・メッセージ正規形・設定値 → `SPEC.md`、大きな変更 → `CHANGELOG.md`、起動手順・セットアップ・設定キー・依存関係 → `README.md`。
- ドキュメント更新が本当に不要な軽微修正の場合のみ、不要と判断した理由を作業完了時に明記する。
- コミットメッセージ・ソースコードのコメント（Rust / JS とも）は基本的に日本語で書く（コミットは英語だけの件名にしない）。`feat` / `fix` / `docs` / `chore` などの短い英語プレフィックスや、API / JSON / trait / 識別子・関数名など技術用語の英語はそのまま使ってよい（無理に和訳しない）。
- 運用や品質に影響する大きめの変更は `CHANGELOG.md` に残す。
- **常に Codex / Claude（作業エージェント自身）の利用クレジット消費を最小化して作業する。** 不要な長文出力、重複調査、過剰なファイル読み込み、不要な並列探索、同じ確認のやり直しを避ける。まず検索・差分確認・必要最小限のファイル読み取りで判断する。
  - 検証コストの優先順位: まず `cargo check`（コンパイル）→ 必要なら `cargo test --lib`（ロジック）で済ませる。**GUI 起動（`npm run dev`）・スクリーンショット・フルビルドは高コストなので最後の手段**とし、目視確認が本当に必要なときだけ、複数の確認をまとめて 1 回で行う。
  - 編集はまとめて行い、**ビルド/テスト/起動は変更が出揃ってから最小回数**だけ実行する。同じ内容のビルド・テスト・起動・読み取りを繰り返さない。
  - 起動して確認したら使い終わったプロセスは止め、ロックや再ビルドの無駄を避ける。確認が自動化しづらい（GUI・音声など）場合は、コードの正しさで判断し、最終目視はユーザーに委ねてよい。
- PowerShell で日本語ファイルを確認するときは、文字化けを避けるため UTF-8 を明示する。表示が文字化けしても、すぐにソース破損と判断せず UTF-8 で読み直してから判断する。
- 既に未コミット変更がある場合は、ユーザーまたは別エージェントの作業として扱い、勝手に巻き戻さない。
- 修正・作業が完了したら、ユーザーからの個別指示がなくても、作業ツリーの未コミット変更を `git add -A` で全てステージしてコミットし、`origin` へプッシュまで行う。自分が編集したファイルだけでなく、既に存在していた未コミット変更も同じコミットに含めてプッシュする（コミットメッセージは日本語。本ファイルの他ルールに従う）。破壊的な履歴操作（force push・reset --hard など）はしない。

## エラー処理・リソース

- **エラーは原則 `Result` で上位へ伝播させる。サイレントスキップ・無言フォールバック・無言デフォルト降格は禁止。** フォールバックを書く場合はその箇所に `// フォールバック許可: 理由` コメントを付ける（例: `config.rs` の破損時空起動）。
  - 例外として、プロバイダの `is_up()` のような「接続可否を返すプローブ」は、未起動・タイムアウトを `false` に正規化してよい（接続できないこと自体が戻り値なので、握りつぶしではない）。
  - ツールコネクタの `run()` 内の失敗は、`Result` で投げず**エラー内容を文字列で返してモデルに伝える**（`tools::dispatch()` も最終防壁として未登録ツールを文字列化する）。エージェントが失敗を理解して回復するための設計上の許可。
- `unwrap()` / `expect()` は、起動時の致命的初期化（`main`/`setup`）や Mutex ロックなど「失敗＝即異常終了が妥当」な箇所に限る。通常経路では `?` と `Result` を使う。
- **取得したリソースは明示的に解放する。** 子プロセス・ファイルハンドル・ネットワーククライアント/接続・タイマー（`setInterval` 等）・イベントリスナ・フロントの `URL.createObjectURL` / `Audio` など、確保したものは「正常終了・失敗・中断（キャンセル）」のどの経路でも確実に解放する。`Result` の早期 return や例外経路で解放が漏れないようにする。
  - 子プロセスは終了時に `kill` し、`wait` でハンドルまで回収する（ゾンビ・ハンドルリークを残さない）。
  - `reqwest` の `is_up` などプローブ系には `timeout` を付ける（接続が永久に滞留しないようにする）。長時間ストリーミング（モデル DL・生成）は全体 timeout を付けない代わりに接続自体の滞留に注意する。
  - フロントの `URL.createObjectURL` は再生終了・失敗・中断で必ず `revokeObjectURL` する。`setInterval` 等のタイマーは全経路で `clearInterval` する。
- ユーザーから明示指示がない限り、同梱実行ファイル・同梱モデル・同梱ランタイムは PicoAgent 管理下の専用ディレクトリ（原則 `src-tauri/resources/bundled/<機能名>/`、実行時データは `app_data_dir` 配下）に設置し、既存の手動インストール先・グローバルPATH・共有ポートを既定で使わない。手動インストールを使う場合は設定で明示されたときだけ優先し、同梱物のアンインストール・一時ファイル削除は PicoAgent 管理下のパスだけを対象にする。

## 設定値・シークレット

- 設定の正本は `src-tauri/src/config.rs`（永続ファイル `app_config_dir/config.json`）。優先順位は **永続ファイル > 環境変数 > ハードコード既定値**。新規設定で環境変数を「正本・保存先」にしてはならない（環境変数は既定値としてのみ参照する）。
- 数値・閾値・パス・モデル名など「設定として変えうる値」はソースに散らさず `cfg.get(キー, 既定値)` で都度読む。コネクタは自分の設定項目を `config_schema()`（`Vec<Field>`）で申告し、設定画面に自動で並ぶようにする。新しい設定キーを足したら README と SPEC に必ず記載する。
- 設定画面（右クリック →「設定…」）が唯一の GUI 設定入口。`ui/settings.{html,css,js}` と `commands.rs` の `settings_*` command で構成し、保存後は実行中に反映する（コネクタは `cfg.get()` で都度読むため再起動不要）。
- API キー・トークン等のシークレットはコミットしない。ログ・エラーメッセージ・ドキュメントに値を出さない。OpenAI 互換プロバイダの `Authorization`（`bearer_auth`）など秘密値を含む箇所は、エラー文へ連結しない。`config.json` はローカル個人 PC 専用で、リポジトリにコミットしない（`app_config_dir` 配下に保存される）。

## コネクタ開発ルール（このプロジェクトの中核）

PicoAgent は「叩く AI（LLM バックエンド）」と「エージェント機能（スキル/ツール）」をコネクタとして拡張する。**どちらも決まった場所へファイルを置けば自動登録される**。`build.rs` が `src/providers/*.rs`・ルートの `skills/<スキル名>/mod.rs` を走査して `#[path] pub mod` を生成し、各ファイルの `inventory::submit!` で登録されるため、中央の `mod.rs` 編集は不要。スキルは扱いやすいようプロジェクト直下の `skills/<スキル名>/mod.rs` に置く（トレイト・レジストリ本体は `src-tauri/src/tools/mod.rs`）。スキル専用の補助モジュール、スクリプト、テンプレート、少量の静的アセットは同じ `skills/<スキル名>/` 配下へ同梱する（例: `skills/chitchat/trends.rs`、`skills/ms_office/scripts/office.ps1`）。

- **プロバイダコネクタ** は `src-tauri/src/providers/` に置き、`Provider` トレイトを実装して `inventory::submit! { ProviderReg { make: || Box::new(<型>) } }` で登録する。`chat()` は正規形メッセージ（`message.rs`）を受け取り正規形 assistant を返す。プロバイダ固有のワイヤ形式変換は各コネクタ内に閉じ込め、`agent.rs` に漏らさない。
- **スキル（ツール）コネクタ** はプロジェクト直下の `skills/<スキル名>/mod.rs` に置き、`Tool` トレイトを実装して `inventory::submit! { ToolReg { ... } }` で登録する。`run()` は `String` を返す（失敗もエラー文字列で返す）。トレイト・レジストリ本体は `src-tauri/src/tools/mod.rs`。
- スキル固有の Rust 補助モジュールは `skills/<スキル名>/*.rs` に置いて `mod xxx;` で読み込む。PowerShell / Python / テンプレート等は `skills/<スキル名>/scripts/` や `skills/<スキル名>/templates/` に置き、必要なら `include_str!` でバイナリへ同梱する。複数スキルで本当に共有するトレイト・型だけを `src-tauri/src/tools/mod.rs` 等の共通層に置く。
- Cargo の外部依存クレートはプロジェクト全体の `src-tauri/Cargo.toml` に集約する。Rust はスキルごとの独立 `Cargo.toml` をこの構成では持たないため、依存クレート自体を各スキルフォルダに複製しない。代わりに、依存クレートを使うためのスキル専用ラッパーや実行スクリプトをスキルフォルダへ同梱する。
- システムに副作用を持つツール（ファイル・ネットワーク・プロセス等）は、必ず境界（許可ルート・サイズ上限・確認など）を設ける。例: `skills/read_file/mod.rs` は `PICOAGENT_TOOLS_ROOT` 配下に限定しトラバーサルを拒否する。
- コネクタが設定値を必要とするなら `config_schema()` で申告する。値はフィールドに固定せず**メソッド内で `cfg.get()` から都度読む**（設定画面での変更を再起動なしに反映するため）。
- コネクタを追加・変更したら `SPEC.md` のインターフェース定義と一覧、必要なら `README.md` の設定キーを更新する。
- メッセージ正規形（コネクタ間の共通言語）を崩さない。詳細は [SPEC.md](SPEC.md) を参照。

詳細仕様は [SPEC.md](SPEC.md) を参照。ここには作業上の注意点だけを書く。

## プロジェクト概要

PicoAgent は同梱推論エンジン（既定）やローカル/リモートの LLM で動く、キャラクターとフキダシだけの透明 **Tauri** デスクトップペット。バックエンドは Rust、フロントは webview（`ui/`）。叩く AI とエージェント機能をコネクタで差し替え・追加できる。

## 実行環境

- Windows / macOS を想定。**Rust（cargo）と Node.js >= 20**、WebView2（Windows ランタイム）。**Ollama は不要**。
- 開発起動: `npm run setup:engine`（同梱エンジン取得）→ `npm run dev`。製品ビルド: `npm run build`（`src-tauri/target/release/bundle/`）。
- **既定の叩く AI は `local`（同梱 llama-server）**。初回起動時に `PICOAGENT_LOCAL_MODEL_PRESET=auto` が PC の VRAM/RAM 目安から Gemma 4 系プリセットを選び、モデル GGUF をユーザーデータ配下の `PicoAgent/bundled/models/` へ DL し、以後オフライン。Ollama / OpenAI 互換は任意プロバイダとして存続。
- **OpenAI 互換プロバイダ（`OPENAI_API_KEY` 利用）は費用が発生するため、ユーザー指示または運用上の明確な必要がある場合だけ実行する。** local / Ollama はコストがかからない。
- 初回 `cargo build` / `tauri build` / モデル DL は時間がかかり、ネットワークが必要。
- 推論エンジン本体（`llama-server` + DLL）と `*.gguf` はリポジトリに含めない（`.gitignore` 済、`npm run setup:engine` で取得）。

## 製品化（配布）

- `src-tauri/tauri.conf.json` の `productName` / `identifier=com.picoagent.desktop` / `version` を保つ。アイコンは `src-tauri/icons/`（`npx tauri icon <1024pxソース>` で再生成。ソースは `src-tauri/icon-source.png`）。
- Windows バンドルは NSIS（`bundle.targets`）。署名（Authenticode）・自動更新（`tauri-plugin-updater`）は鍵・証明書が要るためリリース時に設定する（未設定）。

## 主要ファイル

| ファイル | 役割 |
|---|---|
| `src-tauri/src/main.rs` / `lib.rs` | エントリ・Builder（state・command・menu イベント・setup） |
| `src-tauri/src/commands.rs` | Tauri command（`pet_*` / `settings_*`）と共有状態 `AppState` |
| `src-tauri/src/menu.rs` | 右クリックネイティブメニューと選択イベント処理 |
| `src-tauri/src/config.rs` | 設定の正本（`app_config_dir/config.json`） |
| `src-tauri/src/history.rs` | 会話履歴の永続化（キャラ別 `app_data_dir/history-<キャラID>.json`。起動時・キャラ切替時に直近を文脈へ復元） |
| `src-tauri/src/alarm.rs` | 目覚まし時計の保存（config `PICOAGENT_ALARMS`）と曜日・時刻の正規化。発火はフロント時刻監視 → `pet_alarm_wake` |
| `src-tauri/src/engine.rs` | 同梱 llama-server とモデル GGUF のライフサイクル（DL・spawn・状態通知） |
| `src-tauri/src/agent.rs` | エージェントループ（chat → tool_calls 実行 → 再 chat） |
| `src-tauri/src/message.rs` | 正規形メッセージ・`Field`（設定項目） |
| `src-tauri/src/providers/{mod,local,ollama,openai}.rs` | 叩く AI コネクタ（既定 local・trait + inventory 登録） |
| `scripts/fetch-engine.mjs` | 同梱エンジン取得（`npm run setup:engine`） |
| `scripts/fetch-whisper.mjs` | 同梱Whisper CLIとモデル取得（`npm run setup:whisper`） |
| `scripts/sync-characters.mjs` | ルート `characters/`（正本）を `ui/assets/characters/` へ同期（dev/build 前に自動実行） |
| `scripts/build-ui-dist.mjs` | `ui/` を Tauri 配信用 `ui-dist/` へコピー。製品ビルドでは HTML/CSS のコメント・余分な空白と JS の空行・行頭行末空白を削る |
| `scripts/build-character-animations.py` | `characters/<id>/layers/` から連番アニメーションを生成し、`character.json` の `animations` / `assetVersion` を更新する |
| `src-tauri/src/tools/mod.rs` | スキル（ツール）の `Tool` トレイト・レジストリ本体 |
| `skills/<スキル名>/mod.rs` | エージェント機能（スキル）コネクタ（trait 実装 + inventory 登録。ディレクトリを足すと自動登録）。`chitchat` は会話中にモデルへ提示しない内部スキル（`agent_callable=false`、アイドル雑談用） |
| `characters/` | キャラクター正本（一覧 `index.json`・人格定義・レイヤー・連番フレーム・ローディング画像）。画像準備手順は `characters/README.md` |
| `src-tauri/build.rs` | コネクタの `#[path] pub mod` 自動生成（`src/providers` と `skills`）＋ tauri ビルド |
| `ui/picoagent.js` | 旧 `window.picoagent` を Tauri 上で再現する互換シム |
| `ui/index.html` / `renderer.js` / `styles.css` | ペット UI |
| `ui/settings.{html,js,css}` | 設定ウィンドウ（全設定の GUI 編集） |

## フォルダ構成

```text
20_PicoAgent/
├── characters/                 ← キャラクター正本（index.json・人格定義・連番フレーム）
├── skills/                     ← エージェント機能（スキル）連結ディレクトリ（<スキル名>/mod.rs を足すと自動登録）
│   └── {time,weather,currency_rates,...}/mod.rs
├── ui/                         ← webview フロント正本（編集対象）
│   ├── picoagent.js                 ← window.picoagent 互換シム（Tauri invoke/listen）
│   ├── index.html / renderer.js / styles.css       ← ペット UI
│   ├── settings.html / settings.js / settings.css  ← 設定ウィンドウ
│   └── assets/characters/      ← characters/ の配信コピー（git 管理外、sync:characters で生成）
├── ui-dist/                    ← Tauri frontendDist（git 管理外、build-ui-dist で生成）
├── src-tauri/
│   ├── Cargo.toml / build.rs / tauri.conf.json
│   ├── capabilities/default.json   ← 権限（ACL）
│   ├── icons/                      ← 生成アイコン一式
│   ├── resources/bundled/          ← 同梱物（engine/aivis/whisper。git 管理外、setup:* で取得）
│   └── src/
│       ├── main.rs / lib.rs / commands.rs / menu.rs
│       ├── config.rs / agent.rs / message.rs / engine.rs
│       ├── providers/{mod,local,ollama,openai}.rs ← 叩く AI（.rs を足すと自動登録）
│       └── tools/mod.rs        ← スキルの Tool トレイト・レジストリ本体
├── scripts/fetch-engine.mjs    ← 同梱エンジン取得
├── scripts/fetch-whisper.mjs   ← 同梱Whisper CLIとモデル取得
├── scripts/sync-characters.mjs ← characters/ を ui/assets/characters/ へ同期
├── scripts/build-ui-dist.mjs   ← ui/ を ui-dist/ へコピー（build は簡易 minify）
├── package.json                ← npm run setup:engine / setup:whisper / sync:characters / dev / build / tauri
├── AGENTS.md / CLAUDE.md       ← ルール正本 / リダイレクト
├── README.md / SPEC.md / CHANGELOG.md
```
