# メッシュ変形リグ（独自2.5D）設計

同梱の通常2Dキャラを、差分絵の差し替えではなく連続パラメータによるメッシュ変形で動かすための設計書。現行仕様は [SPEC.md](../SPEC.md)、素材の制作手順は [CHARACTER-ASSET-WORKFLOW.md](CHARACTER-ASSET-WORKFLOW.md)、未完了条件だけは [TASKS.md](TASKS.md) を正本とする。

最初の対象はセナとして検証し、その後にLive2D以外の同梱通常2D 8体へ適用した。各体は原画座標の胴、後ろ髪、左右腕、顔、前髪を制作正本として持つ。

2026年8月27日に第1実装、2026年8月28日に左右視線と`layered-grid-v2`を完成した。PIXIの独立取得、半目・閉眼・開口・左右視線・感情の局所補間、連続した頭部・左右腕・呼吸・髪の遅れ、既存差分絵への安全な退避を8体へ適用している。むぎと実写は1画素の相対移動でも髪の分離線が見えるため髪を頭部へ追従させ、まいは手が顔境界へ重なるため顔の平行移動だけを固定する。

2026年9月7日に、LocalVTuberStudioの現行`lvs-anime25d-v1` schema 3を変換せず直接読む経路を追加した。これはLive2D対応を置き換えず、同じキャラクター選択・音声・描画枠へ並行して載る独自2.5D形式である。Cubism SDK・`.moc3`・Editorは不要で、PicoAgent既存のPIXI 6.5.10描画層を使う。旧PoCの`anime25d-v1` version 2も互換用に残す。

## LocalVTuberStudioの独自2.5D形式

正規入力はLocalVTuberStudioが出力した`schema_version: 3`、`profile: "lvs-anime25d-v1"`の`rig2d/rig.json`と`rig2d/parts/*.png`である。PicoAgent側で中間リグへ変換せず、`scene_graph`、実測した`eye_aperture`、`lip_seam`、`texture_box`を直接検証・描画する。PSDを実行時に解析せず、生成時に原寸で切り詰めたPNGパーツだけを読む。

同梱例`female-a-lvs`は、LocalVTuberStudioで利用者が合格とした女性Aの`rig2d/`をそのまま保持する。LocalVTuberStudio側は`character.json`と`rig2d/`を1つの自己完結パックとして出力し、PicoAgent側はそのフォルダまたはzipをキャラ登録から取り込むだけとする。キャラIDごとの座標変換・描画分岐・素材変換は追加しない。`character.json`の`meshRig.rig`を`rig2d/rig.json`へ向け、素材URLはLocalVTuberStudio正規形`/assets/rig2d/parts/<名前>.png`のまま保持する。PicoAgentはパック内の`rig2d/parts/`へ安全に解決する。

口はAivisの`audio_query`が返す各モーラの母音と長さを音声と同じ時間軸へ変換し、声量包絡線の`ParamMouthOpenY`と母音別の`ParamMouthForm`を同時に渡す。タイムラインが得られない音声だけは従来どおり声量連動の中立口形へ戻る。開眼は解析された原画の上下まぶた曲線を中央から18%広げ、細目の原画でも待機時を半目にしない。閉眼曲線と左右別の補間は維持する。瞬き、呼吸、小角度、髪の遅れは従来のメッシュリグと同じ描画ループで処理する。呼吸は全レイヤーの一体移動に加え、胴レイヤーの上胸部だけを端で0になる重みで約2.4原画px上下させ、首・袖・下腹の境界を保ったまま胸元の動きを読めるようにする。待機に加え、会話は小刻みな上下動、考え中は首傾げ、よろこびは弾む全身動作、困りは頭と体を沈める動作を使う。全身を足元支点で動かしてパーツ間の継ぎ目を保つ。`ParamArmClose`は左右の腕を肩支点から体へ寄せる比較調整だが、残余レイヤーに元の腕境界がある素材では残像になるため、LocalVTuberStudio形式の既定は原画保持の0とする。

開発中のパックはリポジトリ直下をHTTP配信し、次でPicoAgent本番と同じ描画を確認できる。

```powershell
uv run python -m http.server 8776 --bind 0.0.0.0 --directory .
```

