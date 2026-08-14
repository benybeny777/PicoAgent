# むぎVRM実験キャラ 品質レポート

検証日: 2026-08-14 JST

| 項目 | 結果 |
|---|---:|
| VRM形式 | VRM 1.0 / GLB |
| メッシュ / 頂点 | 17 / 347 |
| 顔格子メッシュ | 8 |
| 標準 / カスタムExpression | 17 / 4 |
| Spring Bone | 3チェーン / 5可動ジョイント |
| PicoAgent表示 | 独立した実験キャラ / 上半身標準 |
| 実ランタイムQA | mascot / pet / stream |

肩下地、まぶた、口、連続したスカート裾、白背景マット除去を制作版で確認しています。PicoAgent移行後は `cargo xtask sync-characters` がGLBとmanifestを検査し、`SHOOT_VRM_MATRIX=1` が本物の `renderer.js` とThree.js／three-vrmを通してcanvas描画と継続モーションを検査します。レビュー画像と計測値はGitへ含めず `temp/vrm-layout-matrix/` に置きます。
