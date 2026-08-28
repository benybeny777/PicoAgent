# キャラクター画像追加・更新手順

通常2Dキャラを高精細なまま追加し、古い画像との混在、ぼけ、矩形背景、動かない口・目を再発させないための制作手順の正本です。

キャラクターのPNG／WebPは、2026-08-18以降に追加・更新した世代からGit LFSで管理します。既存Git履歴は
書き換えません。clone後は`git lfs pull`で実体を取得し、コミット前に`git lfs ls-files`で今回更新した画像が
載っていることを確認します。LFSポインタだけの状態では生成・同期・画像QAを始めません。

## 共通の表示構図

PicoAgentは、低頭身2Dを従来の上半身位置 `upper`、長身・実写・Live2Dを胸全体がぎりぎり入る胸上 `bust` にします。構図保存のないキャラだけ描画形式と体型ごとの標準へ戻し、直前に選んでいた別キャラの倍率や位置を引き継ぎません。Live2Dはモデル別`scale` / `offsetY`で胸上へ合わせるため、表示側では`upper=1.55`を中立倍率として扱い、PNG向け`bust`倍率を重ねません。原画固有の余白は manifest の `defaultFraming` / `streamFraming` で吸収し、利用者が保存した構図を上書きしません。チャット／バーの小型アイコンは首上で切り出します。素材自体は後から調整できるよう全身・透過・余白つきで保持し、表示側で非破壊にフレーミングします。旧 `talk` はLINE風の `chat` へ統合済みで、撮影対象へ戻しません。

実画面の「配置」モードではMoveableを使い、キャラクター、フキダシ、入力、履歴、画面内ディスプレイを直接ドラッグ／拡大縮小できます。配置は表示形式ごとにローカル保存し、素材画像やmanifestは書き換えません。通常操作へ戻したときにキャラクタードラッグでウィンドウ移動できることも検査します。

## 成果物

`characters/<id>/` へ次を置きます。

- `preview.png`: 高精細な透過全身立ち絵。顔・髪・服・輪郭の正本
- `layers/base.png`: 通常表情
- `layers/blink_half.png`: 専用作画の半目。閉じ目の透明度変更は禁止
- `layers/blink_closed.png`: 閉じ目
- `layers/mouth_closed.png` / `mouth_open.png`: 閉口・開口
- `layers/gaze_left.png` / `gaze_right.png`: 頭と身体を固定した左右視線
- `layers/expression_happy.png` / `expression_sad.png`: 喜び・悲しみの顔差分
- `runtime-layers/*.webp`: 9枚の制作正本を復号RGBA完全一致で変換した実行用lossless WebP
- `frames/<内容ハッシュ>/*.webp`: 状態フレームを重複除外した実行用lossless WebP単一世代
- `expression-patches/`: 元立ち絵と同じ座標へ局所合成する透過パッチ
- `character.json`: 同梱通常2Dは `motionProfile=full`、高精細優先なら `animationQualityMode=sharp-expression`、生成後は `frameGeneration`、`frameEncoding=webp-lossless-v1`、`runtimeLayers`
- `mesh-rig/source/regions.json`: メッシュリグ用の局所領域、羽根幅、格子数。制作専用で配信しない
- `mesh-rig/parts/*.webp`、`mesh-rig/rig.json`: `scripts/build-mesh-rig.py`が生成する実行資産。手で直さない

## 1. 元立ち絵

1. 全身、正面寄り、手足と髪先を切らず、周囲に余白を取る。
2. 背景は実アルファの透明にする。市松模様が画素として焼き付いたRGB画像は不可。
3. 四隅のアルファだけで判断せず、透明市松と暗色・明色背景の両方で輪郭を見る。
4. 写真調でも床、壁、影、矩形、ハローを残さない。

## 2. 顔差分

画像編集では `preview.png` を参照し、身体、顔形、髪、服、キャンバス寸法を固定して指定箇所だけを変えます。

- 半目: 両目を自然な瞬きの中間へ。眠そうな別表情にはしない
- 閉眼: 両目だけを完全に閉じる
- 開口・閉口: 目と輪郭を変えず、口元だけを変更する
- 左右視線: 頭を動かさず、虹彩・瞳孔だけを左右へ向ける
- 感情: 顔形を変えず、眉・目・口の局所差分だけで喜び／悲しみを作る

生成結果を全身レイヤーとして直接採用せず、元絵と同寸法の局所パッチへしてから合成します。状態切替で服、髪、身体、解像感が変わる事故を防ぐためです。

