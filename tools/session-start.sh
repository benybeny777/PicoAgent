#!/bin/bash
# session-start.sh - 両エージェントが共有するLinuxリモートの準備手順。
#
# リポジトリを clone しただけでは Linux で `cargo check` / `cargo test` が失敗する。
# 原因は2つあり、どちらも毎回同じ手順で解消できるのでここに固定する。
#   1. Tauri / cpal / enigo が要求するシステムライブラリ（gdk-3.0・alsa など）が入っていない。
#   2. `tauri.conf.json` の frontendDist が指す `ui-dist/` が Git 管理外で存在しない。
#      `tauri::generate_context!` はコンパイル時にこのパスの実在を確かめるため、
#      無いとビルドがマクロの段階で止まる。
#
# Ubuntu Linux も対応OSなので、Tauri本体に加えて画面取得のX11／Wayland／PipeWire依存を揃える。
set -euo pipefail

# ローカル（Windows / macOS）の開発環境には触らない。web セッションでだけ動かす。
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "${CLAUDE_PROJECT_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"

log() { printf '[session-start] %s\n' "$1"; }
phase="作業ディレクトリ確認"
trap 'status=$?; log "準備に失敗（工程: $phase、終了コード: $status）" >&2; exit "$status"' ERR

# ── 1. システムライブラリ ───────────────────────────────────────────
# Tauri v2 の Linux 前提パッケージ ＋ 本プロジェクト固有の依存:
#   libasound2-dev … cpal（マイク録音）が使う alsa-sys
#   libxdo-dev     … enigo（PC 操作スキル）
APT_PACKAGES=(
  pkg-config
  libgtk-3-dev
  libwebkit2gtk-4.1-dev
  libsoup-3.0-dev
  librsvg2-dev
  libayatana-appindicator3-dev
  libasound2-dev
  libxdo-dev
  libclang-dev
  libxcb1-dev
  libxrandr-dev
  libdbus-1-dev
  libpipewire-0.3-dev
  libwayland-dev
  libegl-dev
  libgbm-dev
)

if [ "$(id -u)" -eq 0 ]; then
  SUDO=""
else
  SUDO="sudo"
fi

missing=()
for package in "${APT_PACKAGES[@]}"; do
  if ! dpkg-query -W -f='${Status}' "$package" 2>/dev/null | grep -q "install ok installed"; then
    missing+=("$package")
  fi
done

if [ ${#missing[@]} -eq 0 ]; then
  log "システムライブラリは導入済み（${#APT_PACKAGES[@]} 件）"
else
  log "システムライブラリを導入: ${missing[*]}"
  export DEBIAN_FRONTEND=noninteractive
  phase="システムライブラリ一覧の更新"
  # 一部の取得失敗も非0とし、古い一覧で準備完了を報告しない。
  $SUDO apt-get update -qq -o APT::Update::Error-Mode=any
  phase="システムライブラリ導入"
  $SUDO apt-get install -y --no-install-recommends "${missing[@]}"
fi

# ── 2. 配信フロント（ui-dist）の生成 ─────────────────────────────────
# `characters/` を `ui/assets/characters/` へ写してから `ui/` を `ui-dist/` へ配信ビルドする。
# どちらも Git 管理外なので clone 直後は必ず作り直す（dev / build 前の自動実行と同じ手順）。
# キャラクター同期の失敗でセッション準備を止めない。`characters/**/*.webp` と `*.png` は
# Git LFS 管理で、git-lfs の無いコンテナでは実体ではなくポインタ文字列が置かれる。
# そのまま `set -e` で落とすと **ui-dist/ が作られず `cargo check` すら通らない**
# （実際にこの経路で web セッションの環境準備が丸ごと失敗していた）。
# コンパイルにキャラクター画像の実体は要らないので、理由を出して先へ進む。
# 画像の実表示が要る作業は素材が本物か確認すること。
phase="キャラクター素材同期"
# 同期失敗時の続行はAGENTS「Claude Code on the web」とDEVELOPMENTの現行仕様。
log "キャラクター素材を ui/assets/characters/ へ同期"
if ! cargo xtask sync-characters; then
  log "キャラクター素材の同期に失敗（Git LFS 未取得の可能性）。画像を伴わない検証だけ続行する"
fi
# build-ui は Git 管理外の ui/vendor/pixi/pixi.min.js を必須にしており、無いと
# ui-dist/ を作らずに失敗する（このフックが無条件で落ちていた原因）。固定 SHA-256 と
# 一致する取得済みファイルがあれば通信せずに終わる。取得元の cdn.jsdelivr.net を
# 環境のネットワーク許可へ入れていないと、ここで理由付きで止まる。
phase="固定PIXI準備"
log "PIXI 実行ファイルを準備"
cargo xtask setup pixi
phase="配信フロント生成"
log "ui/ を ui-dist/ へ配信ビルド"
cargo xtask build-ui

# ── 3. 依存クレートの取得とビルドの下ごしらえ ──────────────────────────
# フック完了時点のコンテナ状態がキャッシュされるため、ここまで済ませておくと
# 以降のセッションは初回の `cargo test` から待たされない。
phase="依存クレート取得"
log "依存クレートを取得"
cargo fetch --manifest-path src-tauri/Cargo.toml --quiet

# 変数名に PICOAGENT_ は付けない。あの接頭辞はアプリの設定キーの名前空間で、
# `doc_sync::guard` が docs/SETTINGS.md と双方向で突き合わせている（走査対象は今のところ
# Rust と一部の JS だけでシェルは含まないが、これは設定キーではないので紛れさせない）。
if [ "${SKIP_PREBUILD:-}" = "1" ]; then
  log "SKIP_PREBUILD=1 のため事前ビルドは省略"
else
  log "テストバイナリを事前ビルド（初回は数分かかる）"
  phase="テストバイナリ事前ビルド"
  cargo test --lib --manifest-path src-tauri/Cargo.toml --no-run --quiet
  log "事前ビルド完了"
fi

log "準備完了"
