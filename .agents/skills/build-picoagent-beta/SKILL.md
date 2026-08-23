---
name: build-picoagent-beta
description: PicoAgentの開発中アルファ版をWindows NSISまたはUbuntu Linuxのdeb・AppImageとして軽量ビルドし、不要な開発用同梱物や旧成果物を混ぜず検証・公開する依頼で使う。
---

# PicoAgentアルファ版をビルドする

## 事前確認

1. `AGENTS.md`と`HANDOFF.md`を読む。
2. `git status --short`で既存変更を把握し、巻き戻さない。
3. `origin/main`、作業ブランチ、`distribution/release.json`の版・対象OS・許可キャラ、既存GitHub Release、配布リポジトリの`main`、Pages状態を取得し、今回の公開対象を確定する。
4. 配布リポジトリ`C:\00_PG\21_PicoAgent-LP`の`main`を、全追跡ファイル、Pages、Releaseを含む公開状態の正本として確認する。20側の`distribution/site/`は同期元にせず、差分があっても21側を上書きしない。
5. このスキルのパス、コマンド、公開先、版、成果物名が現行実装とずれていたら、同じPRでスキルを先に更新する。
6. ホストOSに対応する成果物だけを作る。WindowsではNSIS、Ubuntu Linux x64ではdebとAppImageを作り、別OSの成果物を推測・改名・流用しない。

## 混入を防ぐ

- 公開アルファ版は必ず`cargo xtask build-beta --bundles <形式>`を正規入口にする。Windowsは`nsis`、Ubuntu Linux x64は`deb,appimage`を指定する。
- `build-beta`が`beta-full` featureを自動付与し、製品情報へアルファ全機能試用中と表示することを確認する。正式版コマンドへこのfeatureを付けない。
- `build-beta`だけが`ALPHA`バッジを配信物へ残す。生成後の`ui-dist/index.html`に`beta-badge`があること、通常`build`では無いことを確認する。
- `cargo xtask build`も軽量構成だがALPHA表示と全機能試用featureが付かないため、公開アルファ版へ使わない。Office、Aivis、Whisper、THA3、開発ツールまで暗黙取得する`build-full`は配布へ使わない。
- `src-tauri/tauri.distribution.conf.json`の資産許可範囲が基本エンジンと`component-manifest.json`だけであることを確認する。
- `distribution/release.json`の許可キャラ以外、Live2Dサンプル、制作途中素材、過去のインストーラを含めない。
- ビルド開始時刻より古い成果物は失敗扱いにし、公開しない。
- ビルド後にコード、UI、資産、設定を変更した場合は、そのNSISを破棄扱いにして必ず再ビルドする。変更前の成果物へ更新フィードを向けない。

## 版を決める

1. 公開中の`latest.json`と公開Releaseの版が一致することを確認する。
2. 通常の修正、依存・モデル・同梱物更新、文書・LP、既存アルファ再配布は同じ`major.minor`系列の`patch`とする。
3. `minor`と`major`はユーザーが版を上げるよう明示した場合だけ選ぶ。機能内容から独断で上げない。
5. `cargo xtask bump-version <major|minor|patch>`を1回だけ実行する。アプリCargo、xtask Cargo、Tauri設定を個別に編集しない。
6. 公開済みと同じ版のRelease資産を上書きしない。採番後の版で新しいプレリリースを作る。
7. 判定した種別と理由を`docs/REVIEW-LOG.md`へ記録する。
8. `patch`版もLP、アプリ、ファイル名、Releaseへ正確に表示する。起動時の自動確認ではpatchを通知せず、利用者が設定から手動確認した場合だけpatchも案内・ダウンロードできるようにする。`major.minor`が上がった場合は起動時にも通知する。
9. アルファ期間中は製品表示、LP、更新説明、Release名へ`Alpha`を付ける。比較用SemVerとタグには接尾辞を混ぜず数値版を使う。

## ビルド

1. 長時間実行は`temp/`へ標準出力・標準エラー・PIDを保存し、隠しウィンドウで起動する。
2. Windowsでは`cargo xtask build-beta --bundles nsis`、Ubuntu Linux x64では`cargo xtask build-beta --bundles deb,appimage`を実行する。LinuxはREADMEのapt依存を先に揃え、AppImageの`bundleMediaFramework`を無効化しない。
3. 親プロセスの終了コードとログ末尾を回収する。`makensis`だけが終了しても成功扱いにしない。
4. NSISの`error mmapping datablock`が出た場合は、通常ビルドの巨大な同梱範囲を使っていないか確認する。圧縮方式を場当たり的に変えたり、古い成果物へ戻したりしない。

