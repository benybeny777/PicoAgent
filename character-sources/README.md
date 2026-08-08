# 同梱2Dキャラの制作正本

`mugi` / `rin` / `mai` / `rin-tha3` をPhotoshopで再仕上げするときに使う、処理前1倍PNGです。
製品実行時には使わず、`cargo xtask sync-characters` の配信対象にも入りません。

通常立ち絵を再生成する場合は、Photoshopを閉じてから次を実行します。

```powershell
powershell -ExecutionPolicy Bypass -File scripts/finish-2d-character-assets.ps1
uv run --with pillow python scripts/build-character-animations.py mugi rin mai rin-tha3 --compact-only
cargo xtask sync-characters
```

仕上げスクリプトはこのフォルダを入力、`characters/` を出力として扱います。全画像を `temp/` で
Photoshop処理・高精細化・透過検査してから製品側へ反映し、失敗時は処理前の製品画像へ戻します。
制作正本を直接上書きしないでください。
