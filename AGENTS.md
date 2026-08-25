# AGENTS.md — Claude Code / AI エージェント向け作業ガイド（ルール正本）

このファイルが唯一のルール正本です。`CLAUDE.md` はこのファイルへのリダイレクトのみです。ルールを追加・変更する場合はこのファイルだけを編集してください。

> **作業を引き継いだ場合は [HANDOFF.md](HANDOFF.md) を先に読む。** 未マージの作業と再開方法だけをまとめてあります。恒久ルールはこのファイル、未完了項目は [docs/TASKS.md](docs/TASKS.md)、過去の経緯はPR、という持ち場です。

## 最重要運用ルール

- `AGENTS.md` はすべて日本語で記述する。英語セクション・英語コメントは追加しない。既存の英語記述を見つけたら日本語に置き換える。
- ルールに反する記述・命名・コメント・dead code を発見した場合は、その場で修正して作業完了後に事後報告する。都度確認を求めない。
- ルール正本は `AGENTS.md` 1本。`CLAUDE.md` はリダイレクトのみなので編集しない。恒久ルールはすべてこのファイルに追記する。
- **ドキュメントは常に最新を保つ。コード・スクリプト・設定・依存関係・フォルダ構成・コネクタのインターフェースなど、README や SPEC.md に記載のある内容を変更したら、必ず同じ作業内でドキュメントも更新する。ユーザーから指摘されてから直すのは禁止。**
- **大きなファイルは、肥大化を検知したら責務を棚卸しし、分割するか一体のまま保つかを先に判断する。** 行数だけを理由に分けず、唯一の正本、参照方向、外部へ公開する最小インターフェース、機械検査の境界を決め、その境界で保守・検証が独立する場合だけ分ける。分割によって読むファイル、共有状態、相互呼び出しが増えるなら、一体の方が効率的として分けない。単なる行移動、同じ状態の相互書き換え、循環依存、同じ説明の複製になる分割は禁止する。文書は読者と更新理由、コードは状態の所有者と副作用を分割基準にする。判断結果と、分ける場合の境界または分けない理由をPR本文へ残す。
- **LP、配布README、`latest.json`、紹介PowerPoint、画像・動画・CSS、Pages、Releaseなど`PicoAgent-Distribution`に属する変更依頼は、この本体リポジトリで受けても、実作業を`C:\00_PG\21_PicoAgent-LP`へ切り替える。** 配布リポジトリ全体の正本は21側の`main`であり、20側の`distribution/site/`を編集正本・同期元にしない。20側で行うのはアプリ実装と配布バイナリ生成までとし、検証済み成果物を21側へ取り込む場合も21側の目的別PRと公開手順で管理する。
- 設定キー一覧の正本は `docs/SETTINGS.md` とし、記載漏れ・削除漏れは `doc_sync::guard` のテストが実装と双方向で検出する（手作業の突き合わせは不要。設定キーでない `PICOAGENT_*` 文字列は理由付きで `NOT_CONFIG_KEYS` へ）。`SPEC.md` は設定の保存・反映機構、`README.md` と `MANUAL.md` は利用者がよく使う項目だけを説明し、全キーを複製しない。
- 対象ドキュメントの目安: アーキテクチャ・コネクタ仕様・メッセージ正規形・設定機構 → `SPEC.md`、全設定キー → `docs/SETTINGS.md`、概要・起動・主要機能 → `README.md`、一連の利用手順 → `MANUAL.md`、開発・xtask・配布ビルド・リポジトリ構成 → `docs/DEVELOPMENT.md`、未完了・条件待ち → `docs/TASKS.md`。変更の履歴はPR本文とGit履歴へ残し、別の変更履歴ファイルを二重管理しない。
- ドキュメント更新が本当に不要な軽微修正の場合のみ、不要と判断した理由を作業完了時に明記する。
- **利用者が「読み取り専用」「編集禁止」と明示していない限り、「レビューして」は修正までの依頼である。指摘を並べて終わりにしない。** 見つけた問題はその作業内で直し、テストまで済ませる。方針判断が要るものも妥当な既定を選び、採否、理由、代替案、検証境界をPR本文へ書く。直さずに残すのは、実機や外部資産が無くてコードでは解消できない場合だけで、再現条件と着手条件を`docs/TASKS.md`へ残す。
- **PR本文を変更記録とレビュー記録の正本にする。** 目的、主な変更、採否と理由、実施した検証、未確認事項をそのPRへ書く。完了した作業を別のMarkdown台帳へ複写しない。恒久ルールは`AGENTS.md`、現行仕様は`SPEC.md`、利用手順は`README.md`または`MANUAL.md`、未完了だけは`docs/TASKS.md`へ移す。
- **実装・配布・検証を簡単にする選択によって、速度、画質、機能、対応機種、互換性、保守性のいずれかを犠牲にする場合は、採用前または判明した時点で利用者へ具体的に伝える。** 「安全」「簡単」「公式資産がある」だけを理由に欠点を隠さず、何がどの環境で劣るか、改善に何が必要か、今回実施するか保留するかをPR本文へ残す。未完了なら`docs/TASKS.md`にも着手条件を書く。簡単な経路を実装しただけで、犠牲にした高速経路や高品質経路まで対応済みと扱わない。
- **CodexとClaudeは互いを自動起動しない。相互レビューはユーザー操作だけをトリガーとする。** CodexからClaude CLIを呼ぶことも、ClaudeからCodex CLIを呼ぶことも禁止する。ユーザーが対象成果物、レビュー担当、レビューしながら開発する範囲を明示した場合だけ実行し、その指示は指定された同一PR内だけで有効とする。別PR・別作業へ過去の個別指示を持ち越さない。通常の実装・レビューは作業中のエージェント自身が完結させ、別エージェントの応答を完了条件にしない。レビュー結果は同じPR本文へ残す。
- コミットメッセージ・ソースコードのコメント（Rust / JS とも）は基本的に日本語で書く（コミットは英語だけの件名にしない）。`feat` / `fix` / `docs` / `chore` などの短い英語プレフィックスや、API / JSON / trait / 識別子・関数名など技術用語の英語はそのまま使ってよい（無理に和訳しない）。
- 公開版の利用者向け変更点は`C:\00_PG\21_PicoAgent-LP`のRelease notesへ版単位で書く。本体RepoではPR本文とGit履歴を変更履歴とする。
- **大きなファイル容量・配布容量・転送量は、一般的なMBまたはGBだけで見やすく書く。bytesを単独表示または併記せず、外部仕様がMiB／GiB表記の場合も10進のMB／GBへ換算する。**
- **常に Codex / Claude（作業エージェント自身）の利用クレジット消費を最小化して作業する。** 不要な長文出力、重複調査、過剰なファイル読み込み、不要な並列探索、同じ確認のやり直しを避ける。まず検索・差分確認・必要最小限のファイル読み取りで判断する。
  - 検証コストの優先順位: まず `cargo check`（コンパイル）→ 必要なら `cargo test --lib`（ロジック）で済ませる。**GUI 起動（`cargo xtask dev`）・スクリーンショット・フルビルドは高コストなので最後の手段**とし、目視確認が本当に必要なときだけ、複数の確認をまとめて 1 回で行う。
  - 目的別PRの最終ローカル検証をまとめるときは`cargo xtask verify`を使う。本体・xtaskの書式、全target静的解析、テスト、追跡JavaScript／MJS構文、Git差分を途中失敗後も最後まで実行し、失敗とスキップを末尾で確認する。日常の小修正で毎回フル検証を回す意味ではない。
  - 編集はまとめて行い、**ビルド/テスト/起動は変更が出揃ってから最小回数**だけ実行する。同じ内容のビルド・テスト・起動・読み取りを繰り返さない。
  - `Cargo.toml` / `Cargo.lock` を変更したら `cargo audit --file <Cargo.lock>` で既知脆弱性を確認する。GitHub Actions は無料枠・請求制限による開始前失敗と通知の連発を避けるため使わない。依存更新時と販売前のローカル検証で監査し、保守終了などの情報警告と脆弱性を分けて評価して、対象OS・実行経路と見送り理由をPR本文へ残す。
  - **外部依存は原則として公式LTSまたは長期保守安定版を採用し、LTS相当が無ければ安定版latestを採用する。LLMモデルはLTS扱いにせず常に公式の最新安定版へ追従する。** Cargo・JavaScript・Python等のパッケージ、同梱アプリ、管理導入ツール、Webランタイム、音声・LLMモデルを毎月1日に`.agents/skills/audit-picoagent-dependencies/SKILL.md`の`all`で点検する。互換性、ライセンス、対象OS、再現可能なハッシュ固定の問題で旧版を維持する場合は理由と確認日をPR本文へ残し、未完了なら`docs/TASKS.md`へ移す。
  - **CUDAを使うPythonランタイムは、Python単体の最新版ではなく、対象GPU、CUDA、PyTorch、CUDA拡張の公式wheelが同時に対応する安定版へ統一する。現在の基準はPython 3.12とする。** FreeTokenの公式Windows wheelがPython 3.12 ABIで配布され、依存するPyTorch 2.11、CUDA 13、Windows版Tritonの対応組合せが揃っており、最新Pythonへ先行するよりCUDA拡張の導入・実行互換性を安定させられるためである。機能ごとにPython本体を重複取得せず、PicoAgent管理下の同じPython本体を再利用して仮想環境だけを分離する。互換性のため別版が必要な機能は、理由、依存組合せ、移行条件をPR本文と未完了時の`docs/TASKS.md`へ残す。移行検証に成功したら不要になった旧Python本体、仮想環境、取得キャッシュを同じ作業内で回収する。基準版を変える場合は、全CUDA機能の公式wheelが新しいPython ABIへ揃ったことを確認し、起動と実処理を新しい組合せで検証してから切り替える。
  - **大容量のモデル・ランタイム・SDK・GPU向けパッケージは、機能の利用意思とOS・CPUアーキテクチャ・GPU種別・必要能力を取得前に判定し、利用できない物を予備またはフォールバック目的で取得しない。** とくにCUDA版はCUDA対応NVIDIA GPUを確認できた場合だけ取得し、AMD／Intel／GPUなし環境ではCPU版が実際に対応する機能だけCPU版を選ぶ。自動判定を明示指定で上書きできる場合も、非対応構成の既定取得を許さない。全対象の一括取得は`cargo xtask setup all`のような明示操作だけにし、起動・通常ビルド・引数省略へ任意機能の取得を混ぜない。
  - **取得・更新・ビルドの正常終了後は、PicoAgent管理下の旧版、退避、staging、`.part`、`.incoming`、`*-old-*`、完了済み再開キャッシュを残さない。** 新版のサイズ・ハッシュ・起動可否など必要な検証が成功してから旧版を削除し、失敗時は復旧に必要な物だけを残す。失敗時に残した再開用ファイルも次回成功時に必ず回収する。共有キャッシュ、利用者が指定した外部配置、他アプリの管理領域は削除対象にしない。
  - 起動して確認したら使い終わったプロセスは止め、ロックや再ビルドの無駄を避ける。確認が自動化しづらい（GUI・音声など）場合は、コードの正しさで判断し、最終目視はユーザーに委ねてよい。
  - **CodexのComputer Useは、Windows画面を実際に操作・撮影する直前だけ開始し、確認が終わった直後に毎回セッションを終了する。** 青い対象枠を使っていない間も残さない。作業全体の終了時だけでなく、ターミナル調査・ビルド・文書編集など別の作業へ移る前にも必ずオフにする。Computer Useや検証のために起動した対象アプリと子プロセスも、不要になった時点で自分が起動した実体を確認して終了する。
  - **Node製の作業ツール（`tools/screenshots/` の撮影・検査など）を起動したら、正常終了・失敗・中断のどの経路でも `node.exe` を残さない。** タイムアウトや上限で打ち切った場合ほど残りやすいので、打ち切った直後にプロセスを確認し、残っていれば回収してから次の作業へ進む。撮影ツールは自前のHTTPサーバとChromiumを抱えるため、残すとポート占有と数百MBのメモリがそのまま居座る。
  - **プロセスを止める前に、必ず親プロセスとコマンドラインで「自分が起動したもの」かを確かめる。** `node.exe` は Codex の MCP サーバやカーネル、Adobe Creative Cloud なども使っており、名前だけで一括終了すると稼働中アプリを壊す。判別には `Get-CimInstance Win32_Process -Filter "Name='node.exe'"` の `ParentProcessId` と `CommandLine` を使い、自分が起動した経路（`tools/screenshots/shoot.mjs` 等）と一致するものだけを対象にする。`cargo xtask stop` は、PicoAgent本体と同梱エンジンに加えて、コマンドラインへ `tools/screenshots/` を含む `node` と、`PicoAgent/browser-profile` を `--user-data-dir` に持つブラウザだけをプロセスツリーごと回収する（区切りをスラッシュへ揃えて判定するため、相対・絶対のどちらの起動でも拾う）。名前一致で止める対象へ `node` を足さないことと、ブラウザを専用プロファイル経路と実行ファイル名の両方で絞ることは `assets` の回帰テストが固定している。
  - **子プロセスを起動したら、`Child` を握っているだけで回収できると考えない。** Chrome / Edge は起動直後に本体を立て直して最初のプロセスが終了するため、こちらの `Child` は本体の親ではなくなり、`kill()` も `taskkill /T` も本体へ届かない（実測で headless Chrome が9プロセス残り、次回起動が「PicoAgent が管理していないブラウザに使われています」で必ず失敗していた）。この形の常駐物は、専用プロファイル・専用ディレクトリなど**自分のものだと分かる起動経路**で名指しして回収する。
