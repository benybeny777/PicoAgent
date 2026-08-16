---
name: review-picoagent-change
description: mainへ入ったPicoAgentの変更をCodexまたはClaudeが事後レビューし、記録と未レビュー管理まで残す。未レビューのコミットを片付けたい、直近の修正に退行やデータ喪失が無いか確かめたい、レビュー台帳を更新したい依頼で使う。
---

# PicoAgentの事後レビュー

`main` へ入った変更は、指示が無くても Codex と Claude の**両方**が見る。片方だけだと書いた本人と同じ見落としが残る（実際に「起動を止めない」修正が、読めなかった会話履歴を次の1往復で上書きする状態のまま `main` にあった）。

## 最初に読む

1. `AGENTS.md`（ルール正本）と `HANDOFF.md`（現状）。
2. `docs/REVIEW-LOG.md` の直近エントリ。同じ指摘を二度出さないため。
3. `git status --short` で既存の未コミット変更を確認し、巻き戻さない。

## 対象を決める

```bash
cargo xtask review-status          # 未レビューのコミット（既定は origin/main）
cargo xtask review-status --strict # 1件でもあれば失敗（ゲート用）
```

古いコミットから順に片付ける。1回のレビューで複数コミットをまとめて見てよいが、記録には全部の SHA を書く。

**さかのぼるのは直近2日分まで。** それより前（`docs/REVIEW-LOG.md` の「レビュー必須の起点」より前）は当時の記録で足りているものとして扱い、掘り返さない。起点より後は日付にかかわらず全件が対象。

## 読む順番（安いものから）

1. `git show --stat <SHA>` で範囲を掴み、画像・`character.json`・生成物は対象外にする。
2. `git show <SHA> -- <コード>` を Rust → フロント → xtask/tools の順で読む。
3. 事実確認は `grep` と対象ファイルの部分読みで済ませる。GUI 起動・フルビルド・撮影は最後の手段。

## 必ず見る観点

- **状態ファイルの喪失**: 「読めない・壊れている → 空で継続」の後に、その空データを書き戻していないか。設定・会話履歴・記憶・使用量・関係性・実行履歴が対象。退避（`store::quarantine_corrupt()`）かエラー返却のどちらかを通っていること。
- **無言の縮退**: 失敗を握り潰していないか。フォールバックには理由コメントがあるか。利用者に見えるべき失敗がログだけになっていないか。
- **退行**: 直前の仕様（`CHANGELOG.md` の直近項目）と矛盾する変更が混じっていないか。消えた CSS 規則・分岐に、まだ必要なものが無いか。
- **到達不能コード**: 呼び出し元が消えて残った引数・分岐・関数。
- **資源解放**: 子プロセス、タイマー、リスナ、`URL.createObjectURL`、外部ライブラリのインスタンス。
- **検査の実効性**: 追加されたテスト・ガードが、対象を消したときに落ちるか（期待値を実装側の観測から取っていないか）。
- **ドキュメント同期**: 変更した挙動が `SPEC.md` / `README.md` / `AGENTS.md` の記述と食い違っていないか。設定キーは `doc_sync::guard` が見るが、文章のずれは見ない。

## 直す

- ルール違反・退行・データ喪失は、その回で直す。都度確認を求めない。
- 直したら回帰テストか設計ガードを足す。**同じ穴が二度目に開いたら、ガードの側を疑う。**
- 直さないものは「見送り」「保留」として理由を書く。判断を保留したまま黙って落とさない。

## 検証（この順で最小回数）

```bash
cargo test --lib --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path xtask/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml --check
git ls-files 'ui/*.js' 'tools/**/*.mjs' | ForEach-Object { node --check $_ }   # PowerShell
node --test tools/tests/<対象>.mjs
cargo run --manifest-path xtask/Cargo.toml -- build-ui --release
```

GUI・音声・実機依存の確認は無理に自動化せず、`HANDOFF.md` の「残る確認境界」へ書いて委ねる。

## 記録する（省略しない）

`docs/REVIEW-LOG.md` の**先頭**へ新しいエントリを足す。見出しの直下に機械可読の1行を置く。

```markdown
## <日付> — <対象>の事後レビュー

対象コミット: d371577, 91ea586 ｜ レビュアー: claude
```

- `レビュアー:` は `codex` か `claude`。もう一方のレビューは別エントリにする（同じコミットを両者が見て初めて完了）。
- 指摘が0件でも「見て0件だった」ことをエントリとして残す。残さないと未レビュー扱いのままになる。
- 表は `# / 優先 / 指摘 / 判定 / 対応・根拠`。判定は `対応済み` / `見送り` / `保留`。
- 恒久ルールになったものは `AGENTS.md`、仕様は `SPEC.md`、進行状況は `HANDOFF.md` へ移す。

最後に `cargo xtask review-status` をもう一度実行し、対象が消えたことを確認してからコミット・プッシュする。
