# IP 提案書 PowerPoint 生成ツール

VTuber 事務所へ送付する提案書 `docs/IP-PROPOSAL-VTUBER.pptx`（全10枚）を生成します。

**文面の正本は [docs/IP-PROPOSAL-VTUBER.md](../../docs/IP-PROPOSAL-VTUBER.md) です。** 先に正本を直し、その内容をこのスクリプトへ反映してから再生成してください。PowerPoint を直接編集すると、次の生成で上書きされます。

## 使い方

```bash
npm install -g pptxgenjs sharp   # 初回だけ。アプリのビルドには不要な作業用ツール
git lfs pull                     # docs/images/ の実体が必要
node tools/ip-proposal-deck/build.js
```

`docs/IP-PROPOSAL-VTUBER.pptx` が上書き生成されます。

## 掲載画像

`docs/images/` の次の 3 点を使います。pptxgenjs は WebP を埋め込めないため、実行時に PNG へ変換して一時領域へ置き、生成後に削除します。

| 画像 | 使用箇所 |
|---|---|
| `layout-shot-stream.webp` | 表紙（配信表示） |
| `layout-shot-workspace.webp` | 3枚目（作業表示） |
| `layout-shot-pet.webp` | 9枚目（常駐表示） |

**Live2D 公式サンプル（ひより／はる／イプシロン）の画像は使いません。** 素材データの再配布を避けるためで、掲載するのは自社キャラクターの画面だけにします（[docs/SALES.md](../../docs/SALES.md) B5）。

## 送付前に埋める箇所

本文の `【 】` は差し替え用のプレースホルダです（9 箇所）。

- 表紙: 貴社名、氏名・屋号、日付
- 9枚目: 個人事業者／法人名、配布 URL
- 10枚目: 氏名／屋号、所在地、連絡先、製品サイト

## 検証

```bash
python3 <pptxスキル>/scripts/office/validate.py docs/IP-PROPOSAL-VTUBER.pptx
soffice --headless --convert-to pdf docs/IP-PROPOSAL-VTUBER.pptx && pdftoppm -jpeg -r 110 IP-PROPOSAL-VTUBER.pdf slide
```

LibreOffice は Meiryo を別フォントへ置換するため、レンダリング結果の文字幅は実際の PowerPoint と完全には一致しません。文字あふれの判定は余裕をもって行ってください。