- **対応 OS は Windows、macOS、Ubuntu Linux 24.04 LTS x64**。Linuxは`.deb`とAppImageを配布形式とし、実行ファイルだけでなく実パッケージの導入・起動・削除まで検証する。WaylandでOS側の許可やポータルに依存する画面取得・入力操作は、拒否や未提供を成功扱いにせず理由付きエラーを返す。Linux専用依存を変えたら`.claude/hooks/session-start.sh`とREADMEのapt一覧を同じ作業で同期する。
- **開発・検証でPowerShellを使う場合は、PowerShell 7の`pwsh`を既定にする。** 作業開始前に`Get-Command pwsh`で実体を確認し、見つからなければWindows PowerShell 5.1へ黙って降格せず、PowerShell 7の導入が必要な理由を利用者へ伝えてインストール許可を求める。Windows PowerShell 5.1は、利用者環境で動く製品付属スクリプトの互換性確認など、5.1自体が検証対象の場合だけ使う。PowerShell 7の実体がCodex等の管理キャッシュにある場合は開発作業へ利用してよいが、PicoAgent製品からその外部管理パスへ依存しない。日本語ファイルを確認するときはUTF-8を明示する。表示が文字化けしても、すぐにソース破損と判断せずUTF-8で読み直してから判断する。
- ファイルや配布物の容量を利用者向け返信・引き継ぎ・PR本文へ書くときは、十進表記のMB／GBへ丸める。生のbyte数やMiB／GiBは併記しない。識別子・実装上のbyte列・検証ロジックの説明は容量報告ではないため対象外とする。
- 既に未コミット変更がある場合は、ユーザーまたは別エージェントの作業として扱い、勝手に巻き戻さない。
- 作業中のスクリーンショット・連番フレーム・変換途中のファイルなど、一時的な作成物はリポジトリ直下の `temp/` 内だけに置く。ルートやソースディレクトリへ `tmp-*` 等を作らない。`temp/` は `.gitignore` で除外し、一時作成物をコミットへ含めない。
- **透過キャラクターの成果物に、背景の消し残しと誤認されるハロー・矩形・色面・太い外周影を付けない。** 画像四隅のアルファ値だけで合格にせず、透過市松または透明な画面で実表示を確認する。意図した背景が必要な配信画面などは、キャラクター画像ではなく画面側の明示的な背景レイヤーとして分離する。
- **画像の引き伸ばし拡大は禁止する。** 縦横のピクセル数だけを増やすnearest／bilinear／bicubic／Lanczos等の補間、OS画像API、Pillow、ImageMagick、ブラウザ、CSSによる拡大結果を高精細素材や最終レイヤーへ採用してはならない。Photoshop前の配置正規化も等倍以下かつ縦横比維持に限定する。`animationQualityMode=sharp-expression` の高精細キャラは、`scripts/rebuild-sharp-expression-assets.ps1` だけを正規の再生成入口にする。工程順は「基準寸法で局所差分を合成 → Photoshopの「ディテールを保持 2.0」で全9レイヤーを1780×3600以上へ再サンプリング → 元アルファ再適用 → 連番生成 → キャラ同期」で固定する。Photoshop処理も寸法を大きくするだけでは合格にせず、顔・髪・輪郭の実効解像度を元絵、Photoshop完成レイヤー、実WebViewの拡大表示で比較し、細部が明瞭に保たれた場合だけ採用する。Photoshop処理後も元絵由来のぼけが残る場合は「高精細」として合格させず、高解像度の元絵または局所再制作を用意する。Photoshop拡大後に差分合成スクリプトを再実行して高精細版を890×1800へ戻すことも禁止する。QAレポートへ方式と処理前後の寸法を残す。`cargo xtask sync-characters` は高精細レイヤーが1780×3600未満なら失敗させる。
- **通常2Dの制作正本と実行画像を分離する。** `characters/<id>/layers/*.png` は上書き・削除せず、`scripts/build-character-animations.py` だけを正規入口として `runtime-layers/*.webp` と `frames/<世代>/*.webp` のlossless WebPを生成する。保存直後に復号した寸法・RGBAを変換元と完全一致させ、1画素でも違えば停止する。`cargo xtask sync-characters` は制作正本を検証してから配信コピーの `layers/` だけを除外し、manifestの `runtimeLayers` と `framePaths` が指すWebPを残す。利用者が追加済みのPNGパックは後方互換で読み続ける。lossy WebPは画質判断を分離した別PRでのみ扱う。
- Gemini、Nano Bananaなどブラウザの画像生成・編集サービスは、ユーザーが対象に指定した画像をその作業目的へアップロードして利用してよい。構図変更、ポーズ変更、局所修正、再生成には利用できるが、拡大・超解像・hires-fixには利用しない。元画像と承認済み成果物を上書きせず、候補を別名で保存して比較する。拡大はPhotoshop「ディテールを保持 2.0」だけを許可する。
- ユーザーが許可した場合、同一キャラクターのLive2D、通常2D、PicoAgent用素材を本人性・ポーズ・パーツ構造の参照として外部画像編集サービスへ添付してよい。利用後に、送信した素材、送信先、参照目的を報告する。関連のないプロジェクトファイルは送信しない。
- 作業エージェント向けの再利用手順は `.agents/skills/<名前>/SKILL.md` を正本にする。Claude Code用の `.claude/skills/<名前>/SKILL.md` は同名の共通スキルを読むだけの薄い入口とし、手順を複製しない。キャラクター追加・再生成は `.agents/skills/rebuild-picoagent-character/SKILL.md` を使い、会話履歴から個別コマンドを組み直さない。
- `.agents/skills/`の共通スキル、付属スクリプト・参照資料・`agents/openai.yaml`、対応する`.claude/skills/`の入口はすべてGit管理対象にする。追加・更新したものをローカル専用や未追跡のまま残さず、実装と同じ目的別PRへ含める。
- **同じ種類の作業が2回以上発生した、工程順を誤ると成果物が壊れる、混入・公開・破壊・課金の危険がある、または複数エージェントへ引き継ぐ作業は、その作業内で共通スキル化する。** 配布ビルド、キャラクター生成、実画面検証、販売前監査、外部公開はこの対象とする。単発の小修正や一般知識だけの手順は無理にスキルへしない。
  - 新規作成前に `.agents/skills/` を検索し、目的が重なるスキルがあれば新設せず既存スキルへ統合する。1つの依頼で複数スキルが必要な場合も、責務が重複しない最小構成にする。
  - スキルを使うたびに、記載されたパス、コマンド、版、成果物、公開先を現行コード・文書・公開状態と照合する。古い情報や不足分を見つけたら、そのスキルを同じPRで更新してから作業を続け、過去の手順をそのまま実行しない。
  - `AGENTS.md`などのルール文書に、恒久ルールではなく特定作業の工程・コマンド・成果物一覧が増えており、上の条件からスキルへ分離した方がよい箇所を見つけたら、対象と分離先候補をユーザーへ報告してスキル分離を促す。既存スキルへ統合できる場合は新設を勧めない。
  - スキル名は英小文字・数字・ハイフンだけの動詞始まりとし、`SKILL.md` のfrontmatterは `name` と「何をするか・どの依頼で使うか」を含む `description` だけにする。本文は日本語、命令形、500行未満を目安とし、一般論や作業日記を入れない。
  - 壊れやすい工程は、正規入口、入力条件、禁止経路、成果物の識別方法、失敗時の停止条件、検証、公開前照合まで書く。単なるコマンド一覧にせず、古い成果物や開発用資産を混ぜない条件を明示する。繰り返し組み直す処理は `scripts/` へ置き、説明を重複させない。
  - 新規スキルは `skill-creator` の初期化手順で作り、`quick_validate.py` を通す。Pythonが直接使えない環境では `uv run --python 3.12 python ...` を使う。作成後は今回の実タスクで手順どおりに実行して検証し、失敗から得た分岐を同じ作業内で反映する。
  - Claude Code入口は共通正本を最後まで読む指示だけにし、同名フォルダへ手順・スクリプト・参照資料を複製しない。スキルの追加・更新時はCodex側とClaude側の名前一致、リンク先の存在、重複手順なしを確認する。
