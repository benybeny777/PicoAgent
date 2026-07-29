# README 用スクリーンショットの撮影

`docs/images/` に置いている README 掲載画像を再生成するツールです。

実物の `ui/` をローカル HTTP で配信し、**バックエンド（`window.__TAURI__`）だけをモック**して
ヘッドレス Chromium で描画します。フロントの JS・CSS・キャラクター画像は本物をそのまま使うため、
出てくる絵はモックアップではなく実際の描画結果です。

- ウィンドウサイズは `src-tauri/tauri.conf.json` の既定値（560×430）に合わせています
- `transparent: true` の実物どおり背景なしで撮影し、不透明領域だけを切り出します
- 文字送りアニメーションの完了を待ってから撮るので、途中で切れた文は写りません

## 使い方

Node はアプリの依存ではなく**開発時の作業ツール**として使います。`package.json` は置かず、
playwright はグローバル導入したものを環境変数で指すだけにしています。

```bash
npm i -g playwright && npx playwright install chromium

PLAYWRIGHT_MODULE=$(npm root -g)/playwright/index.mjs \
  node tools/screenshots/shoot.mjs
```

Windows（PowerShell）ではグローバル導入先が `%APPDATA%\npm\node_modules` になります。

```powershell
$env:PLAYWRIGHT_MODULE = "$(npm root -g)\playwright\index.mjs"
node tools/screenshots/shoot.mjs
```

| 環境変数 | 用途 |
|---|---|
| `PLAYWRIGHT_MODULE` | playwright の `index.mjs` の絶対パス（未指定なら `playwright` を通常解決）。Windows の `C:\...` 形式もそのまま渡せます |
| `PLAYWRIGHT_CHROMIUM` | Chromium の実行ファイルパス（未指定なら playwright の既定） |

引数に画像名を並べると、その絵だけを撮り直します。

```bash
node tools/screenshots/shoot.mjs composer-light.png
```

## 撮れる絵

| ファイル | 内容 |
|---|---|
| `main-light.png` / `main-dark.png` | メイン画面（ライト / ダーク） |
| `composer-light.png` | 入力欄とクイック操作 |
| `fullscreen-light.png` | 全画面フキダシ（履歴の検索・コピー・再利用・書き出し） |
| `live2d-light.png` / `live2d-dark.png` | Live2D 描画（ひより） |

## Live2D の絵について

Live2D の 2 枚は、ランタイムとモデルが未取得だと**スキップされます**（他の絵は撮れます）。

```bash
cargo xtask setup live2d   # 先にこれを実行する
```

必要なのは `ui/vendor/live2d/{live2dcubismcore.min.js,pixi.min.js,cubism4.min.js}` と
`ui/assets/characters/hiyori/live2d/` です。いずれも再配布条件があるためリポジトリには含めていません
（`.gitignore` 済み）。

> [!NOTE]
> **Cubism Core（`live2dcubismcore.min.js`）は Live2D 公式サイトからのみ配布されています。**
> npm にも GitHub にも正規版はありません（公式の `Live2D/CubismWebSamples` も Core は含まず、
> README で公式サイトからのコピーを指示しています）。ネットワークが `cubism.live2d.com` へ
> 到達できない環境では取得できず、Live2D の絵は撮れません。

## 撮り直しについて

PNG は撮るたびにバイト列が変わります（履歴の時刻表示など）。見た目が変わっていないなら
コミットせず `git checkout -- docs/images/` で戻してください。差分が大きいだけで中身は同じです。
