# ローカル動画比較

README / LP 用に生成済みのアニメーション WebP を、通常表示と配信表示で並べる開発QA専用画面です。製品UIやインストーラーには含めません。

リポジトリ直下をHTTP配信して開きます。

```powershell
python -m http.server 8770 --bind 127.0.0.1 --directory C:\00_PG\20_PicoAgent
```

`http://127.0.0.1:8770/tools/video-gallery/` を開き、カードをクリックすると拡大できます。
