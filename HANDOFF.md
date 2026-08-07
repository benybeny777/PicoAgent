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

## 未検証（Windows / macOS の実機確認が必要）

Linux コンテナでは GUI を起動できないため、以下は Chromium で `ui/index.html` を描画した
CSS 検証までしか済んでいない。

- 表示形式 8 種の切り替え（右クリック →「表示形式」）とウィンドウ寸法の追従
- 立ち絵のドラッグ移動（`Esc` で取りやめ／`Ctrl`+ドラッグでウィンドウ移動／
  右クリック →「立ち絵の位置を戻す」）
- **Live2D キャラの表示。ここが一番危ない。** `img` ではなく PIXI が描く `canvas` なので
  経路が違い、アイコン化する形式（chat / talk / bar）の顔の切り出しと、
  `#characterView` に `scale` が効いた状態での描画が未確認

ルールの正本は [AGENTS.md](AGENTS.md)。この文書には進行状況だけを記録する。

