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

## THA3で1枚絵から自作キャラを追加する

[Talking Head Anime 3（THA3）](https://github.com/pkhungurn/talking-head-anime-3-demo)を使う経路では、1枚の立ち絵から必須5差分だけでなく、待機・呼吸・左右揺れ・思考・会話・落胆を含む**17キーポーズ**を作る。続けて既存の `build-character-animations.py` が42枚の実行用連番へ変換し、キャラ一覧と配信コピーにも登録する。

```powershell
cargo xtask setup tha3
python scripts/build-tha3-layers.py my-character --input C:\images\source.png --name "マイキャラ"
```

GUIでは右クリック →「設定…」→「キャラクター」→「1枚絵からキャラクターを作成（THA3）」を使う。画像選択、ID・表示名、進捗、キャンセルを画面内で扱い、生成後は共通のキャラクターパック検証を通ったものだけが追加一覧へ入る。コマンド版はリポジトリ正本へ追加する制作経路として引き続き利用できる。

- 入力は512x512のRGBA PNG。背景はalpha=0、1人の人型キャラを正面直立で全身配置し、頭は上半分中央の128x128付近、手は頭から離す。機械検査できない正面向き・人数・手の位置は実行前に目視確認する。
- Python・PyTorch・モデルはPicoAgent専用領域へ準備され、グローバルPythonや既存環境へ干渉しない。WindowsはCUDA対応NVIDIA GPU（VRAM 4GB以上）を使い、VRAM量に合わせて3モデルから自動選択する。Apple Silicon MacはMPS、Intel Macは対象外。これらを読み込むのは制作時だけで、通常動作と生成済みキャラ表示には使わない。
- 口は `aaa` / `iii` / `uuu` / `eee` / `ooo` / `delta` の候補から閉じ口との差が最大のものを自動選択する。差が見えない小さなデフォルメ口だけ自動補正し、基準ポーズの被写体範囲を使って全レイヤーを445×900へ統一するため、素材ごとの手作業は不要。
- 生成済みの左右思考レイヤーは、待機中のランダム視線とカーソル追従にも使う。読み上げ中は音量連動の口パク、待機中はまばたきと視線を優先する。
- `--force`なしでは既存の `layers/*.png` を上書きしない。生成途中はリポジトリ直下の `temp/` だけを使う。
- 新規IDでは `source.png`、`preview.png`、`character.json`、連番、一覧登録まで作る。通常は表示名に「（自作）」、マニフェストには `generator.type=tha3` を記録する。同じ表示名でもIDが異なれば履歴・関係性を含めて別キャラクターになり、説明と生成元情報で識別できる。人格・挨拶・声は生成後に `character.json` で調整する。
- THA3自作キャラを消す場合は `python scripts/build-tha3-layers.py my-character --remove`。組み込み・手動管理キャラは拒否し、対象フォルダは物理削除せずOSのゴミ箱へ移す。
- ひよりのLive2Dは毎フレーム連続変形するため、PNG連番で完全に同じにはならない。この経路は既存5枚構成より頭・身体・呼吸の動きを増やす経路で、本物のLive2D化は後半の手順を使う。
- 既存PNGキャラの比較試作は元IDを上書きせず別IDで作る。りん・まいは動きが増える可能性がある一方、再生成で顔や輪郭が変わる場合がある。Live2Dキャラと既知のむぎ素材は自動変換しない。

THA3コードはMIT License、学習済みモデルはCC BY 4.0。作者Pramook Khungurn（pkhungurn）とTHA3プロジェクトへの帰属表示が必要。コードと3モデルは `src-tauri/resources/bundled/tha3/` へ取得して製品に含める（実体はGit管理外）。WindowsのPython／CUDA版PyTorchはインストール時に同じ専用領域へ取得し、Apple Silicon MacではOS別ランタイムを同梱する。

## アニメーション生成

全キャラを生成する:

```powershell
python scripts/build-character-animations.py -- --version 2026-06-28-motion-v1
cargo xtask sync-characters
```

一部キャラだけ生成する:

```powershell
python scripts/build-character-animations.py -- rin mai --version 2026-06-28-rin-mai-v1
cargo xtask sync-characters
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
├── preview.png / preview.jpg # 設定画面アイコン・ローディング画面用の静止画（boot.sleeping に指定）
└── live2d/                  # モデル一式（.model3.json・.moc3・テクスチャ・モーション）
```

- `layers/` と `animations/` は不要。まばたき・口パク・モーションはモデル定義が担当する。
- `build-character-animations.py` は Live2D キャラをスキップする。
- サンプルキャラ（Live2D 公式サンプルモデル）は `cargo xtask setup live2d` が `live2d/` へ
  モデルを取得する。再配布条件があるためモデル実体は Git 管理外。
  `preview.jpg` はローディング画面と設定画面アイコンに使い、取得できなかった場合は
  `_preview-placeholder.png` で代替する。
  - **Free Material License**（公式リポジトリ CubismWebSamples の `LICENSE.md` に明記）:
    `hiyori` / `haru` / `mao` / `ren` / `rice` / `wanko`
  - **上記リストに無い別配布**: `epsilon`（`cubism.live2d.com` の epsilon_free）
  - いずれもモデルごとに Live2D のサンプルモデル利用条件への同意が必要。売上高が一定規模を
    超える事業者は Cubism SDK リリースライセンスも必要になる。判断は利用者側で行うこと。
- モデルごとに `motions` / `lipSyncParam` は個体差がある。実際の `model3.json` の
  `FileReferences.Motions` と `Groups: LipSync` を見て割り当てる
  （例: `Mao` の口は `ParamA`、`Wanko` は旧命名、
  `Rice` は口のパラメータ自体が無いので `"lipSync": false` にする）。
  `scale` / `offsetY` はモデルごとに実機で合わせる（同じ値では収まらない）。
- 自作・購入した Live2D モデルを使う場合も同じ構成で `live2d/` に `.model3.json` 一式を置き、
  `motions` にそのモデルのモーショングループ名（例: `Idle` / `TapBody`）を割り当てる。

### 同じ絵で PNG 版と Live2D 版を分ける（別キャラ運用）

**会話履歴と関係の記録はキャラ ID ごと**（`history-<id>.json` / `relationship-<id>.json`）なので、
「PNG 版と Live2D 版で会話も分けたい」場合は variant ではなく**別キャラとして登録する**。
同梱の `mugi`（PNG）と `mugi-live2d`（Live2D、制作中）がその形。

- 画像は複製せず、`../<元キャラ>/…` の相対パスで参照する（むぎのフレームだけで 9.4MB あるため）。
  `animations[].path` と `boot.sleeping` はどちらも URL として解決されるので相対で書ける。
- Live2D 側にも `animations` を残しておくと、**モデルが未書き出し・壊れている間は PNG 表示へ退避**できる。
  `animations` を持たせないと、モデルが読めない時点でそのキャラ自体を読み込めなくなる。
- `persona` は別ファイルへ複製する形になる。**片方だけ直すと食い違う**ので、人格を変えるときは両方直す。

### 同じキャラのまま「絵版」と「Live2D 版」を持つ（variant 運用）

`variants` の項目に `live2d` を書くと、キャラを増やさずに見た目だけ切り替えられる
（切替は ♡ の記録カード内の「雰囲気」セレクタ。選択は端末ごとに記憶する）。
人格・挨拶・声はキャラ本体の定義を共有するので、二重管理にならない。

```jsonc
"variants": [
  { "id": "default", "name": "いつもの姿" },              // 既存の連番フレーム表示
  { "id": "live2d",  "name": "Live2D 版",                 // Cubism ランタイムで描画
    "live2d": {
      "model": "live2d/Mugi.model3.json",
      "motions": { "idle": "Idle", "thinking": "TapBody", "talking": "TapBody", "sad": "Idle" },
      "lipSyncParam": "ParamMouthOpenY",
      "scale": 3.2, "offsetY": 645
    } }
]
```

- キャラ本体が Live2D の場合に、ある variant だけ絵に戻したいときは `"live2d": false` を書く。
- **モデルを書き出してから追記すること。** モデルが無い状態で variant を足すと、
  選んだときに初期化に失敗してフレーム表示へ戻る（通知は出るが無駄な失敗になる）。
- `scale` / `offsetY` はモデルごとに実機で合わせる。

### 既存キャラを Live2D 化するための PSD 生成

`layers/` の差分絵から、Cubism Editor へ読み込める土台 PSD を生成できる。

```powershell
pip install Pillow psd-tools
python scripts/build-live2d-psd.py            # layers/ を持つ全キャラ
python scripts/build-live2d-psd.py -- mugi    # 指定キャラのみ
```

生成先は `characters/<id>/live2d-src/<id>-live2d.psd`（Git 管理外・再生成可能）。
レイヤー構成は Live2D 公式サンプル「ひより」（Hiyori.cdi3.json）のパーツ構成に合わせた
15 グループ（前髪・横髪・まゆ毛・目・目玉・頬・鼻・口・顔・耳・首・腕A・腕B・体・後ろ髪）で、
各グループに base の該当領域の切り抜き（元位置に整列済み）が入る。ほかに
`variants/`（口・目の開閉差分。非表示）と `base_full`（全身原本。非表示）を収録する。

PSD を Cubism Editor で開いたあとの作業（手作業が必要）:

1. 各パーツグループの切り抜きから、そのパーツに属さない部分を消して輪郭を整える
   （`variants/` の差分絵は目・口の開閉形状の作画参考にする）。
2. パーツの重なりで隠れる部分（まぶたの奥・口の中など）を描き足す。
3. メッシュ・デフォーマ・パラメータ（`ParamEyeLOpen` / `ParamMouthOpenY` など）を設定する。
4. 組み込みモデル（`.moc3` + `.model3.json`）を書き出し、`characters/<id>/live2d/` に置いて
   `character.json` に `live2d` 定義を追加する（上記「Live2D キャラクター」参照）。
