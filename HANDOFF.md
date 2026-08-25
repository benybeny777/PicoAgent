# 引き継ぎ

この文書は、現在進行中の作業だけを次の担当へ渡すための短い入口です。完了した作業の経緯はPR、恒久ルールは[AGENTS.md](AGENTS.md)、現行仕様は[SPEC.md](SPEC.md)、未着手・未確認項目は[docs/TASKS.md](docs/TASKS.md)を参照してください。

## 現在の状態

- 最終確認日: 2026-08-26
- 進行中作業: macOS版アルファの公開。本体 PR #179（macOSのシークレット保護・mac配布物の更新フィード対応）と、配布Repo `PicoAgent-Distribution` PR #31（取得導線）が未マージ。
- 次に着手できる内容: [docs/TASKS.md](docs/TASKS.md)のうち、着手条件を満たした項目

### macOS版アルファの再開手順

Windows環境ではmacOSバンドルを作れないため、mac実機での作業待ちで止まっている。

1. mac実機で本体 PR #179 のブランチをチェックアウトし、`cargo xtask build-beta --bundles app,dmg` を実行する。手順と前提は [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) の「macOS版のビルド（未署名アルファ）」。
2. [docs/TASKS.md](docs/TASKS.md)の「macOS版の実機QA」を消化する。Apple SiliconとIntelは別バイナリなので機種ごとに確認する。
3. Windows 10/11 で v1.0.3 のNSISを作る。配布Repo PR #31 はWindows版もv1.0.3にする前提で書いてある。
4. `v1.0.3` Releaseへ両方の成果物を置き、`cargo xtask update-feed <各成果物>` で `latest.json` を生成して配布Repoへ取り込む。
5. 本体 PR #179 → 配布Repo PR #31 の順にマージし、Pagesを公開する。

**PR #179 のmacOS実行時挙動は未確認。** 型検査だけ `aarch64-apple-darwin` 向けに通してある。

## 再開時の順序

1. `AGENTS.md`を読む。
2. `git status --short --branch`、`git log -5 --oneline`、開いているPRを確認する。
3. 進行中PRがあれば、そのPR本文と差分から再開する。
4. 進行中作業がなければ`docs/TASKS.md`から、着手条件を満たす項目だけを選ぶ。

## 更新ルール

- 未マージの作業、再開に必要な環境、直後に行うことだけを書く。
- 完了した作業の説明、テスト結果、過去のPR一覧は残さない。PRへ記録する。
- マージ後に進行中項目を削除し、何もなければ「進行中作業なし」とする。
