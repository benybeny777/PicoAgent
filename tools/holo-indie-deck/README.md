# holo Indie 応募企画書の下書き生成ツール

holo Indie へ応募する作品 PicoAgent の企画書の下書き `temp/HOLO-INDIE-PLAN.pptx`（全10枚）を生成します。提出物はDrive上の動画付きGoogleスライドです。下書きPPTXはGitにも提出先にも置きません。

**応募作の本体は PicoAgent で、ミニゲーム「人間だとバレるな」はおまけです。** 企画書でもミニゲームは1枚だけで紹介します。

**文面の正本は [docs/HOLO-INDIE-PLAN.md](../../docs/HOLO-INDIE-PLAN.md)（ミニゲームは [docs/HOLO-INDIE-MINIGAME.md](../../docs/HOLO-INDIE-MINIGAME.md)）です。** 先に正本を直し、その内容を `build.js` へ反映してから再生成してください。PowerPoint を直接編集すると、次の生成で上書きされます。

## 使い方

```bash
npm install -g pptxgenjs sharp jszip playwright   # 初回だけ。アプリのビルドには不要な作業用ツール
git lfs pull                                 # docs/images/ の実体が必要
node tools/holo-indie-deck/capture-current.mjs --voice-video # Aivis音声を含むさくらみこの配信動画を撮影
node tools/holo-indie-deck/build.js          # temp/HOLO-INDIE-PLAN.pptx を生成
```

`--voice-video`の前に、Aivisを起動して公開モデル「まお」を導入し、`pwsh -File tools/holo-indie-deck/synthesize-demo.ps1`で撮影用WAVを作ります。Aivisのポートが既定と異なる場合は`-Endpoint`を指定します。撮影用WAVは`temp/`だけに置き、Gitと提出先には含めません。

Playwright をグローバル以外に置いている場合は `PLAYWRIGHT_MODULE=<playwright の index.mjs>`、Chromium を指定する場合は `CHROMIUM_PATH` を渡します。

録画にはFFmpeg（H.264エンコーダ付き）、同期済みの`ui/assets/characters/hiyori/live2d/`、Live2D/MMDランタイム、`%APPDATA%/PicoAgent/characters/miko-mmd/character.json`と`korone-mmd/character.json`および各モデルが必要です。自動取得はしません。`PLAYWRIGHT_MODULE`でPlaywrightの`index.mjs`、`SHOOT_FFMPEG`でFFmpegを指定できます。別の作業ツリーに既存UI資産がある場合は`PICO_CAPTURE_SOURCE`にそのリポジトリの絶対パスを指定します。未指定時は現在のリポジトリを使用します。出力はこの生成ツールがあるリポジトリの`docs/images/`です。

## 掲載する画像と動画

本体の画面例には、さくらみこと利用者が追加した戌神ころねのMMDモデルを使います。審査版に同梱する1体と、アプリで追加できるキャラ数は別です。4枚目には、さくらみこのMMDとAivis音声を含むMP4、ひよりのLive2Dを示す無音MP4を埋め込みます。さくらみこは7秒、ひよりは6秒です。ひよりはLive2Dの動作例として紹介し、ホロライブのキャラクターや審査ビルドへの同梱キャラクターとは区別します。Googleスライドへ変換した後はMP4をDriveへアップロードし、静止画化した箇所をDrive動画へ置き換えます。スライドショーで各動画の▶を押すと再生します。編集画面やPDFでは動きません。この条件をファイル名と資料内に明記します。

| 素材 | 使用箇所 |
|---|---|
| `holo-miko-stream.png` | 表紙（さくらみこの配信表示） |
| `holo-korone-stream.png` | 3枚目（ころねの配信表示） |
| `holo-miko-chat.png` | 5枚目（さくらみこのチャット表示） |
| `holo-miko-motion.mp4` と `holo-miko-motion-poster.png` | 4枚目（さくらみこの動作） |
| `holo-hiyori-motion.mp4` と `holo-hiyori-motion-poster.png` | 4枚目（ひよりのLive2D動作） |
| `holo-indie-result.webp` | 6枚目（おまけのミニゲーム） |

`capture-current.mjs`は現行の実フロントを使い、バックエンド応答だけを撮影用に固定します。`--voice-video`では既存のAivis合成音声サンプルを実フロントの読み上げ経路へ渡し、さくらみこの配信動画に音声を収録します。音声の文章と画面上の固定応答の一致は収録前に確認してください。AIの実応答の検証ではありません。導入済みモデルだけを読み取り、PMX・テクスチャや利用者設定は変更しません。モデル本体はPPTにもGitにも追加せず、表示した画面と録画だけを保存します。動画はH.264・yuv420pで、外部リンクに依存せずPPT内へ埋め込みます。

`holo-indie-result.webp` は `capture-current.mjs` が現行ゲーム画面から撮影します。透明なペット画面には説明用の背景色を足さず、スライドには背景を持つ配信画面を掲載します。

録画フレームとPowerPoint用の画像変換はリポジトリの`temp/`内に実行ごとの一時フォルダを作り、処理終了時に削除します。撮影は失敗時もブラウザを閉じ、一時フォルダを回収します。

## 送付前に埋める箇所

表紙の `【氏名／屋号】` と `【日付】` の2箇所です。

## 検証

```bash
python3 <pptxスキル>/scripts/office/validate.py temp/HOLO-INDIE-PLAN.pptx
soffice --headless --convert-to pdf temp/HOLO-INDIE-PLAN.pptx
```

LibreOffice は Meiryo を別フォントへ置き換えるため、文字幅は実際の PowerPoint と完全には一致しません。文字あふれは余裕をもって判定し、送付前に PowerPoint で目視してください。