`http://127.0.0.1:8776/tools/mesh-rig-preview/?character=female-a`を開く。画面は女性A、むぎ、実写テストを選択でき、立ち絵原画と実描画を並べる。独自2.5Dでは左右別閉眼、視線の上下左右、閉口と5母音、通常・笑顔・困り顔・考え顔・寝顔、待機・会話・考え中・よろこび・困り・聞く・起きる・眠る、腕寄せを個別または自動で確認できる。首境界を保つため、頭・首・襟を別々に平行移動・回転させず、全レイヤーへ同じ呼吸と全身動作を適用する。目・口を表情に応じて局所変形する。旧`layered-grid-v2`の実写テストでは両目共通の瞬きと視線、既存の表情差分を確認し、非対応の左右別閉眼と腕寄せは無効表示にする。`view=full`を付けると腕の全身確認になる。市松・黒・白背景を切り替えられ、幅760 px以下では原画と実描画を縦並びにする。同じLANのスマートフォンからは`127.0.0.1`をこのPCのIPv4アドレスへ置き換える。`temp/`のパックと証跡はGitへ含めない。

女性Aは2026年9月7日にLocalVTuberStudio側でまばたき、口形、首・胴の接続、控えめな待機動作を再確認し、利用者が今回の品質として合格にした世代を採用した。旧Anime2.5DRig向けPoCパックは確認画面と同梱対象から外した。

## 分離原画形式

`layered-grid-v2` は `mesh-rig/source/parts/*.png` を制作正本とする。各PNGは `layers/mouth_closed.png` と同じ原画座標・RGBAで、`regions.json` の `separatedParts` に描画順、部位物理、隠れ部分の有無を定義する。生成時に全パーツの中立合成が基準画像と完全一致しない場合は停止する。

実行用には各パーツをlossless WebPへ変換し、透明余白だけを切り詰める。元画像の拡大補間、異なる寸法の混在、CSSによるキャラクター別倍率補正は行わない。`single-sheet-grid-v1` は既存パックの互換用として読み続ける。

## なぜ差分絵では届かないか

セナの `animations.idle` は `frames` 12・`fps` 3.2 で、`frameDurations` の大半が760ミリ秒の保持である。ユニークフレームは16枚しかない。これはパラパラ漫画であり、コマを増やしても素材容量と生成時間が増えるだけで滑らかさの上限は変わらない。

一方、リップシンクの `rigMouthLevel` は適応しきい値と非対称包絡線を通した60fpsの連続値としてすでに算出されている（`ui/renderer.js` の `attachLipSync`）。Live2D経路へは連続値のまま渡しているが、差分絵経路は差し替え先が2枚しか無いため段階へ落とすしかない。**信号は既にあり、受け取れる描画先が無いことが制約になっている。**

## Cubismを使わない理由と、この方式の上限

`.moc3` は Cubism Editor でしか生成できず、AIでも自動でも作れない。`scripts/build-live2d-psd.py` は PSD の雛形までは作るが、パーツ整形とリギングは人手に残る。また Live2D 経路は [TASKS.md](TASKS.md) の販売ゲートにある拡張アプリ審査を必要とするが、自前メッシュは PIXI（MIT）だけで完結するためその判定に触れない。

代償として、**頭の回転は横向きの描き起こしが無い範囲までしか振れない。** パーツごとの奥行き係数による視差とせん断で近似するため、可動域を左右±15度・上下±10度で固定する。これを超える演出は従来のフレームアニメ側で表現する。より広い可動域が要るキャラは Live2D 経路を選ぶ。

## 既存9レイヤーをキーフォームとして再利用する

この設計の要点。`layers/*.png` の9枚はすべて同じ1780×3600・同じ座標系で描かれた同一ポーズの差分であり、実差分がある部位をキーフォームとして使える。セナの`layers/gaze_left.png`と`gaze_right.png`は基準画像と画素単位で一致するため使わない。代わりに`mesh-rig/source/regions.json`で両目の瞳領域と最大16画素の移動量を指定し、`scripts/build-mesh-rig.py`が高精細な閉口正本から左右視線の局所キーフォームを生成する。低解像度素材の拡大や広い生成補完は行わない。

