---
name: rebuild-picoagent-character
description: PicoAgentの通常2D・Live2D・VRMキャラクターを追加・差し替え・高精細化し、瞬き、口パク、視線、感情、単一フレーム世代、実ランタイム表示まで安全に再生成する。キャラがぼやける、状態ごとに新旧画像が混ざる、VRM実験キャラを追加する、Photoshopで仕上げる、キャラクター画像の追加手順を再現する依頼で使う。
---

# PicoAgentキャラクター再生成

## 最初に読む

1. リポジトリ直下の `AGENTS.md` と `HANDOFF.md` を読む。
2. `docs/CHARACTER-ASSET-WORKFLOW.md` を最後まで読む。
3. `git status --short` で既存変更を確認し、巻き戻さない。

## 高精細キャラを再生成する

通常2Dは以前の上半身位置を標準にし、構図未設定時は `upper` を使う。Live2D／VRMは胸上を標準にして `bust` を使う。キャラ切替時は前のキャラの倍率を引き継がない。チャット／バーの小型アイコンは画像面を追加拡大せず、上端基準の `cover` だけで切り出す。主要レイアウトの実表示で頭頂、顔、肩が欠けず、長身キャラでも腰まで入りすぎていないことを確認する。旧 `talk` はLINE風の `chat` へ統合済みなので、検証や公開素材へ再追加しない。

`animationQualityMode=sharp-expression` は個別スクリプトを順不同で実行せず、必ず次の一括入口を使う。

```powershell
powershell -ExecutionPolicy Bypass -File scripts/rebuild-sharp-expression-assets.ps1 -CharacterIds <id>
```

この入口は次の順番を固定する。

1. 890×1800の基準寸法で基本、瞬き、口、視線、感情の局所差分を合成する。
2. Photoshop「ディテールを保持 2.0」で全9レイヤーを1780×3600へ仕上げ、元アルファを再適用する。
3. 内容ハッシュ付き `frames/<世代>/` を生成する。
4. 配信コピーを同期する。

Photoshop後に `build-sharp-expression-layers.py` または `build-extended-expression-layers.py` を再実行しない。高精細版が低解像度で上書きされる。

画像の引き伸ばし拡大を禁止する。nearest／bilinear／bicubic／Lanczos等で縦横ピクセル数だけを増やした画像を成果物へ使わない。Photoshop前の配置正規化も等倍以下かつ縦横比維持に限定する。最終レイヤーの再サンプリングをPillow、ImageMagick、OS画像API、ブラウザ、CSSへ代替しない。QAレポートの方式が `Photoshop Preserve Details 2.0` で、処理前後の寸法が記録されていることを確認する。Photoshop処理も寸法だけでは合格にせず、顔・髪・輪郭がぼける場合は高解像度の元絵または局所再制作へ戻る。

## VRM実験キャラを追加する

- 正式な2Dキャラのvariantへ混ぜず、`characters/<id>/character.json` のトップレベル `vrm` を持つ独立した実験キャラとして登録する。
- VRMも全形式共通の胸上ルールに従い、manifestへ `"framing": "bust"` を明記する。上半身や全身はユーザーが明示した場合だけ `upper-body` / `full-body` を使う。
- VRMモデル本体は `characters/<id>/*.vrm`、再生成可能な原画・PSD・スクリプト・テストは `tools/<id>-vrm/`、説明とレビュー用動画は `docs/vrm/` に分ける。
- `cargo xtask setup vrm` でSHA-256固定のThree.js／three-vrmを取得し、`cargo xtask sync-characters` でGLBヘッダーと参照先を検査する。
- `SHOOT_VRM_MATRIX=1 node tools/screenshots/shoot.mjs` で実際の `renderer.js` を通し、全7形式の胸上表示（チャット／バーの小型アイコンを含む）、透明背景、canvas描画、継続モーションを確認する。構造検査だけで表示完了とはしない。
- 配信形式は1920×1080の実画面を基準にし、背景はフルHD以上を使う。PNGに固定せず、原寸で目立つ劣化がなければWebP／JPEGも許容し、既存の高品質圧縮画像を無意味に再圧縮しない。
- 公開動画は通常表示を2倍密度、配信表示を1920×1080ネイティブで描画してから960pxへ縮小する。低解像度素材や旧動画の拡大で帳尻を合わせない。
- 元リポジトリからVRMを撤去する移行では、PicoAgent側のビルド・構造検査・実表示・コミット・pushが全部成功するまで元ファイルを削除しない。スキルは移行対象外として残す。

## 品質を検証する

- `cargo xtask sync-characters` を通し、全9レイヤー、1780×3600最低寸法、単一世代を検査する。
- PNG寸法だけで合格にしない。元絵、Photoshop完成レイヤー、実WebViewの顔・髪・輪郭を同じ表示倍率で比較する。
- 状態切替で身体、服、髪、顔形、解像感が変わらないことを確認する。
- 半目は閉じ目の透明化で代用しない。
- 透過市松で矩形、ハロー、背景色、太い影がないことを確認する。
- GUI確認は最新の利用モニター指示へ従い、指定がなければヘッドレス検査を優先する。

## 完了条件

```powershell
cargo test --manifest-path xtask/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
git diff --check
```

READMEのキャラプレビューを変更した場合は、実WebViewの `tools/screenshots/shoot.mjs` で対象WebPを再生成する。コード、`SPEC.md`、`CHANGELOG.md`、`docs/REVIEW-LOG.md`、手順書を同じ作業内で同期し、`AGENTS.md`の指示どおりコミットしてpushする。
