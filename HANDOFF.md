# 引き継ぎメモ

- 最終更新: 2026-08-07
- ブランチ: `main`
- 目的: Live2D表示、起動画面、フキダシとウィンドウ位置の仕上げ

## 現在の作業

ひよりの口ArtMesh消失を、SDK 5用Core 6（Cubism 5.3形式）とSDK 4互換Core 5の使い分けで修正した。キャラクター設定には自動／Cubism 4／Cubism 5の世代選択を追加した。通常表示とVTuber形式の起動画面は上半身表示へ揃え、フキダシ拡大、画面内クランプ、キャラ別の既定声も反映済み。

## 残る確認境界

- Windows実機で、利用者の普段の音声に対する口パクの見た目を最終確認する
- macOS実機でウィンドウ位置、音声再生、Live2D描画を確認する

## 検証

```powershell
cargo xtask setup live2d
cargo xtask character-pack validate characters\mugi
cargo test --manifest-path xtask/Cargo.toml        # 15件成功
cargo check --manifest-path crates/picoagent-core/Cargo.toml
cargo test --manifest-path crates/picoagent-core/Cargo.toml # 5件成功
cargo check --manifest-path src-tauri/Cargo.toml --tests
cargo test --lib --manifest-path src-tauri/Cargo.toml       # 283件成功
node tools/screenshots/shoot.mjs live2d-light.webp live2d-dark.webp
git diff --check
```

`mugi-live2d` は制作途中でモデル実体をまだ持たず、別パックへの `..` 参照も残るため、単体パック検証の対象外。正式パック11体は検証済み。

## 視覚検証済み

- 麦のCubism 5／4モデルを別ページで固定読込
- ひよりの閉口／開口を同一ポーズの口元画素差で比較し、README画像を再生成
- 麦とひよりの起動画面を撮影し、左側の上半身表示と右側ログ、画面内収まりを確認
- chat / talk / bar のLive2Dアイコンは285×315の固定描画をCSSで切り出す

ルールの正本は [AGENTS.md](AGENTS.md)。この文書には進行状況だけを記録する。
