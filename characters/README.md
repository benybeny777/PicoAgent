# キャラクター画像の準備手順

PicoAgent のキャラクター画像は、各キャラフォルダを同じ構成にそろえて管理する。
正本は `characters/<キャラクターID>/` で、`ui/assets/characters/` は同期生成物。

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