| パラメータ | 値 | 使うクロップ |
|---|---|---|
| `ParamEyeLOpen` / `ParamEyeROpen` | 1.0 / 0.5 / 0.0 | `base` / `blink_half` / `blink_closed` の目領域 |
| `ParamEyeBallX` | -1 / 0 / +1 | 高精細正本から生成した左視線 / 基準 / 右視線の局所キーフォーム |
| `ParamMouthOpenY` | 0 / 1 | `mouth_closed` / `mouth_open` の口領域 |
| `ParamMouthForm` | -1 / +1 | `expression_sad` / `expression_happy` の口・眉領域 |

旧2D素材の悲しみ差分が半目だけで困り顔として成立しない場合は、`mesh-rig/source/regions.json`へ
`expressionSadMode: "disabled"`を指定する。生成された`rig.json`へ同値を引き継ぎ、通常の開眼を保つ。
未指定時は既存互換の`layer`とし、寝顔の閉眼は悲しみ差分ではなく`blink_closed`で描画する。
生成時はリグJSONと全パーツの内容から`meshRig.assetVersion`を作り、通常2Dフレームとは独立して
ブラウザキャッシュを更新する。リグだけを更新して旧描画が残る状態を許可しない。

生成した`gaze_left` / `gaze_right`は目領域だけを含む局所テクスチャで、頭と身体を固定したまま連続補間する。閉眼中は重ねず、半目から開眼へ戻る区間でだけ滑らかに復帰する。

部位ごとに手法を使い分ける。**まぶたが現れる目の開閉のように、形状変化がテクスチャ補間でしか表現できないものと、頭の向きのようにメッシュ変形でしか表現できないものを混ぜない。**

| 部位 | 手法 | 理由 |
|---|---|---|
| 目の開閉 | テクスチャ補間（主） | まぶたが現れる変化は頂点移動では作れない |
| 視線 | テクスチャ補間 | 眼球位置の差分がキーフォームとして存在する |
| 口の開閉 | テクスチャ補間＋メッシュの縦変形 | 口内が現れる変化は補間、開き量の連続性は変形が担う |
| 頭・首・身体の向き | メッシュ変形のみ | キーフォームが無い。視差とせん断で近似する |
| 髪・胸元の揺れ | メッシュ変形＋物理 | 同上 |

この切り分けにより、隠れ領域の描き足しが要るのは**パーツ同士が離れて動く箇所だけ**に限られる。顔の内部（目・口・眉）はその場で形が変わるだけなので、隠れ領域が発生しない。

## 隠れ領域の原画

前髪の裏の額、腕の裏の胴、横髪の裏の頬は、1枚絵に画素として存在しない。現行の微細変形は、可視画素を6パーツへ分離し、関節境界から数画素だけ内側を同一座標で局所再構成する。生成方式は`regions.json`と`rig.json`へ記録する。

遮蔽物を描かない原画を、既存9レイヤーと同じ1780×3600・同じ座標系で用意し、`characters/<id>/mesh-rig/source/` へ置く。`layers/` へは混ぜない（`build_runtime_layers` は固定の名前リストだけを実行用WebPへ変換し `layers/` を glob しないため、置いても実行用には届かず死蔵になる）。

素材の枚数は制約ではない。`motionProfile=full` の9レイヤーは同期時に検査される**必須の下限であって上限ではなく**、`rin-tha3` は既に21枚を持つ。ランタイムの `probeRigLayers` が必須にしているのは `base` / `mouth_open` / `mouth_closed` / `blink_half` / `blink_closed` の5枚で、視線2枚は任意である。

局所再構成やAIのインペイントで広い面積を作らない。現行より広い可動域が必要なパーツは、遮蔽物を外して描き足した原画を用意するまで可動域を広げない。

## アセット構造

```
characters/<id>/
├── layers/*.png              制作正本9枚（既存、上書き・削除しない。キーフォーム元）
├── runtime-layers/*.webp     既存（差分絵フォールバック用）
├── frames/<ハッシュ>/*.webp  既存（フレームアニメ用）
└── mesh-rig/
    ├── rig.json              パーツ、ノード木、パラメータ、束縛の定義
    ├── parts/*.webp          bboxへ切り詰めた実行用パーツ（lossless WebP）
    └── source/
        ├── regions.json      生成条件、描画順、部位物理
        └── parts/*.png       同一座標の6分離原画（配信コピーから除外する）
```

