---
name: rebuild-picoagent-character
description: PicoAgentの通常2D、LocalVTuberStudio独自2.5D、Live2D、MMDキャラクターを追加・差し替え・高品質化し、胸上構図、透過、局所表情、高フレーム数、実画面QAまで再現する。MMDは元PMXのローカル登録と姿勢・モーフ調整を行う。実写テスト、Photoshopでの有効な高精細化、ユーザー指定時のClaude CLIレビュー、旧3D実験の撤去にも使う。
---

# PicoAgentキャラクター再構築

## 最初に読む

1. `AGENTS.md`、`HANDOFF.md`、`docs/CHARACTER-ASSET-WORKFLOW.md` を読む。
2. `git status --short` で既存変更を確認し、手作業を巻き戻さない。
3. 検証画像、生成途中の素材、旧ファイルの退避先はリポジトリ直下の `temp/` だけにする。

## 対応形式と構図

- 対応ランタイムは通常2D画像、自前メッシュリグ、Live2D、利用者追加のMMD。同梱通常2Dの制作正本はPNG、実行フレームと差分レイヤーはlossless WebPとする。利用者が追加済みのPNGパックは後方互換で読み続ける。旧3D実験は廃止済みであり、MMDは新しい専用描画経路を使う。
- LocalVTuberStudioの現行出力は`rig2d/rig.json`の`schema_version: 3`、`profile: lvs-anime25d-v1`を正規形式とする。Pico用の別リグへ変換せず、`rig2d/`をそのままキャラクターパックへ置き、`meshRig.rig`から直接読む。旧`anime25d-v1` version 2は互換確認に限る。
- 低頭身2Dは従来の上半身位置、長身・実写・Live2Dは胸全体がぎりぎり入る胸上を初期構図にする。
- チャット／バーの小型アイコンは首上にする。保存済みの利用者構図は初期値より優先する。
- 画面位置の手動調整は別作業。依頼がなければ原画やmanifestへ焼き込まない。
- 配信背景は1920×1080以上を基準にする。目立つ劣化がなければWebPやJPEGも使える。
- 起動準備中はネイティブ窓を隠し、仮画像・別キャラ・未読込テクスチャを出さない。選択キャラと背景、必要な音声、最終寸法と描画が揃ってから表示する。寝姿素材は居眠り表示専用として保持し、準備画面へは出さない（`boot.useSleepingOnStartup`と`bootSleeping`は廃止済みなので復活させない）。

## MMDの登録・表示

- `cargo xtask setup mmd`を明示実行して固定ランタイムを準備する。元PMX／テクスチャは変換せず、設定「キャラ登録」のMMD入口で管理コピーを作る。フォルダ・ZIPに複数PMXがある場合は対象PMXを直接指定する。
- `character_pack.rs`とPMX検証モジュールを検証正本とする。外部参照、欠損テクスチャ、容量超過は登録時に止める。原本とコピーの内容一致を検証する。
- 姿勢は`ui/mmd-renderer.js`の実行時ボーン設定、表情と口パクは既存モーフで制御する。PMXを保存し直さない。自然姿勢と原姿勢を比較し、袖・胴体・手の食い込みを確認してモデルごとの`mmd.naturalPose`を調整する。
- 両モデルで透過背景、胸上／全身、瞬き、閉口と音声同期、切替中断、全7表示形式を確認する。ブラウザ検証と本体WebView検証を区別する。剛体物理や音声モデルの学習を初版対応と報告しない。
- 企画デモのモデル、人格、撮影画像はローカルに保持し、汎用コードと自作fixtureだけをPRへ入れる。手順・利用方法はSPEC／MANUAL／DEVELOPMENTの担当範囲へ同期する。

## 原画と解像度

