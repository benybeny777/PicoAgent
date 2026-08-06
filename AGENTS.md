# AGENTS.md — Claude Code / AI エージェント向け作業ガイド（ルール正本）

このファイルが唯一のルール正本です。`CLAUDE.md` はこのファイルへのリダイレクトのみです。ルールを追加・変更する場合はこのファイルだけを編集してください。

## 最重要運用ルール

- `AGENTS.md` はすべて日本語で記述する。英語セクション・英語コメントは追加しない。既存の英語記述を見つけたら日本語に置き換える。
- ルールに反する記述・命名・コメント・dead code を発見した場合は、その場で修正して作業完了後に事後報告する。都度確認を求めない。
- ルール正本は `AGENTS.md` 1本。`CLAUDE.md` はリダイレクトのみなので編集しない。恒久ルールはすべてこのファイルに追記する。
- **ドキュメントは常に最新を保つ。コード・スクリプト・設定・依存関係・フォルダ構成・コネクタのインターフェースなど、README や SPEC.md に記載のある内容を変更したら、必ず同じ作業内でドキュメントも更新する。ユーザーから指摘されてから直すのは禁止。**
- 設定キーの `SPEC.md` への記載漏れ・削除漏れは `doc_sync::guard` のテストが双方向で検出する（手作業の突き合わせは不要。設定キーでない `PICOAGENT_*` 文字列は理由付きで `NOT_CONFIG_KEYS` へ）。
- 対象ドキュメントの目安: アーキテクチャ・コネクタ仕様・メッセージ正規形・設定値 → `SPEC.md`、大きな変更 → `CHANGELOG.md`、起動手順・セットアップ・設定キー・依存関係 → `README.md`。
- ドキュメント更新が本当に不要な軽微修正の場合のみ、不要と判断した理由を作業完了時に明記する。
- コミットメッセージ・ソースコードのコメント（Rust / JS とも）は基本的に日本語で書く（コミットは英語だけの件名にしない）。`feat` / `fix` / `docs` / `chore` などの短い英語プレフィックスや、API / JSON / trait / 識別子・関数名など技術用語の英語はそのまま使ってよい（無理に和訳しない）。
- 運用や品質に影響する大きめの変更は `CHANGELOG.md` に残す。
- **常に Codex / Claude（作業エージェント自身）の利用クレジット消費を最小化して作業する。** 不要な長文出力、重複調査、過剰なファイル読み込み、不要な並列探索、同じ確認のやり直しを避ける。まず検索・差分確認・必要最小限のファイル読み取りで判断する。
  - 検証コストの優先順位: まず `cargo check`（コンパイル）→ 必要なら `cargo test --lib`（ロジック）で済ませる。**GUI 起動（`cargo xtask dev`）・スクリーンショット・フルビルドは高コストなので最後の手段**とし、目視確認が本当に必要なときだけ、複数の確認をまとめて 1 回で行う。
  - 編集はまとめて行い、**ビルド/テスト/起動は変更が出揃ってから最小回数**だけ実行する。同じ内容のビルド・テスト・起動・読み取りを繰り返さない。
  - 起動して確認したら使い終わったプロセスは止め、ロックや再ビルドの無駄を避ける。確認が自動化しづらい（GUI・音声など）場合は、コードの正しさで判断し、最終目視はユーザーに委ねてよい。
- **対応 OS は Windows と macOS の 2 つ。Linux は対応対象外**。Linux 専用の分岐・取得経路を新たに増やさない。開発を Linux で行う場合に備えてビルドは通るようにしておくが（未対応を返すスタブ等）、機能の動作保証はしない。
- PowerShell で日本語ファイルを確認するときは、文字化けを避けるため UTF-8 を明示する。表示が文字化けしても、すぐにソース破損と判断せず UTF-8 で読み直してから判断する。
- 既に未コミット変更がある場合は、ユーザーまたは別エージェントの作業として扱い、勝手に巻き戻さない。
- 作業中のスクリーンショット・連番フレーム・変換途中のファイルなど、一時的な作成物はリポジトリ直下の `temp/` 内だけに置く。ルートやソースディレクトリへ `tmp-*` 等を作らない。`temp/` は `.gitignore` で除外し、一時作成物をコミットへ含めない。
- 修正・作業が完了したら、ユーザーからの個別指示がなくても、作業ツリーの未コミット変更を `git add -A` で全てステージしてコミットし、`origin` へプッシュまで行う。自分が編集したファイルだけでなく、既に存在していた未コミット変更も同じコミットに含めてプッシュする（コミットメッセージは日本語。本ファイルの他ルールに従う）。破壊的な履歴操作（force push・reset --hard など）はしない。
- **PR のマージは squash merge を使う。** 1 PR = 1 コミットになり `main` の履歴が読みやすい、というだけの理由。なお API 経由のマージは、squash / 通常マージのいずれでも GitHub 側が committer を `noreply@github.com` にして署名するため、`git log` 上は Unverified に見える（GitHub の画面上は Verified）。これは避けられないので気にしない。すでに `origin` に載ったコミットを後から書き換えるのは共有履歴の破壊操作になるので行わない（上のルールのとおり）。

