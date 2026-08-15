# キャラクター画像の準備手順

高精細な画像追加、表情・視線差分、Photoshop仕上げ、単一世代生成の再現手順は [キャラクター画像追加・更新手順](../docs/CHARACTER-ASSET-WORKFLOW.md) を正本とする。

PicoAgent のキャラクター画像は、各キャラフォルダを同じ構成にそろえて管理する。
正本は `characters/<キャラクターID>/` で、`ui/assets/characters/` は同期生成物。

低頭身2Dは上半身、長身・実写・Live2Dは胸全体がぎりぎり入る胸上を標準にする。素材は透過・余白つきで保持し、manifestの `bodyProportion`、`defaultFraming`、`streamFraming` で原画固有の余白だけを非破壊に吸収する。利用者が対象キャラへ保存した構図は既定値より優先する。チャット／バーの小型アイコンは共通CSSで首上を切り出す。

組み込みキャラの既定声は `character.json` の `voice` とバックエンドの
`default_character_tts_voice` を同じモデルUUID・話者UUID・スタイル名にそろえる。

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

同梱PNGは `motionProfile=full` を持ち、`base`、半目／閉じ目、閉口／開口などの表情レイヤーから単一世代の84コマ（待機24・考え中24・会話24・悲しみ12）を生成する。口パクは音声ありなら声量、ミュート中なら返答状態へ同期する。物理演算機能は使わず、髪・服・身体の遅れは同寸のキーポーズへ事前描画する。矩形レイヤーや色面は合成せず、透明輪郭を維持する。

## 画像作成ルール

- `preview.png`、`layers/*.png` は同じキャンバスサイズ、同じ全身位置にする。
- 表情差分は、全身や服を描き直さず、目と口だけを変える。
- まばたきや口パクの差分を AI 生成する場合も、最終的には `layers/` に同じサイズで配置する。
- `boot/sleeping.png` は横向きに寝ている構図にし、むぎ・りん・まいで同じローディング画面に見える比率にする。
- 背景は透過 PNG にする。AI 生成で緑背景を使った場合は chroma key 除去後の PNG だけを `boot/` や `layers/` に置く。

## バストアップ1枚から簡易通常2Dキャラを追加する

背景付き画像から簡易パックを作る場合は、制作時だけ `rembg` を使う次の経路を利用する。形式正本は `portrait-2d-format.json`、manifestひな形は `.agents/skills/rebuild-picoagent-character/assets/portrait-2d-template/character.template.json`。物理演算は設定せず、同寸の透過連番でまばたき・口・視線・感情・髪や服の動きを画像どおり再生する。

```powershell
uv run --with pillow --with numpy --with rembg --with onnxruntime `
  python scripts/build-portrait-2d-character.py `
  --input temp/source.png --id my-character --name "マイキャラ" `
  --frames-dir temp/my-character-frames
