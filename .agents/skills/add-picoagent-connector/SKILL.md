---
name: add-picoagent-connector
description: PicoAgentへLLMプロバイダまたはツールコネクタを追加・変更し、自動登録、設定、権限、状態排他、外部プロセス、UTF-8、エラー処理、文書、設計ガードまで検証する依頼で使う。
---

# PicoAgentのコネクタを変更する

## 設計

1. `AGENTS.md`の「コネクタ開発ルール」と「エラー処理・リソース」を読む。
2. LLMなら`src-tauri/src/providers/`、ツールなら`skills/<名前>/`の既存実装を検索し、類似コネクタを拡張できない場合だけ新設する。
3. 外部副作用、ユーザー由来パス、共有状態、秘密設定、子プロセス、ネットワーク、キャンセル時解放を先に列挙する。

## 実装

1. ProviderまたはToolトレイトと`inventory::submit!`で自動登録し、中央の手動一覧を増やさない。
2. 発話候補の語彙と優先度は`intent_keywords()`／`intent_priority()`、先回り実行の引数と直接返却／資料差し込みの別は`intent_invocation()`へ置く。`agent.rs`へスキル名分岐を足さない。長文の資料プロンプトは同じスキルの`prompts/*.md`から`include_str!`で読む。
3. 設定が必要なら`add-picoagent-setting`も使い、`config_schema()`と実行時`cfg.get()`へ接続する。
4. ユーザーパスは`permissions`、外部プロセスは`tools::process`、文字列は`text`、状態更新は`store`の共通層を使う。独自の許可判定、直接起動、直接上書きを作らない。
5. 外部APIは`tools::http::service()`の共有接続プールと要求ごとの全体期限を使い、認証・User-Agentを要求単位に付ける。標準profileや配布専用HTTPとの用途境界は`SPEC.md`と`AGENTS.md`を参照し、コネクタ内へ個別builderを増やさない。
6. Toolの失敗はモデルが回復できる文字列として返し、それ以外は`Result`で伝播する。許可したフォールバックだけ理由コメントを付ける。
7. 正常、失敗、キャンセルの全経路でプロセス、接続、タイマー、リスナ、オブジェクトURLを解放する。

## 検証

1. 正常系に加え、未設定、タイムアウト、権限外、破損状態、並列呼び出し、キャンセルをテストする。
2. 関係する設計ガードを`guard_scan`共通走査で追加または更新する。ガードの例外で実装ミスを隠さない。
3. SPECのインターフェースと一覧、READMEの設定を同期し、採否と検証結果をPR本文へ書く。
4. `cargo check`、関係する単体テスト、設計ガード、`git diff --check`を通す。依存変更時は対象lockfileを`cargo audit`する。
