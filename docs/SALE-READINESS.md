# 販売準備チェックリスト

この文書は販売可否の正本です。価格案は [SALES.md](SALES.md)、技術仕様は [SPEC.md](../SPEC.md) を参照します。法務・税務の最終判断を代替するものではありません。

## 出荷の入口

```powershell
cargo xtask release-check
cargo xtask preview-distribution
cargo xtask build-distribution
```

`release-check` は現在地を一覧表示します。`preview-distribution` は法務・署名の完了前でも販売対象キャラだけのUIを生成し、内容を確認できます。`build-distribution` は未完了項目が1件でもあれば、同梱物取得やビルドを始める前に停止します。開発確認用の `cargo xtask build` は従来どおり全開発キャラを扱います。

販売設定の正本は `distribution/release.json` です。販売対象OSとキャラ、利用条件とプライバシー表示の確認、Live2D審査参照、公開URL、更新署名、OSコード署名の準備状態をここへ記録します。Windows先行販売なら `platforms` をWindowsだけにできます。秘密鍵や証明書は記録しません。

## 必須ゲート

- [ ] EULAを事業者情報・販売条件・返金条件に合わせ、専門家確認後に `eulaReviewed=true`
- [ ] プライバシー表示を実際の販売サイト・外部連携一覧に合わせ、専門家確認後に `privacyReviewed=true`
- [ ] Live2D拡張性アプリケーションの審査と契約を完了し、追跡可能な社内参照番号を登録
- [ ] サポート、プライバシー表示、利用条件の公開URLを登録
- [ ] Tauri updaterの公開鍵と更新先を登録し、秘密鍵を別媒体へバックアップ
- [ ] Windowsコード署名、macOS署名・公証を実機で確認
- [ ] `THIRD-PARTY-NOTICES.md` を実際の同梱物と突合
- [ ] Windows／macOSのクリーン環境でインストール、初回モデル取得、更新、アンインストールを確認
- [ ] 販売ページに事業者情報、価格、動作環境、外部通信、返金・サポート範囲を表示

Live2D公式は、不特定多数のモデルを追加・組み合わせられる作品を拡張性アプリケーションに含め、一般ユーザー・小規模事業者を含めてリリース前の審査と特別契約を求めています。PicoAgentはキャラ追加を提供するため、契約確認を出荷ゲートから外しません。

Tauri updaterは更新ファイルの署名検証を必須としており、秘密鍵を失うと既存利用者へ同じ更新経路で配布できなくなります。公開鍵だけを設定へ入れ、秘密鍵はリポジトリ・ログ・配布物へ含めません。

## 販売用キャラ境界

初期設定は自作PNGキャラの `mugi` / `rin` / `mai` だけです。開発用Live2Dサンプル、制作途中の `mugi-live2d`、外部モデル由来の試作は販売版へ自動では入りません。追加するときは権利根拠、素材の再配布条件、音声、人格文面、全表示形式のQAを確認してから `includedCharacters` へ明示します。

## 公式参照

- [Live2D SDKリリースライセンス](https://www.live2d.com/sdk/license/)
- [Live2D 拡張性アプリケーション](https://www.live2d.com/sdk/license/expandable/)
- [Tauri 配布とコード署名](https://v2.tauri.app/distribute/)
- [Tauri Updaterと更新署名](https://v2.tauri.app/plugin/updater/)