```

```text
temp/my-character-frames/
├── idle/       # 待機、まばたき、自然な髪・服の揺れ
├── thinking/   # 左右視線、考え中の表情・姿勢
├── talking/    # 閉口／開口を交互に置いた会話連番
└── sad/        # 悲しみ・困り顔などの状態連番
```

- 入力は頭頂部と左右に余白があり、腰または胸元まで輪郭が分かる画像を使う。被写体が上端へ接している素材は、既存画素を引き伸ばして補わずエラーで停止する。
- 出力先に同じIDの `preview.png` がある場合は停止する。意図的に作り直す場合だけ `--force` を付ける。
- 連番はファイル名順に読み、すべて基準画像と同じ寸法でなければ停止する。透過済みPNGはそのまま使い、不透明背景なら基準画像と同じ `rembg` セッションで背景を抜く。補間、拡大、パーツ変形は行わない。
- 品質下限は `idle` / `thinking` / `talking` が各24コマ・固有12枚、`sad` が12コマ・固有8枚。開眼／半目／閉眼や閉口／中間／開口だけでなく、髪・服の遅れと戻りも別画像にする。同じ内容のPNGを複製しただけでは固有絵に数えず停止する。
- Nano Banana等を使う場合は、同一人物・同一構図・同一キャンバスを維持し、候補を元画像とは別名で保存する。まばたき、閉口／開口、左右視線、喜び／悲しみ、髪・服の小さな動きだけを変え、超解像やhires-fixには使わない。
- `temp/qa-<ID>.png` の市松・暗色・白・緑の4背景を目視し、ハロー・矩形・色面・太い外周影があれば採用しない。
- 生成後は `cargo xtask sync-characters` を実行し、開発画面で表示を確認する。販売版の同梱対象は別の製品判断であり、このスクリプトは配布許可リストを変更しない。

## THA3で1枚絵から自作キャラを追加する

[Talking Head Anime 3（THA3）](https://github.com/pkhungurn/talking-head-anime-3-demo)を使う経路では、1枚の立ち絵から必須5差分だけでなく、待機・呼吸・左右揺れ・思考・会話・落胆を含む**17キーポーズ**を作る。続けて既存の `build-character-animations.py` が42枚の実行用連番へ変換し、キャラ一覧と配信コピーにも登録する。

```powershell
cargo xtask setup tha3
python scripts/build-tha3-layers.py my-character --input C:\images\source.png --name "マイキャラ"
```

GUIでは右クリック →「設定…」→「キャラクター」→「1枚絵からキャラクターを作成（THA3）」を使う。画像選択、ID・表示名、進捗、キャンセルを画面内で扱い、生成後は共通のキャラクターパック検証を通ったものだけが追加一覧へ入る。コマンド版はリポジトリ正本へ追加する制作経路として引き続き利用できる。

- 入力は512x512のRGBA PNG。背景はalpha=0、1人の人型キャラを正面直立で全身配置し、頭は上半分中央の128x128付近、手は頭から離す。機械検査できない正面向き・人数・手の位置は実行前に目視確認する。
- Python・PyTorch・モデルはPicoAgent専用領域へ準備され、グローバルPythonや既存環境へ干渉しない。WindowsはCUDA対応NVIDIA GPU（VRAM 4GB以上）を使い、VRAM量に合わせて3モデルから自動選択する。Apple Silicon MacはMPS、Intel Macは対象外。これらを読み込むのは制作時だけで、通常動作と生成済みキャラ表示には使わない。
- 口は `aaa` / `iii` / `uuu` / `eee` / `ooo` / `delta` の候補から、顔の口元だけを見比べて閉じ口との差が最大のものを自動選択する。差が見えない小さなデフォルメ口だけ、同じ比較窓の差分重心（差分が無ければ窓の中心）へ小さな口を合成して補正する。
- 全レイヤーは基準ポーズの被写体範囲（頭・身体の振れ幅ぶん少し広げた箱）で切り出し、縦横比を保ったまま890×1800の下端中央へ揃える。位置合わせは常に基準ポーズ基準なので、一部レイヤーだけ作り直しても他とずれない。
- THA3の元出力は512×512で、全身の実画素数には限界がある。890×1800へ拡大しただけでは詳細は増えないため、同梱の `koharu` / `sena` / `noa` は高精細な `preview.png` と局所的な半目・閉眼・口差分を正本にし、`animationQualityMode=sharp-expression` でぼけたTHA3全身ポーズへ切り替えない。半目は閉眼の透明度変更ではなく専用作画を使う。`scripts/build-sharp-expression-layers.py` が基本5レイヤーと連番を再生成し、`scripts/finish-sharp-expression-assets.ps1` がPhotoshop「ディテールを保持 2.0」で1780×3600へ仕上げて元アルファを再適用する。胸元の二次動作は広いフェザーと小さい縦変形だけを使い、本体との境界を見せない。一般のTHA3自作キャラは従来どおり890×1800へ正規化する。
- 生成済みの左右思考レイヤーは、待機中のランダム視線とカーソル追従にも使う。読み上げ中は音量連動の口パク、待機中はまばたきと視線を優先する（よそ見中はまばたきを見送り、正面顔へ一瞬戻らないようにする）。
- `--force`なしでは既存の `layers/*.png` を上書きしない。ただし残す既存レイヤーの寸法が現在の出力寸法と違う場合は、混ざったまま連番アニメが破綻するのを避けるため停止して `--force` での作り直しを促す。生成途中はリポジトリ直下の `temp/` だけを使う。
- 新規IDでは `source.png`、`preview.png`、`character.json`、連番、一覧登録まで作る。通常は表示名に「（自作）」、マニフェストには `generator.type=tha3` を記録する。同じ表示名でもIDが異なれば履歴・関係性を含めて別キャラクターになり、説明と生成元情報で識別できる。人格・挨拶・声は生成後に `character.json` で調整する。
- THA3自作キャラを消す場合は `python scripts/build-tha3-layers.py my-character --remove`。組み込み・手動管理キャラは拒否し、対象フォルダは物理削除せずOSのゴミ箱へ移す。
- ひよりのLive2Dは毎フレーム連続変形するため、PNG連番で完全に同じにはならない。この経路は既存5枚構成より頭・身体・呼吸の動きを増やす経路で、本物のLive2D化は後半の手順を使う。
- 既存PNGキャラの比較試作は元IDを上書きせず別IDで作る。りん・まいは動きが増える可能性がある一方、再生成で顔や輪郭が変わる場合がある。Live2Dキャラと既知のむぎ素材は自動変換しない。

THA3コードはMIT License、学習済みモデルはCC BY 4.0。作者Pramook Khungurn（pkhungurn）とTHA3プロジェクトへの帰属表示が必要。コードと3モデルは `src-tauri/resources/bundled/tha3/` へ取得して製品に含める（実体はGit管理外）。WindowsのPython／CUDA版PyTorchはインストール時に同じ専用領域へ取得し、Apple Silicon MacではOS別ランタイムを同梱する。

## アニメーション生成

全キャラを生成する:

```powershell
uv run --with pillow python scripts/build-character-animations.py --version 2026-06-28-motion-v1
cargo xtask sync-characters
```

一部キャラだけ生成する:

```powershell
uv run --with pillow python scripts/build-character-animations.py -- rin mai --version 2026-06-28-rin-mai-v1
cargo xtask sync-characters
```

このスクリプトは内部で次の42枚を生成したあと、同一内容をハッシュでまとめる。

- `animations/idle/00.png` 〜 `11.png`
- `animations/running/00.png` 〜 `11.png`
- `animations/review/00.png` 〜 `11.png`
- `animations/failed/00.png` 〜 `05.png`
- 固有画像だけを置く `frames/`（既存 `layers/` / `preview.png` と同じ画像はそれらを直接共有）
- `character.json` の `animations.*.framePaths`
- `--version` 指定時は `assetVersion`

最終パックには重複した `animations/` 連番を残さない。実行時のコマ数と順序は `framePaths` が保持する。
既存の連番だけを共有化する場合は `--compact-only` を使う。

```powershell
uv run --with pillow python scripts/build-character-animations.py mugi rin mai rin-tha3 --compact-only
```

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
- ランタイムは公式Coreを世代別に使い、SDK 5モデルはCore 6（Cubism 5.3形式）、SDK 4モデルはCore 5（互換用）で読み込む。`cargo xtask setup live2d` が両方のSHA-256まで照合する。
- サンプルキャラ（Live2D 公式サンプルモデル）は自動取得・製品同梱しない。開発者が
  利用条件を確認して描画回帰に使う場合だけ `cargo xtask setup live2d-samples` が `live2d/` へ
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
  利用者向けには設定画面へ1体分のzip／フォルダを入れるだけで、この構成と
  `character.json` を自動作成する。
- Cubism 4／5の両方を書き出して選べるようにする場合は、`live2d.models` を
  `[{ "sdk":"5", "path":"live2d/sdk5/model.model3.json" }, { "sdk":"4", "path":"live2d/sdk4/model.model3.json" }]`
  の形で定義する。設定画面で自動／4固定／5固定を選べる。単一モデルだけのパックでは `live2d.model` のままでよい。

### 同じ絵で PNG 版と Live2D 版を分ける（別キャラ運用）

**会話履歴と関係の記録はキャラ ID ごと**（`history-<id>.json` / `relationship-<id>.json`）なので、
「PNG 版と Live2D 版で会話も分けたい」場合は variant ではなく**別キャラとして登録する**。同梱サンプルでは、2D表示例に `mugi`、Live2D表示例に `hiyori` を使う。

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
