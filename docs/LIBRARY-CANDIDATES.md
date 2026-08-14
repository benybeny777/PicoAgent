# 外部ライブラリの候補と採否（Moveable 導入回の引き継ぎ）

Moveable の導入に合わせて「他に入れる価値のある依存はあるか」を調べた結果を、判断材料としてここへ残す。
**判断の理由はここ、手を動かす内容は [CODEX-TASKS.md](CODEX-TASKS.md)、判定の履歴は [REVIEW-LOG.md](REVIEW-LOG.md)、
ルールは [AGENTS.md](../AGENTS.md)** が持ち場。

- 調査日: 2026-08-14（Linux コンテナ上のコード読みのみ。**実機での動作確認・実測は一切していない**）
- 調査時点でリポジトリに Moveable の痕跡は無く、コード変更もしていない
- サイズの数値はすべて**未実測の概算**。採用時に実ファイルで測り直すこと

---

## 0. 候補を絞る制約（先に読む）

ここを満たさないライブラリは、良し悪し以前に入らない。

| 制約 | 根拠 | 効いてくる場面 |
|---|---|---|
| Node をアプリのビルド・起動・配布の依存にしない。`package.json` を置かない | `AGENTS.md`（最重要運用ルール） | JS は**単一ファイルの ESM / UMD をそのまま置ける**ものだけ。ビルドステップ前提のライブラリは対象外 |
| CSP は `script-src 'self' 'unsafe-eval' 'wasm-unsafe-eval'` | `src-tauri/tauri.conf.json` の `app.security.csp` | CDN からの読み込みは不可（ローカル同梱のみ）。eval と wasm 実行そのものは許可済み |
| `ui/` に置ける拡張子は12種（`html js css json png jpg jpeg svg webp gif ico woff2`） | `src-tauri/src/bundle_scope.rs::ALLOWED_UI_EXTENSIONS` | **`.wasm` が入っていない**。wasm を配る前提のライブラリは allowlist の変更が要る（＝設計ガードを緩める判断） |
| 販売品なのでライセンスは MIT / Apache-2.0 に限る。第三者表記は `docs/THIRD-PARTY-NOTICES.md` へ追記 | `docs/SALE-READINESS.md` の運用 | GPL / 商用条件付きは候補にしない |
| Rust の依存は `src-tauri/Cargo.toml` へ集約し、変更したら `cargo audit --file <Cargo.lock>` | `AGENTS.md` | スキルごとに依存を複製しない |

---

## 1. Moveable 本体をどこへ置くか（着手前に決める）

`ui/vendor/` の扱いが2通りあり、**Moveable をどちらにするかで壊れかたが変わる**ので先に決める。

現状:

- `.gitignore` が無視しているのは `ui/vendor/live2d/` と `ui/vendor/vrm/` の**2つだけ**（`ui/vendor/` 全体ではない）
- 一方 `bundle_scope.rs::UI_GENERATED_DIRS` は `vendor` を**ディレクトリごと**走査から外し、理由を
  「`cargo xtask setup live2d` が取得する Live2D ランタイム（再配布条件があるため Git 管理外）」と書いている

### 案A: 取得方式（Live2D / VRM と同じ）

`cargo xtask setup moveable` で取得し、`xtask/src/bundled.rs::ensure_pinned_file` と同じく URL と SHA-256 をピン留めする。

- 利点: 既存の導線に完全に乗る。取得元と版が1か所に固定される
- 欠点: **setup を踏んでいない環境では UI が壊れる**。`.claude/hooks/session-start.sh`、他のエージェント、
  ビルド機のそれぞれで取得が要る。Live2D は「対応キャラを使うときだけ」の機能なので欠けても動くが、
  ドラッグ・リサイズは基本操作なので、欠けたら成立しない

### 案B: Git へ直接コミットする（推奨）

`ui/vendor/moveable/moveable.esm.js` を Git 管理下に置く。

- 利点: Moveable は MIT で**再配布に条件が無い**（Live2D と事情が違う）。clone しただけでビルドが通る状態を保てる
- 欠点: 依存の実体がリポジトリに入る。更新は手作業になる
- 採るなら**必ず併せて直すもの**:
  1. `bundle_scope.rs::UI_GENERATED_DIRS` の除外を `vendor` 全体から `vendor/live2d` と `vendor/vrm` へ狭め、
     コメントの説明（「Git 管理外」）を実態と合わせる。**放置すると、同梱物を固定するはずの設計ガードが
     `ui/vendor/` 配下を素通しし続ける**
  2. `docs/THIRD-PARTY-NOTICES.md` へ Moveable（MIT）を追記する
  3. 採用した版・取得元 URL・SHA-256 をコミットメッセージか本ファイルへ残す（案A の `ensure_pinned_file` と同じ意図）