- **修正・作業が完了したら、ユーザーからの個別指示がなくても `main` へのマージまで終わらせる。** 手順は「`git add -A` で全ステージ → コミット → 作業ブランチへプッシュ → PR 作成 → squash merge」。自分が編集したファイルだけでなく、既に存在していた未コミット変更も同じコミットに含める（コミットメッセージは日本語。本ファイルの他ルールに従う）。**止めるのは、利用者が「マージしない」「PRだけ」と明示した場合と、破壊的・不可逆な操作を伴う場合だけ**。判断に迷って作業を宙ぶらりんにしない。
- **すべての変更はPRで行い、`main`へ直接pushしない。** 作業開始時に`origin/main`から目的別ブランチを作り、同じ目的の変更だけを1つのPRへまとめる。無関係な修正は別ブランチ・別PRへ分ける。ユーザーが相互レビューを指定した場合も、その指示の有効範囲は指定された同一PR内だけとする。公開済みコミットの書き換えやforce pushはしない。
- **PR のマージは squash merge を使う。** 1 PR = 1 コミットになり `main` の履歴が読みやすい、というだけの理由。なお API 経由のマージは、squash / 通常マージのいずれでも GitHub 側が committer を `noreply@github.com` にして署名するため、`git log` 上は Unverified に見える（GitHub の画面上は Verified）。これは避けられないので気にしない。すでに `origin` に載ったコミットを後から書き換えるのは共有履歴の破壊操作になるので行わない（上のルールのとおり）。
- **作業ブランチを残さない。** マージまで終えたら、その場でリモートとローカルの両方を消す。`gh pr merge --squash --delete-branch` はリモートしか消さないので、ローカルは `git switch main && git pull --ff-only` のあとに `git branch -d` まで行う。残すのは `main` と、いま作業中のworktreeがチェックアウトしている枝だけにする。
  - **squash merge したブランチは `git branch --merged` で検出できない。** 1コミットへ潰されて親子関係が残らないため、「未マージ」に見える枝が実際は取り込み済み、ということが起きる。判定はブランチ名や `--merged` ではなく、**その枝の中身が現行 `main` に存在するか**で行う（関数名・定数・記録の見出しなど、枝を特徴づける文字列を `main` 側で検索する）。
  - **中身が `main` に無い枝を、確認せずに消さない。逆に、古い枝を確認せずにマージもしない。** 分岐元が古い枝をそのままマージすると、完了済みの作業指示と旧実装を `main` へ戻す（PR #119、および2026-08-19に照合したClaudeブランチ4本がこの形だった）。取り込む価値のある差分が残っているなら、枝をマージするのではなく現行 `main` から作り直した目的別PRへ移植する。
  - 消した枝と、消してよいと判断した根拠は、その整理を行うPR本文へ残す。次に見た人が同じ照合をやり直さずに済む。
- **製品版番号はSemVerで管理し、手作業で3か所を書き換えない。** 通常の修正・依存／モデル／同梱物更新・文書／LP・既存ベータ再配布は同じ`major.minor`系列の`patch`を自動加算する。`minor`と`major`はユーザーから版を上げる明示指示があった場合だけ変更し、エージェントが機能内容から独断で上げない。`cargo xtask bump-version <major|minor|patch>`を正規入口にし、公開中の`latest.json`を起点としてアプリCargo・xtask Cargo・Tauri設定を同期する。同じ版のRelease資産は上書きしない。`patch`差分は正確な版を公開表示して新規配布へ反映するが、起動時には通知しない。利用者が設定から手動確認した場合はpatchも案内・ダウンロードできるようにする。`major.minor`が上がった場合は起動時にも通知する。判定した種別と理由をPR本文へ残す。

## エラー処理・リソース

- **実害のあるバグは見つけ次第その場で直す。** 別の作業中に見つけた場合も「今回の目的外」で保留にしない。実害とは、利用者のデータが消える・機能が黙って動かなくなる・誤った内容を見せる・費用や時間を無駄にさせる、のいずれかを指す。目的が違うなら別ブランチ・別PRへ分けてよいが、**分けることと後回しにすることは違う**。同じ作業内で直しきり、判定をPR本文へ残す。すぐ直せない場合だけ、再現条件、影響範囲、着手条件を`docs/TASKS.md`へ積む。
- **利用者から見える失敗を、ログだけで済ませない。** ログは利用者が見る場所ではない。目覚まし・リマインダーのように裏で繰り返し確認している機能や、会話履歴の保存のように利用者が「できている」と思い込む機能が失敗したら、画面へ理由を出す（`renderer.js` の `notifyBackgroundFailure`）。うるさくならないよう原因ごとに1度だけ知らせ、復旧したら次の失敗はまた知らせる。
- **エラーは原則 `Result` で上位へ伝播させる。サイレントスキップ・無言フォールバック・無言デフォルト降格は禁止。** フォールバックを書く場合はその箇所に `// フォールバック許可: 理由` コメントを付ける（例: `config.rs` の破損時空起動）。
  - 例外として、プロバイダの `is_up()` のような「接続可否を返すプローブ」は、未起動・タイムアウトを `false` に正規化してよい（接続できないこと自体が戻り値なので、握りつぶしではない）。
  - ツールコネクタの `run()` 内の失敗は、`Result` で投げず**エラー内容を文字列で返してモデルに伝える**（`tools::dispatch()` も最終防壁として未登録ツールを文字列化する）。エージェントが失敗を理解して回復するための設計上の許可。
