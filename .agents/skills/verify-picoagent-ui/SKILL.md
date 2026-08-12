---
name: verify-picoagent-ui
description: PicoAgentの画面崩れ、ドラッグ、最大化、キャラ位置、レイヤー順、設定反映、PNGまたはLive2Dの見た目を実WebViewで再現・検証する。コード上は直っているのに画面がおかしい、モニターを邪魔せず確認したい、README画像と実画面を一致させたい依頼で使う。
---

# PicoAgent実画面QA

## 最初に読む

1. `AGENTS.md` と `HANDOFF.md` を読む。
2. `tools/screenshots/README.md` を最後まで読む。
3. 最新のモニター利用指示を確認する。可視GUIを使わずに済む検査はヘッドレスで行う。
4. `git status --short` で既存変更を確認し、巻き戻さない。

## 調査する

- `ui/`だけでなく、`cargo xtask build-ui`後の`ui-dist/`とTauri commandの実行経路を追う。
- CSS、DOM、配信アセット、キャッシュ版、ウィンドウ寸法を別々に測り、見た目だけから原因を決めない。
- キャラ画像は正本`characters/`と配信コピー`ui/assets/characters/`の寸法・内容版・参照世代を比較する。
- レイヤー問題は背景、画面内ディスプレイ、キャラ、字幕、操作UIの積層コンテキストまで確認する。

## 検証する

- PNGは`SHOOT_CHARACTER_MATRIX=1`、操作は`SHOOT_INTERACTION_MATRIX=1`、Live2Dは`SHOOT_LIVE2D_MATRIX=1`を使い、必要ならIDで対象を絞る。
- 代表レイアウトだけで済ませず、変更に関係する8形式を確認する。
- スクリーンショットを実際に開き、キャラの鮮明さ、切れ、左右位置、前後関係、操作部品を目視する。
- 一時画像・結果は`temp/`だけへ置き、成果物のREADME WebPだけを`docs/images/`へ出す。

## 完了条件

原因、修正、機械検査、目視結果を`docs/REVIEW-LOG.md`へ記録する。仕様変更は`SPEC.md`、利用手順変更は`README.md`、大きな変更は`CHANGELOG.md`も同期し、`git diff --check`を通してコミット・pushする。
