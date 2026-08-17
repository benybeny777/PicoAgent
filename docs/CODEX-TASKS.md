# Codexへの作業指示

**直しきれなかった・環境がなくて確認できなかった項目**をここへ作業指示としてまとめる。判断の理由は
レビュー記録、進行状況は `HANDOFF.md`、ルールは `AGENTS.md` が持ち場。ここは「次に手を動かす内容」だけ。
現在の内容は、完了済み項目を除いたレビュー残件と依存ライブラリの検討分。

> **依存ライブラリを足すときに先に読むもの: [LIBRARY-CANDIDATES.md](LIBRARY-CANDIDATES.md)。**
> `ui/vendor/` へ依存を置くときの制約（CSP、拡張子 allowlist、`bundle_scope.rs` の走査除外）と、
> 他に入れる価値のあるライブラリの採否・見送り理由をまとめてある。

優先度は P0（配布前に必須）→ P1（次の作業回で）→ 保留（条件が揃うまで着手しない）。
着手したら、完了条件まで満たしてから `docs/REVIEW-LOG.md` の該当行を更新し、この項目を消す。

---

## P0-3. 配布ページの画像重量を下げる

- **なぜ**: 960px化で `characters.html` が画像だけで**60.2MB**（28本、最大5MB/本）、`index.html` が15.1MBになった。カードの実表示幅は約275pxで、960pxは表示スロットの3倍以上ある。スマホ回線では実質開けない。
- **やること**: 高精細の正本は `docs/images/` に残し、LPが配る `distribution/site/assets/` だけ480〜640px幅で別途書き出す。既存の960pxファイルを縮小再エンコードするのではなく、同じ2倍密度描画から目的の幅で書き出す（引き伸ばしにはならない）。`characters.html` は静止画＋クリックで再生に切り替える案も併せて検討する。
- **完了条件**: `characters.html` の画像合計が15MB以下、`index.html` が5MB以下。カードの見た目が現状と同等であることを実表示で確認する。

## P0-4. Windows実機でプロセス層のテストを走らせる

- **なぜ**: タイムアウトの回帰テストは長らく `#[cfg(unix)]` だけで、Windows中心の開発では**一度も実行されていなかった**。
  今回 `cmd /c` 版を追加したが、Linuxコンテナでは `#[cfg(windows)]` 側が走らないため未実行のまま。
- **やること**: Windowsで `cargo test --lib --manifest-path src-tauri/Cargo.toml tools::process` を実行する。
- **完了条件**: `タイムアウトで子プロセスを残さない` が成功し、所要が1秒指定に対して5秒未満で戻る（30秒まで待たない）。
  失敗する場合は `process::output()` の打ち切り経路（`join_capture` / `abort`）を疑い、テストの期待値を緩めない。

## P1-1. 設定画面の実表示で日本語ラベルの収まりを見る

- **なぜ**: 外部サービス連携22スキルと設定ガイドの英語ラベル40件を日本語へ統一したが、**GUIでの折り返しは未確認**。
- **やること**: `.agents/skills/verify-picoagent-ui/SKILL.md` に従い、設定画面の「スキル」タブを 400px / 600px 幅で確認する。
  長いものから: 「Google スプレッドシートの OAuth アクセストークン」「Bluesky の識別子（ハンドルまたはメール）」
  「Microsoft Graph の OAuth アクセストークン」。
- **完了条件**: ラベルの溢れ・入力欄の潰れ・2段折り返しによる行ずれがないこと。崩れる場合はラベルを短くする
  （キー名と `type` は変えない。`secret::guard` の password 判定に影響する）。

## P1-2. キャラ行列の再撮影と「静止して見えないか」の目視

- **なぜ**: 全身の常時モーションを廃止し、撮影側の検査からも body 側アニメーション名の条件を外した。
  実際の見え方（待機が完全な静止に見えないか）は人の目でしか判断できない。
- **やること**: `SHOOT_CHARACTER_MATRIX=1` でPNG系8キャラ×7形式を再撮影し、中間フレームを目視する。
- **完了条件**: 描画欠け・速度警告0件、`胸元の局所二次動作が適用されていません` が出ないこと、
  まばたき・口パク・胸元の局所動作で「生きている」と見えること。静止に見える場合だけ、
  `pet-success` と同じ1px級の待機モーションを**1つだけ**足す（10種のループを戻さない）。

## P1-3. LINE風チャット統合後の実表示

- **なぜ**: `talk` をLINE風の `chat` へ統合し、通常2Dの切り出しを以前の位置へ戻したため、実WebViewでの確認が必要。
- **やること**: `chat` を実表示し、62×62pxの丸いキャラ枠、白／緑の左右吹き出し、上端3列の操作、接続ランプ・♡の位置を確認する。
- **完了条件**: 整理前の撮影（`temp/character-layout-matrix/`）と見た目が一致すること。

## P1-4. 文字起こしの出力上限が変わった影響を見る

