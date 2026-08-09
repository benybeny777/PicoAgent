# 引き継ぎメモ

- 最終更新: 2026-08-08
- ブランチ: `main`
- 目的: 全表示形式、設定画面、右クリック操作、Live2D表示と事後レビューの仕上げ

## 現在の作業

ひよりの口ArtMesh消失を、SDK 5用Core 6（Cubism 5.3形式）とSDK 4互換Core 5の使い分けで修正した。キャラクター設定には自動／Cubism 4／Cubism 5の世代選択を追加した。通常表示とVTuber形式の起動画面は上半身表示へ揃え、フキダシ拡大、画面内クランプ、キャラ別の既定声も反映済み。2026-08-08に全8形式を実用寸法・文字付き操作・独立入力欄へ揃え、設定画面を8カテゴリへ整理し、右クリックの危険なリセット操作をサブメニューへ移した。基本設定には4つの目的別入口、検索候補・解除、未保存件数、キーボード保存を追加し、レイアウトカードが初期表示から見えるよう大型プレビューを縮めた。さらにキャラ×表示形式の構図、配信背景と画面内ディスプレイ、LIVE・日付・時刻、左右画面配置、状態付き読み上げスイッチを追加した。読み上げはユーザー指示があるまでローカル設定で無効のまま扱う。

Claudeの事後レビューで、復元寸法の下限ずれ、起動中680×430の誤記録、履歴ロック不一致、起動中の形式適用、キャラ別挨拶、配信キャンバス寸法とアセット版の二重管理も修正した。2026-08-08にはClaudeの追加提案を現行mainで再検証してCodex案と統合し、既存の会話コンテキスト圧縮を要約1ブロック置換・system込み予算・localの実コンテキスト長追従へ直した。PNG系4キャラは制作正本を分離し、Photoshop「ディテールを保持 2.0」による高精細化、元アルファ再適用、4背景QA、全件成功後だけ反映する工程へ更新した。42コマは `framePaths` で共有し、待機即時＋120ms後に残りを読む速度優先プリロードにした。制作正本208配置も71個の内容ハッシュblobへ共有化し、Photoshop工程が自動展開する。

販売用ビルドは `cargo xtask build-distribution` へ分離した。軽量配布設定は本体・ローカルAIエンジン・Live2D表示と追加機能マニフェストだけを入れ、Aivis／Whisper／THA3／Office／文書ツールは初回ウィザードまたは設定の「追加機能」からPicoAgent専用領域へ取得する。パッケージは `cargo xtask package-components` でOS別に作り、サイズ・SHA-256・zip slip・展開上限を検証して原子的に導入する。未公開の間は `components.packagesPublished=false` が販売ビルドを止める。初期販売はコード署名・macOS公証・自動更新なしで、`signing.required=false` / `updates.required=false` を正本とし、OS警告と手動更新を販売ページで案内する。通常の`build`を含む製品ビルドの既定キャラは自作PNGの `mugi` / `rin` / `mai` のみで、開発用Live2Dサンプルは同期しない。Live2Dサンプルは利用者が公式配布元等から取得したzip／フォルダを設定画面へ入れ、アプリが `.model3.json` と参照を検査して `character.json` をユーザー領域へ作成する。初回セットアップは用途→追加機能→Live2D案内の三段階で、「使い方」から保存済みデータを消さず再表示できる。詳細は [docs/SALE-READINESS.md](docs/SALE-READINESS.md)。

統合ロードマップ13項目は実装済み。構図保存の検証・個別初期化、キャラ別配信表示、全Live2Dと操作回帰、冷間計測、Windows/macOS中心CI、スキル意図メタデータ、全Tool schemaガード、HTTP・フロントログ共通層、clippy警告ゼロ、CHANGELOG年別化を追加した。指摘と保留境界は [docs/REVIEW-LOG.md](docs/REVIEW-LOG.md) に記録している。

## 残る確認境界

自動撮影できるPNG系4キャラ×8形式は確認済み。以下は実機または人の判断が要る境界。詳細と判断理由は `docs/REVIEW-LOG.md` を参照する。

- Windows実機で、8方向リサイズ・端寄せ位置の復元・起動画面から保存形式への復帰を目視確認する
- 各キャラの挨拶返答（「こんにちは」「おはよう」「hi」）の文面が自然か確認する
- 配信形式の立ち絵サイズが `calc()` 化で 0.5px 変わる点を実機で確認する
- macOS実機でウィンドウ位置、音声再生、Live2D描画を確認する（前回からの持ち越し）

## 検証

```powershell
cargo test --lib --manifest-path src-tauri/Cargo.toml   # 306件
cargo test --manifest-path xtask/Cargo.toml             # 26件成功
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
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
- PNG系4キャラの計208枚を72画像へ重複除外し、Photoshop「ディテールを保持 2.0」で通常立ち絵を2倍化。起動画像は元解像度を維持。白／濃紺／マゼンタ／市松の一覧と自動境界検査は警告0件
- `SHOOT_CHARACTER_MATRIX=1` でPNG系4キャラ×8形式を撮影。平均1,214ms、全32画面で描画を確認し、一覧で見つけたマスコット／ペットの顔だけ表示も修正済み。成果物は `temp/character-layout-matrix/`
- 同じ32画面でmanifest・待機・全件プリロード時刻を記録し、今回平均1,077ms・基準超過警告0件
- `SHOOT_INTERACTION_MATRIX=1` で麦／ひより×8形式の起動遷移・入力欄・縮小後の収まりを16件確認
- `SHOOT_LIVE2D_MATRIX=1` で導入済み8モデル×8形式=64件のSDK・口パラメータ・描画を確認

ルールの正本は [AGENTS.md](AGENTS.md)、レビューの判断履歴は [docs/REVIEW-LOG.md](docs/REVIEW-LOG.md)。この文書には進行状況だけを記録する。