## 3. レイヤー組み立てとPhotoshop仕上げ

```powershell
powershell -ExecutionPolicy Bypass -File scripts/rebuild-sharp-expression-assets.ps1 -CharacterIds <id>
```

この一括入口だけを使います。内部では「890×1800の基準寸法で基本・視線・感情の局所差分を合成 → Photoshop『ディテールを保持 2.0』で全9レイヤーを再サンプリング → 元アルファ再適用 → 連番生成 → 配信コピー同期」の順に実行します。nearest／bilinear／bicubic／Lanczos等で縦横ピクセル数だけを増やす引き伸ばし拡大は禁止です。Photoshop前の配置正規化は、元画像を拡大せず縦横比を保ったまま中央へ配置し、枠を超える場合だけ縮小します。最終レイヤーの処理をPillow、ImageMagick、OS画像API、ブラウザ、CSSへ置き換えてはいけません。Photoshop後に個別の差分生成スクリプトを実行すると1780×3600を890×1800へ戻すため禁止です。一括入口は何度実行しても準備寸法を890×1800へ正規化し、半分ずつ縮小しません。QAレポートは `temp/sharp-expression-finish/<日時>/qa/qa-report.json` で、方式と処理前後の寸法を確認します。

Photoshopの高品質再サンプリングは輪郭の崩れを抑えますが、元絵に存在しない顔・髪・服の細部を新しく作る工程ではありません。完成寸法が1780×3600でも、同じ表示倍率で細部が明瞭に保たれずぼけが残る場合は高精細合格にせず、高解像度の元絵へ差し替えるか局所的に再制作します。

## 4. 単一世代を生成

```powershell
uv run --with pillow python scripts/build-character-animations.py --character-dir characters/<id>
cargo xtask sync-characters
```

メッシュリグを持つキャラは、通常2D世代を完成させてから次も実行します。

```powershell
uv run --with pillow python scripts/build-mesh-rig.py <id>
cargo xtask setup pixi
```

`runtimeScale`は1以下に限定し、制作正本を拡大しません。生成したlossless WebPは直後に復号してRGBA完全一致を検査し、設定から外した旧パーツは成功時に回収します。既存レイヤーだけで作る第1実装は顔の局所補間と一枚メッシュの微細変形までです。前髪、後ろ髪、腕、胴、独立した瞳を動かす場合は、遮蔽物を外した同一座標の原画を先に用意し、仮の切り抜きや引き伸ばしで可動域を作りません。

制作正本の `layers/*.png` は上書き・削除せず、実行用の9レイヤーを `runtime-layers/*.webp`、状態フレームを `frames/<内容ハッシュ>/*.webp` へlossless変換します。各WebPは保存直後に再読込し、元画像と寸法・RGBAが完全一致しなければ生成を停止します。新世代を完成しmanifestを原子的に保存した後だけ旧世代を除去します。複数世代または世代外参照は同期時に拒否します。Live2Dは別管理とし、通常2D世代へ混ぜません。

`cargo xtask sync-characters` は制作正本を検証したあと、配信用コピーからだけ `layers/` を除外し、`runtimeLayers` が指すWebPを残します。利用者が追加済みの `layers/*.png` パックはmanifestに `runtimeLayers` がないため従来どおり読み込めます。lossy WebPは別判断とし、この工程へ混ぜません。

`motionProfile=full` を宣言したパックは、基本、半目／閉じ目、閉口／開口、左右視線、喜び／悲しみの9レイヤーが同期時に機械検査されます。さらに `animationQualityMode=sharp-expression` は全9レイヤーが1780×3600以上でなければ同期が失敗します。1枚でも不足・低解像度の状態では配布へ進めません。

## 5. 画素検査ゲート

```powershell
cargo xtask character-quality
```

実WebViewを開く前に、制作正本を画素単位で機械検査します。目視より先にここで止めるための工程で、
`cargo xtask verify` からも実行されます。判定は次の4点です。

