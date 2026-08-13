---
name: build-picoagent-beta
description: PicoAgentの開発中ベータ版をWindows NSISとして軽量ビルドし、不要な開発用同梱物や旧インストーラを混ぜず、ハッシュ付き更新情報、Distribution README、動画、GitHub Releaseまで検証・公開する依頼で使う。
---

# PicoAgentベータ版をビルドする

## 事前確認

1. `AGENTS.md`と`HANDOFF.md`を読む。
2. `git status --short`で既存変更を把握し、巻き戻さない。
3. `distribution/release.json`の版、対象OS、許可キャラを確認する。
4. WindowsホストではWindows成果物だけを作る。macOS成果物を推測・改名・流用しない。

## 混入を防ぐ

- 公開ベータ版は必ず`cargo xtask build-beta --bundles nsis`を正規入口にする。
- `build-beta`が`beta-full` featureを自動付与し、製品情報へベータ全機能試用中と表示することを確認する。正式版コマンドへこのfeatureを付けない。
- `build-beta`だけが`BETA`バッジを配信物へ残す。生成後の`ui-dist/index.html`に`beta-badge`があること、通常`build`では無いことを確認する。
- `cargo xtask build`はOffice、Aivis、Whisper、THA3、開発ツール等を扱う開発用経路なので、公開ベータ版へ使わない。
- `src-tauri/tauri.distribution.conf.json`の資産許可範囲が基本エンジンと`component-manifest.json`だけであることを確認する。
- `distribution/release.json`の許可キャラ以外、Live2Dサンプル、制作途中素材、過去のインストーラを含めない。
- ビルド開始時刻より古い成果物は失敗扱いにし、公開しない。

## ビルド

1. 長時間実行は`temp/`へ標準出力・標準エラー・PIDを保存し、隠しウィンドウで起動する。
2. `cargo xtask build-beta --bundles nsis`を実行する。
3. 親プロセスの終了コードとログ末尾を回収する。`makensis`だけが終了しても成功扱いにしない。
4. NSISの`error mmapping datablock`が出た場合は、通常ビルドの巨大な同梱範囲を使っていないか確認する。圧縮方式を場当たり的に変えたり、古い成果物へ戻したりしない。

## 成果物を検証する

1. `src-tauri/target/release/bundle/nsis/PicoAgent_<版>_x64-setup.exe`の更新時刻、サイズ、PE先頭`MZ`、SHA-256を確認する。
2. `cargo xtask update-feed <NSIS> <説明>`で`distribution/site/latest.json`を生成する。
   - ハッシュ用バッファを1MiBのスタック配列へ戻さない。Windowsでは`update-feed`開始時にスタックオーバーフローするため、ヒープ確保を維持する。
3. `latest.json`のURL、サイズ、SHA-256が実ファイルと完全一致することを再計算して確認する。
4. `cargo test --manifest-path xtask/Cargo.toml`、変更範囲に応じたRustテスト、全JSの`node --check`、`git diff --check`を通す。
5. UIや動画を変えた場合は`verify-picoagent-ui`も使い、実WebView成果物を更新する。
   - キャラ一覧動画は変則的な小型形式を使わず、実利用と同じ標準ペット表示と配信表示を各キャラ480px以上で撮る。10fps以下へ戻さず、16fps・8.4秒・134コマ以上にする。短時間に感情や視線を連続巡回させず、会話→考え中→成功→待機の3切替を上限にし、まばたきと口パクを自然に見せる。
   - Live2Dを紹介するときはひよりだけに固定せず、取得済みの全公式サンプルを同条件で撮影する。中間フレームを目視し、画面外・極端な拡大縮小・透明化があれば`character.json`の正本を直して再同期する。

## Distributionへ公開する

1. `distribution/site/`のREADME、LP、`latest.json`、規約類、`assets/`を配布リポジトリへ同じ版として同期する。
2. READMEへ開発中ベータ、対応OS、未署名警告、インストール、基本操作、問題報告先を明記する。
3. 本体リポジトリをコミット・pushしてから、配布リポジトリをコミット・pushする。
4. 同じ版のGitHub Releaseをプレリリースとして作り、検証済みNSISだけを添付する。既存Releaseを更新する場合も、資産のサイズとSHA-256を再照合する。
5. 公開Release、ダウンロードURL、Pages、動画表示を外部から確認する。

## 記録

- `docs/REVIEW-LOG.md`へ対応、見送り、実機確認境界、成果物のサイズとSHA-256を残す。
- `README.md`、`SPEC.md`、`CHANGELOG.md`、`HANDOFF.md`を同じ作業で同期する。
- インストーラを実際に起動する試験は、既存アプリや利用中画面を妨げない条件を確認する。実行できなければ、未確認境界として明記する。