- **状態の「読む → 直す → 書き戻す」は `store::lock_state(名前)` を取ってから行う**（設定に JSON 文字列で持つ一覧＝目覚まし・リマインダー・スケジュール等、およびトークン使用量・長期メモリのようなファイル状態）。 エージェントは1ターンの複数ツール呼び出しを**並列実行**する（`agent.rs` の `join_all`）ため、ロックが無いと同じ一覧を触るスキル同士（`reminder` と `focus_timer` は同じキー）が互いの追加を上書きして消す。ロックを保持したまま `.await` しないこと（状態更新は同期処理で完結させる。跨ぐとコンパイルエラーになる）。`store::guard` のテストが、ロックを取らない書き戻しを検出して失敗させる。なお `Config::set_all()` 自体も「スナップショット採取 → 書き込み」を内部で直列化している（これが無いと、同時保存でメモリ上には残るのにファイルからは片方の変更が消える）。
- **「壊れていたら空で継続」するフォールバックを書いたら、その空データをそのまま保存し返さないこと。** 破損時に空で開始し、次の保存で書き戻すと**ユーザーのデータが恒久的に消える**。設定に JSON 文字列で持つ一覧（目覚まし・リマインダー・スケジュール等）は `store::load_list()` で読み、読めなければ**空一覧にせずエラーを返して書き戻しを止める**。ファイルで持つ状態（記憶・使用量・会話履歴・実行履歴）は `store::quarantine_corrupt()` で破損ファイルを退避してから空で続ける。設定画面の一覧エディタは読めなかった元の値を温存して警告を出す。`store::guard` のテストが、退避もエラー返却も伴わない「空で置き換え」を検出して失敗させる。
- `unwrap()` / `expect()` は、起動時の致命的初期化（`main`/`setup`）や Mutex ロックなど「失敗＝即異常終了が妥当」な箇所に限る。通常経路では `?` と `Result` を使う。
- **取得したリソースは明示的に解放する。** 子プロセス・ファイルハンドル・ネットワーククライアント/接続・タイマー（`setInterval` 等）・イベントリスナ・フロントの `URL.createObjectURL` / `Audio` など、確保したものは「正常終了・失敗・中断（キャンセル）」のどの経路でも確実に解放する。`Result` の早期 return や例外経路で解放が漏れないようにする。
  - 子プロセスは終了時に `kill` し、`wait` でハンドルまで回収する（ゾンビ・ハンドルリークを残さない）。
  - `reqwest` の `is_up` などプローブ系には `timeout` を付ける（接続が永久に滞留しないようにする）。長時間ストリーミング（モデル DL・生成）は全体 timeout を付けない代わりに接続自体の滞留に注意する。
  - フロントの `URL.createObjectURL` は再生終了・失敗・中断で必ず `revokeObjectURL` する。`setInterval` 等のタイマーは全経路で `clearInterval` する。
- **PicoAgent は同時に1つしか動かさない**（`tauri-plugin-single-instance`）。`store::lock_state` はプロセス内 Mutex でプロセスをまたぐと効かず、設定・記憶・使用量・実行履歴・会話履歴はどれも「全体を読んで全体を書き戻す」ため、2つ目のインスタンスがあると片方の保存が古いスナップショットで丸ごと上書きされる（＝利用者のデータが黙って消える）。2つ目の起動は既存ウィンドウを前面へ出して終了する。**この前提を外す変更（複数ウィンドウ・複数プロセス）をするなら、状態ファイルのプロセス間排他を先に用意すること。**
- ユーザーから明示指示がない限り、同梱実行ファイル・同梱モデル・同梱ランタイムは PicoAgent 管理下の専用ディレクトリ（原則 `src-tauri/resources/bundled/<機能名>/`、実行時データは `app_data_dir` 配下）に設置し、既存の手動インストール先・グローバルPATH・共有ポートを既定で使わない。手動インストールを使う場合は設定で明示されたときだけ優先し、同梱物のアンインストール・一時ファイル削除は PicoAgent 管理下のパスだけを対象にする。

## 設定値・シークレット

設定項目を追加・変更・削除する作業は、先に `.agents/skills/add-picoagent-setting/SKILL.md` を読む。以下は個別手順ではなく、すべての設定実装へ常時適用する安全規則とする。

- 設定の正本は `src-tauri/src/config.rs`（永続ファイル `app_config_dir/config.json`）。優先順位は **永続ファイル > 環境変数 > ハードコード既定値**。新規設定で環境変数を「正本・保存先」にしてはならない（環境変数は既定値としてのみ参照する）。
- 数値・閾値・パス・モデル名など「設定として変えうる値」はソースに散らさず `cfg.get(キー, 既定値)` で都度読む。コネクタは自分の設定項目を `config_schema()`（`Vec<Field>`）で申告し、設定画面に自動で並ぶようにする。新しい設定キーを足したら `docs/SETTINGS.md` に必ず記載し、利用者が日常的に触るキーなら README と MANUAL も更新する。設定の保存・反映方式を変えた場合は SPEC も更新する。
- 設定画面（右クリック →「設定…」）が唯一の GUI 設定入口。`ui/settings.{html,css,js}` と `commands.rs` の `settings_*` command で構成し、保存後は実行中に反映する（コネクタは `cfg.get()` で都度読むため再起動不要）。
- API キー・トークン等のシークレットはコミットしない。ログ・エラーメッセージ・ドキュメントに値を出さない。OpenAI 互換プロバイダの `Authorization`（`bearer_auth`）など秘密値を含む箇所は、エラー文へ連結しない。`config.json` はローカル個人 PC 専用で、リポジトリにコミットしない（`app_config_dir` 配下に保存される）。
- **API キー・トークン等は必ず `type="password"` で申告し、キー名に `API_KEY` / `TOKEN` / `SECRET` / `PASSWORD` / `WEBHOOK` / `CREDENTIAL` のいずれかを含める。** 暗号化の要否は「スキーマでの password 申告」に加えて**キー名でも判定する**（`secret::is_secret_key`）。スキーマ（`config_schema()`）に載らない設定項目——設定画面 `ui/settings.js` が直接組み立てる `field(...)` など——は型申告だけでは拾えないため（実際にローカル API の `PICOAGENT_API_KEY` がこの経路で平文保存されていた）。`secret::guard` のテストが (1) コネクタのスキーマで秘密っぽいキーが password 型でない申告、(2) 設定画面の password 項目が `is_secret_key` に載らない名前、の両方を検出して失敗させる。
- 設定スキーマで `type="password"` の値（API キー等）は平文で `config.json` に置かない。Windows では DPAPI（`src-tauri/src/secret.rs`）で暗号化して `enc:dpapi:v1:` 形式で保存し、`config.get()` が読み取り時に透過的に復号する（同一 Windows ユーザーのみ復号可能）。既存の平文値は起動時に自動で暗号化へ移行する。**復号できないまま暗号文が残る値（別PC・別Windowsユーザーの `config.json` を持ち込んだ場合）は `config.get()` が未設定として扱う。**以前は暗号文をそのままAPIキーとして外部サービスへ送っており、返るのは 401 だけで原因が分からなかった。未設定にすることで各プロバイダの「設定画面で入力してください」という案内へ乗る（保存値自体は上書きしない）。非 Windows は DPAPI が無いため平文保存（フォールバック許可）。新しいシークレット設定は必ず `type="password"` で申告し、この暗号化経路に乗せる。

## コネクタ開発ルール（このプロジェクトの中核）

プロバイダまたはツールコネクタを追加・変更する作業は、先に `.agents/skills/add-picoagent-connector/SKILL.md` を読む。以下は個別手順ではなく、すべてのコネクタへ常時適用する設計規則とする。

PicoAgent は「叩く AI（LLM バックエンド）」と「エージェント機能（スキル/ツール）」をコネクタとして拡張する。**どちらも決まった場所へファイルを置けば自動登録される**。`build.rs` が `src/providers/*.rs`・ルートの `skills/<スキル名>/mod.rs` を走査して `#[path] pub mod` を生成し、各ファイルの `inventory::submit!` で登録されるため、中央の `mod.rs` 編集は不要。スキルは扱いやすいようプロジェクト直下の `skills/<スキル名>/mod.rs` に置く（トレイト・レジストリ本体は `src-tauri/src/tools/mod.rs`）。スキル専用の補助モジュール、スクリプト、テンプレート、少量の静的アセットは同じ `skills/<スキル名>/` 配下へ同梱する（例: `skills/chitchat/trends.rs`、`skills/run_command/jobs.rs`、`skills/browser_control/cdp.rs`）。