| 検査 | 判定 | 落ちる原因 |
|---|---|---|
| 引き伸ばし拡大 | 不合格 | `generator.source` が指す原画より完成レイヤーの被写体が大きい。拡大した画素は情報を増やさない |
| 生成元 | 不合格 | `generator.source` が指す原画が無く、引き伸ばしの有無を確認できない |
| 顔以外の書き換え | 不合格 | 差分レイヤーが顔より下の非輪郭部を被写体比2%超書き換えている。生成結果を全身のまま採用した証拠 |
| 半目 | 不合格 | `blink_half` が `blink_closed` と色は同一でアルファだけ違う、または完全に一致する |
| 余白・透過 | 不合格 | 被写体が左右上端で切れている、四隅が透明でない |
| 寸法一致 | 不合格 | 差分レイヤーが `base` と違う寸法 |
| 輪郭ドリフト | 注意 | 顔より下の輪郭が被写体比0.1%超ずれている。レイヤーを個別に再サンプリングすると必ず出る |
| 差分なし | 注意 | レイヤーが `base` と一致し、宣言されていても動かない |
| 矩形 | 注意 | アルファ境界が被写体高さの35%超にわたって完全な直線。背景板や貼り付けた矩形の疑い |
| 原寸 | 注意 | 既定構図の倍率で原寸を割る。デバイス比2の実機でぼける |

「引き伸ばし拡大」は統計ではなく実寸の比で判定します。`character.json` の `generator.source` に
原画を記録している生成経路（THA3など）では、その原画の被写体と完成レイヤーの被写体を直接比べる
ため、絵柄によらず確実に判定できます。生成元を記録していないパックは対象外です。

報告値の「実効解像度」は、被写体を半分へ縮めて戻したときの二乗平均平方根誤差です。引き伸ばして
作った画像は最初から半分ぶんの情報しか持たないため往復してもほとんど変わらず、値が小さくなります。
絵柄によって水準が変わる（写実は線画よりはるかに大きい）ので合否には使わず、同じ絵柄どうしを
比べるための指標として出しています。

「顔以外の書き換え」は単純な変更面積比ではなく、**顔帯より下で、かつ輪郭でもない画素**だけを数えます。
レイヤーを個別に再サンプリングすると輪郭のサブピクセル差だけで数%動くため、面積比では正しく作った
局所パッチと全身の描き直しを区別できないからです。同梱キャラクターの実測では、局所パッチで作った
素材が最大1.1%、全身を作り直した素材が最小3.0%で、しきい値の2%はその間にあります。

既存キャラクターの指摘は `characters/quality-baseline.json` に記録済みで、**ベースラインに無い不合格
だけ**が失敗になります。素材を作り直して指摘が消えたら `--update-baseline` で記録を減らします。
記録を増やしてよいのは、既存素材を作り直せないと判断した場合だけです。新しいキャラクターを追加する
ときは `--strict` を付け、注意も含めて0件にしてから進めます。

```powershell
cargo xtask character-quality --character <id> --strict   # 新規追加時
cargo xtask character-quality --json temp/quality.json    # 数値を残す
cargo xtask character-quality --update-baseline           # 解消した指摘を記録から外す
```

## 6. 実WebView検査

- 実寸が完成レイヤーと一致する
- 全WebPを復号した寸法・RGBAが変換元PNGと完全一致する
- 半目→閉眼→半目が順に出る
- 音声ありは声量同期、ミュート中はタイマー式で口が開閉する
- 左右視線で身体・髪・服が変化しない
- 感情切替で顔以外の解像感・輪郭・位置が変化しない
- PNG寸法だけで合格にせず、顔・髪・輪郭を元絵、完成レイヤー、実WebViewの拡大表示で並べて確認する
- 胸元二次動作は広いフェザーと小さい縦変形で境界が見えない
- 配信ではPICO STUDIOの後ろ、背景の前に描画される
- `SHOOT_CHARACTER_MATRIX=1` の全キャラ×7形式で描画欠け・速度警告が0件
- メッシュリグは`SHOOT_MESH_RIG=1`で閉眼、開口、頭部メッシュ、省モーション退避、縦1400画素以上の描画バッファが合格する

最後に `cargo test --manifest-path xtask/Cargo.toml`、`cargo check --manifest-path src-tauri/Cargo.toml`、必要なら `cargo test --manifest-path src-tauri/Cargo.toml --lib`、`git diff --check` を実行します。

## 禁止事項

- 低解像度の全身生成物を高精細レイヤーへ混ぜない
- nearest／bilinear／bicubic／Lanczos等による引き伸ばし拡大を高精細素材へ使わない
- Photoshop前の配置合わせで拡大または縦横比変更をしない
- 閉じ目の透明度だけで半目を作らない
- 市松模様、背景色面、ハロー、太い外周影を透過キャラへ含めない
- 生成途中の画像を `characters/` へ置かない。一時物はリポジトリ直下の `temp/` だけに置く
- `cargo xtask character-quality` の不合格を、素材を直さずベースラインへ足して回避しない