- 顔がアップになる用途では、最初から胸上のネイティブ解像度で制作する。低解像度の全身絵を拡大して成果物にしない。
- Photoshop「ディテールを保持 2.0」は、同倍率で顔、髪、輪郭が実際に改善するときだけ使う。候補を別名で保存し、元画像を上書きしない。
- Photoshopの生成AIは、権利を持つ画像の局所補修に限定する。本人性、服、髪型、構図を無断で作り替えない。
- Pillow、ブラウザ、CSS等による単純なピクセル拡大を高精細化と呼ばない。
- `characters/<id>/layers/*.png` は制作正本として残し、`scripts/build-character-animations.py` だけで `runtime-layers/*.webp` と `frames/<世代>/*.webp` へ変換する。変換後は寸法と復号RGBAを元画像と完全一致させ、1画素でも違えば採用しない。
- 生成後に `cargo xtask sync-characters` を通し、配信コピーから制作正本の `layers/` が除外され、manifestの `runtimeLayers` と `framePaths` がlossless WebPだけを指すことを確認する。
- 自前メッシュリグは`mesh-rig/source/regions.json`を制作設定とする。同梱通常2Dは`build-separated-mesh-sources.py`で胴、後ろ髪、左右腕、顔、前髪の同一座標PNGを先に生成し、`build-mesh-rig.py`で`parts/*.webp`と`rig.json`を再生成する。中立合成が基準画像と画素一致しない場合は停止する。`runtimeScale`は1以下、`source/`は配信除外とし、広可動域へ広げる場合は隠れ領域を描き足した原画が揃うまで実装済みと扱わない。

## 表情差分と透過

- 画像生成サービスが返した全身画像をそのまま状態フレームにしない。目と口の局所パッチだけを基準画像へ合成し、マスク外の画素を同一に保つ。
- 半目は基準目と閉じ目の中間から作り、下まぶたを巻き込まない。自動モーションOFF時は必ず中立の目と口へ戻す。
- 専用クロマ背景では、外周だけでなく腕と胴の間・髪の隙間など閉じた背景色も除去する。色かぶり除去、柔らかいマット、不透明な被写体色から半透明縁への色伸長を組み合わせる。
- 透過素材は白／黒／暖色背景へ原寸合成し、髪や服の周囲に背景色、閉じた色面、矩形、太い縁がないことを確認する。
- 実写素材では髪、胸、肩、腰を独立した `springPhysics.regions` として重ねる。`prefers-reduced-motion` では完全に静止させる。
- 髪の物理マスクは外側の毛先だけに置き、ぼかし端も目、鼻、口、輪郭、頬へ入れない。顔の二重線、局所拡縮、サイズ変化が出たら不合格にする。
- 物理演算は最終レイアウトの表示倍率で確認する。移動量が1描画ピクセル未満なら「見える物理演算」として採用せず、複数マスクの重なりで被写体が変形する設定も採用しない。
- 通常、笑顔、考え中、悲しみ、会話は、顔全体を拡縮せず目元と口元で判別できる差を付ける。静止画1枚ではなく複数時点を比較する。
- 実画面を見る前に`cargo xtask character-quality`で画素検査を通す。差分レイヤーが顔より下の非輪郭部を書き換えている、半目が閉じ目の透明度変更である、レイヤーが基準画像と一致する、といった不合格は目視より先にここで止める。新規キャラは`--strict`で注意も0件にする。不合格を素材の修正なしに`characters/quality-baseline.json`へ足して回避しない。
- 表情を比較するときは待機動作・自動まばたき・視線を固定した顔の並列画像も確認する。全身の揺れによるハッシュ差を表情成功と扱わない。旧実写リグと統合顔レイヤーへ眉・口角の計算が届くこと、照れの赤みと頬線が目へ重ならないことを確認する。
- 1体で見つかった見た目の不具合は、現行の全キャラクターで同じ箇所を確認する。修正後も全員の同条件画像を比較し、確認した対象と未確認の有無を報告する。代表3体だけの合格で共通修正を完了扱いにしない。
- `node tools/check-rig-feature-matrix.mjs temp/<新規検証名>`で全7体の6表情・6口形・閉眼・発話/停止を単独撮影する。胴体の一様平行移動を除いた局所変位と停止後の収束を検査するが、全画像の目視を省略しない。`scripts/test-face-landmarks.py`で別原画の解析拒否と口差分の目への混入禁止も検査する。
- 口操作の検査は全員の完走とreport.errorsが空であることを確認する。途中で保存されたreportを全体成功と扱わない。原寸の目素材は完全一致、GPU縮小描画は色差2以下かつ変化画素0.1%以下を許容上限とし、二重の目や輪郭変化を丸め誤差として通さない。
- 旧パックへ実測座標を追加する場合は`python scripts/import-face-landmarks.py <正規パイプラインのキャラフォルダ> characters/<ID>`を使う。原画SHA一致が条件であり、手書き座標や同じ表示名の別原画で代用しない。続けて通常の`build-mesh-rig.py`を実行し、正本の特徴点がリグと開口素材へ再反映されることを確認する。
- 高品質試作は84フレームを標準とし、待機24、考え中24、会話24、悲しみ12を基準にする。