- **プロバイダコネクタ** は `src-tauri/src/providers/` に置き、`Provider` トレイトを実装して `inventory::submit! { ProviderReg { make: || Box::new(<型>) } }` で登録する。`chat()` は正規形メッセージ（`message.rs`）を受け取り正規形 assistant を返す。プロバイダ固有のワイヤ形式変換は各コネクタ内に閉じ込め、`agent.rs` に漏らさない。
- **スキル（ツール）コネクタ** はプロジェクト直下の `skills/<スキル名>/mod.rs` に置き、`Tool` トレイトを実装して `inventory::submit! { ToolReg { ... } }` で登録する。`run()` は `String` を返す（失敗もエラー文字列で返す）。トレイト・レジストリ本体は `src-tauri/src/tools/mod.rs`。
- **発話からスキルを候補へ出す語彙、優先度、先回り実行の引数組み立ては、各`Tool`の`intent_keywords()`／`intent_priority()`／`intent_invocation()`へ置く。** `agent.rs`へスキル名の分岐を足さない。整形済み出力を直接返すか、内部資料としてモデルへ渡すかも`IntentInvocation`でスキル側が申告する。長文の内部資料プロンプトは`skills/<名前>/prompts/*.md`へ置いて`include_str!`で同梱し、ループ本体へ直書きしない。
- スキル固有の Rust 補助モジュールは `skills/<スキル名>/*.rs` に置いて `mod xxx;` で読み込む。PowerShell / Python / テンプレート等は `skills/<スキル名>/scripts/` や `skills/<スキル名>/templates/` に置き、必要なら `include_str!` でバイナリへ同梱する。複数スキルで本当に共有するトレイト・型だけを `src-tauri/src/tools/mod.rs` 等の共通層に置く。
- アプリとスキルが使う外部依存クレートは `src-tauri/Cargo.toml` に集約する。Rust はスキルごとの独立 `Cargo.toml` を持たないため、依存クレート自体を各スキルフォルダに複製しない。例外は、Tauriや製品設定から独立した内部共通部品の `crates/picoagent-core/` だけで、このcrate自身が必要最小限の依存を申告する。外部公開は行わず `publish = false` を維持する。
- **標準的な外向きHTTPは`tools::http::client()`の接続プールを共有する。専用User-Agent、Cookie、プロキシ、DNS固定・リダイレクト制御、長時間ストリームなどコネクタ固有の構築が必要なら`Client::builder()`を使ってよいが、要求全体の`.timeout()`、または長時間処理向けの`.connect_timeout()`＋`.read_timeout()`を必ず設定する。** `Client::new()`と`reqwest::get()`は禁止する。`tools::http::guard`がスキルとバックエンドを走査し、上限なしの個別builderも検出する。
- **外部プロセスの起動は `tools::process::command()` を使う**（`Command::new` を直接使わない）。Windows のコンソール窓抑止（CREATE_NO_WINDOW）・中断時の子プロセス回収（kill_on_drop）・stdio の既定を1箇所に集約している。実行は `process::output()` / `output_text()` でタイムアウト付きにする。常駐プロセス（同梱エンジン・AivisSpeech）のように future の終了後も動かし続けるものは同期版の `process::command_sync()` を使う（窓抑止だけを共通化し、stdio と待ち方は用途ごとに指定する）。以前は `hide_window` を各所へコピーする運用で散らばり、抜けたファイルでは Windows で窓が明滅し子プロセスが残った。`tools::process::guard` のテストが**スキルとバックエンド `src-tauri/src/` の両方を走査して**直接起動と自前 `hide_window` を検出する。
  - **一度きりの実行（起動 → 出力回収 → 終了待ち）を自前で組み立てない。** 同じ処理を書き直すと出力回収の待ち方だけが抜ける。実際に共通層・`whisper_transcribe`・`meeting_recorder` の3箇所が同型の実装を持ち、**どれもタイムアウト後に回収タスクを待ち切っていた**。`sh -c` / `powershell -Command` のように子がさらにプロセスを産む形では、子を kill しても孫がパイプの書き込み端を握ったままで終端（EOF）が来ないため、1秒指定が孫の寿命（30秒）まで戻らなかった。進捗表示や中断が要る長時間処理で `child` を自分で持つ場合も、**出力回収の待ちは `process::join_capture()` を通す**（上限を超えたら中断して戻る）。`tools::process::guard` が `timeout(..., wait())` の自前実装を検出する。
- **`PICOAGENT_COMMAND_ALLOWLIST` でユーザー / AI 指定のコマンドを実行する経路は、`process::is_physical_delete()` による物理削除の拒否も必ず通す。** この判定は以前 `run_command` だけが持っており、同じ allowlist で動くスケジューラには無かった（許可コマンドに `powershell` があれば予約実行から `Remove-Item` を通せた）。`tools::process::guard` のテストが、この設定キーを読むのに判定を通していないファイルを検出して失敗させる。
- **ストリーミング受信のバイト列は `text::Utf8Stream` を通してデコードする**（チャンクごとに `String::from_utf8_lossy` しない）。HTTP のチャンク境界は文字の途中に落ちるため、直接 lossy 変換すると「まだ続きが来ていないだけ」の末尾バイトが `\u{FFFD}` に置き換わって確定し、日本語の応答に文字化けが混ざる。`text::guard` のテストが `bytes_stream()` を使うファイルでの `from_utf8_lossy` を検出して失敗させる。
- **設計ガード（`mod guard`）を書くときは走査範囲を自前で決めず、`crate::guard_scan` の `all_sources()` / `skill_sources()` を使う。** 各ガードが独自にディレクトリを歩いていた結果、`tools::process::guard` は `skills/` しか見ておらず、同じ規則を破っていたバックエンド5ファイルを1つも検出できていなかった。**ガードの穴はガードが無いのと同じ。** `guard_scan::guard` のメタガードが自前走査を検出して失敗させる。
- **許可済みパスへの書き込みは `permissions::fs::write()` を使う**（`SafePath` は `AsRef<Path>` なので `std::fs::write(&safe_path, ..)` がそのまま書けてしまい、ファサードの親フォルダ自動作成と統一エラーメッセージを素通りする）。例外は理由付きの allowlist のみ。`permissions::guard` が検出する。
- **取り消し・再実行など「あとから同じパスを操作する」機能は、記憶した許可を再利用せず実行時点の設定で判定し直す。** `workspace_files` の undo は履歴の生パスへ再検証なしに書き戻しており、許可ルートを狭めたあとでも古い許可のまま操作できていた。
- **文字列の切り詰めは `text::preview()` / `text::truncate_bytes()` を使う**（`&s[..120]` のようなバイト添字で切らない）。Rust の文字列は UTF-8 バイト列なので、位置が文字の途中だと **panic** する。日本語は 1 文字 3 バイトのため固定長で切ればほぼ必ず境界を外す（実際にエージェントループで、モデルが返した不正な引数 JSON のプレビュー生成が日本語入力で panic していた）。`text::guard` のテストが固定長のバイト添字を検出して失敗させる。
- **アプリの状態ファイル（設定・記憶・履歴・集計など）の書き込みは `store::write_atomic()` / `store::write_json_atomic()` を使う**（`fs::write` で直接上書きしない）。状態ファイルは「全体を読んで全体を書き戻す」更新なので、直接上書きすると書き込み中の異常終了で中身が途中までのファイルが残る。各ストアは破損時に空で継続するため、次の保存でその空の状態が書き戻され**設定や記憶が恒久的に消える**。共通層は「一時ファイル → `sync_all` → `rename`」で更新するので、どの瞬間に落ちても完全な旧版か新版のどちらかしか残らない。読めなかったファイルは `store::quarantine_corrupt()` で退避してから空で継続する。`store::guard` のテストが `fs::write` の直接使用を検出して失敗させる（例外は理由付きの allowlist のみ）。
- システムに副作用を持つツール（ファイル・ネットワーク・プロセス等）は、必ず境界（許可ルート・サイズ上限・確認など）を設ける。**ユーザー由来のパスは必ず `permissions::resolve(cfg, input)` で `SafePath` に変換してから使う**。`SafePath` は許可判定を通過したパスを表す型で、生成経路が `resolve` / `SafePath::join` / `SafePath::sibling` に限られている（派生パスも `..` による脱出を防ぐため再検証される）。ファイル操作は `permissions::fs`（`read_to_string` / `write` / `create_dir_all`）を使う。アプリ自身が決めた内部パス（一時ファイル等、ユーザー入力を含まないもの）だけ `SafePath::app_owned` を使ってよい。**許可判定を各スキルで再実装しない**（リンク実体検査・追加許可ルート・拒否パス判定が抜ける。過去に7スキルで同じ抜けが発生した）。`permissions::guard` のテストがスキルを走査して、自前の許可判定とパス引数の `resolve` 漏れを検出し失敗させる。新しいスキルで独自のルート設定キー（`PICOAGENT_*_ROOT` 等）を作らず、`PICOAGENT_ALLOWED_ROOT` / `PICOAGENT_ALLOWED_PATHS` / `PICOAGENT_DENIED_PATHS` を使う。`C:\Windows` / `Program Files` は permissions が常に拒否する。例: `skills/read_file/mod.rs` は `permissions::resolve` で許可ルート外・保護パス・トラバーサルを拒否する。
- スキル経由でユーザーファイルを削除相当の扱いにする場合、物理削除は禁止し、必ず OS のゴミ箱へ移動する。`workspace_files` などに削除機能を追加するときも `delete` ではなく `trash` / `move_to_trash` として実装し、`remove_file` / `remove_dir_all` でユーザー指定ファイルを消さない。`run_command` でも `rm` / `del` / `Remove-Item` / `git clean` などの物理削除コマンドは拒否する。内部一時ファイル、ログローテーション、ビルド生成物、同梱物アンインストールなど PicoAgent 管理下の掃除はこの限りではない。
- コネクタが設定値を必要とするなら `config_schema()` で申告する。値はフィールドに固定せず**メソッド内で `cfg.get()` から都度読む**（設定画面での変更を再起動なしに反映するため）。
- コネクタを追加・変更したら `SPEC.md` のインターフェース定義と一覧、`docs/SETTINGS.md` の設定キー、必要なら `README.md` と `MANUAL.md` の利用手順を更新する。
- メッセージ正規形（コネクタ間の共通言語）を崩さない。詳細は [SPEC.md](SPEC.md) を参照。

