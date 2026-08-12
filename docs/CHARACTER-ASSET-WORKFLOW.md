# キャラクター画像追加・更新手順

通常2Dキャラを高精細なまま追加し、古い画像との混在、ぼけ、矩形背景、動かない口・目を再発させないための制作手順の正本です。

## 成果物

`characters/<id>/` へ次を置きます。

- `preview.png`: 高精細な透過全身立ち絵。顔・髪・服・輪郭の正本
- `layers/base.png`: 通常表情
- `layers/blink_half.png`: 専用作画の半目。閉じ目の透明度変更は禁止
- `layers/blink_closed.png`: 閉じ目
- `layers/mouth_closed.png` / `mouth_open.png`: 閉口・開口
- `layers/gaze_left.png` / `gaze_right.png`: 頭と身体を固定した左右視線
- `layers/expression_happy.png` / `expression_sad.png`: 喜び・悲しみの顔差分
- `expression-patches/`: 元立ち絵と同じ座標へ局所合成する透過パッチ
- `character.json`: 同梱PNGは `motionProfile=full`、高精細優先なら `animationQualityMode=sharp-expression`、生成後は `frameGeneration`

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
uv run --with pillow python scripts/build-sharp-expression-layers.py --prepare-photoshop <id>
scripts/finish-sharp-expression-assets.ps1 -CharacterIds <id> -Scale 2
```

1つ目は高精細元絵と局所差分からレイヤーを作ります。`sharp-expression` に旧THA3全身ポーズが残る場合は新旧混在を避けるため失敗します。2つ目はPhotoshop「ディテールを保持」で2倍化し、元アルファを再適用します。既定完成寸法は1780×3600、QAレポートは `temp/sharp-expression-finish/<日時>/qa/qa-report.json` です。

## 4. 単一世代を生成

```powershell
uv run --with pillow python scripts/build-character-animations.py --character-dir characters/<id>
cargo xtask sync-characters
```

生成先は `frames/<内容ハッシュ>/` です。新世代を完成しmanifestを原子的に保存した後だけ旧世代を除去します。複数世代または世代外参照は同期時に拒否します。Live2Dは別管理とし、PNG世代へ混ぜません。

`motionProfile=full` を宣言したパックは、基本、半目／閉じ目、閉口／開口、左右視線、喜び／悲しみの9レイヤーが同期時に機械検査されます。1枚でも不足した状態で配布へ進めません。

## 5. 実WebView検査

- 実寸が完成レイヤーと一致する
- 半目→閉眼→半目が順に出る
- 音声ありは声量同期、ミュート中はタイマー式で口が開閉する
- 左右視線で身体・髪・服が変化しない
- 感情切替で顔以外の解像感・輪郭・位置が変化しない
- 胸元二次動作は広いフェザーと小さい縦変形で境界が見えない
- 配信ではPICO STUDIOの後ろ、背景の前に描画される

最後に `cargo test --manifest-path xtask/Cargo.toml`、`cargo check --manifest-path src-tauri/Cargo.toml`、必要なら `cargo test --manifest-path src-tauri/Cargo.toml --lib`、`git diff --check` を実行します。

## 禁止事項

- 低解像度の全身生成物を高精細レイヤーへ混ぜない
- 閉じ目の透明度だけで半目を作らない
- 市松模様、背景色面、ハロー、太い外周影を透過キャラへ含めない
- 生成途中の画像を `characters/` へ置かない。一時物はリポジトリ直下の `temp/` だけに置く