## エラー処理・リソース

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
- ユーザーから明示指示がない限り、同梱実行ファイル・同梱モデル・同梱ランタイムは PicoAgent 管理下の専用ディレクトリ（原則 `src-tauri/resources/bundled/<機能名>/`、実行時データは `app_data_dir` 配下）に設置し、既存の手動インストール先・グローバルPATH・共有ポートを既定で使わない。手動インストールを使う場合は設定で明示されたときだけ優先し、同梱物のアンインストール・一時ファイル削除は PicoAgent 管理下のパスだけを対象にする。

## 設定値・シークレット

- 設定の正本は `src-tauri/src/config.rs`（永続ファイル `app_config_dir/config.json`）。優先順位は **永続ファイル > 環境変数 > ハードコード既定値**。新規設定で環境変数を「正本・保存先」にしてはならない（環境変数は既定値としてのみ参照する）。
- 数値・閾値・パス・モデル名など「設定として変えうる値」はソースに散らさず `cfg.get(キー, 既定値)` で都度読む。コネクタは自分の設定項目を `config_schema()`（`Vec<Field>`）で申告し、設定画面に自動で並ぶようにする。新しい設定キーを足したら README と SPEC に必ず記載する。
- 設定画面（右クリック →「設定…」）が唯一の GUI 設定入口。`ui/settings.{html,css,js}` と `commands.rs` の `settings_*` command で構成し、保存後は実行中に反映する（コネクタは `cfg.get()` で都度読むため再起動不要）。
- API キー・トークン等のシークレットはコミットしない。ログ・エラーメッセージ・ドキュメントに値を出さない。OpenAI 互換プロバイダの `Authorization`（`bearer_auth`）など秘密値を含む箇所は、エラー文へ連結しない。`config.json` はローカル個人 PC 専用で、リポジトリにコミットしない（`app_config_dir` 配下に保存される）。
- **API キー・トークン等は必ず `type="password"` で申告し、キー名に `API_KEY` / `TOKEN` / `SECRET` / `PASSWORD` / `WEBHOOK` / `CREDENTIAL` のいずれかを含める。** 暗号化の要否は「スキーマでの password 申告」に加えて**キー名でも判定する**（`secret::is_secret_key`）。スキーマ（`config_schema()`）に載らない設定項目——設定画面 `ui/settings.js` が直接組み立てる `field(...)` など——は型申告だけでは拾えないため（実際にローカル API の `PICOAGENT_API_KEY` がこの経路で平文保存されていた）。`secret::guard` のテストが (1) コネクタのスキーマで秘密っぽいキーが password 型でない申告、(2) 設定画面の password 項目が `is_secret_key` に載らない名前、の両方を検出して失敗させる。
- 設定スキーマで `type="password"` の値（API キー等）は平文で `config.json` に置かない。Windows では DPAPI（`src-tauri/src/secret.rs`）で暗号化して `enc:dpapi:v1:` 形式で保存し、`config.get()` が読み取り時に透過的に復号する（同一 Windows ユーザーのみ復号可能）。既存の平文値は起動時に自動で暗号化へ移行する。非 Windows は DPAPI が無いため平文保存（フォールバック許可）。新しいシークレット設定は必ず `type="password"` で申告し、この暗号化経路に乗せる。

## コネクタ開発ルール（このプロジェクトの中核）