詳細仕様は [SPEC.md](SPEC.md) を参照。ここには作業上の注意点だけを書く。

## プロジェクト概要

PicoAgent は同梱推論エンジン（既定）やローカル/リモートの LLM で動く、キャラクターとフキダシだけの透明 **Tauri** デスクトップペット。バックエンドは Rust、フロントは webview（`ui/`）。叩く AI とエージェント機能をコネクタで差し替え・追加できる。

## 実行環境

- Windows / macOS を想定。**Rust（cargo）と Tauri CLI**（`cargo install tauri-cli --version '^2'`）、WebView2（Windows ランタイム）。**Node.js も Ollama も不要**（開発タスクは `cargo xtask`）。
- **Node.js は「アプリと開発タスクの依存」としては使わない。ただし作業用ツールとしては使ってよい**（例: `tools/screenshots/` の Playwright 撮影ツール）。判断基準は「アプリのビルド・起動・配布に Node が要るようになるかどうか」で、`package.json` を置いて依存を増やす形にはしない（グローバル導入のツールを環境変数で指す）。
- 開発起動: `cargo xtask setup engine`（同梱エンジン取得）→ `cargo xtask dev`。製品ビルド: `cargo xtask build`（`src-tauri/target/release/bundle/`）。
- **既定の叩く AI は `local`（同梱 llama-server）**。初回起動時に `PICOAGENT_LOCAL_MODEL_PRESET=auto` が PC の VRAM/RAM 目安から Gemma 4 系プリセットを選び、モデル GGUF をユーザーデータ配下の `PicoAgent/bundled/models/` へ DL し、以後オフライン。Ollama / OpenAI 互換は任意プロバイダとして存続。
- **OpenAI 互換プロバイダ（`OPENAI_API_KEY` 利用）は費用が発生するため、ユーザー指示または運用上の明確な必要がある場合だけ実行する。** local / Ollama はコストがかからない。
- **アプリがプリセットとして案内・取得するローカルモデルは、原則として量子化済みGGUFを使う。** 一般PCでの容量・メモリ・速度を優先し、非量子化重みをプリセットへ追加しない。モデル公式の量子化版が無い場合は、出所と形式を確認できるGGUFを使う。利用者が明示する`custom` URLはこの制限の対象外。
- 初回 `cargo build` / `tauri build` / モデル DL は時間がかかり、ネットワークが必要。
- 推論エンジン本体（`llama-server` + DLL）と `*.gguf` はリポジトリに含めない（`.gitignore` 済、`cargo xtask setup engine` で取得）。
- **Claude Code on the web（Linux コンテナ）で作業する場合は `.claude/hooks/session-start.sh` が環境を用意する。** clone 直後は (1) Tauri / cpal / enigo が要求するシステムライブラリが無い、(2) `frontendDist` が指す `ui-dist/` が Git 管理外で存在せず `tauri::generate_context!` がコンパイル時に落ちる、の2点で `cargo check` すら通らない。フックが apt 導入と `sync-characters` → `build-ui` を済ませ、テストバイナリまで事前ビルドする。手で毎回入れ直さないこと。Windows / macOS のローカル環境では `CLAUDE_CODE_REMOTE` を見て何もしない。

## 製品化（配布）

- `src-tauri/tauri.conf.json` の `productName` / `identifier=com.picoagent.desktop` / `version` を保つ。アイコンは `src-tauri/icons/`（`npx tauri icon <1024pxソース>` で再生成。ソースは `src-tauri/icon-source.png`）。同じコマンドが作る `icons/ios/` `icons/android/` はデスクトップ3OSでは参照されないため、`.gitignore` で追跡しない。
- Windows バンドルはNSIS、Ubuntu Linuxは`.deb`とAppImage。初期販売はWindows先行で、コード署名と更新署名付きの完全自動更新なしで行い、`distribution/release.json` の `platforms=["windows"]` / `signing.required=false` / `updates.required=false` を正本とする。Windows版は利用者が設定から明示した場合だけ、公開Gitのサイズ・SHA-256を検証してNSISを実行する。Windows SmartScreenの警告と更新手順は販売ページで事前案内する。macOSまたはLinuxを販売対象へ追加する場合は、対象パッケージの署名方針と実機QAを先に行い、完全自動更新を有効化するときは各 `required` を `true` にして、証明書・更新鍵・配信先を販売ゲートで必須化する。

## 主要ファイル