## 成果物を検証する

1. Windowsは`src-tauri/target/release/bundle/nsis/PicoAgent_<版>_x64-setup.exe`の更新時刻、サイズ、PE先頭`MZ`、SHA-256を確認する。Linuxは`bundle/deb/*.deb`と`bundle/appimage/*.AppImage`の更新時刻、形式、SHA-256を確認し、隔離環境でdebの導入・起動・削除とAppImageの起動を行う。
2. Windows公開時だけ`cargo xtask update-feed <NSIS> <説明>`で検証用の`distribution/site/latest.json`を生成する。このファイルは20側の正本ではなく、実ファイルとの照合後に21側へ取り込むための生成物として扱う。Linux更新情報は専用生成経路が実装されるまで空欄を維持し、Windows用PE検証を流用しない。
   - ハッシュ用バッファを1MiBのスタック配列へ戻さない。Windowsでは`update-feed`開始時にスタックオーバーフローするため、ヒープ確保を維持する。
3. `latest.json`のURL、サイズ、SHA-256が実ファイルと完全一致することを再計算して確認する。
4. `cargo test --manifest-path xtask/Cargo.toml`、変更範囲に応じたRustテスト、全JSの`node --check`、`git diff --check`を通す。
5. UIや動画を変えた場合は`verify-picoagent-ui`も使い、実WebView成果物を更新する。
   - キャラ一覧動画は変則的な小型形式を使わず、実利用と同じ標準ペット表示と配信表示で撮る。README用は各キャラ12秒・960px・30fps、LP用は`SHOOT_SITE_GALLERY=1`で同じ撮影経路の生PNGから6秒・480px・16fpsへ直接書き出す（既存960pxの再エンコードは禁止）。通常版は2倍密度描画、配信版はネイティブ1920px画面から縮小し、撮影ツールの引き伸ばし検査とLP容量検査を通す。LPは音声口パクを十分見せてから考え中→待機へ戻し、短時間に多数の状態を巡回させない。
   - Live2Dを紹介するときはひよりだけに固定せず、取得済みの全公式サンプルを同条件で撮影する。中間フレームを目視し、画面外・極端な拡大縮小・透明化があれば`character.json`の正本を直して再同期する。

## Distributionへ公開する

1. LP、README、規約、キャラ構成、現行UI画像、`latest.json`、紹介PowerPointなど配布リポジトリに属する変更は、依頼を本体側で受けた場合も`C:\00_PG\21_PicoAgent-LP`へ移って実施する。21側の目的別PRをsquash mergeし、20側のコピーを先に直さない。
2. 20側から21側へ取り込むのは、正規入口で生成・検証したインストーラと、その実体から生成した更新情報など明示した成果物だけにする。`distribution/site/`一式の同期や、20側に残る古いLP・PPT・資産による上書きを行わない。
3. READMEへ開発中アルファ、対応OS、未署名警告、インストール、基本操作、問題報告先を明記する。
4. 配布リポジトリも目的別PRでsquash mergeし、`main`への直接pushをしない。
5. 同じ版のGitHub Releaseをプレリリースとして作り、検証済みNSISだけを添付する。既存Releaseを更新する場合も、資産を差し替えた後に外部から再取得してサイズとSHA-256を照合する。
6. Pagesの最新ビルドが配布リポジトリのマージコミットを`built`としていることを確認する。公開LPをPC幅とスマホ幅で開き、CTA、現行UI、キャラの見切れ、旧機能の残存、動画表示を目視する。
7. 公開`latest.json`、公開Release、外部再取得したNSISがURL・サイズ・SHA-256で完全一致し、公開Pagesの内容が21側のマージ後`main`と一致するまで完了扱いにしない。20側`distribution/site/`との全ファイル一致は完了条件にしない。

## 記録

- `docs/REVIEW-LOG.md`へ対応、見送り、実機確認境界、成果物のサイズとSHA-256を残す。
- `README.md`、`SPEC.md`、`CHANGELOG.md`、`HANDOFF.md`を同じ作業で同期する。
- インストーラを実際に起動する試験は、既存アプリや利用中画面を妨げない条件を確認する。実行できなければ、未確認境界として明記する。
