# Windows Process Jobs

Windowsで有限の長時間コマンドをバックグラウンド実行し、CodexまたはClaude Codeの後続セッションから状態、ログ、結果を取得する開発運用プラグインです。

PowerShell 7を使い、管理プロセスがWindows Job Objectを保持します。ジョブ状態は`%LOCALAPPDATA%\Codex\windows-process-jobs`へ保存し、再実行に必要な引数は現在のWindows利用者だけが復号できるDPAPIで保護します。表示用引数は秘密値らしいフラグを伏せます。

プラグインは`start`、`status`、`tail`、`result`、`cancel`、`rerun`の6スキルをCodexとClaude Codeへ共通提供します。開始時の見積り時間を記録し、状態確認では経過時間から推定残り時間を逆算します。見積りを超えた場合は目安を自動延長します。対話入力が必要な処理、常駐サーバー、意図的なデーモンは対象外です。

## 導入

リポジトリ直下で次を実行します。導入後はCodexまたはClaude Codeの新しいセッションを開きます。

```powershell
codex plugin marketplace add . --json
codex plugin add windows-process-jobs@personal --json

claude plugin marketplace add . --scope user
claude plugin install windows-process-jobs@picoagent-dev-tools --scope user
```

PowerShell 7の`pwsh`が見つからない場合は処理を開始せず、導入を促すエラーを返します。Windows PowerShell 5への無言降格は行いません。
