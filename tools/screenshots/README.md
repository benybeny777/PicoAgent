# README 用スクリーンショット（動く絵）の撮影

`docs/images/` に置いている README 掲載画像を再生成するツールです。通常プレビューは**透過つきの
アニメーション WebP**（無限ループ）、7形式の比較画像は1フレームの透過 WebPです。

実物の `ui/` をローカル HTTP で配信し、**バックエンド（`window.__TAURI__`）だけをモック**して
ヘッドレス Chromium で描画します。フロントの JS・CSS・キャラクター画像は本物をそのまま使うため、
出てくる絵はモックアップではなく実際の描画結果です。

- ウィンドウサイズは各表示形式の既定値に合わせています（ペット形式は 900×600）
- `transparent: true` の実物どおり背景なしで撮影し、不透明領域だけを切り出します
- 通常の動画は1本あたり6秒・30fps・180コマ。文字送りとキャラクターのモーションがそのまま入ります
- キャラクター一覧（`character-*.webp`）は標準ペット表示、`character-*-stream.webp`は配信表示です。README公開用は`SHOOT_README_GALLERY=1`で6秒・480px・16fpsとして`docs/images/`へ直接書き出します。通常版は900px画面を2倍ピクセル密度で描画し、配信版はフルHD 1920×1080のネイティブ画面から縮小します
- LP配信用は`SHOOT_SITE_GALLERY=1`で、同じ撮影経路の生PNGから`distribution/site/assets/`へ6秒・480px・16fpsで直接書き出します。既存WebPを入力にした縮小・再エンコードは行いません。全件実行時はキャラ一覧15MB、トップ4本5MBの上限も検査します
- **公開用キャラクター一覧は口パクを先に十分見せてから、考え中 → 待機へ戻ります。** まばたき、Live2D固有モーション／物理、対応キャラの局所二次動作はそのまま動きます
  - 考え中を明示的に入れているのは、`record()` が録画前に時計を進めるため、送信直後の考え中が録画開始時点で終わっているからです
- 12秒・30fps・960pxの高解像度版は、個別の比較・保管を明示された場合だけ環境変数なしで生成します。READMEやLPの公開埋め込みには使いません
- **口パクは実際の音声再生開始後だけ動きます。** 読み上げ無効（ミュート）または音量0では、2D／Live2Dとも口を閉じたままです。キャラクター一覧と Live2D の絵は `voice: true`（`openScene` のオプション）で音節風の振幅を持つ WAV を返させ、実アプリと同じ音声解析経路を確認します。まばたきは音声と無関係に常時動きます（`layers/` を持つキャラのみ）
- **ffmpeg は WebP エンコーダ（`libwebp_anim`）付きのものが要ります。** Playwright 同梱の ffmpeg は最小構成で WebP を持たないため使えません（`SHOOT_FFMPEG` に指定すると入力オープンで失敗します）

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
| `SHOOT_README_GALLERY` | `1`ならキャラクター一覧を同じ撮影経路の生PNGからREADME用6秒・480px・16fpsとして`docs/images/`へ直接書き出す。引数なしの全件実行では合計20MB、最大2MBの上限も検査する |
| `SHOOT_SITE_GALLERY` | `1`ならキャラクター一覧を同じ撮影経路の生PNGからLP用6秒・480px・16fpsとして`distribution/site/assets/`へ直接書き出す。引数なしの全件実行では容量上限も検査する |
| `SHOOT_DEBUG_MOTION` | `1` でキャラクター一覧の撮影中、コマごとに実際に適用された `data-motion` を出力する。モーション巡回を足したのに切り替わっていない、という取り違えを防ぐ |
| `SHOOT_CHARACTER_MATRIX` | `1` ならPNG系8キャラ×7表示形式を `temp/character-layout-matrix/` へ撮影し、初回表示・manifest・待機／全件プリロード時刻、描画範囲、局所二次動作、一覧画像を作る。閾値は `performance-baseline.json`、`SHOOT_PERF_STRICT=1` なら超過を失敗扱いにする。ffmpeg不要 |
| `SHOOT_CHARACTER_IDS` | `SHOOT_CHARACTER_MATRIX=1` と併用し、`koharu,sena,noa` のように撮影対象だけを絞る |
| `SHOOT_LAYOUT_IDS` | `SHOOT_CHARACTER_MATRIX=1` または `SHOOT_LIVE2D_MATRIX=1` と併用し、`stream,pet` のように撮影する表示形式だけを絞る |
| `SHOOT_LAYOUT_SIZE` | `SHOOT_CHARACTER_MATRIX=1` と併用し、`pet=520x430,mascot=300x480` のように形式ごとの撮影寸法を上書きする。窓を狭めたときも回答操作が1段のままか、フキダシが潰れていないかを確認する用 |
| `SHOOT_LAYOUT_EDITOR` | `1` ならMoveable編集モードのドラッグ、拡大縮小、ウィンドウドラッグ抑止、保存、再読込復元、既定へ戻すを検査する |
| `SHOOT_INTERACTION_MATRIX` | `1` なら麦／ひより×7形式で起動遷移、入力欄、ボタン、縮小後の画面内収まり、描画ループの休止・再開、視線イベントのフレーム合流を `temp/layout-interaction-matrix/` へ検査する |
| `SHOOT_LIVE2D_MATRIX` | `1` なら導入済み全Live2Dモデル×7形式のSDK、口パラメータ、canvas表示を `temp/live2d-layout-matrix/` へ検査する |
| `SHOOT_LIVE2D_IDS` | `SHOOT_LIVE2D_MATRIX=1` と併用し、`hiyori` のようにLive2D検査対象だけを絞る |
| `SHOOT_DEBUG_ONBOARDING` | `1` なら初回セットアップの追加機能ステップを `temp/onboarding-check/` へ撮影する |

