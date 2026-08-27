# 未完了タスク

コードだけでは完了できない項目、または着手条件がまだ揃っていない項目だけを置く。作業の経緯と完了結果はPR、恒久ルールは[AGENTS.md](../AGENTS.md)、現行仕様は[SPEC.md](../SPEC.md)を正本とする。完了した項目はこの文書から削除する。

## 販売ゲート

`cargo xtask release-check`が検出する次の4件は、販売版の作成前に外部確認を完了する。

| 項目 | 完了条件 |
|---|---|
| EULA | 専門家確認を反映し、`distribution/release.json`の判定を更新する |
| プライバシー | 専門家確認を反映し、判定を更新する |
| 第三者資産 | 再配布条件とライセンス台帳を確認し、判定を更新する |
| Live2D | 拡張アプリ審査と契約参照を記録する |

## 条件待ち

| 内容 | 着手条件・完了条件 |
|---|---|
| CLI・初回案内のWindows実機確認 | Claude Code／Codex CLIのセッション再開と更新された前提の反映、停止、初回案内から戻った後の再操作、Aivis導入直後の読み上げを確認する |
| 完全な新規Windows利用者での初回起動 | Windows Sandboxまたは専用利用者を用意し、既存データを動かさず確認する |
| 保存状態を伴うWindows操作 | 8方向リサイズ、端寄せ復元、旧版Aivis更新表示、稼働中ローカルAIでのキャラ別挨拶を安全な隔離状態でまとめて確認する |
| 読み取り不能データの保護 | 隔離した`config.json`と`history-<キャラID>.json`を権限またはロックで読めない状態にし、理由表示と原本非上書きを確認する |
| Windows表示の残り | 配信立ち絵の0.5px差、待機時の局所動作、400px／600px幅の日本語ラベル、2つ目の起動による既存ウィンドウ前面化を実画面で確認する |
| 別物理ボリュームへの配置 | 一時ディレクトリと配置先を別のローカル物理ボリュームにして、`cargo xtask setup engine`のEXDEVコピー経路を確認する |
| 旧名ローカルモデルの照合表示 | 現行固定サイズ・SHA-256に合う旧保存名GGUFを隔離状態で用意し、起動画面の「ローカルモデルを確認中」表示を確認する |
| Linuxの専用GPU経路 | 固定llama.cpp版に対応するLinux CUDA／ROCm資産を独自ビルド・配布し、対応実GPUで起動とGPU利用を確認する。現状のUbuntu版はVulkan→CPU |
| Ubuntu実Wayland | Ubuntu 24.04 LTS x64の実Wayland環境で、画面共有許可、画面取得、入力拒否時の理由付きエラーを確認する |
| macOS版の実機QA | Apple Silicon機で`cargo xtask build-beta --bundles app,dmg`を実行し、Gatekeeper回避後の起動、マイク・画面収録・アクセシビリティの許可、ログインキーチェーンへのAPIキー保存と再起動後の復号、同梱ローカルAIエンジンの起動、対応OS版の下限（現在はmacOS 11）を確認する |
| macOS Intel版 | 現在は配布対象外。`xtask/src/bundled.rs`が同梱エンジンの資産をxtask自身のホストarch（`cfg!(target_arch)`）で選ぶため、Apple Silicon機から`--target x86_64-apple-darwin`で作るとarm64版エンジンが入ったx64アプリになり、ローカルAIが起動しない。対象へ加えるには、実機のIntel Macでビルドするか、`bundled.rs`をビルド対象archへ追従させる（xtaskへ対象archを渡し、資産選択・SHA-256照合・配置先をすべてその値で行う）。どちらを採るか決めてから着手する |
| macOS版の署名・公証 | Apple Developer Programの証明書と`notarytool`を用意し、署名・公証を通してから`distribution/release.json`の`platforms`へ`macos`を追加し、`signing.macosReady`を判定する。未署名アルファの間はGatekeeperの回避手順を取得ページで案内する |
| 高精細キャラの分離取得 | アルファ版の試用結果を踏まえ、画質を落とさず配布容量を分離する設計を決める |
| Free／Pro設計 | アルファ期間終了時に、スキル制限、ライセンス入力欄、`beta-full`時の挙動を制限設計から見直す |
| 紹介資料の動画再生 | Windows PowerPointのスライドショーで`PicoAgent-Overview.pptx`表紙の2動画が自動・無音再生されることを確認する |
| 複数AIへの画像入力 | 候補AIごとの画像能力差と失敗時の扱いを設計してから、単一AI限定を変更する |

