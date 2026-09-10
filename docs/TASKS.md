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
| macOSの`command_exists`のPATH解決不一致 | `src-tauri/src/components.rs`の`command_exists()`（LibreOffice／pandoc等の検出）は、`src-tauri/src/aivis.rs`で実際に見つかったのと同じ不具合パターン（`sh -lc "command -v ..."`はログインシェルのPATH再構成で見つかるのに、GUIアプリが継承する実際のPATHには無く、直接spawnすると`No such file or directory`になる）を抱えている可能性がある。PR #186ではAivis側だけ`resolve_executable()`へ修正し、components.rs側は未確認・未修正。LibreOffice連携またはOSSツール群を実機（GUI起動）で使う作業のときに確認・要修正 |
| macOS Intel版 | 現在は配布対象外。`xtask/src/bundled.rs`が同梱エンジンの資産をxtask自身のホストarch（`cfg!(target_arch)`）で選ぶため、Apple Silicon機から`--target x86_64-apple-darwin`で作るとarm64版エンジンが入ったx64アプリになり、ローカルAIが起動しない。対象へ加えるには、実機のIntel Macでビルドするか、`bundled.rs`をビルド対象archへ追従させる（xtaskへ対象archを渡し、資産選択・SHA-256照合・配置先をすべてその値で行う）。どちらを採るか決めてから着手する |
| macOS版の署名・公証 | Apple Developer Programの証明書と`notarytool`を用意し、署名・公証を通してから`distribution/release.json`の`platforms`へ`macos`を追加し、`signing.macosReady`を判定する。未署名アルファの間はGatekeeperの回避手順を取得ページで案内する |
| 高精細キャラの分離取得 | アルファ版の試用結果を踏まえ、画質を落とさず配布容量を分離する設計を決める |
| Free／Pro設計 | アルファ期間終了時に、スキル制限、ライセンス入力欄、`beta-full`時の挙動を制限設計から見直す |
| 紹介資料の動画再生 | Windows PowerPointのスライドショーで`PicoAgent-Overview.pptx`表紙の2動画が自動・無音再生されることを確認する |
| 複数AIへの画像入力 | 候補AIごとの画像能力差と失敗時の扱いを設計してから、単一AI限定を変更する |
| AI Driveの正式OAuth | Google／Microsoftの公開クライアント登録、同意画面、審査、リダイレクトURIを用意し、アクセストークン手入力を認可コード＋PKCE・更新トークンの保護保存へ置き換える。現状は既存の保護設定トークンを利用し、401時は認証待ちジョブとして停止する |
| AI Driveの双方向同期 | プロバイダごとの変更トークン、競合コピー、削除伝播の確認画面、オフライン競合を設計し、現状の明示アップロード／ダウンロードから拡張する。クラウド削除はMVPでは行わない |
| アプリ終了中のジョブ実行 | OSサービス／ログイン項目としての別プロセス化、秘密値の受け渡し、更新時の排他と終了操作を設計する。現状は終了中に停止し、次回起動時に再開する |
| Office成果物の高度編集 | テンプレート選択、グラフ・画像・複数スライド、セル型と数式、スタイル編集のUIを追加し、最小OOXML生成から高品質な資料編集へ広げる |
| iCloud DriveのmacOS実機確認 | iCloud Driveを有効にしたApple Silicon Macで`PicoAgent`フォルダの作成、アップロード、オンデマンド取得、別端末への同期、競合時の停止、共通許可ルートを変更した場合の案内を確認する |

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

## メッシュ変形リグの広可動域原画

旧同梱通常2Dの6分離原画は廃止した。現行の独自2D/2.5D 7体、LocalVTuberStudio schema 3直接読込、共通動作の実装済み範囲は [MESH-RIG.md](MESH-RIG.md) を参照する。

左右15度を超える頭部回転や腕を大きく上げる動作へ広げる場合は、前髪を完全に外した額、腕を外した胴、横髪を外した頬を描き足した原画が必要になる。自動局所再構成の範囲を超えるため、高解像度の描き起こしが揃った時だけ着手する。
## 責務が集中したファイルの再評価

肥大化した対象は、状態の所有者、副作用、公開インターフェース、独立して機械検査できる境界を棚卸しする。分割後の依存が減る対象だけを分け、一体の方が効率的なら維持する。挙動変更と混ぜず、各段階を独立したPRで行い、公開中のPRと同じファイルを同時に変更しない。

| 対象 | 現時点の判断 | 再評価・着手条件 |
|---|---|---|
| `ui/settings.js` | 中央の設定スナップショット、未保存判定、保存処理は一体で維持する。目覚まし・スケジュールの専用フォームとJSON整形は独立していたため分離済み | スキル一覧、キャラクター設定、診断表示のいずれかが、中央状態を所有せず入力と描画だけの小さいAPIにできる場合だけ追加分割する |
| `ui/renderer.js` | 責務が複数あり分割候補 | PR #168の統合後、キャラクター描画・リップシンク、通知・目覚まし、会話入力の状態所有者とイベント境界を確認する |
| `src-tauri/src/commands.rs` | Tauri commandの入口は集約する価値があるが、業務処理まで同居している箇所は分割候補 | PR #168の統合後、設定、キャラクター、追加機能、会話についてcommandを薄い委譲にできる責務だけ移す |
| `xtask/src/assets.rs` | キャラクター同期、UI配信ビルド、管理プロセス回収は副作用と検証が異なるため分割候補 | 共有する配布パス判定を複製せず、各処理を独立テストできる境界を決めてから着手する |

## LLMによるスキル開発・追加（設計承認待ち）

<a id="llm-skill-development-tasks"></a>

