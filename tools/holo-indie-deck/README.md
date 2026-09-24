# holo Indie 企画書 PowerPoint 生成ツール

holo Indie へ応募するゲーム「人間だとバレるな」の企画書 `docs/HOLO-INDIE-GAME-PLAN.pptx`（全10枚）を生成します。応募には企画書と概ね遊べるビルドの両方が必要で、これはその企画書です（VTuber事務所との契約向け提案書 `docs/IP-PROPOSAL-VTUBER.pptx` とは別物です）。

**文面の正本は [docs/HOLO-INDIE-MINIGAME.md](../../docs/HOLO-INDIE-MINIGAME.md) です。** 先に正本を直し、その内容を `build.js` へ反映してから再生成してください。PowerPoint を直接編集すると、次の生成で上書きされます。

## 使い方

```bash
npm install -g pptxgenjs sharp playwright   # 初回だけ。アプリのビルドには不要な作業用ツール
node tools/holo-indie-deck/capture.mjs       # ゲーム画面を docs/images/holo-indie-*.webp へ撮影
node tools/holo-indie-deck/build.js          # docs/HOLO-INDIE-GAME-PLAN.pptx を生成
```

Playwright をグローバル以外に置いている場合は `PLAYWRIGHT_MODULE=<playwright の index.mjs>`、Chromium を指定する場合は `CHROMIUM_PATH` を渡します。

## 掲載画像

`capture.mjs` が `ui/index.html`・`ui/styles.css`・`ui/game.js` をそのまま読み込み、採点AIだけを固定の採点結果へ差し替え、乱数も固定して撮影します。撮るのは操作・採点欄だけで、右側のキャラクター表示は含めません（公式キャラクターの素材はリポジトリに無く、企画書へも同梱しないため）。

| 画像 | 使用箇所 |
|---|---|
| `holo-indie-result.webp` | 表紙（問題の結果画面） |
| `holo-indie-question.webp` | 3枚目（出題画面） |
| `holo-indie-interrogation.webp` | 5枚目（追及タイム） |
| `holo-indie-final.webp` | 8枚目（監査終了画面） |

ゲーム画面を変えたら撮り直してください。

## 送付前に埋める箇所

表紙の `【氏名／屋号】` と `【日付】` の2箇所です。

## 検証

```bash
python3 <pptxスキル>/scripts/office/validate.py docs/HOLO-INDIE-GAME-PLAN.pptx
soffice --headless --convert-to pdf docs/HOLO-INDIE-GAME-PLAN.pptx
```

LibreOffice は Meiryo を別フォントへ置き換えるため、文字幅は実際の PowerPoint と完全には一致しません。文字あふれは余裕をもって判定し、送付前に PowerPoint で目視してください。