| ファイル | 役割 |
|---|---|
| `src-tauri/src/main.rs` / `lib.rs` | エントリ・Builder（state・command・menu イベント・setup） |
| `src-tauri/src/commands.rs` | Tauri command（`pet_*` / `settings_*`）と共有状態 `AppState` |
| `src-tauri/src/menu.rs` | 右クリックネイティブメニューと選択イベント処理 |
| `src-tauri/src/config.rs` | 設定の正本（`app_config_dir/config.json`）。`get()` でシークレットを透過復号 |
| `crates/picoagent-core/` | **非公開の内部共通コア**。正規形メッセージとUTF-8安全処理など、Tauri・OS・製品設定に依存しない処理を置く。外部公開は行わず `publish = false` を保つ |
| `src-tauri/src/layout.rs` | **表示形式（画面構成）**。`PICOAGENT_LAYOUT` 1キーで7形式を切り替え、形式ごとのウィンドウ寸法を `PICOAGENT_WINDOW_LAYOUTS` に覚える。旧 `talk` はLINE風の `chat` へ移行する。HTML はどの形式でも同じで、配置は `ui/styles.css` の `[data-layout=…]` が決める。右クリックメニューは `layout::select()`、設定保存は共通の `remember_current_size()` → `apply()` を通る |
| `src-tauri/src/window_surface.rs` | メインウィンドウを利用者の目の前へ出す共通手順（表示 → 最小化解除 → 前面化）。ホットキー呼び出しと、多重起動時に2つ目の起動が既存ウィンドウを前面化する経路が共有する |
| `src-tauri/src/hotkey.rs` | グローバルホットキー（`tauri-plugin-global-shortcut`）。起動時と設定保存時に `apply()` が全解除→設定どおり再登録し、終了時は `release()` で OS へ返す。呼び出しキーが無いまま隠れて操作不能にならないよう、登録できなければ非表示のウィンドウを表示へ戻す |
| `src-tauri/src/secret.rs` | シークレット（`password` 型設定）の DPAPI 暗号化／復号（Windows）。非 Windows は平文 |
| `src-tauri/src/history.rs` | 会話履歴の永続化（キャラ別 `app_data_dir/history-<キャラID>.json`。起動時・キャラ切替時に直近を文脈へ復元） |
| `src-tauri/src/alarm.rs` | 目覚まし時計の保存（config `PICOAGENT_ALARMS`）と曜日・時刻の正規化。発火はフロント時刻監視 → `pet_alarm_wake` |
| `src-tauri/src/engine.rs` | 同梱 llama-server とモデル GGUF のライフサイクル（DL・spawn・状態通知） |
| `src-tauri/src/agent.rs` | エージェントループ（chat → tool_calls 実行 → 再 chat） |
| `src-tauri/src/api_server.rs` | Ollama 風ローカル HTTP API（既定無効・127.0.0.1 のみ・`PICOAGENT_API_KEY` で Bearer 認証。`/api/chat` `/api/generate` `/v1/chat/completions` `/mcp` ほか） |
| `src-tauri/src/mcp/{mod,client,server}.rs` | MCP 層（agent / skills / mcp の3層構成の相互運用部）。client: `PICOAGENT_MCP_SERVERS` の stdio サーバを起動し `mcp__<サーバ>__<ツール>` として動的登録（tools::dispatch / schemas_for_text からフォールバック参照）。server: `/mcp` でスキルを MCP ツールとして公開 |
| `src-tauri/src/permissions.rs` | **全スキル共通のファイル許可の正本**。`resolve()` が許可判定を通した `SafePath` を返し（生成経路は resolve / join / sibling のみ）、`permissions::fs` が `SafePath` 専用のファイル操作を提供する。`permissions::guard` のテストがスキルの自前実装・`resolve` 漏れを検出 |
| `src-tauri/src/text.rs` | **文字列を安全に扱う共通層**。`preview()`（文字数＋`…`）、`truncate_bytes()`（バイト上限だが文字の途中で切らない）、`Utf8Stream`（ストリーミング受信の逐次 UTF-8 デコード）。`text::guard` のテストが固定長バイト添字での切り出しと、ストリーミングでの直接 lossy 変換を検出 |
| `src-tauri/src/doc_sync.rs` | **ドキュメントと実装のズレを検出する設計ガード**（テスト専用）。実装が参照する `PICOAGENT_*` がすべて `docs/SETTINGS.md` に載っているか、逆に一覧だけに残った削除漏れが無いかを双方向で照合する。`skills/` と SPEC の**スキル分類表**（役割と副作用の一覧）も同じく双方向で照合し、載せ忘れ・消し忘れを検出する |
| `src-tauri/src/character_pack.rs` | **キャラクターパック検証規則の正本**。アプリ（設定画面からの導入）と `cargo xtask character-pack` の両方が使う。CLI 側は `#[path]` でこのファイルを取り込むため、`std` / `serde_json` / `anyhow` 以外に依存しないこと（`crate::` のアプリ内部モジュールを参照しない）。Live2D は `model3.json` の参照ファイルの実在まで確かめる。**キャラクターIDの判定 `is_valid_id()` もここが唯一の定義元**（IDはフォルダ名と設定キーの一部になる。以前は9箇所へ写され、うち4箇所で長さ制限が抜けていた） |
| `src-tauri/src/components.rs` | **初回追加機能の公式取得元・検証・導入境界**。Aivis／Whisperは各公式配布元からPicoAgent管理領域へ直接取得し、THA3／LibreOffice／文書ツールは公式ページと手動設定を案内する。第三者ソフトをPicoAgentのReleaseへ再配布しない |
| `src-tauri/src/user_characters.rs` | 自分で追加したキャラクターの導入・一覧・削除（`app_data_dir/characters/<id>/`）。zip は zip slip と件数・容量を検査しながら展開し、検証を通ってから `rename` で確定する。削除は OS のゴミ箱へ移動。`user_characters::guard` が、検証規則のコピーが CLI 側へ復活していないか、キャラクターID判定の写しが増えていないか、`character_pack.rs` が `crate::` のアプリ内部へ依存していないか（xtask のビルドが壊れる）を検出する |
| `src-tauri/src/tha3_creator.rs` | 設定画面の1枚絵THA3生成。同梱／開発用の専用Python子プロセス・進捗・キャンセル・一時パックを管理し、共通検証後に追加キャラクター領域へ導入する |
| `src-tauri/src/bundle_scope.rs` | **配布物の範囲を固定する設計ガード**（テスト専用）。製品に入るのは `ui/` を写した `ui-dist/` と `bundle.resources` の2経路だけ。開発用のツール・ドキュメント・撮影用画像（`tools/` `docs/` `scripts/` `xtask/`）が混ざらないよう、リソース指定・配信元・`ui/` に置ける拡張子を allowlist で固定する。CSPの`unsafe-eval`へ依存できるのはGit管理外のLive2D用PIXIだけとし、追跡JavaScriptの`eval`／`Function`コンストラクタも拒否する |
| `src-tauri/src/guard_scan.rs` | **設計ガードの走査範囲の正本**（テスト専用）。`all_sources()` がスキルとバックエンド `src-tauri/src/` の全ソースを返す。スキル側は `mod.rs` だけでなく `skills/<スキル名>/*.rs` の補助モジュール（`run_command/jobs.rs` など）も含める（`mod.rs` だけ見ると、補助モジュールへ書いた違反を見逃す）。各ガードが自前でディレクトリを歩くと範囲がずれて違反を見逃すため、`guard_scan::guard` のメタガードが自前走査を検出 |
| `src-tauri/src/store.rs` | **アプリ状態ファイル永続化の共通層**。`write_atomic()` / `write_json_atomic()` が「一時ファイル → `sync_all` → `rename`」で原子的に書き換え、途中で落ちても既存データを壊さない。`quarantine_corrupt()` は読めなかったファイルを `.corrupt-<日時>` へ退避する。`store::guard` のテストが `fs::write` の直接使用を検出 |
| `src-tauri/src/tools/process.rs` | **外部プロセス実行の共通層**。`command()` が Windows のコンソール窓抑止・`kill_on_drop`・stdio 既定を適用し、`output()` / `output_text()` がタイムアウト付きで実行する。`tools::process::guard` のテストが `Command::new` の直接使用を検出 |
| `src-tauri/src/scheduler.rs` | cron 風スケジュールタスクの常駐実行と実行履歴（`app_data_dir/schedule-runs.json`）。日と曜日を両方制限した場合は標準cron互換のOR条件。**検出（同期）と実行（単一ワーカー）はキューで分離**し、長時間タスク中も検出が止まらないようにしている（`scheduler::tests` の設計ガードが検出関数の `async` 化を検出） |
| `src-tauri/src/memory.rs` / `emotion.rs` | 暗黙的な長期メモリ（`app_data_dir/memory.json`）／ルールベースの感情状態推定 |
| `src-tauri/src/usage.rs` / `tracer.rs` | トークン使用量の永続集計／ツール実行レイテンシの計測（診断タブ） |
| `src-tauri/src/aivis.rs` | 同梱 AivisSpeech Engine のライフサイクルと音声モデル導入（読み上げは同梱エンジンのみ） |
| `src-tauri/src/log_paths.rs` / `migration.rs` | ログ置き場と保持期間の管理／旧識別子データの移行 |
| `src-tauri/src/boot_screen.rs` | フロントを読み込めなかったときの案内画面。読込開始（`pet_frontend_ready`）と起動完了（`pet_finish_boot`）を別々に監視し、`PICOAGENT_BOOT_TIMEOUT_MS` の区間内にどちらかが届かなければ、webview 既定のエラーページの代わりにキャラクター付きの案内画面へ `navigate` する（監視は2区間なので案内までの最長はその2倍）。**起動完了の記録はウィンドウ寸法の復元より先に行う**（寸法変更の失敗で、動いている画面を案内画面へ差し替えないため）。**配信元が死んでいるときにこそ出る画面なので、HTML（`src-tauri/assets/boot_screen/`）と画像はバイナリへ埋め込み、専用 URI スキーム `picoboot` から返す**。CSP の `script-src` にインライン許可（`'unsafe-inline'`）が無いため案内ページに JavaScript は置けない（再読み込みはリンク遷移で行う） |
| `src-tauri/src/message.rs` | 正規形メッセージ・`Field`（設定項目） |
| `src-tauri/src/providers/*.rs` | 叩く AI コネクタ（既定 local・trait + inventory 登録）。`mod` / `local` / `ollama` / `openai` / `anthropic` / `gemini` / `deepseek` / `xai` / `mistral` / `agent_cli` |
| `xtask/src/util.rs` | **同梱物取得と差分同期の共通処理**。`download_to()` が応答をストリームで一時ファイル（`<出力先>.part`）へ書き、完了してから rename で確定する（巨大モデルを全部メモリに載せない／中断で壊れたファイルを残さない）。`sync_dir()`はサイズと更新日時が同じファイルを飛ばし、同期元にない生成先だけを削除する。展開は OS 標準の tar / unzip / PowerShell へ委譲 |
| `xtask`（`bundled::fetch_engine`） | 同梱エンジン取得（`cargo xtask setup engine`） |
| `xtask`（`bundled::fetch_whisper`） | 同梱Whisper CLIとモデル取得（`cargo xtask setup whisper`） |
| `xtask/src/office.rs` | OfficeCLI（LibreOffice）と AivisSpeech Engine の取得（`cargo xtask setup office` / `setup aivis`。`src-tauri/resources/bundled/` 配下、Git 管理外。どちらもベストエフォート） |
| `xtask`（`tools::fetch_all`） | 精度向上用 OSS CLI 取得（`cargo xtask setup tools`。Pandoc/ripgrep/ffmpeg/qpdf/poppler/ImageMagick を `src-tauri/resources/bundled/tools/<ツール名>/` へ、Git 管理外。配布が無い OS 分は警告のみで PATH へフォールバック） |
| `xtask`（`cli`） | ローカル API を叩く最小 CLI（`cargo xtask cli status/models/chat/ask/repl`。接続先は `PICOAGENT_API_PORT` / `PICOAGENT_API_KEY`） |
| `xtask`（`bundled::fetch_live2d_runtime`） | Live2D ランタイムを `ui/vendor/live2d/` へ取得（`cargo xtask setup live2d`）。公式サンプルは自動取得せず、開発者が条件確認後に `cargo xtask setup live2d-samples` を明示実行する。いずれも git 管理外 |
| `xtask`（`bundled::fetch_tha3`） | THA3コード・VRAM別モデル・OS別Pythonを準備する（`cargo xtask setup tha3`）。コードとモデルは `resources/bundled/tha3/` へ、Windows開発用Pythonは製品外の `tools/tha3-runtime/` へ置き、製品PythonはNSISインストール時に専用領域へ取得する |
| `xtask/src/assets.rs` | ルート `characters/`（正本）を検証後に `ui/assets/characters/` へ差分同期、`ui/` を `ui-dist/` へ差分ビルド、残存プロセス停止（いずれも dev/build 前に自動実行）。停止は名前一致の `picoagent` / `llama-server` / `whisper-cli`、PicoAgent管理下のAivis、`tools/screenshots/` を起動経路に持つ `node` に限る。全再構築は直接コマンドの`--force`だけ |
| `xtask/src/bundled.rs` / `tools.rs` | 同梱推論エンジン・Whisper・Live2D の取得／精度向上用 OSS CLI 群の取得 |
| `xtask/src/cli.rs` | ローカル API を叩く最小 CLI（`cargo xtask cli status/models/chat/ask/repl`） |
| `xtask/src/verify.rs` | PR前の一括ローカル検証（`cargo xtask verify`）。本体・xtask・追跡JavaScript／MJS・Git差分を途中失敗後も最後まで実行し、`node`未導入だけを明示スキップする |
| `xtask`（`assets::build_ui_dist`） | `ui/` を Tauri 配信用 `ui-dist/` へコピー。製品ビルドでは HTML/CSS のコメント・余分な空白と JS の空行・行頭行末空白を削る。仕上げに `stamp_asset_versions()` が HTML の同梱 CSS / JS 参照へ内容ハッシュの `?v=…` を打つ（版を手書きしない） |
| `scripts/build-character-animations.py` | `characters/<id>/layers/*.png` を正本に、復号RGBA完全一致の `runtime-layers/*.webp` と単一世代 `frames/<世代>/*.webp` を生成し、`character.json` の `animations` / `runtimeLayers` / `assetVersion` を更新する |
| `scripts/build-tha3-layers.py` | 512x512透過立ち絵からTHA3キーポーズ17枚と連番42枚を生成する。CLIでは正本へ登録し、GUI連携では一時キャラクターパックを出力する。削除はTHA3自作キャラだけをOSのゴミ箱へ移す |
| `scripts/build-live2d-psd.py` | `characters/<id>/layers/` から Live2D Cubism Editor 用のレイヤー分け PSD（ひよりのパーツ構成）を `live2d-src/` へ生成する（`python scripts/build-live2d-psd.py`。生成物は Git 管理外） |
| `scripts/manage-character-sources.py` | 制作正本 PNG を内容ハッシュの blob へ格納・展開する（同じ絵を何度も持たないための共有化） |
| `scripts/finish-2d-character-assets.ps1` | PNG 系キャラの高精細化工程（Photoshop 連携）を通しで実行する Windows 専用スクリプト |
| `scripts/CharacterAssetImageProcessor.cs` | 上の工程が呼ぶ画像処理（アルファ再適用・境界検査）の C# 実装 |
| `.claude/hooks/session-start.sh` | **Claude Code on the web のセッション開始フック**（`.claude/settings.json` の `hooks.SessionStart` で登録）。Linux コンテナで `cargo` が通る状態を作る（システムライブラリの apt 導入 → `sync-characters` → `build-ui` で `ui-dist/` 生成 → 依存取得とテストバイナリの事前ビルド）。冪等・非対話で、`CLAUDE_CODE_REMOTE!=true` のローカル環境では即 exit する |
| `.agents/skills/<名前>/SKILL.md` | Codex・Claude Codeなど複数エージェントで共有する作業スキルの正本。Claude Codeの自動認識入口は`.claude/skills/`に置くが、実手順はここだけを更新する |
| `docs/TASKS.md` | **未完了・条件待ちだけを置く一覧**。完了結果はPRへ残し、一覧から削除する |
| `tools/screenshots/shoot.mjs` | README 掲載画像（`docs/images/`）の撮影。実物の `ui/` を配信して `window.__TAURI__` だけモックし、ヘッドレス Chromium で描画する。合成ではなく実際の描画結果。使い方は `tools/screenshots/README.md` |
| `src-tauri/src/tools/mod.rs` | スキル（ツール）の `Tool` トレイト・レジストリ本体 |
| `skills/<スキル名>/mod.rs` | エージェント機能（スキル）コネクタ（trait 実装 + inventory 登録。ディレクトリを足すと自動登録）。`chitchat` は会話中にモデルへ提示しない内部スキル（`agent_callable=false`、アイドル雑談用） |
| `characters/` | キャラクター正本（一覧 `index.json`・人格定義・レイヤー・連番フレーム・ローディング画像。Live2D キャラは `live2d/` にモデル一式）。画像準備手順は `characters/README.md` |
| `src-tauri/build.rs` | コネクタの `#[path] pub mod` 自動生成（`src/providers` と `skills`）＋ tauri ビルド |
| `ui/picoagent.js` | 旧 `window.picoagent` を Tauri 上で再現する互換シム |
| `ui/index.html` / `renderer.js` / `styles.css` | ペット UI |
| `ui/settings.{html,js,css}` | 設定ウィンドウ（全設定の GUI 編集） |