- **なぜ**: `whisper_transcribe` / `meeting_recorder` を共通層 `process::output()` へ寄せたため、
  出力の保持上限がスキル独自の 32KiB から共通層の 1MiB になった（表示側の `trim_output` は32KiBのまま）。
- **やること**: 長時間音声（10分以上）で文字起こしし、メモリと応答の体感を確認する。
- **完了条件**: 問題がなければそのまま。過大なら共通層側へ上限の設定キーを足す（スキル側で再実装しない）。

## P1-5. 導入済み Moveable の未処理3点（設計ガード・LICENSE・第三者表記）

Moveable 本体（`ui/vendor/moveable/moveable.min.js` の直コミットと「配置」モード）は導入済み。以下はその残り。
背景と選択肢は [LIBRARY-CANDIDATES.md](LIBRARY-CANDIDATES.md) §1。

- **なぜ**: `.gitignore` が無視しているのは `ui/vendor/live2d/` だけなのに、
  `src-tauri/src/bundle_scope.rs::UI_GENERATED_DIRS` は `vendor` を**ディレクトリごと**走査から外し、
  理由を「Live2D ランタイム（再配布条件があるため Git 管理外）」と書いている。**`ui/vendor/moveable/` は
  Git 管理下にあり製品へ入るのに、同梱物の範囲を固定するはずの検査が当たっていない。** さらに
  `docs/THIRD-PARTY-NOTICES.md` に Moveable の項目が無く、MIT の表示義務を満たしていない。
- **やること**:
  1. `UI_GENERATED_DIRS` の除外を `vendor/live2d` へ狭め、コメントの説明を実態へ直す。
  2. 狭めると `ui/vendor/moveable/LICENSE` が `ui配下に開発用ファイルを置かない` で落ちる（拡張子なしは
     空文字になり `ALLOWED_UI_EXTENSIONS` に一致しない）。ライセンス本文を `THIRD-PARTY-NOTICES.md` へ移して
     `ui/` から消す／`LICENSE.txt` へ改名して `txt` を allowlist へ足す／例外として明記する、から選ぶ。
     **1案目を推奨**（`ui/` は実行時に使うものだけ、という現行の考えかたに合う）。
  3. `docs/THIRD-PARTY-NOTICES.md` へ Moveable（MIT）を追記し、採用した版・取得元 URL・SHA-256 を記録する。
- **完了条件**: `cargo test --lib --manifest-path src-tauri/Cargo.toml bundle_scope` が通り、除外範囲と
  コメントの説明が一致していること。`cargo xtask build-ui` の `ui-dist/` に `moveable.min.js` が入ること。
  第三者表記に Moveable があること。
- **禁止**: CDN からの読み込み（CSP が `script-src 'self'`）。`package.json` の追加。ライセンス本文を消すこと。

## P1-6. `cargo-deny` でライセンス検査を足す

- **なぜ**: 現在の監査は `cargo audit`（既知脆弱性）だけで、**ライセンスの混入検査が無い**。販売品では
  GPL 系の紛れ込みのほうが致命的になりうる。アプリの依存は1つも増えない（開発ツール）。
- **やること**: `deny.toml` を置いて `licenses`（MIT / Apache-2.0 / BSD 系を allow）と `advisories` を有効にし、
  `.agents/skills/prepare-picoagent-release/SKILL.md` の販売前監査へ実行手順を1行足す。GitHub Actions は使わない。
- **完了条件**: `cargo deny check` が通ること。落ちる依存があれば、置き換えか例外登録の理由を `docs/REVIEW-LOG.md` へ残す。

---

## 保留（条件が揃うまで着手しない）

| # | 内容 | 着手条件 |
|---|---|---|
| 1 | 配布物412MBの分離ダウンロード設計（高精細7キャラで405.6MiBを占める） | 画質を落とさない前提での設計判断。ベータの試用結果を見てから |
| 2 | macOS版のビルド・署名・公証と実機QA。**着手時は `secret.rs` の平文フォールバックを先に塞ぐ**（`keyring` で Keychain へ寄せる。`enc:dpapi:v1:` の Windows 経路と `secret::guard` の password 判定はそのまま残す） | macOSホストが要る。WindowsホストとGitHub Actionsは使わない方針。詳細は [LIBRARY-CANDIDATES.md](LIBRARY-CANDIDATES.md) §2-1 |
| 3 | Windows実機の残り目視（8方向リサイズ、端寄せ位置の復元、起動画面からの復帰、多重起動時の前面化、Aivis更新導線の新旧版表示、キャラ別挨拶の文面、配信立ち絵の0.5px差） | Windows実機を触る回にまとめて |
| 4 | Free／Pro のスキル制限とライセンス入力欄の設計見直し（`beta-full` ビルドでも入力欄が出る、`license::CONFIG_KEY` の cfg 分岐、制限の粒度） | **ベータ期間中は全機能試用で運用する方針のため着手しない。** 正式版の準備時に、制限そのものの設計から見直す |