パーツは胴→後ろ髪→左腕→右腕→顔→前髪の順に重ねる。目、口、視線、感情はこの上へ局所キーフォームとして重ねる。

`scripts/build-separated-mesh-sources.py`で6分離原画を作り、`scripts/build-mesh-rig.py`でlossless WebPと`rig.json`を作る。後者は中立合成が`layers/mouth_closed.png`と完全一致しない場合に停止する。

## rig.json

```json
{
  "version": 1,
  "canvas": { "width": 1780, "height": 3600 },
  "contentBox": [222, 284, 1567, 3317],
  "runtimeScale": 0.5,
  "nodes": [
    { "id": "body", "pivot": [890, 2600] },
    { "id": "neck", "parent": "body", "pivot": [890, 1520] },
    { "id": "head", "parent": "neck", "pivot": [890, 1180] }
  ],
  "drawables": [
    {
      "id": "PartMouth",
      "node": "head",
      "texture": "parts/mouth.webp",
      "offset": [612, 848],
      "order": 80,
      "depth": 0.35,
      "grid": [6, 4],
      "keyforms": [
        { "param": "ParamMouthOpenY", "at": 0, "texture": "parts/mouth.webp" },
        { "param": "ParamMouthOpenY", "at": 1, "texture": "parts/mouth-open.webp" }
      ],
      "occlusion": { "source": "source/no-hairfront.png", "inpaintRatio": 0.0 }
    },
    {
      "id": "PartHairFront",
      "node": "head",
      "texture": "parts/hair-front.webp",
      "offset": [210, 260],
      "order": 140,
      "depth": 0.85,
      "grid": [8, 8],
      "physics": { "stiffness": 18, "damping": 6, "follow": 0.6, "axis": [1, 0.3] },
      "occlusion": { "source": "source/no-hairfront.png", "inpaintRatio": 0.02 }
    }
  ],
  "parameters": [
    { "id": "ParamAngleX", "min": -15, "max": 15, "default": 0 },
    { "id": "ParamMouthOpenY", "min": 0, "max": 1, "default": 0 }
  ],
  "bindings": [
    { "param": "ParamAngleX", "node": "head", "rotate": 0.35, "translateX": 2.4, "skewX": 0.12 },
    { "param": "ParamAngleX", "node": "neck", "rotate": 0.12 },
    { "param": "ParamBreath", "node": "body", "translateY": -3.0, "scaleY": 0.004 }
  ]
}
```

`offset` は原画座標系での左上位置。`depth` は頭の回転時に視差を掛ける強さで、手前ほど大きい（前髪0.85、目0.5、顔0.2、後ろ髪-0.6）。`order` は描画順で `HIYORI_PARTS` の並びに対応する。`grid` は格子メッシュの分割数で、変形が要らないパーツは `[1, 1]` にしてよい。`occlusion.inpaintRatio` は隠れ領域のうちインペイント由来の面積比で、QAで検査する。

`keyforms` を持つ drawable は、パラメータ値に応じて隣接する2つのキーフォームをテクスチャ補間する。持たない drawable は単一テクスチャをメッシュ変形するだけになる。

## パラメータ

命名は Live2D の標準名に合わせる。ひよりとメッシュリグで駆動側のコードを共用し、キャラごとに別名を作らないためである。

