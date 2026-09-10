# 第三者ソフトウェア・素材の表示（販売前台帳）

> 実際の販売成果物に含まれるファイルと突合して確定する台帳です。ここに名称があるだけで再配布が許可されるわけではありません。

| 区分 | 主な対象 | 販売前の確認 |
|---|---|---|
| デスクトップ基盤 | Tauri、Rustクレート、WebView2 | Cargo.lockと同梱ライセンス一覧を生成し、表示方法を確定 |
| ローカル推論 | llama.cpp、利用者が取得するGGUFモデル | 実行ファイルのライセンス表示、モデルごとの規約同意・帰属 |
| 音声 | AivisSpeech Engine、音声モデル、whisper.cpp | Aivis／Whisper本体は公式配布元から直接取得。各音声・認識モデルの商用条件と表示を確認 |
| Live2D | Cubism Core、PIXI、pixi-live2d-display、利用者モデル | 拡張性アプリ審査・契約、指定ロゴ・文言、SDKとモデルの条件 |
| 文書・メディア | LibreOffice、Pandoc、FFmpeg、qpdf、Poppler、ImageMagick、ripgrep | 販売物へ再配布せず各公式ページと設定手順を案内。ライセンスは一律ではないため、実際に使う版・ビルド・依存物ごとに確認する |
| 画面配置 | Moveable 0.53.0 | `ui/vendor/moveable/moveable.min.js` を同梱。MIT。取得元 `https://unpkg.com/moveable@0.53.0/dist/moveable.min.js`、SHA-256 `6C6C78751303AFD6212398C148DD767D83F1D6BC1858569C140A8F0DAFE22F41` |

販売用キャラは `distribution/release.json` の明示リストだけを同期します。開発用のLive2D公式サンプルは通常ビルドを含む販売成果物へ同期せず、アプリからも自動取得しません。利用者が配布元から取得したモデルは利用者領域へ入り、PicoAgentの配布物にはなりません。

販売版ではAivis／Whisperを各公式配布元から利用者領域へ直接取得し、LibreOffice／文書・メディアツールは外部導入に変更しました。この変更でPicoAgentのGitHub Releaseからの再配布は避けますが、実行時取得や同梱ランタイムにも表示・ライセンス義務が残り得ます。llama.cpp、Cubism Core、開発版同梱物、各モデル固有条件を含め、実成果物と取得経路を確認してから `thirdPartyInventoryReviewed` を真にします。

販売前には、バイナリ・ライブラリ・モデル・画像・音声・フォント・アイコンの全ファイルを一覧化し、名称、版、配布元、ライセンス、変更有無、表示文、ソース提供要否、取得方式を記録します。

## Moveable 0.53.0

Copyright (c) 2019 Daybrush

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
