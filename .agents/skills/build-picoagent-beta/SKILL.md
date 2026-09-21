---
name: build-picoagent-beta
description: PicoAgentの開発中アルファ版をWindows NSIS、macOSの.app・.dmg、Ubuntu Linuxのdeb・AppImageとして軽量ビルドし、不要な開発用同梱物や旧成果物を混ぜず検証・公開する依頼で使う。
---

# PicoAgentアルファ版をビルドする

## 事前確認

1. `AGENTS.md`と`HANDOFF.md`を読む。
2. `git status --short`で既存変更を把握し、巻き戻さない。
3. `origin/main`、作業ブランチ、`distribution/release.json`の版・対象OS・許可キャラ、既存GitHub Release、配布リポジトリの`main`、Pages状態を取得し、今回の公開対象を確定する。
4. 配布リポジトリ`C:\00_PG\21_PicoAgent-LP`の`main`を、全追跡ファイル、Pages、Releaseを含む公開状態の正本として確認する。20側の`distribution/site/`は同期元にせず、差分があっても21側を上書きしない。
5. このスキルのパス、コマンド、公開先、版、成果物名が現行実装とずれていたら、同じPRでスキルを先に更新する。
6. ホストOSに対応する成果物だけを作る。WindowsではNSIS、macOSでは`.app`と`.dmg`、Ubuntu Linux x64ではdebとAppImageを作り、別OSの成果物を推測・改名・流用しない。macOSバンドルはmacOS実機でだけ作れる。クロスビルドは行わない。**配布対象はApple Siliconだけで、Intel Macは対象外**（理由と解除条件は`docs/TASKS.md`の「macOS Intel版」）。Intel機でビルドした成果物を配らない。

## 混入を防ぐ

- 公開アルファ版は必ず`cargo xtask build-beta --bundles <形式>`を正規入口にする。Windowsは`nsis`、macOSは`app,dmg`、Ubuntu Linux x64は`deb,appimage`を指定する。
- `build-beta`が`beta-full` featureを自動付与し、製品情報へアルファ全機能試用中と表示することを確認する。正式版コマンドへこのfeatureを付けない。
- `build-beta`だけが`ALPHA`バッジを配信物へ残す。生成後の`ui-dist/index.html`に`beta-badge`があること、通常`build`では無いことを確認する。
- `cargo xtask build`も軽量構成だがALPHA表示と全機能試用featureが付かないため、公開アルファ版へ使わない。Office、Aivis、Whisper、THA3、開発ツールまで暗黙取得する`build-full`は配布へ使わない。
- `src-tauri/tauri.distribution.conf.json`の資産許可範囲が基本エンジンと`component-manifest.json`だけであることを確認する。
- `distribution/release.json`の許可キャラ以外、Live2Dサンプル、制作途中素材、過去のインストーラを含めない。
- ビルド開始時刻より古い成果物は失敗扱いにし、公開しない。
- ビルド後にコード、UI、資産、設定を変更した場合は、そのNSISを破棄扱いにして必ず再ビルドする。変更前の成果物へ更新フィードを向けない。

## 版を決める