## FreeTokenローカルエンジンの段階導入

2026-08-26時点では、[FreeToken](https://github.com/FlashML-org/FreeToken)を全環境の`llama-server`と一律に置き換えない。FreeTokenはOpenAI互換APIを提供し、公式のWindows向けPython 3.12 wheelを使ってネイティブ動作できるが、NVIDIA GPU、対応するPyTorch／CUDA／Tritonの組合せが必要である。対応モデルもHugging Face safetensors／FTWとGemma 4の一部GGUFに限られ、既存のPicoAgent用GGUFをすべてそのまま使えるわけではない。

初期実装は、既定の`llama.cpp`を維持したまま、明示選択した対応NVIDIA環境だけでFreeTokenを使う第2の管理エンジンとする。FreeTokenの起動失敗を無言で`llama.cpp`へ降格させず、理由と`llama.cpp`へ戻す操作を画面に出す。FreeToken本体、CUDA／Pythonランタイム、モデルは、利用意思とGPU要件を確認する前に取得しない。

| 段階 | 実施内容 | 完了条件 |
|---|---|---|
| 1. Windows接続PoC | 公式Windows wheelをPython 3.12の分離仮想環境へ導入し、実体パスと`--version`を検証してPicoAgent管理ポートで`ft serve`を起動する。PowerShellの`ft`は標準の`Format-Table`エイリアスでもあるため名前だけで判定しない。PicoAgentがFreeToken自体を再配布する処理はまだ作らない | RTX 5060 Laptop GPU搭載Windows実機で、公式既知対応モデルによる`/health`・`/v1/models`・通常会話・SSE・ツール呼び出し・停止後の子プロセス回収を確認する。長いプロンプト、VRAM不足、WDDMのピン留め可能RAM上限で失敗した場合は画面へ理由を出す |
| 2. エンジン管理の分離 | `engine.rs`の共通ライフサイクルと、`llama.cpp`／FreeToken固有の実行ファイル解決、モデル、引数、probeを分離する。`local`プロバイダのOpenAI互換メッセージ変換は共用する | 既定`llama.cpp`のWindows／macOS／Ubuntu経路とCUDA→Vulkan→CPUの既存フォールバックが変わらず、FreeToken選択時だけ固有経路へ入る。ポート上の既存サーバーはモデルIDと所有区分を照合し、別モデルや管理外プロセスを誤用・停止しない。アプリ終了、`cargo xtask stop`、導入・削除の全経路で、実体パスまたはPicoAgent固有のコマンドラインに一致するプロセスツリーだけを停止する |
| 3. モデル管理 | モデル実体とエンジン互換性を分離して管理する。同じGGUFが両エンジンの対応形式なら1ファイルを共用し、HF safetensors／FTWなどFreeToken専用形式だけ別実体にする。モデル名と量子化方式だけで互換とみなさず、HFリポジトリID、固定revision、必要ファイル、容量、SHA-256、ライセンス、GGUF変換元と対応エンジンを記録する | 対応外GGUFを渡さず、重複ダウンロードを避け、取得前に必要ディスク・RAM・GPU条件を表示する。中断時の`.part`／stagingを回収し、利用者の共有Hugging Faceキャッシュや外部配置モデルを削除しない |
| 4. 管理導入 | 上流がWindows向けの版固定可能な取得元と再配布条件を提供した場合だけ、FreeTokenランタイムの明示導入を追加する | Python、Torch、CUDA、FreeToken、ネイティブwheelを版・サイズ・SHA-256・ライセンスで固定し、導入・更新・失敗時復元・削除を実パッケージで確認する。固定できない間はFreeToken Desktop／利用者導入済みランタイムを使う |
| 5. 対応範囲拡大 | Ubuntu 24.04 x64のNVIDIA実機を追加し、macOS／AMD／Intelは上流の公式対応後に別判定する | 対応GPUで実起動とGPU利用を確認する。非対応環境ではFreeTokenの大容量資産を取得せず、既定`llama.cpp`を維持する |

設計上の設定正本は、実装時に`PICOAGENT_LOCAL_ENGINE=llama|freetoken`を追加し、既定を`llama`とする。FreeToken固有のプログラム、モデル、GPU、ポート、追加引数は同じ設定画面の選択中エンジン内だけに表示する。任意引数は安全な構造化項目を優先し、自由記述を追加する場合もshellを介さず引数配列として渡す。

実機PoCではFreeToken 0.1.2、Python 3.12.11、PyTorch 2.11.0、CUDA 13.0の組合せでWindowsネイティブAPIサーバー起動まで確認した。一方、Gemma 4 E4B Q4_0は公式既知対応一覧外で、GGUF設定パーサーが`attention.head_count_kv`の単一整数を層別配列として扱い、その後もMoEメタデータを要求するため推論開始前に停止した。さらに、Gemma 4 26B A4B Q4_0でも、Bartowski変換のGGUFはMoE重み結合時のテンソル寸法不一致（2310対2992）で停止した。同じモデル名とQ4_0表記でもGGUF変換元によってFreeToken互換性が異なるため、E4Bとこの26B GGUFは`llama-server`専用として扱う。FreeTokenを実装へ進める前に、上流が対応を確認した正確なGGUFまたはHF safetensors／FTWを固定し、通常会話まで完走させる。上流の未解決問題により起動または長文が停止する場合を成功扱いにせず、要求timeout、キャンセル、プロセスツリー回収まで合格条件に含める。

## 通常2Dキャラのメッシュ変形リグ（疑似Live2D v2）

同梱の通常2Dキャラは、差分絵の差し替えと剛体のバネ変形だけで動いている。セナの`animations.idle`は12コマ・`fps`3.2で、`frameDurations`の大半が760ミリ秒の保持である。これはパラパラ漫画であり、パラメータを毎フレーム連続補間してメッシュを変形させるLive2D（ひより）とは原理が違う。フレーム枚数を増やしても、素材容量と生成時間が増えるだけで滑らかさの上限は変わらない。この項目は、差分絵をパーツへ分解し、PIXIのメッシュを連続パラメータで変形させる自前リグを追加して、Cubism Editorとモデル審査に依存せずに滑らかな動きを得ることを目的とする。

既存のフレームアニメと差分絵リグは削除せず、メッシュリグを第3の描画形式として足す。WebGL初期化の失敗、`prefers-reduced-motion`、`chat`／`bar`表示のように現在もバネ変形を止めている経路は、従来の差分絵表示へ落とす。最初の対象はセナとし、`layers/*.png`（1780×3600、内容bbox 222,284-1567,3317）を制作正本のまま使う。

素材の枚数は制約ではない。`motionProfile=full`の9レイヤーは同期時に検査される必須の下限であって上限ではなく、`rin-tha3`は既に21枚を持つ。ランタイムの`probeRigLayers`が必須にしているのは`base`／`mouth_open`／`mouth_closed`／`blink_half`／`blink_closed`の5枚で、視線2枚は任意である。`build_runtime_layers`は固定の名前リストだけを実行用WebPへ変換し`layers/`をglobしないため、原画を増やしても既存の生成・同期経路は変わらない。メッシュリグに必要な原画は遠慮なく足す前提で設計する。

### 採用理由と、この方式の上限

Cubism経由を採らないのは、`.moc3`がCubism Editorでしか生成できず、AIでも自動でも作れないためである。`scripts/build-live2d-psd.py`はPSDの雛形までは作るが、パーツ整形とリギングは人手に残る。またLive2D経路は「販売ゲート」の拡張アプリ審査を必要とするが、自前メッシュはPIXI（MIT）だけで完結するためその判定に触れない。

代償として、頭の回転は横向きの描き起こしが無い範囲までしか破綻せずに振れない。パーツごとの奥行き係数による視差とせん断で近似するため、目安として左右±15度、上下±10度を超えると顔が歪む。この上限をパラメータの可動域として実装で固定し、超える演出は従来のフレームアニメ側で表現する。滑らかさと引き換えに首振りの角度を捨てる判断であり、より広い可動域が要るキャラはLive2D経路を選ぶ。

### 成果物とパラメータ

`characters/<id>/mesh-rig/`へ、`rig.json`とパーツPNGを置く。パーツ構成と描画順は`scripts/build-live2d-psd.py`の`HIYORI_PARTS`（後ろ髪→体→腕→首→耳→顔→口→鼻→頬→目玉→目→まゆ毛→横髪→前髪）をそのまま採用し、Live2D側と対応付けられる状態を保つ。

パーツPNGは必ず各パーツのbboxへ切り詰め、原画座標のオフセットを`rig.json`へ持つ。全画面サイズのまま15枚持つと1780×3600×4バイト×15で約384MBのテクスチャになり、常駐マスコットとして成立しない。切り詰めれば合計約60MBまで下がるが、表示は`LIVE2D_RENDER_WIDTH`285×`LIVE2D_RENDER_HEIGHT`315のCSS画素であり原寸は過剰なので、実行用に縮小段階を1つ作る。縮小は許可されるが、`AGENTS.md`のとおり引き伸ばし拡大は禁止する。

パラメータ名はLive2Dの標準名（`ParamAngleX`／`Y`／`Z`、`ParamBodyAngleX`／`Y`／`Z`、`ParamEyeLOpen`／`ROpen`、`ParamEyeBallX`／`Y`、`ParamBrowLY`／`RY`、`ParamMouthOpenY`、`ParamMouthForm`、`ParamBreath`）へ合わせる。ひよりとメッシュリグで駆動側のコードを共用し、キャラごとに別の名前を作らないためである。

### 段階

| 段階 | 実施内容 | 完了条件 |
|---|---|---|
| 1. PIXIの分離 | `xtask/src/bundled.rs`の`fetch_live2d_runtime`が`ui/vendor/live2d/`へまとめて置いているPIXI 6.5.10を、`ui/vendor/pixi/`へ分けて`cargo xtask setup pixi`で単独取得できるようにする。Cubism CoreとpixiのSDK読み込み順（`ui/renderer.js:1543`）は現状を維持する | Live2Dランタイム未取得の環境でメッシュリグが動く。Live2Dキャラの表示と世代切替が回帰しない。`ensure_file`のままハッシュ未照合になっている`pixi.min.js`と`cubism4.min.js`へSHA-256照合を足し、Cubism Coreと同じ固定水準へ揃える。第三者ライセンス台帳へPIXI単独（MIT）の記載を追加する |
| 2. パーツ分解と隠れ領域の原画 | 隠れ領域は原則として原画を足して実画素から取る。前髪を外した額、腕を外した胴、横髪を外した頬など、遮蔽物を描かない原画を既存9レイヤーと同じ1780×3600・同じ座標系で用意し、`characters/<id>/mesh-rig/source/`へ置く。`layers/`へは混ぜず、実行用変換の対象にしない。`scripts/build-mesh-rig.py`を追加し、通常の原画と隠れ無し原画を突き合わせてパーツマスク・パーツPNG・`rig.json`を出力する。`HIYORI_PARTS`の比率は初期bboxの目安として使い、最終マスクは実画素から決める。AIのインペイントは、原画を足しても埋まらない細部の補助に限る | パーツを可動域の端まで動かしても穴・素抜けが出ない。隠れ領域が原画由来かインペイント由来かを`rig.json`へ記録し、インペイント面積の割合をQAへ残す。透過市松と透明画面の両方で、ハローと矩形の消し残しが無いことを確認する。制作正本の`layers/`を上書き・削除しない |
| 3. メッシュ描画 | `ui/renderer.js`へ`rigActive`／`live2dActive`と並ぶ第3経路を足し、`loadCharacter`（`ui/renderer.js:955`付近）で`mesh-rig`の有無を判定して分岐する。各パーツを格子メッシュとして描画し、体→首→頭→顔パーツの親子変換で頭の動きが目・口・前髪へ伝わるようにする。頭の回転はパーツごとの奥行き係数による視差とせん断で近似する | 描画キャンバスがLive2Dと同じ`characterView`内へ入り、`petSprite`との重なり順、ドラッグ領域、配置モードのMoveable操作が壊れない。WebGL初期化に失敗したら差分絵表示へ落ちる |
| 4. 連続入力の接続 | 現在は捨てている連続値をパラメータへ直結する。`rigMouthLevel`は振幅の包絡線をすでに持っており、Live2Dへは連続値のまま渡しているのに、差分絵経路だけ`rigMouthLevel > 0.3`の真偽値へ潰している（`ui/renderer.js:1391`）。この連続値をそのまま`ParamMouthOpenY`へ渡す。`rigLookDirection`の-1／0／1はカーソル位置からの連続値にして`ParamAngleX`／`Y`と`ParamEyeBallX`／`Y`へ分配する。`scheduleBlink`の`blink_half`→`blink_closed`→`blink_half`は`ParamEyeLOpen`の連続イージングへ置き換える。呼吸は`ParamBreath`の正弦で与える | 読み上げ音量に口の開きが追従し、しきい値の跳ねが出ない。視線追従が既定OFFのまま（`gazeTrackingEnabled`）で、有効時だけ連続追従する。差分絵経路のまばたき・口パク・視線は従来どおり動く |
| 5. 物理演算 | `animateSpringMotion`のバネ積分（`ui/renderer.js:944`）を、固定の正弦入力ではなく`ParamAngleX`／`ParamBodyAngleX`の速度で駆動する形へ一般化し、髪と胸元のパーツへ適用する。既存の`secondaryMotion`／`springPhysics`定義は差分絵キャラ向けに残す | 頭を振ったときだけ髪が遅れて揺れて収束する。`reduce-motion`、`document.hidden`、`napping`、`chat`／`bar`表示で停止する現在の条件を維持する |
| 6. 検証と登録 | `character_pack.rs`の必須判定（`live2d.model`または`animations.idle`。`src-tauri/src/character_pack.rs:93`）へメッシュリグを第3の形式として足す。ただしセナは`animations.idle`を残し、フォールバックを常に成立させる。`docs/CHARACTER-ASSET-WORKFLOW.md`と`characters/README.md`へ制作手順を追記する | `cargo xtask verify`が通る。`cargo xtask sync-characters`が`mesh-rig/`を配信コピーへ含め、制作正本の`layers/`は従来どおり除外する。実画面でセナのメッシュリグ表示、フォールバック表示、Live2Dのひより、差分絵の他キャラを1回の起動でまとめて確認する |

### 差分絵経路の先行改善（メッシュリグと独立）

メッシュリグを待たずに、差分絵経路の状態切り替えだけでも品質は上がる。リップシンクの`rigMouthLevel`は適応しきい値と非対称包絡線を通した60fpsの連続値としてすでに算出されており、粗いのは最後の量子化とタイミングだけだからである。まばたきの非対称配分と口のしきい値ヒステリシスは数値だけで閉じるため先行して実施した。効果の切り分けにもなり、今の不自然さのうちどれだけが量子化由来だったかをメッシュリグ着手前に把握できる。

残りは素材か描画先の変更が要るため、この項目に含めない。

| 見送り | 理由 |
|---|---|
| 口の中間レイヤー追加（`mouth_half`等） | 量子化を4段へ細かくできるが専用作画が要る。`AGENTS.md`が半目の透明度生成を禁じているのと同じ理由で、既存の開閉2枚からの合成で作らない |
| 目・口のクロスフェード | `syncSpringMotionFrame`が`petSprite.currentSrc`を全バネ領域の`<img>`へ複製している（`ui/renderer.js:825`）。本体だけクロスフェードすると上に重なるバネ領域がハードカットのまま残り、差分が領域へ掛かるキャラで顔が二重に見える。バネ領域側も同時にクロスフェードするか、領域と顔の非重複をキャラごとに保証する必要がある |
| 頭・髪・呼吸の連続化 | パーツが独立して動く必要があり、量子化を細かくしても近づかない。段階3以降で扱う |

段階1と2は独立しているため並行してよいが、段階3以降は順に進める。段階2の隠れ領域の品質がこの方式の見た目をほぼ決めるため、段階3へ進む前にパーツを可動域の端まで動かした静止画で穴の有無を確認する。隠れ無し原画を用意できないパーツが残る場合は、そのパーツの可動域を狭めるか、メッシュリグの対象から外して従来の差分絵へ委ねる。インペイントで広い面積を作らない。

## 責務が集中したファイルの再評価

肥大化した対象は、状態の所有者、副作用、公開インターフェース、独立して機械検査できる境界を棚卸しする。分割後の依存が減る対象だけを分け、一体の方が効率的なら維持する。挙動変更と混ぜず、各段階を独立したPRで行い、公開中のPRと同じファイルを同時に変更しない。

| 対象 | 現時点の判断 | 再評価・着手条件 |
|---|---|---|
| `ui/settings.js` | 中央の設定スナップショット、未保存判定、保存処理は一体で維持する。目覚まし・スケジュールの専用フォームとJSON整形は独立していたため分離済み | スキル一覧、キャラクター設定、診断表示のいずれかが、中央状態を所有せず入力と描画だけの小さいAPIにできる場合だけ追加分割する |
| `ui/renderer.js` | 責務が複数あり分割候補 | PR #168の統合後、キャラクター描画・リップシンク、通知・目覚まし、会話入力の状態所有者とイベント境界を確認する |
| `src-tauri/src/commands.rs` | Tauri commandの入口は集約する価値があるが、業務処理まで同居している箇所は分割候補 | PR #168の統合後、設定、キャラクター、追加機能、会話についてcommandを薄い委譲にできる責務だけ移す |
| `xtask/src/assets.rs` | キャラクター同期、UI配信ビルド、管理プロセス回収は副作用と検証が異なるため分割候補 | 共有する配布パス判定を複製せず、各処理を独立テストできる境界を決めてから着手する |
