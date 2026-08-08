# 同梱2Dキャラの制作正本

`mugi` / `rin` / `mai` / `rin-tha3` をPhotoshopで再仕上げするときに使う、処理前1倍PNGです。
同じ画像を何十枚もGitへ置かないよう、`index.json` の配置表と `blobs/` の内容ハッシュ別PNGを正本にします。
製品実行時には使わず、`cargo xtask sync-characters` の配信対象にも入りません。

通常立ち絵を再生成する場合は、Photoshopを閉じてから次を実行します。

```powershell
powershell -ExecutionPolicy Bypass -File scripts/finish-2d-character-assets.ps1
uv run --with pillow python scripts/build-character-animations.py mugi rin mai rin-tha3 --compact-only
cargo xtask sync-characters
```

仕上げスクリプトは共有ストアを自動で `temp/` へ展開し、`characters/` を出力として扱います。全画像を `temp/` で
Photoshop処理・高精細化・透過検査してから製品側へ反映し、失敗時は処理前の製品画像へ戻します。
素材そのものを編集するときだけ一時展開し、確認後に共有ストアへ戻します。

```powershell
uv run python scripts/manage-character-sources.py expand --output temp/character-sources-expanded
# temp/character-sources-expanded 内を編集・確認
uv run python scripts/manage-character-sources.py compact --input temp/character-sources-expanded
```

`compact` は4キャラをすべて検査してから共有ストアを置き換えます。制作正本のblobを直接上書きしないでください。