1. 公開中の`latest.json`と公開Releaseの版が一致することを確認する。
2. **採番・ビルド・アップロードの前に、最新公開版のビルド元コミットと、現在公開しようとしているコミットの実効差分を確認する。** アプリ、UI、同梱資産、依存、配布設定に変更がなく、公開バイナリの内容が実質同じなら、その時点でアプリ公開を停止する。版を上げず、ビルドせず、Releaseを作らず、「同じ内容なので公開しなかった」と報告する。
3. README、LP、規約、画像など配布サイトだけの変更は、21側の目的別PRで更新し、同じアプリを再ビルド・再アップロードしない。アプリの更新フィードや版も変えない。
4. 事前比較では同一と断定できずビルドした場合も、公開中の成果物と形式・対象OS・SHA-256が同じならアップロード前に停止する。版変更だけで同一判定を回避しない。
5. **内容が変わったアプリを一般公開へアップロードする回ごとに、必ず版を上げる。** 利用者が`major`または`minor`を明示しない限り、公開中の版から`patch`を1つ上げる。同じ数値版で公開作業を繰り返さない。
6. 1回の公開作業で同時に用意・検証した複数OS成果物だけは、同じ新規版のReleaseへまとめてよい。その公開作業が完了した後の追加添付、再アップロード、差し替え、欠けたOS成果物の後日追加は、すべて次の`patch`版として新しいReleaseを作る。
7. 通常の修正、依存・モデル・同梱物更新、既存アルファ再配布は同じ`major.minor`系列の`patch`とする。
8. `minor`と`major`はユーザーが版を上げるよう明示した場合だけ選ぶ。機能内容から独断で上げない。
9. `cargo xtask bump-version <major|minor|patch>`を1回だけ実行する。アプリCargo、xtask Cargo、Tauri設定を個別に編集しない。
10. 公開済みと同じ版のRelease資産を上書き・追加・差し替えしない。採番後の版で新しいプレリリースを作る。
11. 実効差分の判定、採番種別、その理由をPR本文へ記録する。
12. `patch`版もLP、アプリ、ファイル名、Releaseへ正確に表示する。起動時の自動確認ではpatchを通知せず、利用者が設定から手動確認した場合だけpatchも案内・ダウンロードできるようにする。`major.minor`が上がった場合は起動時にも通知する。
13. アルファ期間中は製品表示、LP、更新説明、Release名へ`Alpha`を付ける。比較用SemVerとタグには接尾辞を混ぜず数値版を使う。

## ビルド

- Windowsのまとめ実行は`Windows全種類をビルドしてUP.bat`、個別は`Windows公開Alpha版をビルドしてUP.bat`／`Windows関係者版をビルドしてUP.bat`／`Windows審査版をビルドしてUP.bat`を使える。共通正本`scripts/publish-windows.ps1`は各既存ビルド入口を直列に呼び、MZ・時刻・SHAと再取得を検査する。最初は`-PrepareOnly`で新しい版・最新main・保存先のvisibility・直前版との実効差分を確かめる。不明なビルド元は確認して`-PreviousSourceCommit`で指定し、素材は`-PartnerSourceDir`で絶対パスも指定できる。
- 全種類はAlphaだけ公開先Draft、関係者・審査は非公開の新規`v<版>-windows-private`へ分離する。既存数値版へ追加せず、Release作成後の失敗でバッチを最初から再実行するときも新patchを使う。公開先Draftまでの場合は終了コード2であり公開完了としない。21側の目的別PR・更新情報・Pages検証後に公開を完了する。詳細は`docs/DEVELOPMENT.md`のWindows各版手順を使う。
- 途中のmain更新でRelease作成前に停止したら、新旧コミットの全変更パスを確認する。文書だけでビルド入力が同一なら、今回の実行ディレクトリへ退避済みの成果物・元コミット・時刻・SHA-256を照合し、未完成の種類だけ正規入口から実行できる。アプリ・UI・資産・設定が変わっていたら全種類を再ビルドする。バッチに自動再開機能はないため、手動で回収する場合も3種類の成功・公開範囲・新規Release・再取得SHA照合を省略せず、PRへ停止原因と再開元を記録する。
- 実効差分検査を変更したら`scripts/test-publish-windows.ps1`で、日本語の資産名、依存・モデルのversion更新、自パッケージ版だけの変更を区別できることを確認する。差分行全体からversionを消す方法へ戻さない。

1. 長時間実行も終了コードと結果を同じ作業内で回収する。利用者が非同期実行を禁止した場合は、追跡可能な実行セッションで完了まで待つ。GUIを起動する場合は最新のモニター指定を守り、検証が終わったら閉じる。
2. Windowsでは`cargo xtask build-beta --bundles nsis`、macOSでは`cargo xtask build-beta --bundles app,dmg`、Ubuntu Linux x64では`cargo xtask build-beta --bundles deb,appimage`を実行する。LinuxはREADMEのapt依存を先に揃え、AppImageの`bundleMediaFramework`を無効化しない。macOSはXcode Command Line Tools、Tauri CLI、`p7zip`（Aivisの分割書庫展開）を先に揃える。手順は`docs/DEVELOPMENT.md`の「macOS版のビルド（未署名アルファ）」。
   - **macOS向けのコンパイルはWindows環境で検証できていない。** 初回は自前コードの型エラーや同梱物取得のmac経路で止まりうる。止まったら原因を直して同じブランチへコミットし、PRを完成させる。失敗ログだけ残して終えない。
   - 直し方で機能を削らない。macOS分岐のスタブ化、`bundle.targets`の縮小、`Info.plist`の用途説明削除、同梱物の省略でビルドを通した場合は、何を落としたかをPR本文と`docs/TASKS.md`へ必ず残す。