### どちらでも共通

- 使うのは **dist の ESM 単一ファイル（依存を内包した版）**。CJS 版・非 minified の開発用ビルドは入れない
- CSP は `'self'` なので追加の緩和は不要。`ui-dist/` へは `ui/` の写しとして自動で入る
- 読み込みは既存の2通りが参考になる。`ui/index.html` の importmap（three / three-vrm）か、
  `renderer.js::loadScriptOnce`（Live2D）。**欠けたときに UI 全体を道連れにしない**書き方にすること

---

## 2. 採用を推す候補（優先度順）

### 2-1. `keyring`（Rust / MIT・Apache-2.0）— macOS を販売対象へ入れる前の必須条件

- **なぜ**: `src-tauri/src/secret.rs` は Windows DPAPI で暗号化する一方、コメントのとおり
  **非 Windows では平文保存**にフォールバックする。初期販売対象は Windows だけなので現時点の実害は無いが、
  この状態のまま macOS 版を出すと **API キーが `config.json` に平文で置かれる**
- **やること**: `keyring` で macOS Keychain（および Secret Service）へ寄せる。Windows の DPAPI 経路と
  保存形式のプレフィックス（`enc:dpapi:v1:`）はそのまま残し、OS ごとの分岐として足す
- **注意**: `secret::guard` の password 判定（`SECRET_WORDS` / `NOT_SECRET`）は保存先が変わっても正本のまま。
  判定側をいじらないこと
- **関連**: `CODEX-TASKS.md` の保留 #2（macOS のビルド・署名・公証と実機 QA）の前提条件になる

### 2-2. `cargo-deny`（開発ツール / アプリの依存は増えない）

- **なぜ**: `AGENTS.md` が求めているのは `cargo audit`（＝既知脆弱性）だけで、**ライセンスの混入検査が無い**。
  販売品にとっては GPL 系の紛れ込みのほうが致命的になりうる。重複依存も見える
- **やること**: `deny.toml` を置き、`licenses`（allow に MIT / Apache-2.0 / BSD 系など）と `advisories` を有効にする。
  販売前監査の手順（`.agents/skills/prepare-picoagent-release`）へ1行足す
- **注意**: アプリのバイナリには何も足さない。CI は使わない方針なのでローカル実行の手順として書く

### 2-3. `@floating-ui/dom`（MIT / 概算 7KB・ESM 単一ファイルあり）

- **なぜ**: フキダシ、クイックアクション、トースト、承認ボタンの位置決め。透明ウィンドウは画面端での
  はみ出しが起きやすく、いまは CSS で個別に手当てしている
- **置き換わる現物**: `ui/styles.css` の位置指定と、`renderer.js` の `showToast` / `setPendingChoices` 周辺
- **Moveable との関係**: 役割が重ならない（Moveable = 掴んで動かす、floating-ui = 自動で置き場所を決める）

### 2-4. BudouX（Apache-2.0 / Google / 概算 15〜30KB。モデル同梱）

- **なぜ**: 日本語の分かち書き改行。デスクトップペットのフキダシは幅が狭く、
  `ペットアプリを起/動します` のような不自然な折り返しが出る。**導入の軽さの割に見た目の印象へ効く**
- **やること**: `renderMarkdown` の `appendInline`（`ui/renderer.js`）で、分割位置へ `<wbr>` を入れるだけで足りる
- **注意**: JS 版はモデルを内包した ESM を選ぶ（`.wasm` を要求する構成にしない。§0 の拡張子 allowlist）

### 2-5. `tauri-plugin-window-state` — 採否より先に `layout.rs` との重複を決める

- **なぜ**: Moveable で動かせるようにするなら、位置とサイズの永続化が必ず要る
- **ただし**: `src-tauri/src/layout.rs` が既に `PICOAGENT_WINDOW_LAYOUTS` で**表示形式ごとに寸法を覚えている**。
  さらに `lib.rs::restore_window_layout` と `clamp_to_best_monitor` が復元とモニタ内への丸めを持っている。
  プラグインを足すと保存経路が二重になり、**どちらが勝つかが起動順に依存する不具合**になりやすい
- **推奨**: 表示形式ごとに寸法を持つ現行仕様のほうが要件に近い。**プラグインは入れず `layout.rs` を伸ばす**。
  入れる判断をするなら、`PICOAGENT_WINDOW_*` 系の保存を捨てる前提でまとめて置き換えること（併存させない）

---

## 3. 中優先（次回以降でよい）

