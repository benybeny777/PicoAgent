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
| macOS版 | macOSホストを用意し、`secret.rs`の平文フォールバックをOSのKeychainへ移してから、ビルド・署名・公証・実機QAを行う |
| 高精細キャラの分離取得 | アルファ版の試用結果を踏まえ、画質を落とさず配布容量を分離する設計を決める |
| Free／Pro設計 | アルファ期間終了時に、スキル制限、ライセンス入力欄、`beta-full`時の挙動を制限設計から見直す |
| 紹介資料の動画再生 | Windows PowerPointのスライドショーで`PicoAgent-Overview.pptx`表紙の2動画が自動・無音再生されることを確認する |
| 配布Repoの残存ブランチ | `C:\00_PG\21_PicoAgent-LP`の`claude/picoagent-ai-agent-n0qqia`がマージ済みであることを再確認し、通常のpush経路またはWeb UIから削除する |
| 複数AIへの画像入力 | 候補AIごとの画像能力差と失敗時の扱いを設計してから、単一AI限定を変更する |

## FreeTokenローカルエンジンの段階導入

2026-08-26時点では、[FreeToken](https://github.com/FlashML-org/FreeToken)を全環境の`llama-server`と一律に置き換えない。FreeTokenはOpenAI互換APIを提供し、公式のWindows向けPython 3.12 wheelを使ってネイティブ動作できるが、NVIDIA GPU、対応するPyTorch／CUDA／Tritonの組合せが必要である。対応モデルもHugging Face safetensors／FTWとGemma 4の一部GGUFに限られ、既存のPicoAgent用GGUFをすべてそのまま使えるわけではない。

初期実装は、既定の`llama.cpp`を維持したまま、明示選択した対応NVIDIA環境だけでFreeTokenを使う第2の管理エンジンとする。FreeTokenの起動失敗を無言で`llama.cpp`へ降格させず、理由と`llama.cpp`へ戻す操作を画面に出す。FreeToken本体、CUDA／Pythonランタイム、モデルは、利用意思とGPU要件を確認する前に取得しない。

| 段階 | 実施内容 | 完了条件 |
|---|---|---|
| 1. Windows接続PoC | 公式Windows wheelをPython 3.12の分離仮想環境へ導入し、実体パスと`--version`を検証してPicoAgent管理ポートで`ft serve`を起動する。PowerShellの`ft`は標準の`Format-Table`エイリアスでもあるため名前だけで判定しない。PicoAgentがFreeToken自体を再配布する処理はまだ作らない | RTX 5060 Laptop GPU搭載Windows実機で、公式既知対応モデルによる`/health`・`/v1/models`・通常会話・SSE・ツール呼び出し・停止後の子プロセス回収を確認する。長いプロンプト、VRAM不足、WDDMのピン留め可能RAM上限で失敗した場合は画面へ理由を出す |
| 2. エンジン管理の分離 | `engine.rs`の共通ライフサイクルと、`llama.cpp`／FreeToken固有の実行ファイル解決、モデル、引数、probeを分離する。`local`プロバイダのOpenAI互換メッセージ変換は共用する | 既定`llama.cpp`のWindows／macOS／Ubuntu経路とCUDA→Vulkan→CPUの既存フォールバックが変わらず、FreeToken選択時だけ固有経路へ入る。ポート上の既存サーバーはモデルIDと所有区分を照合し、別モデルや管理外プロセスを誤用・停止しない。アプリ終了、`cargo xtask stop`、導入・削除の全経路で、実体パスまたはPicoAgent固有のコマンドラインに一致するプロセスツリーだけを停止する |
| 3. モデル管理 | モデル実体とエンジン互換性を分離して管理する。同じGGUFが両エンジンの対応形式なら1ファイルを共用し、HF safetensors／FTWなどFreeToken専用形式だけ別実体にする。HFリポジトリID、固定revision、必要ファイル、容量、SHA-256、ライセンス、対応エンジンと量子化を記録する | 対応外GGUFを渡さず、重複ダウンロードを避け、取得前に必要ディスク・RAM・GPU条件を表示する。中断時の`.part`／stagingを回収し、利用者の共有Hugging Faceキャッシュや外部配置モデルを削除しない |
| 4. 管理導入 | 上流がWindows向けの版固定可能な取得元と再配布条件を提供した場合だけ、FreeTokenランタイムの明示導入を追加する | Python、Torch、CUDA、FreeToken、ネイティブwheelを版・サイズ・SHA-256・ライセンスで固定し、導入・更新・失敗時復元・削除を実パッケージで確認する。固定できない間はFreeToken Desktop／利用者導入済みランタイムを使う |
| 5. 対応範囲拡大 | Ubuntu 24.04 x64のNVIDIA実機を追加し、macOS／AMD／Intelは上流の公式対応後に別判定する | 対応GPUで実起動とGPU利用を確認する。非対応環境ではFreeTokenの大容量資産を取得せず、既定`llama.cpp`を維持する |

設計上の設定正本は、実装時に`PICOAGENT_LOCAL_ENGINE=llama|freetoken`を追加し、既定を`llama`とする。FreeToken固有のプログラム、モデル、GPU、ポート、追加引数は同じ設定画面の選択中エンジン内だけに表示する。任意引数は安全な構造化項目を優先し、自由記述を追加する場合もshellを介さず引数配列として渡す。

実機PoCではFreeToken 0.1.2、Python 3.12.11、PyTorch 2.11.0、CUDA 13.0の組合せでWindowsネイティブ起動まで確認した。一方、Gemma 4 E4B Q4_0は公式既知対応一覧外で、GGUF設定パーサーが`attention.head_count_kv`の単一整数を層別配列として扱い、その後もMoEメタデータを要求するため推論開始前に停止した。E4Bは`llama-server`専用として扱い、FreeToken比較は公式既知対応の26B A4B Q4_0で継続する。上流の未解決問題により起動または長文が停止する場合を成功扱いにせず、要求timeout、キャンセル、プロセスツリー回収まで合格条件に含める。

## 責務が集中したファイルの再評価

肥大化した対象は、状態の所有者、副作用、公開インターフェース、独立して機械検査できる境界を棚卸しする。分割後の依存が減る対象だけを分け、一体の方が効率的なら維持する。挙動変更と混ぜず、各段階を独立したPRで行い、公開中のPRと同じファイルを同時に変更しない。

| 対象 | 現時点の判断 | 再評価・着手条件 |
|---|---|---|
| `ui/settings.js` | 中央の設定スナップショット、未保存判定、保存処理は一体で維持する。目覚まし・スケジュールの専用フォームとJSON整形は独立していたため分離済み | スキル一覧、キャラクター設定、診断表示のいずれかが、中央状態を所有せず入力と描画だけの小さいAPIにできる場合だけ追加分割する |
| `ui/renderer.js` | 責務が複数あり分割候補 | PR #168の統合後、キャラクター描画・リップシンク、通知・目覚まし、会話入力の状態所有者とイベント境界を確認する |
| `src-tauri/src/commands.rs` | Tauri commandの入口は集約する価値があるが、業務処理まで同居している箇所は分割候補 | PR #168の統合後、設定、キャラクター、追加機能、会話についてcommandを薄い委譲にできる責務だけ移す |
| `xtask/src/assets.rs` | キャラクター同期、UI配信ビルド、管理プロセス回収は副作用と検証が異なるため分割候補 | 共有する配布パス判定を複製せず、各処理を独立テストできる境界を決めてから着手する |