| ID | 範囲 | 既定 | 駆動元 |
|---|---|---|---|
| `ParamAngleX` / `Y` / `Z` | -15..15 / -10..10 / -8..8 度 | 0 | カーソル追従（`gazeTrackingEnabled` 有効時）、待機の自発動作 |
| `ParamBodyAngleX` / `Y` / `Z` | -6..6 / -4..4 / -4..4 度 | 0 | `ParamAngle*` の遅れ追従 |
| `ParamEyeLOpen` / `ParamEyeROpen` | 0..1 | 1 | `scheduleBlink` |
| `ParamEyeBallX` / `Y` | -1..1 | 0 | カーソル追従 |
| `ParamBrowLY` / `RY` | -1..1 | 0 | `character.dataset.motion` |
| `ParamMouthOpenY` | 0..1 | 0 | `rigMouthLevel`（リップシンクの包絡線） |
| `ParamMouthForm` | -1..1 | 0 | Aivis母音タイムライン（無い場合は0） |
| `ParamArmClose` | 0..1 | リグ既定値（旧`anime25d-v1`は0.72） | 生成時の既定姿勢 |
| `ParamBreath` | 0..1 | 0 | 正弦（周期約3.8秒） |
| `ParamHairFront` / `Side` / `Back` | -1..1 | 0 | 物理演算の出力 |

可動域は `rig.json` の `parameters` で宣言し、駆動側は必ずクランプする。上限を超えると視差近似が破綻するため、可動域はキャラごとの調整値ではなく**この方式の制約**として扱う。

## 変形モデル

ノード変換は `translate(pivot) ∘ rotate ∘ scale ∘ skew ∘ translate(-pivot)` を親から子へ合成する。`body → neck → head` の木構造により、頭を動かすと目・口・前髪が連れて動く。差分絵経路にはこの連動が無く、差分ごとに独立して切り替わっている。

頭の回転は、ノード回転だけでは平面が回るだけで立体に見えない。drawable ごとに `depth` に比例した水平移動とせん断を加え、手前のパーツほど大きく動かすことで奥行きを近似する。

```
offsetX += ParamAngleX * depth * k
skewX   += ParamAngleX * depth * k2
```

これは Live2D が手描きのキーフォームで表現している擬似3Dの近似であり、**可動域の端では必ず不自然になる。** 上限値はその破綻が始まる手前に置く。

## 描画経路の統合

`ui/renderer.js` に、`rigActive`（差分絵）/ `live2dActive` と並ぶ第3経路を足す。`loadCharacter` で `mesh-rig/rig.json` の有無を判定して分岐する。

描画キャンバスは Live2D と同じく `characterView` 内へ挿入する。位置決めは `.character-view > img` / `> canvas` の子セレクタが担っており、`#petSprite` を名指しするCSSは success/pleased のバウンス1本だけなので、既存の枠組みへそのまま乗る。

フォールバックは既存の停止条件をすべて引き継ぐ。

| 条件 | 挙動 |
|---|---|
| WebGL初期化の失敗 | 差分絵表示へ落とし、`logApp` へ理由を残す |
| `prefers-reduced-motion` | 差分絵表示（現在バネ変形を止めているのと同じ扱い） |
| `chat` / `bar` 表示 | 差分絵表示 |
| `document.hidden` / `napping` / 起動中 | 描画を止める |
| `rig.json` の検証失敗 | 差分絵表示へ落とす。無言で退行させない |

セナは待機・思考中・会話を同じメッシュキャンバス上で連続描画し、状態遷移で表示寸法を変えない。悲しみは専用表情をメッシュ補間するまでは差分絵へ切り替える。`animations.idle` は残し、フォールバックが常に成立する状態を保つ。

## 入力の接続

現在捨てている連続値をパラメータへ直結する。

**口** — `rigMouthLevel` は60fpsの連続値として算出済みだが、差分絵経路では2値へ落としている。そのまま `ParamMouthOpenY` へ渡す。Live2D経路が既にこの形で受け取っているため、駆動側は共用できる。

**視線** — `rigLookDirection` の -1 / 0 / 1 を、カーソル位置からの連続値にして `ParamAngleX` / `Y` と `ParamEyeBallX` / `Y` へ分配する。頭と眼球で追従の速さを変える。既定OFF（`gazeTrackingEnabled`）は維持する。

**まばたき** — `scheduleBlink` の `blink_half` → `blink_closed` → `blink_half` を `ParamEyeLOpen` の連続イージングへ置き換える。閉じが速く開きが遅い非対称配分は差分絵経路で既に導入済みなので、その時間配分をそのまま連続値へ移す。

**呼吸** — `ParamBreath` の正弦。差分絵経路には無い。

## 物理演算

