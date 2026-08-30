# 共通作業スキル

このフォルダは、Codex・Claude Codeなど複数の作業エージェントで共有する手順の正本です。

| スキル | 用途 |
|---|---|
| `add-picoagent-connector` | プロバイダ／ツールコネクタの追加・変更 |
| `add-picoagent-setting` | 設定項目の追加・変更（永続化、画面、秘密値、文書） |
| `audit-picoagent-dependencies` | パッケージ、同梱物、外部CLI、モデル、ランタイムの依存監査 |
| `build-picoagent-beta` | 開発中アルファ版のビルド・検証・公開 |
| `rebuild-picoagent-character` | キャラクター追加、高精細化、モーション、単一世代、READMEプレビュー |
| `verify-picoagent-ui` | 実WebViewの画面崩れ、レイヤー、ドラッグ、レイアウト、PNG／Live2D QA |
| `prepare-picoagent-release` | 配布前のコード、資産、文書、監査、ローカルテスト、Git確認 |
| `update-picoagent-contest-presentation` | 社内AIコンテスト用PowerPointの構成、訴求、プレビュー、公開条件の更新 |

Claude Codeが自動認識する`.claude/skills/<名前>/SKILL.md`には、同名の共通スキル正本を読む指示だけを置きます。実際の手順を追加・修正するときは、このフォルダの正本だけを編集してください。名前一致、参照先、frontmatter、自動認識ファイル本文の肥大化は`cargo xtask verify`が検査します。
