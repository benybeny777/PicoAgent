# 引き継ぎ

この文書は、現在進行中の作業だけを次の担当へ渡すための短い入口です。完了した作業の経緯はPR、恒久ルールは[AGENTS.md](AGENTS.md)、現行仕様は[SPEC.md](SPEC.md)、未着手・未確認項目は[docs/TASKS.md](docs/TASKS.md)を参照してください。

## 現在の状態

- 最終確認日: 2026-08-26
- 進行中作業: macOS版アルファの公開。本体 PR #179（macOSのシークレット保護・mac配布物の更新フィード対応）と、配布Repo `PicoAgent-Distribution` PR #31（取得導線）が未マージ。
- 次に着手できる内容: [docs/TASKS.md](docs/TASKS.md)のうち、着手条件を満たした項目

### macOS版アルファの再開手順

**コード側の作業はWindows環境で完了している。mac実機ですることはビルドと、そこで初めて出る環境依存問題の解決だけ。** キーチェーン移行、更新フィードのDMG対応、`Info.plist`、対応OS版の下限はすべてPR #179に入っており、mac側で書き足す設計判断は無い。

1. mac実機で PR #179 のブランチ `feat/macos-keychain-and-mac-distribution` をチェックアウトし、`cargo xtask build-beta --bundles app,dmg` を実行する。前提と手順は [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) の「macOS版のビルド（未署名アルファ）」。
2. **ビルドが通らない場合は、その場で直して同じブランチへコミットし、PRを完成させる。** 失敗ログだけ持ち帰らない。想定される詰まり方は次の3種類で、いずれもmac側でしか解決できない。
   - **前提ツールの不足** … Xcode Command Line Tools、Tauri CLI、`p7zip`（Aivisの分割書庫展開）。`docs/DEVELOPMENT.md`のコマンドで揃う
   - **一度もmac向けにコンパイルされていない自前コードの誤り** … `secret.rs`（今回追加）、`engine.rs`の`sysctlbyname`、`aivis.rs`のGPU判定、`engine_backend.rs`のMetal表示、`skills/trash_file`の`~/.Trash`、`skills/browser_control/cdp.rs`のブラウザ既定パス。読んだ限り問題は無いが、コンパイルで確定させる
   - **同梱物取得のmac経路** … llama.cpp・Whisperのmac資産、LibreOffice DMGのマウント、Aivisの`.7z.001`展開
3. [docs/TASKS.md](docs/TASKS.md)の「macOS版の実機QA」を消化する。Apple SiliconとIntelは別バイナリなので機種ごとに確認する。
4. Windows 10/11 で v1.0.3 のNSISを作る。配布Repo PR #31 はWindows版もv1.0.3にする前提で書いてある。
5. `v1.0.3` Releaseへ両方の成果物を置き、`cargo xtask update-feed <各成果物>` で `latest.json` を生成して配布Repoへ取り込む。
6. PR #179 → 配布Repo PR #31 の順にマージし、Pagesを公開する。

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
