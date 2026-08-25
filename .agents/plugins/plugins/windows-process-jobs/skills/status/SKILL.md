---
name: status
description: 後続ターンでWindows Process Jobsの実行中または直近ジョブの状態と短いログ概要を確認するときに使う。
---

# Windowsジョブ状態

この`SKILL.md`の2階層上を`<plugin-root>`とする。

```powershell
pwsh -NoProfile -File "<plugin-root>/scripts/job.ps1" status [ジョブID] --json
```

ジョブIDを省略すると直近20件を返す。特定ジョブでは状態、経過時間、開始時見積りから逆算した推定残り時間、標準出力・標準エラー各4行だけを返す。見積りを超えた実行中ジョブは目安を自動延長し、更新後の残り時間を示す。推定を実進捗と断定しない。開始した同じターンでは監視しない。
