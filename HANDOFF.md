# 引き継ぎメモ

作業を別のエージェント（Codex 等）や別セッションへ引き継ぐための現状記録です。
**ルールの正本は [AGENTS.md](AGENTS.md)。まずそちらを読んでください。** この文書は「いま何がどこまで終わっていて、次に何をすべきか」だけを書きます。

- 最終更新: 2026-08-07
- 作業ブランチ: `claude/app-improvement-1irrg5`（`origin` へプッシュ済み・`main` 未マージ）

---

## 1. まず動かす（Linux コンテナ / Claude Code on the web）

`clone` 直後は `cargo check` すら通りません。セッション開始フックが環境を作ります。

```bash
CLAUDE_CODE_REMOTE=true ./.claude/hooks/session-start.sh   # 手動で流すとき
cargo test --lib --manifest-path src-tauri/Cargo.toml       # 281 passed で正常
```

フックがやること（詳細は [`.claude/hooks/session-start.sh`](.claude/hooks/session-start.sh) の冒頭コメント）:

1. Tauri v2 の Linux 前提パッケージ ＋ `libasound2-dev`（cpal）＋ `libxdo-dev`（enigo）を apt で導入
2. `cargo xtask sync-characters` → `cargo xtask build-ui` で `ui-dist/` を生成
   （`tauri.conf.json` の `frontendDist` が指すこのディレクトリは Git 管理外。無いと `tauri::generate_context!` がコンパイル時に落ちる）
3. 依存取得とテストバイナリの事前ビルド（`SKIP_PREBUILD=1` で省略可）

**Windows / macOS のローカル環境では `CLAUDE_CODE_REMOTE` を見て即 exit します**（何もしません）。

### 検証コマンド

| やること | コマンド |
|---|---|
| コンパイル確認 | `cargo check --manifest-path src-tauri/Cargo.toml --all-targets` |
| テスト（設計ガード含む） | `cargo test --lib --manifest-path src-tauri/Cargo.toml` |
| 整形 | `cargo fmt --manifest-path src-tauri/Cargo.toml --check` |
| Lint | `cargo clippy --manifest-path src-tauri/Cargo.toml --lib --all-targets` |
| JS 構文 | `node --check ui/renderer.js`（`settings.js` / `picoagent.js` も） |

`clippy` は `tha3_creator.rs:411` の `too_many_arguments` 1 件だけ警告が出ます。**これは以前からあるもので、今回の変更由来ではありません。**

---

## 2. このブランチで入れたもの（4 コミット）

| コミット | 内容 |
|---|---|
| `171eb31` | **グローバルホットキー**。他アプリの裏からでも呼び出す／隠す（`src-tauri/src/hotkey.rs`） |
| `7b23de0` | **セッション開始フック**。Linux コンテナのビルド環境を自動構築 |
| `1d2bff7` `d49dc6f` | **表示形式の設計資料**（[docs/layout-proposal.md](docs/layout-proposal.md)） |
| `0a5f606` | **表示形式 8 種類の実装**。右クリックから切り替え（`src-tauri/src/layout.rs`） |

### 触ったファイルと役割

| ファイル | 役割 |
|---|---|
| `src-tauri/src/layout.rs` | 表示形式の定義・選択・ウィンドウ寸法の当て込み。**新規** |
| `src-tauri/src/hotkey.rs` | グローバルホットキーの登録／解除。**新規** |
| `src-tauri/src/menu.rs` | 右クリックメニュー。「表示形式」サブメニューと位置リセットを追加 |
| `src-tauri/src/lib.rs` | 起動時の `layout::apply` / `hotkey::apply`、終了時の `hotkey::release`、`pull_back_on_screen` |
| `src-tauri/src/commands.rs` | `pet_init` / `settings_save` が `layout` を返す・即反映する |
| `ui/styles.css` | `[data-layout="…"]` による 8 形式の配置（ファイル末尾に集約） |
| `ui/renderer.js` | `applyLayout()`、ウィジェット時計、履歴の常設判定 |
| `.claude/hooks/session-start.sh` | Linux コンテナのビルド環境構築 |

---

## 3. 未着手・確認が必要なこと

### 実機確認が済んでいない（最優先）

Linux では GUI が起動できないため、**Chromium で `ui/index.html` を各形式のウィンドウ寸法で描画して CSS を検証**しました。次は Windows / macOS で実際に動かす確認が要ります。