## 外部サービスとレビュー

- ユーザーが許可した画像だけを外部画像生成・編集サービスへ送る。利用後に送信素材、送信先、目的を事後報告する。
- 画像品質と採用方針は作業中のエージェントが実画像で確認する。ユーザーが同一PR内でClaudeレビューを明示した場合だけ対象パス、用途、判定基準をClaudeへ渡し、回答を実画像と照合する。
- ユーザー指定でClaudeへ渡す7形式一覧は、対象を絞った直前の撮影で一覧が上書きされていないことを確認する。チャット／バー／配信は個別画像も渡す。
- 透過PNGをビューアーの緑や黒の背景色だけで未透過と判定しない。四隅alpha、透明／半透明／不透明ピクセル数、最上段の不透明数を数値で添え、実合成画面の縁と一緒に判定する。
- Claudeの判定だけで合格にしない。PicoAgentの実レンダラーで通常、配信、チャット、バー、作業、ウィジェットを撮影する。
- README／LPの通常・配信公開動画は、`SHOOT_README_GALLERY=1`／`SHOOT_SITE_GALLERY=1`を正規入口にし、実画面の生PNGから6秒・16fps・960px幅へ直接書き出す。既存WebPの再圧縮や引き伸ばしをしない。実音声の再生開始後だけ口パクを見せ、考え中から待機へ戻す。対応キャラは撮影中に口開閉値が実際に発生したことを自動検査する。12秒・30fps・960pxの高解像度版は個別の比較・保管を明示された場合だけ作り、公開埋め込みへ採用しない。
- 動画は先頭、中間、終端を確認し、瞬き、音声同期の口パク、透過、継ぎ目、胸上構図、物理演算、背景サイズを検査する。元モデルに口パラメータがない例外は明記し、別の身体モーションを口パク成功として扱わない。

## 再構築と検証

Windowsで手動確認だけを行う場合は、リポジトリ直下の`キャラクター確認画面を開く.bat`を使う。独自2.5Dは比較画面、MMDと登録済みLive2Dは保存中の選択を変えない実アプリ確認を選べる。MMDとLive2Dをブラウザだけの代替表示で合格にしない。

Aivis音声をキャラクターへ割り当てる場合は、本人または権利者が明示的に許諾した公式モデル・収録素材だけを使う。配信・動画から切り出した音声は、利用者が権利者の許諾を明示した場合だけ再学習へ使う。生成サンプルは複数文で聴取し、0dBへ達するクリップ、定常ノイズ、破裂音、声の混入があれば採用を止める。PicoAgentは合成前の`volumeScale`を0.72以下へ制限して約3dBの余裕を取るが、これは入力素材や学習済みモデルのノイズを直す処理ではない。モデル自体の不良は、次の手順で清浄な素材から作り直す。

### 音声モデルの再作成（Style-Bert-VITS2 JP-Extra）