PicoAgent は「叩く AI（LLM バックエンド）」と「エージェント機能（スキル/ツール）」をコネクタとして拡張する。**どちらも決まった場所へファイルを置けば自動登録される**。`build.rs` が `src/providers/*.rs`・ルートの `skills/<スキル名>/mod.rs` を走査して `#[path] pub mod` を生成し、各ファイルの `inventory::submit!` で登録されるため、中央の `mod.rs` 編集は不要。スキルは扱いやすいようプロジェクト直下の `skills/<スキル名>/mod.rs` に置く（トレイト・レジストリ本体は `src-tauri/src/tools/mod.rs`）。スキル専用の補助モジュール、スクリプト、テンプレート、少量の静的アセットは同じ `skills/<スキル名>/` 配下へ同梱する（例: `skills/chitchat/trends.rs`、`skills/ms_office/scripts/office.ps1`）。

- **プロバイダコネクタ** は `src-tauri/src/providers/` に置き、`Provider` トレイトを実装して `inventory::submit! { ProviderReg { make: || Box::new(<型>) } }` で登録する。`chat()` は正規形メッセージ（`message.rs`）を受け取り正規形 assistant を返す。プロバイダ固有のワイヤ形式変換は各コネクタ内に閉じ込め、`agent.rs` に漏らさない。
- **スキル（ツール）コネクタ** はプロジェクト直下の `skills/<スキル名>/mod.rs` に置き、`Tool` トレイトを実装して `inventory::submit! { ToolReg { ... } }` で登録する。`run()` は `String` を返す（失敗もエラー文字列で返す）。トレイト・レジストリ本体は `src-tauri/src/tools/mod.rs`。
- スキル固有の Rust 補助モジュールは `skills/<スキル名>/*.rs` に置いて `mod xxx;` で読み込む。PowerShell / Python / テンプレート等は `skills/<スキル名>/scripts/` や `skills/<スキル名>/templates/` に置き、必要なら `include_str!` でバイナリへ同梱する。複数スキルで本当に共有するトレイト・型だけを `src-tauri/src/tools/mod.rs` 等の共通層に置く。
- Cargo の外部依存クレートはプロジェクト全体の `src-tauri/Cargo.toml` に集約する。Rust はスキルごとの独立 `Cargo.toml` をこの構成では持たないため、依存クレート自体を各スキルフォルダに複製しない。代わりに、依存クレートを使うためのスキル専用ラッパーや実行スクリプトをスキルフォルダへ同梱する。
- **外部プロセスの起動は `tools::process::command()` を使う**（`Command::new` を直接使わない）。Windows のコンソール窓抑止（CREATE_NO_WINDOW）・中断時の子プロセス回収（kill_on_drop）・stdio の既定を1箇所に集約している。実行は `process::output()` / `output_text()` でタイムアウト付きにする。常駐プロセス（同梱エンジン・AivisSpeech）のように future の終了後も動かし続けるものは同期版の `process::command_sync()` を使う（窓抑止だけを共通化し、stdio と待ち方は用途ごとに指定する）。以前は `hide_window` を各所へコピーする運用で散らばり、抜けたファイルでは Windows で窓が明滅し子プロセスが残った。`tools::process::guard` のテストが**スキルとバックエンド `src-tauri/src/` の両方を走査して**直接起動と自前 `hide_window` を検出する。
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
- コネクタを追加・変更したら `SPEC.md` のインターフェース定義と一覧、必要なら `README.md` の設定キーを更新する。
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
- 初回 `cargo build` / `tauri build` / モデル DL は時間がかかり、ネットワークが必要。
- 推論エンジン本体（`llama-server` + DLL）と `*.gguf` はリポジトリに含めない（`.gitignore` 済、`cargo xtask setup engine` で取得）。

## 製品化（配布）

- `src-tauri/tauri.conf.json` の `productName` / `identifier=com.picoagent.desktop` / `version` を保つ。アイコンは `src-tauri/icons/`（`npx tauri icon <1024pxソース>` で再生成。ソースは `src-tauri/icon-source.png`）。
- Windows バンドルは NSIS（`bundle.targets`）。署名（Authenticode）・自動更新（`tauri-plugin-updater`）は鍵・証明書が要るためリリース時に設定する（未設定）。

## 主要ファイル