- [ ] 右クリック →「表示形式」の 8 項目が出て、選択中にチェックが付くか
- [ ] 形式を切り替えたときウィンドウ寸法が変わり、フロントの配置も追従するか
- [ ] 「表示位置とサイズをリセット」が、選択中の形式の既定へ戻るか
- [ ] グローバルホットキー（`Ctrl+Alt+P` / `Ctrl+Alt+H`）が他アプリの前面から効くか
- [ ] Live2D / THA3 キャラで、アイコン化する形式（chat / talk / bar）が破綻しないか
      **`.character canvas` を使う Live2D は `img` と経路が違うため、ここが一番危ないです**

CSS 検証に使ったハーネスの作り方は「[6. 画面まわりを直すとき](#6-画面まわりを直すとき)」に書きました。

### 意図的に実装していないもの

設計資料のモックアップと実装で違う点です。資料の冒頭にも明記済み。

- **トークの既読マーク** — 送信済み／既読を判定する仕組みがアプリ側に無いため未実装
- **ウィジェットの予定リスト** — 時計は実装。予定は `pet_daily_events` を専用 UI に出す代わりに、アクションセンター（通知・定期実行の結果）を常設する形にした

### 決まっていないこと（ユーザーへ要確認）

1. **キャラクターを消せる形式を作るか** — 作らない前提で実装済み。「ただのチャットアプリ」になるため非推奨と伝えてある
2. **8 形式すべて残すか** — 全部実装済み。減らす判断があれば `layout.rs` の `LAYOUTS` から削るだけ（設定値が消えた形式を指していても `pet` へ丸まる）

---

## 4. 落とし穴（先に知っておくと事故らない）

- **設定キーを足したら `SPEC.md` に書く。** `doc_sync::guard` のテストが双方向で照合し、書き忘れ・消し忘れをその場で落とします。`SPEC.md` 本文に `PICOAGENT_WINDOW_*` のような**ワイルドカード表記を書くとキー名として拾われて落ちます**（実キー名で書く）。
- **`PICOAGENT_` はアプリの設定キーの名前空間。** シェルスクリプトの環境変数などに流用しないこと（`session-start.sh` は `SKIP_PREBUILD` を使っています）。
- **表示形式ごとのウィンドウ寸法は `PICOAGENT_WINDOW_LAYOUTS` に持つ。** `PICOAGENT_WINDOW_WIDTH_PX` などの 1 組だけでは、形式を切り替えるたび配置が壊れます。
- **`ui-dist/` と `ui/assets/characters/` は Git 管理外の生成物。** UI を直したら `cargo xtask build-ui` を流さないと、ビルドしたアプリに反映されません。
- **Linux は対応対象外**（AGENTS.md）。ビルドは通す、機能の動作保証はしない。Linux 専用の分岐を新たに増やさないこと。
- **クレジットを無駄にしない**（AGENTS.md）。`cargo check` → 必要なら `cargo test --lib` の順。GUI 起動とフルビルドは最後の手段。

---

## 5. ドキュメントの持ち場

| 文書 | 何を書くか |
|---|---|
| [AGENTS.md](AGENTS.md) | **ルール正本。** 作業前に必読 |
| [SPEC.md](SPEC.md) | アーキテクチャ・コネクタ仕様・**設定キー一覧**（ガードが照合する） |
| [README.md](README.md) | 起動手順・機能の全体像・主要な設定キー |
| [MANUAL.md](MANUAL.md) | 使いかた（エンドユーザー向け） |
| [CHANGELOG.md](CHANGELOG.md) | 運用・品質に影響する大きめの変更 |
| [docs/layout-proposal.md](docs/layout-proposal.md) | 表示形式の設計意図とモックアップ（**仕様の正本ではない**） |

---

## 6. 画面まわりを直すとき

GUI を起動せずに CSS を検証する手順です。実際にこれで配置バグを 3 件見つけました。

1. Chromium（`/opt/pw-browsers/chromium-1194/chrome-linux/chrome`）を Playwright で起動
2. `ui/index.html` へ `goto` してから、`<script>` を除いた body を `setContent` で流し込む
   （`goto` を先に済ませないと相対パスの `styles.css` が解決されません）
3. `#petApp` の `is-booting` を外し、`data-layout` を切り替えて代表的な DOM 状態を手で作る
4. 各形式のウィンドウ寸法を viewport にしてスクリーンショットと**横スクロールの有無**を確認

`renderer.js` は読み込まないので、**CSS の検証だけで JS の挙動は見ていません**。JS を変えたときは実機確認が要ります。

見つけた 3 件は、いずれも読むだけでは気づけないものでした。

- `.character` の `display:grid` + `place-items:end` で、子の `height:100%` が「行の内容高さ」に対して解決され立ち絵が枠を突き抜ける
- 一括指定の詳細度（`:not()` 3 連 = 0,3,1）に個別指定（0,2,1）が負けて要素が消える
- 背景を `z-index` の高い要素の中に置いて、その下の要素が塗りつぶされる