3. 親プロセスの終了コードとログ末尾を回収する。`makensis`だけが終了しても成功扱いにしない。
4. NSISの`error mmapping datablock`が出た場合は、通常ビルドの巨大な同梱範囲を使っていないか確認する。圧縮方式を場当たり的に変えたり、古い成果物へ戻したりしない。

## 成果物を検証する

1. Windowsは`src-tauri/target/release/bundle/nsis/PicoAgent_<版>_x64-setup.exe`の更新時刻、サイズ、PE先頭`MZ`、SHA-256を確認する。macOSは`bundle/dmg/PicoAgent_<版>_aarch64.dmg`の更新時刻、サイズ、末尾512バイトのUDIFトレーラ`koly`、SHA-256を確認し、マウントして`.app`をアプリケーションフォルダへコピーし、初回だけ右クリック→「開く」でGatekeeperを越えて起動する。Linuxは`bundle/deb/*.deb`と`bundle/appimage/*.AppImage`の更新時刻、形式、SHA-256を確認し、隔離環境でdebの導入・起動・削除とAppImageの起動を行う。
2. WindowsとmacOSの公開時に`cargo xtask update-feed <成果物> <説明>`で検証用の`distribution/site/latest.json`を生成する。`.exe`はPE、`.dmg`はUDIFトレーラを確認し、ファイル名のCPU種別から`windows-x64`／`macos-arm64`／`macos-x64`の枠へ書く。同じ版なら他OSの枠を残したまま該当枠だけが差し替わるので、OSごとに1回ずつ実行する。版が上がった場合は前の版のURLを引き継がず枠が空になるため、その版の全OS分をあらためて登録する。このファイルは20側の正本ではなく、実ファイルとの照合後に21側へ取り込むための生成物として扱う。Linux更新情報は専用生成経路が実装されるまで空欄を維持し、他OSの検証を流用しない。
   - ハッシュ用バッファを1MiBのスタック配列へ戻さない。Windowsでは`update-feed`開始時にスタックオーバーフローするため、ヒープ確保を維持する。
3. `latest.json`のURL、サイズ、SHA-256が実ファイルと完全一致することを再計算して確認する。
4. `cargo test --manifest-path xtask/Cargo.toml`、変更範囲に応じたRustテスト、全JSの`node --check`、`git diff --check`を通す。
5. UIや動画を変えた場合は`verify-picoagent-ui`も使い、実WebView成果物を更新する。
   - キャラ一覧動画は変則的な小型形式を使わず、実利用と同じ標準ペット表示と配信表示で撮る。README用は`SHOOT_README_GALLERY=1`、LP用は`SHOOT_SITE_GALLERY=1`で同じ撮影経路の生PNGから6秒・960px・16fpsへ直接書き出す（既存動画の再エンコードは禁止）。通常版は2倍密度描画、配信版はネイティブ1920px画面から縮小し、撮影ツールの引き伸ばし検査とLP容量検査を通す。LPは音声口パクを十分見せてから考え中→待機へ戻し、短時間に多数の状態を巡回させない。
   - Live2Dを紹介するときはひよりだけに固定せず、取得済みの全公式サンプルを同条件で撮影する。中間フレームを目視し、画面外・極端な拡大縮小・透明化があれば`character.json`の正本を直して再同期する。

## Distributionへ公開する

