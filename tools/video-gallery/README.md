# ローカル動画比較

README / LP 用に生成済みのアニメーション WebP を、通常表示と配信表示で並べる開発QA専用画面です。製品UIやインストーラーには含めません。

リポジトリ直下をHTTP配信して開きます。

```powershell
python -m http.server 8770 --bind 127.0.0.1 --directory C:\00_PG\20_PicoAgent
```

`http://127.0.0.1:8770/tools/video-gallery/` を開き、カードをクリックすると拡大できます。

動画一覧の「キャラクター画像パーツを見る」から `parts.html` へ移動できます。正本の `characters/index.json` と各 `character.json` を読み、プレビュー、起動画像、表情レイヤー、連番フレーム、Live2Dテクスチャをキャラ別・種類別に絞り込んで原寸確認できます。存在しない慣例パスはHEAD検査で一覧から外すため、キャラごとの構成差をそのまま確認できます。

画面の読込、一覧からの遷移、絞り込み、原寸ダイアログをヘッドレスChromiumで検査し、`temp/video-gallery-parts.png` を撮る場合:

```powershell
$env:PLAYWRIGHT_MODULE = "$(npm root -g)\playwright\index.mjs"
node tools/video-gallery/check.mjs
```
