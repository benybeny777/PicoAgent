# キャラクター画像の準備手順

PicoAgent のキャラクター画像は、各キャラフォルダを同じ構成にそろえて管理する。
正本は `characters/<キャラクターID>/` で、`ui/assets/characters/` は同期生成物。

Live2D キャラ（`character.json` に `live2d` を持つキャラ）は本ページの連番フレーム構成を使わない。
末尾の「Live2D キャラクター」を参照。

## 必須ファイル

```text
characters/<id>/
├── character.json
├── preview.png              # 通常立ち絵
├── boot/
│   └── sleeping.png         # ローディング画面用の横向き寝姿
└── layers/
    ├── base.png             # 通常表情。通常は preview.png と同じ
    ├── blink_half.png       # 半目または閉じ目へ向かう中間表情
    ├── blink_closed.png     # 目を閉じた表情
    ├── mouth_closed.png     # 口を閉じた表情。通常は base.png と同じ
    └── mouth_open.png       # 会話中の口開き表情
```

`animations/` 配下は `scripts/build-character-animations.py` で生成する。手で直接直さない。
`source.png` などの元画像は任意の制作素材。実行時に使わないPSD・確認用シート・生成途中画像は、必要がなくなったらリポジトリへ残さない。

## 画像作成ルール

- `preview.png`、`layers/*.png` は同じキャンバスサイズ、同じ全身位置にする。
- 表情差分は、全身や服を描き直さず、目と口だけを変える。
- まばたきや口パクの差分を AI 生成する場合も、最終的には `layers/` に同じサイズで配置する。
- `boot/sleeping.png` は横向きに寝ている構図にし、むぎ・りん・まいで同じローディング画面に見える比率にする。
- 背景は透過 PNG にする。AI 生成で緑背景を使った場合は chroma key 除去後の PNG だけを `boot/` や `layers/` に置く。

## アニメーション生成

全キャラを生成する:

```powershell
npm run build:character-animations -- --version 2026-06-28-motion-v1
npm run sync:characters
```

一部キャラだけ生成する:

```powershell
npm run build:character-animations -- rin mai --version 2026-06-28-rin-mai-v1
npm run sync:characters
```

このスクリプトは以下を生成する。

- `animations/idle/00.png` 〜 `11.png`
- `animations/running/00.png` 〜 `11.png`
- `animations/review/00.png` 〜 `11.png`
- `animations/failed/00.png` 〜 `05.png`
- `character.json` の `animations`
- `--version` 指定時は `assetVersion`

## ローディング画像の確認

各キャラは `character.json` に次を持つ。

```json
{
  "boot": { "sleeping": "boot/sleeping.png" }
}
```

`boot/sleeping.png` が無い場合、`build-character-animations.py` は警告を出す。
警告が出たキャラは、ローディング画面で通常立ち絵に戻ってしまうため、配布前に必ず寝姿画像を追加する。

## Live2D キャラクター

`character.json` に `live2d` を定義すると、連番フレームの代わりに Cubism ランタイムでモデルを描画する。

```text
characters/<id>/
├── character.json           # live2d.model / motions / lipSyncParam / scale / offsetY を定義
├── preview.png              # 設定画面アイコン・ローディング画面用の静止画（boot.sleeping に指定）
└── live2d/                  # モデル一式（.model3.json・.moc3・テクスチャ・モーション）
```

- `layers/` と `animations/` は不要。まばたき・口パク・モーションはモデル定義が担当する。
- `build-character-animations.py` は Live2D キャラをスキップする。
- サンプルの `hiyori`（Live2D 公式サンプルモデル）は `npm run setup:live2d` が `live2d/` へモデルを取得する。
  再配布条件があるためモデル実体は Git 管理外（`preview.png` は差し替え可能なプレースホルダ）。
- 自作・購入した Live2D モデルを使う場合も同じ構成で `live2d/` に `.model3.json` 一式を置き、
  `motions` にそのモデルのモーショングループ名（例: `Idle` / `TapBody`）を割り当てる。

### 既存キャラを Live2D 化するための PSD 生成

`layers/` の差分絵から、Cubism Editor へ読み込める土台 PSD を生成できる。

```powershell
pip install Pillow psd-tools
npm run build:live2d-psd            # layers/ を持つ全キャラ
npm run build:live2d-psd -- mugi    # 指定キャラのみ
```

生成先は `characters/<id>/live2d-src/<id>-live2d.psd`（Git 管理外・再生成可能）。
レイヤー構成は `base`（全身）＋ `variants/`（mouth_open・mouth_closed・blink_half・blink_closed。
初期状態は非表示）。差分絵が目・口だけの局所差分として検出できた場合は
`eye/` `mouth/` グループに切り抜きパーツとして収録される。

PSD を Cubism Editor で開いたあとの作業（手作業が必要）:

1. `base` から顔・前髪・後髪・眉・目・口・体・腕などのパーツを切り分ける
   （`variants/` の差分絵は目・口の開閉形状の作画参考にする）。
2. パーツの重なりで隠れる部分（まぶたの奥・口の中など）を描き足す。
3. メッシュ・デフォーマ・パラメータ（`ParamEyeLOpen` / `ParamMouthOpenY` など）を設定する。
4. 組み込みモデル（`.moc3` + `.model3.json`）を書き出し、`characters/<id>/live2d/` に置いて
   `character.json` に `live2d` 定義を追加する（上記「Live2D キャラクター」参照）。