- 既存の元音声、`Data/<ID>`、`model_assets/<ID>`、導入済みモデルを上書き・削除しない。作業物は`temp/voice-models/`だけに置き、新データセットは`Data/<ID>-clean-v<N>`、中間の切り出しは`Data/<ID>-v<N>-src*`の新しい名前にする。SBV2の`slice.py`は出力先の`raw`が既にあると削除するため、既存名を指定しない。
- 元音声はBGM・効果音・ゲーム音・ゲストの声がない単独雑談を選ぶ。候補は途中5分だけ取得し、50msフレームの下位10%、1秒以上-45dB未満になる回数、静区間の狭帯域ピーク数で比べる。BGMが常時ある配信は断片選別でもほぼ全件落ちるため、閾値を緩めず素材を替える。
- `scripts/select-voice-training-clips.py`だけを選別の正規入口にする。SBV2の`.venv`のPythonで、`quiet-sections`（1分区間の抽出。冒頭・末尾5分を除外）→ SBV2の`slice.py --time_suffix`と`transcribe.py` → `measure`（クリップ、DC、発話率、合間のノイズ床とSNR、境界区間、破裂音、話者類似度、Whisper再認識のカナ誤り率）→ `select`の順に実行する。測定レポートはWAVのSHA-256、話者、言語、文章と結び付け、元WAVを変えた場合は新しいレポート名で測定し直す。`select`は複数の元データセットをまとめられるが、同名WAVが重複する場合は元データ側で一意の名前へ直す。出力先が存在すれば停止し、まず`--dry-run`で除外理由と分布を確認する。
- 選別の既定値は-23LUFS正規化後のノイズ床-48dB以下、SNR 22dB以上、境界区間-30dB以下、破裂音フレーム3以下、話者類似度0.55以上（2秒窓0.40以上）、カナ誤り率0.25以下、2〜12秒。音響指標だけを直した場合は`measure --reuse-report`でWhisperと話者埋め込みを流用する。
- 採用断片はDC除去、発話前120ms・後150msまでの切り詰め、10msフェード、-23LUFSかつピーク-1dBFS以下の単純ゲインだけを行う。SBV2の`resample.py`へ`--normalize`を付けない（pyloudnormの正規化でクリップが発生するため）。
- 学習データの合間（0.25秒以上の間）は、合成音の検査と同じピーク基準でも測る。SBV2は学習データの合間に残る環境音を覚え、合成時の読点・疑問符・言いよどみの間にそのまま出す（clean-v2のみこは合間の中央値がピーク基準-41.9dBで、合成音の合間ノイズも-42〜-48dBだった）。-23LUFS基準の絶対値だけで「清浄」と判定しない。合間がうるさい場合は`scripts/clean-voice-training-pauses.py`で発話区間外だけを減衰させた別名データセット（`Data/<ID>-clean-v<N>-gated`）を作り、`pause-cleaning.json`の前後値を確認する。声に重なる雑音や残響は`scripts/enhance-voice-training-chunks.py`の`pack`→`deepfilter`→`unpack`で除去し、`Data/<ID>-clean-v<N>-deepfilter`へ切り戻す。[DeepFilterNet公式リリース](https://github.com/Rikorose/DeepFilterNet/releases/tag/v0.5.6)のWindows版0.5.6（約27MB、SHA-256 `75e11fa16445f560cb6b021521ddb89e89270d13b83089705d98776f58fd7915`）を明示取得し、`--compensate-delay`は使わない。外部サービスへのアップロードと手動補正を正規工程にしない。合間減衰版と同じstep数で検査し、ノイズと話者類似度で採否を決める。学習量の追加、合成パラメータ、読み上げ文の分割では直らない。
- 学習は`scripts/train-voice-model.ps1`だけを正規入口にし、`-Stage preprocess` → `train` → `export`の順に実行する。SBV2の`config.yml`の`model_name`は学習とスタイル生成の保存先を決める共有状態なので、キャラを替えるたびにこのスクリプトを通す。GUIや個別コマンドで複数キャラを交互に扱わない。
- 8GB級GPUではbatch 2、fp32（`bf16_run`は使わない）、保存間隔1000 stepにする。JP-Extraの学習スクリプトは勾配累積に対応しない。学習開始後にWindowsの`\GPU Process Memory(pid_<PID>*)\Shared Usage`を確認し、数GB単位で共有メモリへあふれて1 stepが極端に遅い場合は、品質設定を変えず`-Stage train -BatchSize`だけを下げて再開する。止めるときは、コマンドラインで自分が起動した`train_ms_jp_extra.py`とその親の`train-voice-model.ps1`を確認してからプロセスツリーごと終了する。
- チェックポイントの検査を学習と同時に動かさない。GPUではVRAMがあふれ、CPUではWhisper large-v3と合成モデルがメインメモリを使い切り、学習ごと停止する（32GB機で実際に発生）。保存直後に自分が起動した学習を止め、`scripts/check-synthesized-voice.py sbv2 --device cuda`で検査してから`-Stage train`で再開する（最新の`G_*.pth`から続きを学習する）。合成は毎回ゆらぐため、1文1回の比較で判断しない。`--text-set pauses --repeat 3`（既定6文に読点・疑問符・感嘆符・言いよどみを含む4文を加え、全体を3周）で合成し、旧モデル・別データセットと同じstep数どうしで比べる。学習は`-Stage train -StopAtStep <N>`で指定stepの保存後に自動で止めてから検査する。保存済み候補が停止目標以上なら、既存候補を検査するか、より大きい目標を指定する。ノイズ床・SNR・電子音・冒頭末尾・カナ誤り率・話者類似度が改善しなくなった点、または悪化し始めた点（過学習）を採用候補にする。SBV2の直接合成はピークを0dBへ正規化するため、クリップと音量はAivis経由（`aivis`モード、`volumeScale`上限0.72）で判定する。
- 採用候補は`-Stage export`でONNXとAIVMXへ書き出し、AIVMX内の`model_name`と話者を確認する。既存ONNXは再利用せず、採用チェックポイントのコピーから一時出力先へ変換し、整合確認後だけ確定名へ移す。書き出し先または未完了の`.incoming`があれば停止する。

```powershell
uv run --with pillow python scripts/build-character-animations.py --character-dir characters/<id>
uv run --python 3.12 --with pillow --with numpy --with opencv-python-headless python scripts/build-separated-mesh-sources.py <id> # 同梱通常2Dの分離原画
uv run --python 3.12 --with pillow python scripts/build-mesh-rig.py <id> # meshRigを持つ場合だけ
cargo xtask sync-characters
cargo xtask character-quality --character <id> --strict
uv run python -m http.server 8776 --bind 0.0.0.0 --directory .
$env:PICO_PLAYWRIGHT_MODULE='C:\Users\benyb\.cache\codex-runtimes\codex-primary-runtime\dependencies\node\node_modules\playwright\index.mjs'
node tools/check-lvs-character.mjs temp/pico-lvs-check # LocalVTuberStudio形式の場合
node --check ui/renderer.js
cargo test --manifest-path xtask/Cargo.toml
cargo test --lib --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
git diff --check
```

PlaywrightのパスはCodex Desktopの`load_workspace_dependencies`で現行値を取得し、上記に存在しなければ推測しない。`check-lvs-character.mjs`の固有ハッシュ数は動作停止の検出に使うだけで、画質合格の根拠にしない。続けてChromeの`tools/mesh-rig-preview/`で原画と並べ、閉口と5母音、650ミリ秒のまばたき、首・胴を裂かない控えめな待機動作、白・黒・市松背景を目視する。

画像を追加・更新したコミットでは `git lfs ls-files` に対象PNG／WebPが載ることも確認する。READMEやLPのプレビューを変更した場合は `tools/screenshots/shoot.mjs` で正本から撮り直す。通常2Dの実行画像を変えた場合は `SHOOT_CHARACTER_MATRIX=1` で全キャラ・全7形式を撮影し、速度警告0件と実画像の欠け・矩形・輪郭劣化なしを確認する。実画面の確認後にREADME、配布README、LP、`SPEC.md`を同期し、採否と検証結果をPR本文へ書く。未確認項目だけを`docs/TASKS.md`へ残す。