引数に画像名を並べると、その絵だけを撮り直せます。全件撮影は時間がかかるため、普段はこちらを使います。

```bash
node tools/screenshots/shoot.mjs composer-light.webp
```

README用キャラクター動画を全件再生成する場合は、先に全キャラを`ui/assets/characters/`へ同期してから実行します。

```powershell
cargo xtask sync-characters
$env:SHOOT_README_GALLERY = "1"
node tools/screenshots/shoot.mjs
Remove-Item Env:SHOOT_README_GALLERY
```

LP用も同じ正規入口を使い、出力先だけを切り替えます。

```powershell
cargo xtask sync-characters
$env:SHOOT_SITE_GALLERY = "1"
node tools/screenshots/shoot.mjs
Remove-Item Env:SHOOT_SITE_GALLERY
```

## 撮れる絵

| ファイル | 内容 | 何が動くか |
|---|---|---|
| `main-light.webp` / `main-dark.webp` | メイン画面（ライト / ダーク） | 文字送りと、考え中 → 返答のキャラクターモーション |
| `composer-light.webp` | 入力欄とクイック操作 | 次の依頼を 1 文字ずつ打ち込むところ |
| `fullscreen-light.webp` | 全画面フキダシ（履歴の検索・コピー・再利用・書き出し） | 返答が履歴へ積まれて末尾までスクロールするところ |
| `live2d-light.webp` / `live2d-dark.webp` | Live2D 描画（ひより、ライト / ダーク） | Cubism ランタイムのまばたき・モーション・口パク |
| `character-<キャラID>.webp` | README の通常表示一覧（標準ペット形式900×600、胸上構図、2倍密度描画、6秒・16fps・480px幅） | 音声同期の口パク → 考え中 → 待機。対応キャラは録画中の口開閉も自動検査する |
| `character-<キャラID>-stream.webp` | README の配信表示一覧（フルHD 1920×1080、ネイティブ描画、6秒・16fps・480px幅） | 通常表示と同じ巡回を、背景 → PICO STUDIO → キャラ → 字幕・操作UIの実レイヤー順で撮る |
| `distribution/site/assets/character-*.webp` | LP用一覧（上記と同じ撮影経路から6秒・480px・16fpsで直接生成） | 音声同期の口パクを十分見せてから、考え中 → 待機へ戻る。表示スロット向けに尺・解像度・フレームレートを分ける |
| `layout-shot-*.webp` | 7種類の表示形式（設定プレビュー兼用） | 各形式の実際の既定寸法から1フレームを生成 |

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
  跳ね上がり、数百コマの書き出しが極端に長くなります。サイズはほとんど変わりません
- 書き出し前に切り出し元の実ピクセル幅が出力幅以上か検査し、引き伸ばしになる指定は停止します。書き出し後もWebPヘッダーの実寸が指定幅と一致しなければ失敗します
- 論理幅が出力幅に届かない小型形式（マスコット360px、ウィジェット440px、チャット520px）は `deviceScaleFactor: 2` で描画してから縮小します。等倍のままだと上の検査で止まります

## Live2D の絵について

Live2D の絵は、ランタイムとモデルが未取得だと**スキップされます**（他の絵は撮れます）。

```bash
cargo xtask setup live2d   # 先にこれを実行する
```

必要なのは `ui/vendor/live2d/{live2dcubismcore-5.3.min.js,live2dcubismcore-compat.min.js,pixi.min.js,cubism4.min.js}` と
`ui/assets/characters/hiyori/live2d/` です。いずれも再配布条件があるためリポジトリには含めていません
（`.gitignore` 済み）。

Live2D撮影は、ひよりのCubism 5／4モデルを個別に読み込んだあと、tickerを止めて同一ポーズの
口元だけを閉口・開口比較します。その後、実際のテストWAVをWebAudio解析へ通して録画するため、
身体モーションだけを口パク成功と誤認しません。

起動画面だけを確認する場合は `SHOOT_DEBUG_BOOT=1` を指定する。保存形式が小型でも共通画面に
なることは `SHOOT_BOOT_LAYOUT=chat`（または `bar`）、キャラは
`SHOOT_BOOT_CHARACTER=hiyori` のように指定し、生成された `temp/boot-check/` を確認する。

> [!NOTE]
> **Cubism Core（5.3用／互換用）は Live2D 公式サイトからのみ配布されています。**
> npm にも GitHub にも正規版はありません（公式の `Live2D/CubismWebSamples` も Core は含まず、
> README で公式サイトからのコピーを指示しています）。ネットワークが `cubism.live2d.com` へ
> 到達できない環境では取得できず、Live2D の絵は撮れません。

## 撮り直しについて

撮るたびにファイル内容が変わります（履歴の時刻表示など）。見た目が変わっていないならコミットせず
`git checkout -- docs/images/` で戻してください。差分が大きいだけで中身は同じです。

> [!NOTE]
> **`ffmpeg` はアニメーション WebP を読めません**（書き出し専用。`-i` に渡すと
> `image data not found` で失敗します）。中身を確認したいときはブラウザで開くか、
> libwebp の `anim_dump` / `webpmux -get frame N` を使ってください。
