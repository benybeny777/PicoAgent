# README 用スクリーンショット（動く絵）の撮影

`docs/images/` に置いている README 掲載画像を再生成するツールです。**出力はすべて透過つきの
アニメーション WebP**（無限ループ）です。

実物の `ui/` をローカル HTTP で配信し、**バックエンド（`window.__TAURI__`）だけをモック**して
ヘッドレス Chromium で描画します。フロントの JS・CSS・キャラクター画像は本物をそのまま使うため、
出てくる絵はモックアップではなく実際の描画結果です。

- ウィンドウサイズは `src-tauri/tauri.conf.json` の既定値（560×430）に合わせています
- `transparent: true` の実物どおり背景なしで撮影し、不透明領域だけを切り出します
- 1本あたり 4 秒・12fps・48 コマ。文字送りとキャラクターのモーションがそのまま入ります

## 使い方

Node はアプリの依存ではなく**開発時の作業ツール**として使います。`package.json` は置かず、
playwright はグローバル導入したものを環境変数で指すだけにしています。書き出しには `ffmpeg` が要ります。

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
| `SHOOT_FFMPEG` | ffmpeg の実行ファイルパス（未指定なら PATH の `ffmpeg`） |

引数に画像名を並べると、その絵だけを撮り直します。全部で 3 分ほどかかるので、普段はこちらを使います。

```bash
node tools/screenshots/shoot.mjs composer-light.webp
```

## 撮れる絵

| ファイル | 内容 | 何が動くか |
|---|---|---|
| `main-light.webp` / `main-dark.webp` | メイン画面（ライト / ダーク） | 文字送りと、考え中 → 返答のキャラクターモーション |
| `composer-light.webp` | 入力欄とクイック操作 | 次の依頼を 1 文字ずつ打ち込むところ |
| `fullscreen-light.webp` | 全画面フキダシ（履歴の検索・コピー・再利用・書き出し） | 返答が履歴へ積まれて末尾までスクロールするところ |
| `live2d-light.webp` / `live2d-dark.webp` | Live2D 描画（ひより、ライト / ダーク） | Cubism ランタイムのまばたき・モーション・口パク |

## 撮り方（なぜ遠回りをしているか）

- **形式はアニメーション WebP**。GIF は透過が 1bit に落ちてキャラクターの輪郭がギザつきます。APNG は
  透過を保てますが同じ絵で 4.8MB、WebP なら 0.9MB でした。透過のおかげで GitHub のライトテーマでも
  ダークテーマでも背景なしのまま出せます
- **透過を保ったまま動画キャプチャする方法がヘッドレス Chromium にはありません**（compositor 出力には
  背景が焼き込まれます）。そのため透過 PNG を連写して `ffmpeg` で束ねています
- **ページ内の時計を止めて 1 コマずつ進めます。** キャプチャ 1 枚に 200ms 以上かかるので、実時間で
  連写すると 5fps も出ずカクつきます。Playwright の `page.clock` で時計を止め、
  「1/fps だけ時間を進める → 1 枚撮る」を繰り返すと、撮影に何秒かかってもページ内の経過時間は一定なので
  狙ったフレームレートで滑らかになります
- **`ffmpeg` の `-compression_level` は既定（4）のまま**にしてください。6 にすると libwebp の探索が
  跳ね上がり、48 コマの書き出しが数分たっても終わりません。サイズはほとんど変わりません

## Live2D の絵について

Live2D の絵は、ランタイムとモデルが未取得だと**スキップされます**（他の絵は撮れます）。

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

撮るたびにバイト列が変わります（履歴の時刻表示など）。見た目が変わっていないならコミットせず
`git checkout -- docs/images/` で戻してください。差分が大きいだけで中身は同じです。

> [!NOTE]
> **`ffmpeg` はアニメーション WebP を読めません**（書き出し専用。`-i` に渡すと
> `image data not found` で失敗します）。中身を確認したいときはブラウザで開くか、
> libwebp の `anim_dump` / `webpmux -get frame N` を使ってください。