1. LP、README、規約、キャラ構成、現行UI画像、`latest.json`、紹介PowerPointなど配布リポジトリに属する変更は、依頼を本体側で受けた場合も`C:\00_PG\21_PicoAgent-LP`へ移って実施する。21側の目的別PRをsquash mergeし、20側のコピーを先に直さない。
2. 20側から21側へ取り込むのは、正規入口で生成・検証したインストーラと、その実体から生成した更新情報など明示した成果物だけにする。`distribution/site/`一式の同期や、20側に残る古いLP・PPT・資産による上書きを行わない。
3. READMEへ開発中アルファ、対応OS、未署名警告、インストール、基本操作、問題報告先を明記する。macOSを含める場合はGatekeeperの回避手順（初回だけ右クリック→「開く」）と、マイク・画面収録・アクセシビリティの許可場所も書く。
4. 配布リポジトリも目的別PRでsquash mergeし、`main`への直接pushをしない。
5. 新しく採番した版のGitHub Releaseをプレリリースとして作り、その公開作業で同時に用意・検証した成果物（Windows NSIS、macOS DMG）だけを添付する。既存Releaseへ資産を追加・再アップロード・差し替えしない。公開後に変更や追加が必要になった場合は、`patch`を上げた別Releaseとして全成果物を公開し直す。
6. Pagesの最新ビルドが配布リポジトリのマージコミットを`built`としていることを確認する。公開LPをPC幅とスマホ幅で開き、CTA、現行UI、キャラの見切れ、旧機能の残存、動画表示を目視する。
7. 公開`latest.json`、公開Release、外部再取得したNSISがURL・サイズ・SHA-256で完全一致し、公開Pagesの内容が21側のマージ後`main`と一致するまで完了扱いにしない。20側`distribution/site/`との全ファイル一致は完了条件にしない。

## 許諾済み関係者向けの同梱版

- MMD・音声同梱版の正規入口はリポジトリ直下の`関係者版インストーラをビルド.bat`（本体は`scripts/build-partner-installer.ps1`、PowerShell 7必須）にする。スクリプトは同梱素材（`miko-mmd`・`korone-mmd`の登録データ、2人分のAIVMX、`temp/settings-runtime/seed-voices.ps1`）と`origin/main`との一致を検査し、専用Tauri設定とNSIS hookを`temp/partner-installer/`へ現在のチェックアウトの絶対パスで生成してから、`preview-distribution`→`build-ui --release --beta`→`beta-full`付き`cargo tauri build --bundles nsis`を実行する。成果物は`temp/partner-installer/PicoAgent_<版>_Partner-MMD-Voice_x64-setup.exe`へ移し、容量とSHA-256を表示する。hookの絶対パスを手で書き換えない。
- 関係者版の取得先は非公開Release一覧を正本にし、版ごとのリンク・ファイル名・容量を本体の文書へ書き写さない（`docs/DEVELOPMENT.md` の関係者版節は一覧へのリンクだけを持つ）。ファイル名・容量・SHA-256はReleaseの説明へ書く。
- 利用者が指定した許諾範囲と共有先に限定する。非公開配布を指定された場合、一般公開LP・公開Release・latest.jsonへ専用資産を混ぜない。指定リポジトリが実際に非公開であることをAPIで確認する。
- 検証済み本体と同じ版・feature・配信UIを使い、専用Tauri設定とNSIS hookは`temp/`に分離する。公開用設定や許可キャラ一覧を専用版の都合で変更しない。
- 追加するモデル・テクスチャ・音声だけを名指しする。利用者の設定ファイル、履歴、APIキー、ブラウザープロファイルはコピーしない。
- 初期設定は新規ディレクトリを作成でき、再実行しても既存のキャラ選択・声・背景・読み上げ無効を保持することを検査する。製品付属のPowerShellはWindows PowerShell 5.1でも検証する。
- 本体のコンパイル後、梱包内容だけを変更した場合は、同じ設定とfeatureの`cargo tauri bundle`で再梱包できる。中断後に同梱形式マーカーの警告が出たら、バイナリ内の`__TAURI_BUNDLE_TYPE_VAR_NSS`を検査し、未設定を成功扱いにしない。
- 専用ファイル名で保存し、導入・同梱設定を検証する。指定された非公開Releaseへ添付し、認証付きで再取得してSHA-256を照合する。外部共有先を勝手に公開へ切り替えない。
- 保存先の説明はREADMEへ書く。利用者が不要と指定した注意書きファイルはインストーラーへ追加しない。

## 記録の残し方

- PR本文へ対応、見送り、実機確認境界、ビルド元コミット、成果物のサイズとSHA-256を残す。公開版の利用者向け変更点とビルド元コミットは21側のRelease notesへ版単位で書き、次回の実効差分判定に使えるようにする。
- `README.md`と`SPEC.md`を現行状態へ同期する。未完了は`docs/TASKS.md`、未マージ作業の再開情報だけは`HANDOFF.md`へ反映する。
- インストーラを実際に起動する試験は、既存アプリや利用中画面を妨げない条件を確認する。実行できなければ、未確認境界として明記する。