| 候補 | ライセンス | 置き換わる現物 | 判断材料 |
|---|---|---|---|
| `streaming-markdown`（smd） | MIT / 概算 10KB | `renderer.js::renderMarkdown`（毎回 `replaceChildren` で全体を組み直す。ストリーム中は rAF で間引いている） | 増分で DOM へ追記でき、長文の生成中が軽くなる。ただし承認ボタン・クイックアクション・表など**独自拡張が乗っているので全置換はコストが高い**。差分描画の考えかただけ取り込むのも可 |
| Motion（旧 Motion One） | MIT / 概算 4KB | `boop` / `pet-heart` / トーストの CSS アニメーション | WAAPI の薄いラッパ。既存の `reduceMotion` フラグとの連動が素直に書ける |
| `@tanstack/virtual` | MIT | `renderer.js::renderHistory`（全件を DOM 化） | 履歴の件数上限が見当たらないため、長い会話で効いてくる。**まず件数上限を入れるほうが安い**ので、それでも足りないときに |
| `rodio` / `symphonia` | MIT・Apache-2.0 | `cpal` + `hound` の手組み再生 | TTS が wav 以外を返す場合に強い。いま困っていないなら急がない |
| `tauri-plugin-autostart` | MIT・Apache-2.0 | — | デスクトップペットではログイン時起動は定番の要望。設定1項目で足りる |
| `tauri-plugin-updater` | MIT・Apache-2.0 | `latest.json` を見て手動更新する現行導線 | 価値は大きいが**署名鍵の運用が増える**（`license.rs` の配布鍵とは別建て）。`signing.required=false` / `updates.required=false` を正本にしている現在の方針と衝突するので、方針変更とセットで |
| `insta`（スナップショットテスト） | Apache-2.0 | `message.rs` の正規形、`character_pack.rs` の検証結果の期待値 | 仕様の固定に向く。テスト専用依存 |
| `@pixiv/three-vrm-animation` | MIT | — | `mugi-vrm-experimental` があるので VRMA モーション対応は自然な次の一歩。実験キャラの位置づけを決めてから |

---

## 4. 見送り（理由つき。蒸し返さないための記録）

| 候補 | 判定 | 理由 |
|---|---|---|
| `markdown-it` + `DOMPurify` | 見送り | 現行の `renderMarkdown` は `innerHTML` を使わず DOM を直接組んでおり、XSS 面では**今のほうが安全**。HTML 文字列を経由する方式へ変えるのは後退 |
| `highlight.js` / Shiki | 見送り | 数百 KB 規模。フキダシのコード表示に対して過剰で、配布サイズを下げたい現状（`CODEX-TASKS.md` P0-3）と逆行する |
| `pixi-live2d-display` | 見送り | Cubism SDK を直接呼ぶ経路（`renderer.js::ensureLive2DRuntime` / `startLive2D`）が既に動いている。いまからの乗り換えは割に合わない |
| React / Vue / Svelte など | 見送り | `package.json` を置かない方針と正面から衝突する。UI 全体の作り直しになる |
| CDN からの読み込み全般 | 不可 | CSP が `script-src 'self'`。オフライン動作の前提も崩れる |

---

## 5. 依存を1つ足すたびに毎回やること

**JS（`ui/vendor/`）**

1. 版・取得元 URL・SHA-256 を記録する（案A なら `ensure_pinned_file`、案B ならコミットメッセージ）
2. ライセンスを確認し `docs/THIRD-PARTY-NOTICES.md` へ追記する
3. `bundle_scope.rs` の走査対象・拡張子 allowlist に触れていないか確かめる
4. `cargo xtask build-ui` で `ui-dist/` へ入ることと、`stamp_asset_versions()` の `?v=…` が付くことを確認する
5. `README.md`（依存関係）と、仕様に関わるなら `SPEC.md` を同じ作業内で更新する

**Rust（`src-tauri/Cargo.toml`）**

1. 依存は `src-tauri/Cargo.toml` へ集約（スキルごとに複製しない）
2. `cargo audit --file src-tauri/Cargo.lock`。情報警告と脆弱性を分けて評価し、見送るなら理由を `REVIEW-LOG.md` へ
3. 対応 OS 限定のものは `[target.'cfg(...)'.dependencies]` へ（`xcap` と同じ形。Linux は対応対象外）

---

## 6. この回でやっていないこと

- 各ライブラリの**実サイズ・実動作は未確認**。数値は概算で、採用時に実測すること
- Moveable 自体の統合設計（どの要素を掴めるようにするか、`layout.rs` / `character_framing.rs` の
  保存経路とどう繋ぐか、8方向リサイズの既存実装との住み分け）は codex 側の作業として手を付けていない
- Linux コンテナでの調査のみ。Windows / macOS 実機での確認はしていない
