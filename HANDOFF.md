# 引き継ぎメモ

- 最終更新: 2026-08-07
- ブランチ: `main`
- 目的: PicoAgentのエージェント中核を、将来OSS化できる内部crateへ段階的に分離する

## 現在の作業

`crates/picoagent-core` を追加し、正規形メッセージとUTF-8安全処理を最初の切り出し対象にした。PicoAgent本体は互換モジュール経由でこのcrateを利用するため、公開用のコピーを別管理していない。

設計方針と次の境界は [docs/oss-core-plan.md](docs/oss-core-plan.md) を参照する。

## 未決定

- OSSライセンス
- 別リポジトリへ分ける時期
- `Provider` / `Tool`から`Config`依存を外す公開API
- 最小CLIの公開範囲

## 検証

```powershell
cargo check --manifest-path crates/picoagent-core/Cargo.toml
cargo test --manifest-path crates/picoagent-core/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml --tests
cargo test --lib --manifest-path src-tauri/Cargo.toml
git diff --check
```

ルールの正本は [AGENTS.md](AGENTS.md)。この文書には進行状況だけを記録する。

