# 引き継ぎ

恒久ルールは[AGENTS.md](AGENTS.md)、未完了の実機受入・素材品質・将来開発は[docs/TASKS.md](docs/TASKS.md)、改修設計と検証結果は各PRを正本とする。

## 現在の状態

- 最終確認日: 2026-10-07
- 未マージ（利用者のマージ判断待ち）: PR #431「会話の文字サイズ変更とタイトルバーのシーン吸着・フロー表示」（`feat/chat-font-titlebar`、Draft）。本文に改修設計・検証結果・未確認事項（Windows実機の目視）を書いてある。続きは本文の「未完了の作業」から再開する。閉じた #422（`fix/ui-mode-visual-polish`）の差分を現行mainへ移植する場合は、#431 と同じ `ui/components/*.css` と `tools/screenshots/` を触るため、後から入れる方で `main` を取り込み、`node --test tools/tests/*.mjs` と `tools/screenshots/ui-check.mjs` を再実行する。
- PR #389〜#394 は利用者のマージ指示を受けて順に `main` へ取り込んだ。配信の既定構図・チャット・タイトルバー、全キャラ確認画面、配置編集の境界、ダウンロード済みMMD・Live2Dモデルの登録候補表示を含む。統合後の `cargo xtask verify` は128件成功、`cargo audit` は脆弱性0件（既存警告7件）。Windows WebView2の残る目視項目は `docs/TASKS.md` を参照する。
- 配信MMDの構図は描画器で修正し、#387 の描画方式別の初期配置補正は撤去した。利用者の `PICOAGENT_SCENES` に保存済みのさくらみこ711px配置は移行・上書き・削除していない。新しい胸上構図で位置が合わない場合は、右クリック「キャラを自動配置へ戻す」でそのキャラ・形式だけ既定配置へ戻せる。
- 利用者の許可を受けGemma 4 E2B画像資産（約0.99GB）を導入済み。共通の `local-vision` へ配置し、通常チャットのローカル画像認識を実機確認した。画像入力設定は有効。再取得は不要。
- 既存セーブを読み込み店内画面の取得と無入力停止を確認済み。利用者へHP・満腹度が見えるダンジョン画面の準備を依頼中。次は構造化認識の拒否経路とダンジョンで最大1操作を検証する。ゲーム自動操作自体は許可取得済みとの申告あり。PicoAgentの認識→1操作→結果確認は未検証。受入項目は `docs/TASKS.md` を参照する。
- 音声の最終聴感・画像動画の実機受入残件は`docs/TASKS.md`を参照する。採用済み音声モデル・比較素材・既存の`output/`は変更しない。モデルを再取得せず、HFトークンを表示しない。
- 配布・リリースは今回のマージと別作業とする。

## 保護している未統合作業

- `codex/dev-data-isolation`: PR整理の依頼によりDraft PR #411を閉じた。保存先分離の設計と枝は保持し、実装は未着手。worktreeは`C:/Users/benyb/.codex/worktrees/dev-data-isolation/20_PicoAgent`。再開時は閉じたPRの設計を確認する。
- `fix/ui-mode-visual-polish`: PR整理の依頼によりDraft PR #422を閉じた。未マージのUI修正はリモート枝に保持している。再開時は現行mainへ必要な差分だけ移植して再検証する。
- `codex/isolate-managed-runtime-data`と`codex/krea-ltx25-lowvram`: 対応PRのマージ後にも枝の先端が異なり、未統合の差分がある。worktreeはそれぞれ`temp/news-summary-reliability`と`temp/krea-ltx25-lowvram`。古い枝全体を再マージせず、必要な差分だけ現行mainへ移植する。
- `codex/pr-322-plan`: 元worktreeのパスがなく、未統合の差分があるため枝を保護している。再開前に内容を確認する。
- ホロ関連の正本は別リポジトリ`C:/00_PG/PicoAgent-holo`。旧`temp/holo-explanation-video`はdetached HEADで、未コミットの文書・スライド・生成スクリプトが残る。ファイルは削除していないため、必要なら移行先との差分を確認して再開する。企画改訂案は移行先のPR #1と`codex/holo-proposal-preserved`に保存済み。

## 再開時の順序

1. `AGENTS.md`と`git status --short --branch`を確認する。
2. 開いているPRの本文、差分、実行中プロセスを照合する。
3. 未完了項目と検証境界を引き継ぎ、既存成果物を保護する。

## 更新ルール

- 完了した作業の経緯はPRへ残し、この文書には未マージ作業と再開方法だけを書く。
- マージ後は該当する進行中項目を削除する。
