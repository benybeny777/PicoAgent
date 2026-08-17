---
name: prepare-picoagent-release
description: PicoAgentの配布・販売・リリース前に、実装、文書、キャラ資産、ローカルテスト、依存監査、配布設定、Git差分をまとめて検証する。リリースしてよいか、販売前監査、配布物を作る、mainをきれいにしてpushする依頼で使う。
---

# PicoAgent配布前検証

## 最初に読む

1. `AGENTS.md`、`HANDOFF.md`、`distribution/release.json`を読む。
2. `git status --short`、現在ブランチ、直近コミット、`origin`との差を確認する。
3. 既存変更を利用者または別エージェントの作業として扱い、巻き戻さない。

## 検証する

1. `cargo check --manifest-path src-tauri/Cargo.toml`を行う。
2. `cargo test --manifest-path src-tauri/Cargo.toml --lib`と`cargo test --manifest-path xtask/Cargo.toml`を行う。
3. キャラ変更があれば`cargo xtask sync-characters`と該当する実WebView QAを行う。
4. `Cargo.toml`または`Cargo.lock`を変更した場合だけ、対象lockfileへ`cargo audit --file <Cargo.lock>`を実行する。
5. 販売前はリポジトリ直下の`deny.toml`を使い、`cargo deny --manifest-path src-tauri/Cargo.toml check`と`cargo deny --manifest-path xtask/Cargo.toml check`で既知脆弱性・ライセンス・取得元を検査する。`cargo-deny`が無ければ`cargo install cargo-deny --version 0.20.2 --locked`で導入し、実行時に公式安定版と照合して古ければスキルと版を同じPRで更新する。
6. `cargo xtask build`または依頼で指定された配布検査を行い、`distribution/release.json`のWindows先行・署名・更新方針と照合する。
7. `git diff --check`、文書同期ガード、配布対象と販売資料の整合を確認する。

## 判定する

- 実行していない確認を合格と書かない。
- 警告、保留、環境由来の未確認は`docs/REVIEW-LOG.md`へ理由と再確認方法を残す。
- 起動中アプリのファイルロックで通常ビルド先を使えない場合、利用者のアプリを勝手に止めず、`temp/`配下の隔離`--target-dir`で再検証する。
- 変更と検証が完了したら`AGENTS.md`のルールどおり全差分をコミットし、`origin`へpushする。PRのマージは利用者の指示がある場合だけ行う。
