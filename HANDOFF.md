# 引き継ぎ

恒久ルールは[AGENTS.md](AGENTS.md)、未完了の実機受入・素材品質・将来開発は[docs/TASKS.md](docs/TASKS.md)、改修設計と検証結果は各PRを正本とする。

## 現在の状態

- 最終確認日: 2026-09-27
- **UI全面再設計（PR #382、ブランチ `claude/sweet-johnson-yvmlsl`）は設計確定。実装は Codex がこのブランチの続きから行う。** 設計の正本は PR #382 の本文（「過去の修正履歴から導いた設計要件」の表A・B・C が受入条件）と、コミット済みの `SPEC.md` 画面の節・`docs/UI-CONTRACT.md`・`docs/SETTINGS.md`。前提の3形式化（`pet` 既定・`mascot`＝ミニ・`stream`）は #383 でマージ済み。
  - **済（段階1＋段階2のRust側）**: 仕様3文書。`src-tauri/src/scene.rs`（`PICOAGENT_SCENES` の正規化・保存）、`pet_save_scene`、`pet_init` の `scenes` / `uiTheme`、`menu.rs` の項目定義表と文脈付き `pet_menu`・`pet:context-action`、`ui_contract.rs`（UI契約と登録 command の双方向照合）。旧 `layout_placements.rs` / `character_framing.rs` と `pet_set_bubble_size` / `pet_set_bubble_layout` / `pet_set_character_offset` / `pet_save_layout_placements` は削除済み。`cargo test --lib` 642件成功。
  - **このブランチは途中状態**: バックエンドは新方式、フロント（`ui/`）は旧いまま。旧 `ui/picoagent.js` は削除済み command を呼ぶので、このままでは配置編集・フキダシ寸法の保存が動かない。`node --test tools/tests/*.mjs` は旧フロント前提の2件（`ai-games-contract` の「遊ぶ」メニュー、`layout-editor-contract` の保存先）が落ちる。フロントを差し替えるまでマージしない。
  - **次にやること（Codex）**: PR本文「段階（Codex 向け）」の段階2のフロント側から。`ui/app`・`ui/character`・`ui/sources`・`ui/chat`・`ui/components`・`ui/themes`・`ui/scenes` を作り、旧 `renderer.js` / `styles.css` / `layout-editor.js` / `top-menu.js` / `game.js` / `vendor/moveable` を消し、旧ファイルを読む契約テスト（`tools/tests/*.mjs` 29本、`src-tauri/src/doc_sync.rs`・`bundle_scope.rs` の一部）を新ファイルへ付け替える。
  - **参考の試作（使うかは Codex が判断）**: Claude が設計段階で先走って作ったフロント一式を、マージしない参照用ブランチ `claude/ui-rebuild-prototype`（このブランチが土台）に置いた。ヘッドレス検査 `tools/screenshots/ui-check.mjs` と JS テスト282件は成功、Rust側の付け替え（`doc_sync.rs` 等）・`xtask build-ui` の `import` 版付与・`settings.js` の見直し・文書更新・WebView2 実機確認は未了。採用・部分利用・破棄のどれでもよい。採用する場合も表A・B・C で検査し直すこと。
- 音声の最終聴感・画像動画の実機受入残件は`docs/TASKS.md`を参照する。採用済み音声モデル・比較素材・既存の`output/`は変更しない。モデルを再取得せず、HFトークンを表示しない。
- 配布・リリースは今回のマージと別作業とする。

## 再開時の順序

1. `AGENTS.md`と`git status --short --branch`を確認する。
2. 開いているPRの本文、差分、実行中プロセスを照合する。
3. 未完了項目と検証境界を引き継ぎ、既存成果物を保護する。

## 更新ルール

- 完了した作業の経緯はPRへ残し、この文書には未マージ作業と再開方法だけを書く。
- マージ後は該当する進行中項目を削除する。