| ファイル | 役割 |
|---|---|
| `src-tauri/src/main.rs` / `lib.rs` | エントリ・Builder（state・command・menu イベント・setup） |
| `src-tauri/src/commands.rs` | Tauri command（`pet_*` / `settings_*`）と共有状態 `AppState` |
| `src-tauri/src/menu.rs` | 右クリックネイティブメニューと選択イベント処理 |
| `src-tauri/src/config.rs` | 設定の正本（`app_config_dir/config.json`）。`get()` でシークレットを透過復号 |
| `src-tauri/src/secret.rs` | シークレット（`password` 型設定）の DPAPI 暗号化／復号（Windows）。非 Windows は平文 |
| `src-tauri/src/history.rs` | 会話履歴の永続化（キャラ別 `app_data_dir/history-<キャラID>.json`。起動時・キャラ切替時に直近を文脈へ復元） |
| `src-tauri/src/alarm.rs` | 目覚まし時計の保存（config `PICOAGENT_ALARMS`）と曜日・時刻の正規化。発火はフロント時刻監視 → `pet_alarm_wake` |
| `src-tauri/src/engine.rs` | 同梱 llama-server とモデル GGUF のライフサイクル（DL・spawn・状態通知） |
| `src-tauri/src/agent.rs` | エージェントループ（chat → tool_calls 実行 → 再 chat） |
| `src-tauri/src/api_server.rs` | Ollama 風ローカル HTTP API（既定無効・127.0.0.1 のみ・`PICOAGENT_API_KEY` で Bearer 認証。`/api/chat` `/api/generate` `/v1/chat/completions` `/mcp` ほか） |
| `src-tauri/src/mcp/{mod,client,server}.rs` | MCP 層（agent / skills / mcp の3層構成の相互運用部）。client: `PICOAGENT_MCP_SERVERS` の stdio サーバを起動し `mcp__<サーバ>__<ツール>` として動的登録（tools::dispatch / schemas_for_text からフォールバック参照）。server: `/mcp` でスキルを MCP ツールとして公開 |
| `src-tauri/src/permissions.rs` | **全スキル共通のファイル許可の正本**。`resolve()` が許可判定を通した `SafePath` を返し（生成経路は resolve / join / sibling のみ）、`permissions::fs` が `SafePath` 専用のファイル操作を提供する。`permissions::guard` のテストがスキルの自前実装・`resolve` 漏れを検出 |
| `src-tauri/src/text.rs` | **文字列を安全に扱う共通層**。`preview()`（文字数＋`…`）、`truncate_bytes()`（バイト上限だが文字の途中で切らない）、`Utf8Stream`（ストリーミング受信の逐次 UTF-8 デコード）。`text::guard` のテストが固定長バイト添字での切り出しと、ストリーミングでの直接 lossy 変換を検出 |
| `src-tauri/src/doc_sync.rs` | **ドキュメントと実装のズレを検出する設計ガード**（テスト専用）。実装が参照する `PICOAGENT_*` がすべて `SPEC.md` に載っているか、逆に SPEC だけに残った削除漏れが無いかを双方向で照合する。`skills/` と SPEC の**スキル分類表**（役割と副作用の一覧）も同じく双方向で照合し、載せ忘れ・消し忘れを検出する |
| `src-tauri/src/character_pack.rs` | **キャラクターパック検証規則の正本**。アプリ（設定画面からの導入）と `cargo xtask character-pack` の両方が使う。CLI 側は `#[path]` でこのファイルを取り込むため、`std` / `serde_json` / `anyhow` 以外に依存しないこと（`crate::` のアプリ内部モジュールを参照しない）。Live2D は `model3.json` の参照ファイルの実在まで確かめる |
| `src-tauri/src/user_characters.rs` | 自分で追加したキャラクターの導入・一覧・削除（`app_data_dir/characters/<id>/`）。zip は zip slip と件数・容量を検査しながら展開し、検証を通ってから `rename` で確定する。削除は OS のゴミ箱へ移動。`user_characters::guard` が、検証規則のコピーが CLI 側へ復活していないかを検出する |
| `src-tauri/src/tha3_creator.rs` | 設定画面の1枚絵THA3生成。Python子プロセス・進捗・キャンセル・一時パックを管理し、共通検証後に追加キャラクター領域へ導入する |
| `src-tauri/src/bundle_scope.rs` | **配布物の範囲を固定する設計ガード**（テスト専用）。製品に入るのは `ui/` を写した `ui-dist/` と `bundle.resources` の2経路だけ。開発用のツール・ドキュメント・撮影用画像（`tools/` `docs/` `scripts/` `xtask/`）が混ざらないよう、リソース指定・配信元・`ui/` に置ける拡張子を allowlist で固定する |
| `src-tauri/src/guard_scan.rs` | **設計ガードの走査範囲の正本**（テスト専用）。`all_sources()` がスキルとバックエンド `src-tauri/src/` の全ソースを返す。スキル側は `mod.rs` だけでなく `skills/<スキル名>/*.rs` の補助モジュール（`run_command/jobs.rs` など）も含める（`mod.rs` だけ見ると、補助モジュールへ書いた違反を見逃す）。各ガードが自前でディレクトリを歩くと範囲がずれて違反を見逃すため、`guard_scan::guard` のメタガードが自前走査を検出 |
| `src-tauri/src/store.rs` | **アプリ状態ファイル永続化の共通層**。`write_atomic()` / `write_json_atomic()` が「一時ファイル → `sync_all` → `rename`」で原子的に書き換え、途中で落ちても既存データを壊さない。`quarantine_corrupt()` は読めなかったファイルを `.corrupt-<日時>` へ退避する。`store::guard` のテストが `fs::write` の直接使用を検出 |
| `src-tauri/src/tools/process.rs` | **外部プロセス実行の共通層**。`command()` が Windows のコンソール窓抑止・`kill_on_drop`・stdio 既定を適用し、`output()` / `output_text()` がタイムアウト付きで実行する。`tools::process::guard` のテストが `Command::new` の直接使用を検出 |
| `src-tauri/src/scheduler.rs` | cron 風スケジュールタスクの常駐実行と実行履歴（`app_data_dir/schedule-runs.json`）。**検出（同期）と実行（単一ワーカー）はキューで分離**し、長時間タスク中も検出が止まらないようにしている（`scheduler::tests` の設計ガードが検出関数の `async` 化を検出） |
| `src-tauri/src/memory.rs` / `emotion.rs` | 暗黙的な長期メモリ（`app_data_dir/memory.json`）／ルールベースの感情状態推定 |
| `src-tauri/src/usage.rs` / `tracer.rs` | トークン使用量の永続集計／ツール実行レイテンシの計測（診断タブ） |
| `src-tauri/src/aivis.rs` | 同梱 AivisSpeech Engine のライフサイクルと音声モデル導入（読み上げは同梱エンジンのみ） |
| `src-tauri/src/log_paths.rs` / `migration.rs` | ログ置き場と保持期間の管理／旧識別子データの移行 |
| `src-tauri/src/boot_screen.rs` | フロントを読み込めなかったときの案内画面。`PICOAGENT_BOOT_TIMEOUT_MS` 以内に `pet_frontend_ready` が届かなければ、webview 既定のエラーページの代わりにキャラクター付きの案内画面へ `navigate` する。**配信元が死んでいるときにこそ出る画面なので、HTML（`src-tauri/assets/boot_screen/`）と画像はバイナリへ埋め込み、専用 URI スキーム `picoboot` から返す**。CSP が `script-src 'self'` のため案内ページに JavaScript は置けない（再読み込みはリンク遷移で行う） |
| `src-tauri/src/message.rs` | 正規形メッセージ・`Field`（設定項目） |
| `src-tauri/src/providers/{mod,local,ollama,openai}.rs` | 叩く AI コネクタ（既定 local・trait + inventory 登録） |
| `xtask/src/util.rs` | **同梱物取得の共通処理**。`download_to()` が応答をストリームで一時ファイル（`<出力先>.part`）へ書き、完了してから rename で確定する（巨大モデルを全部メモリに載せない／中断で壊れたファイルを残さない）。展開は OS 標準の tar / unzip / PowerShell へ委譲 |
| `xtask`（`bundled::fetch_engine`） | 同梱エンジン取得（`cargo xtask setup engine`） |
| `xtask`（`bundled::fetch_whisper`） | 同梱Whisper CLIとモデル取得（`cargo xtask setup whisper`） |
| `xtask/src/office.rs` | OfficeCLI（LibreOffice）と AivisSpeech Engine の取得（`cargo xtask setup office` / `setup aivis`。`src-tauri/resources/bundled/` 配下、Git 管理外。どちらもベストエフォート） |
| `xtask`（`tools::fetch_all`） | 精度向上用 OSS CLI 取得（`cargo xtask setup tools`。Pandoc/ripgrep/ffmpeg/qpdf/poppler/ImageMagick を `src-tauri/resources/bundled/tools/<ツール名>/` へ、Git 管理外。配布が無い OS 分は警告のみで PATH へフォールバック） |
| `xtask`（`cli`） | ローカル API を叩く最小 CLI（`cargo xtask cli status/models/chat/ask/repl`。接続先は `PICOAGENT_API_PORT` / `PICOAGENT_API_KEY`） |
| `xtask`（`bundled::fetch_live2d`） | Live2D ランタイム（`ui/vendor/live2d/`）とサンプルモデル「ひより」（`characters/hiyori/live2d/`）の取得（`cargo xtask setup live2d`。いずれも git 管理外） |
| `xtask`（`bundled::fetch_tha3`） | THA3コードと学習済みモデルを制作専用の `tools/tha3/` へ取得する（`cargo xtask setup tha3`。Git管理外・製品バンドル外・引数なしsetupの対象外） |
| `xtask/src/assets.rs` | ルート `characters/`（正本）を `ui/assets/characters/` へ同期、`ui/` を `ui-dist/` へ配信ビルド、残存プロセス停止（いずれも dev/build 前に自動実行） |
| `xtask/src/bundled.rs` / `tools.rs` | 同梱推論エンジン・Whisper・Live2D の取得／精度向上用 OSS CLI 群の取得 |
| `xtask/src/cli.rs` | ローカル API を叩く最小 CLI（`cargo xtask cli status/models/chat/ask/repl`） |
| `xtask`（`assets::build_ui_dist`） | `ui/` を Tauri 配信用 `ui-dist/` へコピー。製品ビルドでは HTML/CSS のコメント・余分な空白と JS の空行・行頭行末空白を削る |
| `scripts/build-character-animations.py` | `characters/<id>/layers/` から連番アニメーションを生成し、`character.json` の `animations` / `assetVersion` を更新する |
| `scripts/build-tha3-layers.py` | 512x512透過立ち絵からTHA3キーポーズ17枚と連番42枚を生成する。CLIでは正本へ登録し、GUI連携では一時キャラクターパックを出力する。削除はTHA3自作キャラだけをOSのゴミ箱へ移す |
| `scripts/build-live2d-psd.py` | `characters/<id>/layers/` から Live2D Cubism Editor 用のレイヤー分け PSD（ひよりのパーツ構成）を `live2d-src/` へ生成する（`python scripts/build-live2d-psd.py`。生成物は Git 管理外） |
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
├── characters/                 ← キャラクター正本（index.json・人格定義・連番フレーム）
├── skills/                     ← エージェント機能（スキル）連結ディレクトリ（<スキル名>/mod.rs を足すと自動登録）
│   └── {time,weather,currency_rates,...}/mod.rs
├── ui/                         ← webview フロント正本（編集対象）
│   ├── picoagent.js                 ← window.picoagent 互換シム（Tauri invoke/listen）
│   ├── index.html / renderer.js / styles.css       ← ペット UI
│   ├── settings.html / settings.js / settings.css  ← 設定ウィンドウ
│   ├── vendor/live2d/          ← Live2D ランタイム（git 管理外、setup live2d で取得）
│   └── assets/characters/      ← characters/ の配信コピー（git 管理外、sync:characters で生成）
├── ui-dist/                    ← Tauri frontendDist（git 管理外、build-ui-dist で生成）
├── src-tauri/
│   ├── Cargo.toml / build.rs / tauri.conf.json
│   ├── capabilities/default.json   ← 権限（ACL）
│   ├── icons/                      ← 生成アイコン一式
│   ├── resources/bundled/          ← 同梱物（engine/aivis/whisper。git 管理外、cargo xtask setup で取得）
│   └── src/
│       ├── main.rs / lib.rs / commands.rs / menu.rs
│       ├── config.rs / agent.rs / message.rs / engine.rs
│       ├── providers/{mod,local,ollama,openai}.rs ← 叩く AI（.rs を足すと自動登録）
│       └── tools/mod.rs        ← スキルの Tool トレイト・レジストリ本体
├── xtask/                      ← 開発タスクランナー（cargo xtask setup / dev / build / cli / stop）
├── scripts/*.py                ← キャラクター画像の生成（連番アニメ・Live2D 用 PSD）
├── tools/tha3/                 ← THA3制作ツール・重み・Python環境（git管理外、setup tha3で取得）
├── AGENTS.md / CLAUDE.md       ← ルール正本 / リダイレクト
├── README.md / SPEC.md / CHANGELOG.md
```
