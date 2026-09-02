# 引き継ぎ

この文書は、現在進行中の作業だけを次の担当へ渡すための短い入口です。完了した作業の経緯はPR、恒久ルールは[AGENTS.md](AGENTS.md)、現行仕様は[SPEC.md](SPEC.md)、未着手・未確認項目は[docs/TASKS.md](docs/TASKS.md)を参照してください。

## 現在の状態

- 最終確認日: 2026-09-02
- 進行中作業: macOS版アルファの公開。**残りはmac実機での`.dmg`生成とRelease公開だけ。** 本体 PR #179 と配布Repo `PicoAgent-Distribution` PR #31 はどちらもマージ済み。
- 公開LPには配布Repo PR #1で`v1.0.4`のmacOS版公開準備中ボタンを追加済み。Release作成まではリンク先の404を許容し、公開後に同じURLが200を返すことを確認する。
- 次に着手できる内容: [docs/TASKS.md](docs/TASKS.md)のうち、着手条件を満たした項目

### macOS版アルファの再開手順

**コード側の作業は完了してmainへ入っている。mac実機ですることはビルドと、そこで初めて出る環境依存問題の解決だけ。** キーチェーン移行、更新フィードのDMG対応、`Info.plist`、対応OS版の下限はすべてPR #179でマージ済みで、mac側で書き足す設計判断は無い。macOSの`cargo check`が警告なく通ることはPR #180で確認済み。**未確認なのは`.app`／`.dmg`のバンドル生成と実機動作。**

**完了条件は、現行`main`の版である`.dmg`がv1.0.4のプレリリースへ上がること。** PR #180は`cargo check`と`cargo test`までで完了扱いになり、バンドル生成まで到達していなかった。ビルド直前に`src-tauri/Cargo.toml`の版が1.0.4であることを確認し、`main`が先へ進んでいたら古い版を公開せず、このPRの版・ファイル名・Release notesを新しい版へ更新してから続ける。

1. mac実機で本体`main`を最新へ更新する。`src-tauri/Cargo.toml`、`xtask/Cargo.toml`、`src-tauri/tauri.conf.json`の版がすべて1.0.4で一致することを確認してから、`cargo xtask build-beta --bundles app,dmg`を実行する。前提と手順は [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) の「macOS版のビルド（未署名アルファ）」。この作業依頼の古い分岐点をそのままビルドしない。
2. **ビルドが通らない場合は、その場で目的別ブランチを作り、修正・検証・PR・マージまで完了する。** 失敗ログだけ持ち帰らない。想定される詰まり方は次の3種類で、いずれもmac側でしか解決できない。
   - **前提ツールの不足** … Xcode Command Line Tools、Tauri CLI、`p7zip`（Aivisの分割書庫展開）。`docs/DEVELOPMENT.md`のコマンドで揃う
   - **一度もmac向けにコンパイルされていない自前コードの誤り** … `secret.rs`（今回追加）、`engine.rs`の`sysctlbyname`、`aivis.rs`のGPU判定、`engine_backend.rs`のMetal表示、`skills/trash_file`の`~/.Trash`、`skills/browser_control/cdp.rs`のブラウザ既定パス。読んだ限り問題は無いが、コンパイルで確定させる
   - **同梱物取得のmac経路** … llama.cpp・Whisperのmac資産、LibreOffice DMGのマウント、Aivisの`.7z.001`展開
3. [docs/TASKS.md](docs/TASKS.md)の「macOS版の実機QA」を消化する。配布対象はApple Siliconだけで、Intel Macは対象外。
4. `.dmg`を`v1.0.4`のプレリリースとして配布Repoへ上げる。成果物は`src-tauri/target/release/bundle/dmg/PicoAgent_1.0.4_aarch64.dmg`を使う。**Windows版は待たない。** Windows版は公開中のv1.0.2のまま据え置き、macOS版だけを現行ソース版のv1.0.4として公開する。
5. **`cargo xtask update-feed`を実行せず、`latest.json`も変更しない。** `latest.json`はWindows向けアプリ内更新の正本で現在1.0.2を指す。macOSはアプリ内更新に未対応（`install_supported = cfg!(windows) && cfg!(target_arch = "x86_64")`）のため登録が要らない。1.0.4へ書き換えると、Windows利用者に「新しいバージョンがありますが、このPC向けインストーラは準備中です」と表示される。
6. 配布Repo PR #1で追加済みのmacOS版ボタンが`v1.0.4`を指すことと、Pages反映後にReleaseページが200を返すことを確認する。Release作成前の404は許容する。

**Windows環境ではmacOS向けの全体コンパイルができない。** `objc2-exception-helper`のビルドスクリプトがCコンパイラを要求し、Apple SDKはApple製ハードウェア上での利用に限られるため、クロス環境を用意しない。`secret.rs`と`security-framework`だけは切り出した検査用クレートで`aarch64-apple-darwin`向けの型検査を通してある。

## 再開時の順序

1. `AGENTS.md`を読む。
2. `git status --short --branch`、`git log -5 --oneline`、開いているPRを確認する。
3. 進行中PRがあれば、そのPR本文と差分から再開する。
4. 進行中作業がなければ`docs/TASKS.md`から、着手条件を満たす項目だけを選ぶ。

## 更新ルール

- 未マージの作業、再開に必要な環境、直後に行うことだけを書く。
- 完了した作業の説明、テスト結果、過去のPR一覧は残さない。PRへ記録する。
- マージ後に進行中項目を削除し、何もなければ「進行中作業なし」とする。
