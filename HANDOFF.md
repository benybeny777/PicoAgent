# 引き継ぎ

この文書は、現在進行中の作業だけを次の担当へ渡すための短い入口です。完了した作業の経緯はPR、恒久ルールは[AGENTS.md](AGENTS.md)、現行仕様は[SPEC.md](SPEC.md)、未着手・未確認項目は[docs/TASKS.md](docs/TASKS.md)を参照してください。

## 現在の状態

- 最終確認日: 2026-09-12
- MMDキャラ実装: PR #273、`codex/mmd-character-design`。#270上に積み、Aivisと剛体物理を除外して登録・描画・自然姿勢・表情・口パクを追加。次は#270のmain統合後にPRのマージ先をmainへ付け替え、実機確認を進める。#270やその作業ブランチへ本作業からマージしない。作業場所は`temp/mmd-character-design`。ローカル人格は同worktreeの`temp/mmd-demo-personas.json`、2体はアプリの追加キャラクター領域へ登録済み。比較画面は同worktreeの`temp/mmd-app-check/compare.html`。検証結果と未確認事項はPR本文を参照する。
- UI操作・設定保存の信頼性改善: `codex/ui-ux-reliability` の修正用PR #270。差分保存、キー単位の競合検出、スケジュール状態分離、入力草稿、添付付き再試行まで実装済み。最新main（PR #271）との競合を解消し、`cargo xtask verify` は全54項目、Rust lib 535件、UI 132件が合格。次はWindows実機受入。条件は `docs/TASKS.md`。利用者の依頼はPR作成までで、マージしない。
- 未完了の素材補修・実機受入・設計のみの将来開発は[docs/TASKS.md](docs/TASKS.md)を参照し、着手条件を満たす項目から進める。
- 配布版の更新は本体のマージと別に扱い、配布リポジトリの公開状態を確認する。

## 再開時の順序

1. `AGENTS.md`を読む。
2. `git status --short --branch`、`git log -5 --oneline`、開いているPRを確認する。
3. 進行中PRがあれば、そのPR本文と差分から再開する。
4. 進行中作業がなければ`docs/TASKS.md`から、着手条件を満たす項目だけを選ぶ。

## 更新ルール

- 未マージの作業、再開に必要な環境、直後に行うことだけを書く。
- 完了した作業の説明、テスト結果、過去のPR一覧は残さない。PRへ記録する。
- マージ後に進行中項目を削除し、何もなければ「進行中作業なし」とする。
