# キャラクター画像追加・更新手順

通常2Dキャラを高精細なまま追加し、古い画像との混在、ぼけ、矩形背景、動かない口・目を再発させないための制作手順の正本です。

## 共通の表示構図

PicoAgentは、低頭身2Dを従来の上半身位置 `upper`、長身・実写・Live2Dを胸全体がぎりぎり入る胸上 `bust` にします。構図保存のないキャラだけ描画形式と体型ごとの標準へ戻し、直前に選んでいた別キャラの倍率や位置を引き継ぎません。Live2Dはモデル別`scale` / `offsetY`で胸上へ合わせるため、表示側では`upper=1.55`を中立倍率として扱い、PNG向け`bust`倍率を重ねません。原画固有の余白は manifest の `defaultFraming` / `streamFraming` で吸収し、利用者が保存した構図を上書きしません。チャット／バーの小型アイコンは首上で切り出します。素材自体は後から調整できるよう全身・透過・余白つきで保持し、表示側で非破壊にフレーミングします。旧 `talk` はLINE風の `chat` へ統合済みで、撮影対象へ戻しません。

実画面の「配置」モードではMoveableを使い、キャラクター、フキダシ、入力、履歴、画面内ディスプレイを直接ドラッグ／拡大縮小できます。配置は表示形式ごとにローカル保存し、素材画像やmanifestは書き換えません。通常操作へ戻したときにキャラクタードラッグでウィンドウ移動できることも検査します。

## 成果物

`characters/<id>/` へ次を置きます。

- `preview.png`: 高精細な透過全身立ち絵。顔・髪・服・輪郭の正本
- `layers/base.png`: 通常表情
- `layers/blink_17.png` / `blink_33.png` / `blink_half.png` / `blink_67.png` / `blink_83.png` / `blink_closed.png`: `base`の開眼を含む目7段階
- `layers/mouth_closed.png` / `mouth_17.png` / `mouth_33.png` / `mouth_half.png` / `mouth_67.png` / `mouth_83.png` / `mouth_open.png`: 閉口から最大開口までの口7段階
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

- 目7段階: 上まぶたが開眼から閉眼まで単調に下がる固有差分にし、下まぶたを巻き込んだ眠そうな別表情にしない
- 閉眼: 両目だけを完全に閉じる
- 口7段階: 目と輪郭を変えず、閉口から最大開口まで開口量が単調に増える固有差分にする
- 左右視線: 頭を動かさず、虹彩・瞳孔だけを左右へ向ける
- 感情: 顔形を変えず、眉・目・口の局所差分だけで喜び／悲しみを作る

生成結果を全身レイヤーとして直接採用せず、元絵と同寸法の局所パッチへしてから合成します。状態切替で服、髪、身体、解像感が変わる事故を防ぐためです。

## 3. レイヤー組み立てとPhotoshop仕上げ

```powershell
powershell -ExecutionPolicy Bypass -File scripts/rebuild-sharp-expression-assets.ps1 -CharacterIds <id>
```

この一括入口だけを使います。内部では「890×1800の基準寸法で基本・視線・感情の局所差分を合成 → Photoshop『ディテールを保持 2.0』で基準9レイヤーを再サンプリング → 元アルファ再適用 → 同寸の目7段階・口7段階を生成 → 連番生成 → 配信コピー同期」の順に実行します。nearest／bilinear／bicubic／Lanczos等で縦横ピクセル数だけを増やす引き伸ばし拡大は禁止です。Photoshop前の配置正規化は、元画像を拡大せず縦横比を保ったまま中央へ配置し、枠を超える場合だけ縮小します。最終レイヤーの処理をPillow、ImageMagick、OS画像API、ブラウザ、CSSへ置き換えてはいけません。Photoshop後に個別の差分生成スクリプトを実行すると1780×3600を890×1800へ戻すため禁止です。一括入口は何度実行しても準備寸法を890×1800へ正規化し、半分ずつ縮小しません。QAレポートは `temp/sharp-expression-finish/<日時>/qa/qa-report.json` で、方式と処理前後の寸法を確認します。

Photoshopの高品質再サンプリングは輪郭の崩れを抑えますが、元絵に存在しない顔・髪・服の細部を新しく作る工程ではありません。完成寸法が1780×3600でも、同じ表示倍率で細部が明瞭に保たれずぼけが残る場合は高精細合格にせず、高解像度の元絵へ差し替えるか局所的に再制作します。

## 4. 単一世代を生成

```powershell
uv run --with pillow --with numpy --with opencv-python-headless python scripts/build-character-animations.py --character-dir characters/<id>
cargo xtask sync-characters
```

生成先は `frames/<内容ハッシュ>/` です。新世代を完成しmanifestを原子的に保存した後だけ旧世代を除去します。複数世代または世代外参照は同期時に拒否します。Live2Dは別管理とし、PNG世代へ混ぜません。

`motionProfile=full` を宣言したパックは、目7段階、口7段階、左右視線、喜び／悲しみの18レイヤーが同期時に機械検査されます。さらに `animationQualityMode=sharp-expression` は全18レイヤーが1780×3600以上でなければ同期が失敗します。1枚でも不足・低解像度の状態では配布へ進めません。

旧素材を7段階へ移行する場合だけ、キャラ別の正規化eyes/mouth ROI内で双方向optical-flowを使います。ROI外のRGBAはbaseと完全一致させ、フェザーはROI内側だけに置きます。NaN／無限値、50pxまたはROI対角25%を超えるflow、双方向不整合、段階差分の非単調、半透明画素またはalpha勾配の悪化を検出したら成果物を書かず停止します。新規`picoagent-portrait-2d-v1`は22レイヤーすべてを手制作し、不足素材を旧素材移行処理で補いません。

## 5. 実WebView検査

- 実寸が完成レイヤーと一致する
- 目7段階が約240msで開眼→閉眼→開眼の順に飛びなく出る
- 口7段階が声量に応じて隣接段階へ移り、閉口と最大開口だけを往復しない
- 音声ありは声量同期、ミュート中はタイマー式で口が開閉する
- 左右視線で身体・髪・服が変化しない
- 感情切替で顔以外の解像感・輪郭・位置が変化しない
- PNG寸法だけで合格にせず、顔・髪・輪郭を元絵、完成レイヤー、実WebViewの拡大表示で並べて確認する
- 物理演算項目がなく、髪・服・身体の動きが同寸連番へ事前描画されている
- 配信ではPICO STUDIOの後ろ、背景の前に描画される

最後に `cargo test --manifest-path xtask/Cargo.toml`、`cargo check --manifest-path src-tauri/Cargo.toml`、必要なら `cargo test --manifest-path src-tauri/Cargo.toml --lib`、`git diff --check` を実行します。

## 禁止事項

- 低解像度の全身生成物を高精細レイヤーへ混ぜない
- nearest／bilinear／bicubic／Lanczos等による引き伸ばし拡大を高精細素材へ使わない
- Photoshop前の配置合わせで拡大または縦横比変更をしない
- 閉じ目の透明度だけで半目を作らない
- 市松模様、背景色面、ハロー、太い外周影を透過キャラへ含めない
- 生成途中の画像を `characters/` へ置かない。一時物はリポジトリ直下の `temp/` だけに置く