## フォルダ構成

```text
20_PicoAgent/
├── crates/picoagent-core/       ← 非公開の内部共通コア（publish=false）
├── characters/                 ← キャラクター正本（index.json・人格定義・連番フレーム）
├── skills/                     ← エージェント機能（スキル）連結ディレクトリ（<スキル名>/mod.rs を足すと自動登録）
│   └── {time,weather,currency_rates,...}/mod.rs
├── ui/                         ← webview フロント正本（編集対象）
│   ├── picoagent.js                 ← window.picoagent 互換シム（Tauri invoke/listen）
│   ├── index.html / renderer.js / styles.css       ← ペット UI
│   ├── settings.html / settings*.js / settings.css ← 設定ウィンドウ
│   ├── vendor/live2d/          ← Live2D ランタイム（git 管理外、setup live2d で取得）
│   └── assets/characters/      ← characters/ の配信コピー（git 管理外、sync:characters で生成）
├── ui-dist/                    ← Tauri frontendDist（git 管理外、build-ui-dist で生成）
├── src-tauri/
│   ├── Cargo.toml / build.rs / tauri.conf.json
│   ├── capabilities/default.json   ← 権限（ACL）
│   ├── icons/                      ← 生成アイコン一式
│   ├── resources/bundled/          ← 同梱物（engine/aivis/whisper/tha3。git 管理外、cargo xtask setup で取得）
│   └── src/
│       ├── main.rs / lib.rs / commands.rs / menu.rs
│       ├── config.rs / agent.rs / message.rs / engine.rs
│       ├── providers/*.rs      ← 叩く AI（local/ollama/openai/anthropic/gemini/deepseek/xai/mistral/agent_cli。.rs を足すと自動登録）
│       └── tools/mod.rs        ← スキルの Tool トレイト・レジストリ本体
├── xtask/                      ← 開発タスクランナー（cargo xtask setup / dev / build / cli / stop）
├── scripts/                    ← キャラクター画像の生成・高精細化（*.py と Windows 専用の *.ps1 / *.cs）
├── tools/tha3-runtime/         ← Windows開発用THA3 Python環境（git管理外、製品には含めない）
├── .agents/skills/             ← 複数の作業エージェントで共有する手順スキルの正本
├── .claude/                    ← Claude Code の設定、共通スキルへの入口、セッション開始フック
├── AGENTS.md / CLAUDE.md       ← ルール正本 / リダイレクト
├── README.md / MANUAL.md / SPEC.md
├── docs/SETTINGS.md             ← 全設定キーの唯一の一覧
├── docs/DEVELOPMENT.md          ← 開発・xtask・配布ビルド・リポジトリ構成
├── HANDOFF.md                  ← 未マージ作業の短い引き継ぎ
├── docs/TASKS.md               ← 未完了・条件待ちだけを置く一覧
```