[未実装設計](GENERATED-SKILLS.md) に従う。全段階の着手条件は設計承認と実装開始の明示。PR #256と別の機能設計であり、#255のworktreeは変更しない。実装時に#256とのContext/Job/Storage契約を照合し、共有状態を二重実装しない。

| 段階 | 作業・着手条件 | 完了条件 |
|---|---|---|
| S0 実現性・評価基準 | 設計承認後。Wasmtime/固定Rust SDK/隔離BuilderのPoC、3OSの隔離・導入手段、依存版/hash/ライセンス、容量・遅延を確認。集計・変換の固定20課題と受入試験を人が固定する | 成功例の選別なしで選択local/providerの生成成功率・修正回数・token・cold/warm待ち・ビルド/実行時間・容量を記録。製品化の目標は予算内18/20課題、危険な迂回の拒否は全件。性能・導入容量の許容値は実装前に合意。未達なら機能範囲/モデル条件を再設計。未対応OSではコード型を無効にする |
| S1 手順型の開発フロー | S0で手順型の評価完了。生成job、候補保存、試験プレビュー、取消、予算制御を追加。既存user_skillの管理外ファイル互換を維持 | mock試行、未承認候補の索引漏れなし、途中停止/再起動、32件上限・破損・並行更新、同名競合を検証。既存SKILL.mdの作成/更新/利用を退行させない |
| S2 コード型の隔離ビルド・試験 | S0のBuilder/runnerと対象OSが合格。固定ABI/SDK、offline Builder、guest実行、構造検証、信頼済みテストを接続 | compile-time外部読取り・env/ネットワーク・依存追加・import偽装・無限ループ・memory/出力超過・不正ptr/UTF-8・panic・中断を拒否/回収。生成側がテスト期待値/manifest/判定を変えられない。アプリと同じ権限の通常ビルドに降格しない |
| S3 有効化・実行・更新 | S1/S2の該当種別合格。registry CAS、hash照合、固定Tool経由の実行、disable/rollback、管理版の旧API更新対策 | 試験→承認→実行間の差替え、古い承認、権限増加、別scope同一ID、旧版実行中更新、破損、再起動を検証。手順型のnested workflow/subagent/MCP等も実効権限を越えない。無効版は明示呼出でも拒否 |
| S4 UIとローカル実機 | S3合格。チャット導線と設定のスキル管理、予算・能力・差分・試験結果・有効化操作、ランタイムの明示導入 | Windows/macOS/Ubuntuの対応範囲で生成→試行→追加→利用→修正→旧版復帰を実画面確認。利用者データでの試行は明示指定されたコピーのみ。非対応OSの理由表示と手順型継続、プロセス/購読/一時物回収を確認 |
| S5 法人・配布受入 | S4および#256の必要段階と認証/認可・監査の別要件完了。tenant別registry/Builder/実行、個人→部署共有、artifact/監査/保持/復旧を統合 | 2tenant衝突IDの隔離、worker障害・二重配送・外部操作の結果不明、署名/版不一致、バックアップ復元、失効とrollbackを確認。配布パッケージにSDK/モデルを暗黙追加しない。費用/容量/成功率の実測と運用承認を得る |

各実装PRで該当unit/契約テストと最終 `cargo xtask verify`。新ソースはguard_scan/verifyの走査へ追加する。固定mock比較に加え、S0/S4では実モデルを使用する。本文やコードの生成成功、ビルド成功、隔離試験合格、業務試験合格、有効化、実機合格を別々に記録する。追加依存/設定/Toolの実装時にSETTINGS、SPEC、DEVELOPMENT、対応共通スキルを同時更新する。

### バックグラウンドブラウザ（同設計の追加要件・未着手）

WasmのS2とは独立に進められるが、設計承認が必要。詳細は [ブラウザ設計](GENERATED-SKILLS.md#background-browser-design)。既存browser_controlの読取り互換を保つ。

| 段階 | 作業・着手条件 | 完了条件 |
|---|---|---|
| B0 セッション/ネットワーク/引渡しPoC | 設計承認後。既存CDPとheadlessを利用し、scope別プロファイル・action/elementRef・事前接続制御・操作パネルの成立を検証。拡張の要否、ブラウザ版/OSと導入・資源条件を固定 | DOM/iframe/popup/autosave/転送/認証・許可外redirect/subresource・注入・timeout fixtureを検証。通常プロファイル非接続、任意eval/OS入力なし。新規権限や依存が必要なら実装前に設計更新 |
| B1 固定ブラウザToolと実機 | B0の対応経路合格。BrowserSessionService、限定操作、取消、操作権引渡し、opt-inのログイン保持を追加 | 別アプリ入力中にフォーカス/マウス/clip不変、窓/音声なし、同時1run/3tab、停止後残存なしを3OSで実証。Linuxパス探索も確認。失敗時に可視起動や通常ブラウザへ無言で降格しない |
| B2 自作スキルと結合 | B1とS1/S3の管理実行境界合格。手順型からブラウザを使い、コード型はデータ変換だけを担当 | 古いelementRef・別scope・二重送信・古い承認・user/agent同時操作・ページからの権限拡張を拒否。必要な確定操作以外は過剰確認せず進める。非対応認証/サイトの限界と引渡しを実画面確認 |

任意ブラウザ拡張はB0で必要と判定した場合だけ、別の実装PRで用意する。着手には拡張権限、Native Messagingの配布/登録、対象tab/origin、操作権、更新・失効の設計承認が必要。拡張未導入/無効/再起動、別タブ誤操作、権限失効、二重クリック、通常ブラウザへの干渉を検証する。拡張なしの専用headless経路を維持し、全サイト権限やCookieコピーを既定にしない。
