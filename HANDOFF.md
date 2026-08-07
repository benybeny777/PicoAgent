# 引き継ぎメモ

- 最終更新: 2026-08-08
- ブランチ: `main`
- 目的: 全表示形式、設定画面、右クリック操作、Live2D表示と事後レビューの仕上げ

## 現在の作業

ひよりの口ArtMesh消失を、SDK 5用Core 6（Cubism 5.3形式）とSDK 4互換Core 5の使い分けで修正した。キャラクター設定には自動／Cubism 4／Cubism 5の世代選択を追加した。通常表示とVTuber形式の起動画面は上半身表示へ揃え、フキダシ拡大、画面内クランプ、キャラ別の既定声も反映済み。2026-08-08に全8形式を実用寸法・文字付き操作・独立入力欄へ揃え、設定画面を8カテゴリへ整理し、右クリックの危険なリセット操作をサブメニューへ移した。読み上げはユーザー指示があるまでローカル設定で無効のまま扱う。

Claudeの事後レビューで、復元寸法の下限ずれ、起動中680×430の誤記録、履歴ロック不一致、起動中の形式適用、キャラ別挨拶、配信キャンバス寸法とアセット版の二重管理も修正した。指摘と保留項目は [docs/REVIEW-LOG.md](docs/REVIEW-LOG.md) に記録している。

## 残る確認境界

いずれも GUI 起動が要るため自動検証できない。詳細と判断理由は `docs/REVIEW-LOG.md` の #10〜#12。

- Windows実機で、8方向リサイズ・端寄せ位置の復元・起動画面から保存形式への復帰を目視確認する
- 各キャラの挨拶返答（「こんにちは」「おはよう」「hi」）の文面が自然か確認する
- 配信形式の立ち絵サイズが `calc()` 化で 0.5px 変わる点を実機で確認する
- macOS実機でウィンドウ位置、音声再生、Live2D描画を確認する（前回からの持ち越し）

## 検証

```powershell
cargo test --lib --manifest-path src-tauri/Cargo.toml   # 292件成功
cargo test --manifest-path xtask/Cargo.toml             # 18件成功
cargo run --manifest-path xtask/Cargo.toml -- build-ui  # ?v= の自動付与を確認
git diff --check
```

`mugi-live2d` は制作途中でモデル実体をまだ持たず、別パックへの `..` 参照も残るため、単体パック検証の対象外。正式パック11体は検証済み。

## 視覚検証済み

- 麦のCubism 5／4モデルを別ページで固定読込
- ひよりの閉口／開口を同一ポーズの口元画素差で比較し、README画像を再生成
- 麦とひよりの起動画面を撮影し、左側の上半身表示と右側ログ、画面内収まりを確認
- chat / talk / bar のLive2Dアイコンは285×315の固定描画をCSSで切り出す
- 8形式の実フロント描画を `tools/screenshots/shoot.mjs layout-shot-*.webp` で再生成し、設定カード8件の選択回帰も `settings-display.mjs` で確認する

ルールの正本は [AGENTS.md](AGENTS.md)、レビューの判断履歴は [docs/REVIEW-LOG.md](docs/REVIEW-LOG.md)。この文書には進行状況だけを記録する。
