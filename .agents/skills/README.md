# 共通作業スキル

このフォルダは、Codex・Claude Codeなど複数の作業エージェントで共有する手順の正本です。

| スキル | 用途 |
|---|---|
| `rebuild-picoagent-character` | キャラクター追加、高精細化、モーション、単一世代、READMEプレビュー |
| `verify-picoagent-ui` | 実WebViewの画面崩れ、レイヤー、ドラッグ、レイアウト、PNG／Live2D QA |
| `prepare-picoagent-release` | 配布前のコード、資産、文書、監査、ローカルテスト、Git確認 |

Claude Codeが自動認識する`.claude/skills/<名前>/SKILL.md`には、同名の共通スキルを読む指示だけを置きます。手順を追加・修正するときは、このフォルダの正本だけを編集してください。
