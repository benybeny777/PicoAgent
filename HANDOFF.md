# 引き継ぎ

この文書は、現在進行中の作業だけを次の担当へ渡すための短い入口です。完了した作業の経緯はPR、恒久ルールは[AGENTS.md](AGENTS.md)、現行仕様は[SPEC.md](SPEC.md)、未着手・未確認項目は[docs/TASKS.md](docs/TASKS.md)を参照してください。

## 現在の状態

- 最終確認日: 2026-08-26
- 進行中作業: `docs/freetoken-engine-design`でFreeTokenのWindowsネイティブPoCと`llama-server`比較を中断中
- 開発プラグイン: [PR #175](https://github.com/benybeny777/PicoAgent/pull/175)でCodex・Claude Code共用の`windows-process-jobs`を`main`へマージ済み。`temp\windows-process-jobs-worktree`は`main`へ切り替え、両エージェントの導入元として維持している
- 次に着手する内容: Gemma 4 26B A4B Q4_0の取得を再開し、同一GGUFで`llama-server`とFreeTokenを比較する

## FreeToken再開情報

- Windowsネイティブ環境: `C:\Users\benyb\AppData\Local\PicoAgent-FreeToken\freetoken-0.1.2-py312-cu130\venv`
- 組合せ: FreeToken 0.1.2、Python 3.12.11、PyTorch 2.11.0、CUDA 13.0、RTX 5060 Laptop GPU
- 26B部分ファイル: `C:\Users\benyb\AppData\Roaming\PicoAgent\bundled\models\google_gemma-4-26B-A4B-it-Q4_0-10f3b41b.gguf.part`。中断時点で8.30GB、完成時14.76GB、期待SHA-256は`a6be80ed54a90d47e5c0ef6f1cf087c1162edc327781ff3ce9a2018b2d66f6ef`
- 再開入口: PowerShell 7で`temp\freetoken\download-gemma4-q40.ps1`を実行する。`curl --continue-at -`で部分ファイルから再開し、完了後にスクリプトがSHA-256を検証する
- ベンチ入口: `temp\freetoken\benchmark_gemma4.py`。一時ファイルなのでGit管理外。26B取得後にQ4_0のパスを使って両エンジンを実行する
- E4B結果: 同じ4.84GBのQ4_0で`llama-server`は短文平均約68.8 tokens/s、長文脈込み約30.0 tokens/s、最大GPUメモリ約3.58GB。FreeTokenはE4Bが公式既知対応外で、GGUF設定パーサー停止のため計測不能
- 結果ファイル: `temp\freetoken\benchmark-results\gemma4-e4b-q40-comparison.json`
- 中断処理: ダウンロード用`curl.exe`と親PowerShellだけをコマンドラインで特定して終了済み。FreeToken、Python、`llama-server`の検証プロセスは残っていない
- 後続比較: 現作業が完了してからWSL2と`sglang-kernel`を別途比較する。vLLMはWindows非対応のため今回の候補から除外する

## 再開時の順序

1. `AGENTS.md`を読む。
2. `git status --short --branch`、`git log -5 --oneline`、開いているPRを確認する。
3. 進行中PRがあれば、そのPR本文と差分から再開する。
4. 進行中作業がなければ`docs/TASKS.md`から、着手条件を満たす項目だけを選ぶ。

## 更新ルール

- 未マージの作業、再開に必要な環境、直後に行うことだけを書く。
- 完了した作業の説明、テスト結果、過去のPR一覧は残さない。PRへ記録する。
- マージ後に進行中項目を削除し、何もなければ「進行中作業なし」とする。
