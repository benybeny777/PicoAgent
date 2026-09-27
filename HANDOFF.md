# 引き継ぎ

恒久ルールは[AGENTS.md](AGENTS.md)、未完了の実機受入・素材品質・将来開発は[docs/TASKS.md](docs/TASKS.md)、改修設計と検証結果は各PRを正本とする。

## 現在の状態

- 最終確認日: 2026-09-27
- **UI全面再設計（PR #382、ブランチ `claude/sweet-johnson-yvmlsl`）は設計確定。実装は Codex がこのブランチの続きから行う。** 設計の正本は PR #382 の本文（「過去の修正履歴から導いた設計要件」の表A・B・C が受入条件）と、コミット済みの `SPEC.md` 画面の節・`docs/UI-CONTRACT.md`・`docs/SETTINGS.md`。前提の3形式化（`pet` 既定・`mascot`＝ミニ・`stream`）は #383 でマージ済み。
  - **済（段階1＋段階2のRust側）**: 仕様3文書。`src-tauri/src/scene.rs`（`PICOAGENT_SCENES` の正規化・保存）、`pet_save_scene`、`pet_init` の `scenes` / `uiTheme`、`menu.rs` の項目定義表と文脈付き `pet_menu`・`pet:context-action`、`ui_contract.rs`（UI契約と登録 command の双方向照合）。旧 `layout_placements.rs` / `character_framing.rs` と `pet_set_bubble_size` / `pet_set_bubble_layout` / `pet_set_character_offset` / `pet_save_layout_placements` は削除済み。`cargo test --lib` 642件成功。
  - **試作を採用**: PR #384 のフロント一式をこのブランチへ取り込んだ。残件は PR #382 の本文を参照する。
- PR #380 のゲーム操作スキルはマージ済み。ゲームの実機受入は未確認で、条件は `docs/TASKS.md` を参照する。
- 利用者の許可を受けGemma 4 E2B画像資産（約0.99GB）を導入済み。共通の `local-vision` へ配置し、通常チャットのローカル画像認識を実機確認した。画像入力設定は有効。再取得は不要。
- 既存セーブを読み込み店内画面の取得と無入力停止を確認済み。利用者へHP・満腹度が見えるダンジョン画面の準備を依頼中。次は構造化認識の拒否経路とダンジョンで最大1操作を検証する。ゲーム自動操作自体は許可取得済みとの申告あり。PicoAgentの認識→1操作→結果確認は未検証。受入項目は `docs/TASKS.md` を参照する。
- 音声の最終聴感・画像動画の実機受入残件は`docs/TASKS.md`を参照する。採用済み音声モデル・比較素材・既存の`output/`は変更しない。モデルを再取得せず、HFトークンを表示しない。
- 配布・リリースは今回のマージと別作業とする。

## 再開時の順序

1. `AGENTS.md`と`git status --short --branch`を確認する。
2. 開いているPRの本文、差分、実行中プロセスを照合する。
3. 未完了項目と検証境界を引き継ぎ、既存成果物を保護する。

## 更新ルール

- 完了した作業の経緯はPRへ残し、この文書には未マージ作業と再開方法だけを書く。
- マージ後は該当する進行中項目を削除する。