`animateSpringMotion` のバネ積分（stiffness / damping による二階系）をそのまま流用する。現在は `Math.sin` の固定周期で駆動しているため、何も起きていなくても同じリズムで揺れ続け、見慣れると機械的になる。入力を `ParamAngleX` / `ParamBodyAngleX` の**速度**へ差し替え、頭を振ったときだけ髪が遅れて追随して収束する形にする。

既存の `secondaryMotion` / `springPhysics` 定義は差分絵キャラ向けに残す。メッシュリグのパーツ物理は `rig.json` の `drawables[].physics` を使う。

停止条件（`reduce-motion`、`document.hidden`、`napping`、`chat` / `bar` 表示）は現在の実装を維持する。

## 解像度とテクスチャ容量

パーツを全画面サイズのまま15枚持つと、1780×3600×4バイト×15で約384MBのテクスチャになり常駐マスコットとして成立しない。bboxへ切り詰めれば合計約60MBまで下がる。

表示は `LIVE2D_RENDER_WIDTH` 285 × `LIVE2D_RENDER_HEIGHT` 315 のCSS画素だが、CSS拡大前に画素を失わないよう描画バッファは1283×1418にする。`runtimeScale` 0.5の実行テクスチャはベース890×1800、局所差分は頭部bboxへ切り詰める。実行資産は体ごとに約1.0MBから3.3MBで、透明な全画面テクスチャを局所差分ごとに常駐させない。

**縮小は許可されるが、`AGENTS.md` のとおり引き伸ばし拡大は禁止する。** `runtimeScale` は 1.0 以下に限り、実行用パーツを原画より大きく書き出さない。

1920×1080配信形式は6分離原画と局所差分の初回復号により約6秒から8秒で、通常2D向け4秒基準を超える。描画バッファを下げると輪郭が粗くなるため、今回は画質を優先して維持し、局所テクスチャを頭部bboxへ限定して転送量だけを抑える。

## 段階

| 段階 | 実施内容 | 完了条件 |
|---|---|---|
| 1. PIXIの分離 | 完了 | PIXI 6.5.10を`ui/vendor/pixi/`へSHA-256固定で取得し、Cubism 4も固定した |
| 2. 局所キーフォーム生成 | 完了 | 既存正本を変更せず、目・口・視線・感情の羽根付きlossless WebPを生成する |
| 3. メッシュ描画 | 完了 | 6分離原画、同一9×15格子、WebGL失敗時の差分絵退避、既存配置との統合を実装した |
| 4. 連続入力 | 完了 | 口、まばたき、頭部視線、瞳の局所視線、感情差分を連続入力へ接続した |
| 5. 物理演算 | 完了 | 頭部速度による髪の遅れ、左右腕、呼吸を部位別変形へ接続した |
| 6. 検証と登録 | 完了 | 8体×7形式、同期・形式検証、省モーションとchat/bar退避を追加した |

左右15度を超える頭部回転や大きな腕移動は、完全に遮蔽物を外して描き足した原画が揃うまで行わない。局所再構成や引き伸ばしで可動域を広げない。

## 見送った選択肢

| 項目 | 理由 |
|---|---|
| Cubism で本物の Live2D を作る | `.moc3` が Cubism Editor でしか生成できず、リギングが人手に残る。販売ゲートの拡張アプリ審査も必要になる |
| THA3 のリアルタイム推論 | 1枚絵から連続パラメータで動かせるが、出力が512×512で高精細キャラの解像度に届かず、常駐マスコットがGPUを掴み続ける |
| 口の中間レイヤー追加（`mouth_half` 等） | 差分絵経路の量子化を4段へ細かくできるが専用作画が要る。`AGENTS.md` が半目の透明度生成を禁じているのと同じ理由で、既存の開閉2枚から合成しない。メッシュリグへ進めばキーフォーム補間が置き換える |
| 目・口のクロスフェード（差分絵経路） | `syncSpringMotionFrame` が `petSprite.currentSrc` を全バネ領域の `<img>` へ複製している。本体だけクロスフェードすると上に重なるバネ領域がハードカットのまま残り、差分が領域へ掛かるキャラで顔が二重に見える。バネ領域側も同時にクロスフェードするか、領域と顔の非重複をキャラごとに保証する必要がある |
