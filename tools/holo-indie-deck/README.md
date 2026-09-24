# holo Indie 応募企画書 PowerPoint 生成ツール

holo Indie へ応募する作品 PicoAgent の企画書 `docs/HOLO-INDIE-PLAN.pptx`（全9枚）を生成します。応募には企画書と概ね遊べるビルドの両方が必要で、これはその企画書です（VTuber事務所との契約向け提案書 `docs/IP-PROPOSAL-VTUBER.pptx` とは別物です）。

**応募作の本体は PicoAgent で、ミニゲーム「人間だとバレるな」はおまけです。** 企画書でもミニゲームは1枚だけで紹介します。

**文面の正本は [docs/HOLO-INDIE-PLAN.md](../../docs/HOLO-INDIE-PLAN.md)（ミニゲームは [docs/HOLO-INDIE-MINIGAME.md](../../docs/HOLO-INDIE-MINIGAME.md)）です。** 先に正本を直し、その内容を `build.js` へ反映してから再生成してください。PowerPoint を直接編集すると、次の生成で上書きされます。

## 使い方

```bash
npm install -g pptxgenjs sharp playwright   # 初回だけ。アプリのビルドには不要な作業用ツール
git lfs pull                                 # docs/images/ の実体が必要
node tools/holo-indie-deck/capture.mjs       # ミニゲームの結果画面を撮影（ゲーム画面を変えたときだけ）
node tools/holo-indie-deck/build.js          # docs/HOLO-INDIE-PLAN.pptx を生成
```

Playwright をグローバル以外に置いている場合は `PLAYWRIGHT_MODULE=<playwright の index.mjs>`、Chromium を指定する場合は `CHROMIUM_PATH` を渡します。

## 掲載画像

本体の画面は自社キャラクター「むぎ」の表示例です。公式キャラクター（審査版のさくらみこ公式MMD）の素材はリポジトリに無く、企画書にも入れません。画像の下に、審査版では公式モデルを表示する旨を書いています。

| 画像 | 使用箇所 |
|---|---|
| `layout-shot-stream.webp` | 表紙、4枚目（配信用の表示） |
| `layout-shot-pet.webp` | 3枚目（ペット形式） |
| `layout-shot-chat.webp` | 5枚目（チャット形式） |
| `holo-indie-result.webp` | 6枚目（おまけのミニゲーム） |

`holo-indie-result.webp` は `capture.mjs` が撮ります。`ui/index.html`・`ui/styles.css`・`ui/game.js` をそのまま読み込み、採点AIだけを固定の採点結果へ差し替え、乱数も固定するので、同じゲーム画面からは同じ画像ができます。撮るのは操作・採点欄だけで、右側のキャラクター表示は含めません。

撮影用HTMLとPowerPoint用の画像変換はリポジトリの`temp/`内に実行ごとの一時フォルダを作り、処理終了時に削除します。撮影は起動・操作の失敗時もブラウザを閉じ、一時フォルダを回収します。

## 送付前に埋める箇所

表紙の `【氏名／屋号】` と `【日付】` の2箇所です。

## 検証

```bash
python3 <pptxスキル>/scripts/office/validate.py docs/HOLO-INDIE-PLAN.pptx
soffice --headless --convert-to pdf docs/HOLO-INDIE-PLAN.pptx
```

LibreOffice は Meiryo を別フォントへ置き換えるため、文字幅は実際の PowerPoint と完全には一致しません。文字あふれは余裕をもって判定し、送付前に PowerPoint で目視してください。
