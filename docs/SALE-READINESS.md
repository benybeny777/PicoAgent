# 販売準備チェックリスト

この文書は販売可否の正本です。価格案は [SALES.md](SALES.md)、技術仕様は [SPEC.md](../SPEC.md) を参照します。法務・税務の最終判断を代替するものではありません。

## 出荷の入口

```powershell
cargo xtask release-check
cargo xtask preview-distribution
cargo xtask build-beta --bundles nsis
cargo xtask build-distribution
```

`release-check` は現在地を一覧表示します。`preview-distribution` は法務確認の完了前でも販売対象キャラだけのUIを生成し、内容を確認できます。`build-beta` は販売条件を完了扱いにせず、正式配布と同じ軽量同梱範囲で試用インストーラを生成します。`build-distribution` は必須項目が1件でも残っていれば、同梱物取得やビルドを始める前に停止します。`cargo xtask dev` は全開発キャラを扱いますが、製品成果物を作る通常の `cargo xtask build` も配布許可リストだけを使います。

販売設定の正本は `distribution/release.json` です。販売対象OSとキャラ、利用条件・プライバシー表示・第三者同梱物台帳の確認、Live2D審査参照、公開URLをここへ記録します。初期販売は `signing.required=false` / `updates.required=false` とし、署名・公証・自動更新を必須ゲートにしません。Windows先行販売なら `platforms` をWindowsだけにできます。秘密鍵や証明書は記録しません。

## 必須ゲート

- [ ] EULAを事業者情報・販売条件・返金条件に合わせ、専門家確認後に `eulaReviewed=true`
- [ ] プライバシー表示を実際の販売サイト・外部連携一覧に合わせ、専門家確認後に `privacyReviewed=true`
- [ ] `src-tauri/resources/bundled/` と `ui/vendor/` の実ファイルを台帳へ落とし、再配布・帰属・ソース提供条件を確認後に `thirdPartyInventoryReviewed=true`
- [ ] `component-manifest.json` の全取得元が公式または作者配布であることを確認し、Aivis／Whisperの実取得と外部導入リンクをクリーンWindowsで確認
- [ ] 公開配布Gitの `latest.json` とWindowsインストーラReleaseを同じ版へ更新し、URL・サイズ・SHA-256を実成果物と照合
- [ ] Live2D拡張性アプリケーションの審査と契約を完了し、追跡可能な社内参照番号を登録
- [ ] サポート、プライバシー表示、利用条件の公開URLを登録
- [ ] `THIRD-PARTY-NOTICES.md` を実際の同梱物と突合
- [ ] 販売対象OS（現在はWindows）のクリーン環境で未署名警告、インストール、初回モデル取得、手動更新、アンインストールを確認
- [ ] 販売ページに事業者情報、価格、動作環境、外部通信、返金・サポート範囲を表示
- [ ] 販売対象OSの未署名警告（現在はWindows SmartScreen）と、正規ダウンロード元・手動更新手順を販売ページに表示

`distribution/site/index.html` に販売用LPの初稿を用意し、機能、動作環境、外部通信、未署名警告、正規ダウンロード元、更新方法、サポート導線を表示した。確定していないPro価格、事業者情報、返金範囲は仮置きせず、販売開始前に確定情報を反映するまで上のチェックは未完了のままとする。キャラ例は完成済み2DのむぎとLive2D動作例のひよりだけを使う。ひよりにはLive2D公式指定の著作権表示と利用条件へのリンクを付ける。

2026-08-10の公開状態確認では `PicoAgent-Distribution` のReleaseが0件で、PagesもLP更新前の内容だった。取得ボタンは誤案内を避けて「準備中」に戻し、公開RepoのIssuesを無料版サポート窓口として案内した。実インストーラ、更新フィード、LP一式を公開Repoへ同時に同期し、Pages上で再確認するまで販売・取得開始とは表示しない。

Live2D公式は、不特定多数のモデルを追加・組み合わせられる作品を拡張性アプリケーションに含め、一般ユーザー・小規模事業者を含めてリリース前の審査と特別契約を求めています。PicoAgentはキャラ追加を提供するため、契約確認を出荷ゲートから外しません。

Windows版を公開するときは、NSISを作成後に `cargo xtask update-feed <インストーラ.exe> "更新内容"` を実行します。生成された `distribution/site/latest.json` と同じインストーラを `PicoAgent-Distribution` の `v<version>` Releaseへ置き、Pages反映後に設定画面から更新確認・ダウンロード・SHA-256検証・インストーラ起動を実機確認します。

初期版のアプリ内更新は利用者が設定から明示実行し、公開配布GitのURL・サイズ・SHA-256を検証してNSISを起動します。オフライン時は見送ります。将来、完全自動更新へ進める場合はTauri updaterの更新署名を必須にし、`updates.required=true`、公開鍵、更新成果物生成を揃えます。秘密鍵はリポジトリ・ログ・配布物へ含めません。コード署名を導入する場合も `signing.required=true` に変えてWindows署名とmacOS署名／公証を販売ゲートへ戻します。

## 販売用キャラ境界

初期設定は自作PNGキャラの `mugi` / `rin` / `mai` / `koharu` / `sena` / `noa` だけです。後半3体も17キーポーズ・42フレームの通常2Dパックで、まばたき・口パク・呼吸・左右揺れ・思考・会話・落胆を同梱PNGだけで再生します。開発用Live2Dサンプルと外部モデル由来の試作は販売版へ自動では入りません。追加するときは権利根拠、素材の再配布条件、音声、人格文面、全表示形式のQAを確認してから `includedCharacters` へ明示します。

Live2D公式サンプルはアプリから自動取得しません。利用者が公式配布ページ等で条件を確認して自分で取得し、設定画面へzip／フォルダを入れます。これはモデル素材の再配布を避けるための境界であり、Cubism Coreの出版条件と拡張性アプリ審査は別途必要です。

## 公式参照

- [Live2D SDKリリースライセンス](https://www.live2d.com/sdk/license/)
- [Live2D 拡張性アプリケーション](https://www.live2d.com/sdk/license/expandable/)
- [Tauri 配布とコード署名](https://v2.tauri.app/distribute/)
- [Tauri Updaterと更新署名](https://v2.tauri.app/plugin/updater/)
