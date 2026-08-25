---
name: rerun
description: 利用者が明示的に承認したとき、終了済みWindows Process Jobsの保護済みargvと作業ディレクトリで新しいジョブを作るために使う。
---

# Windowsジョブ再実行

この`SKILL.md`の2階層上を`<plugin-root>`とする。

```powershell
pwsh -NoProfile -File "<plugin-root>/scripts/job.ps1" rerun <ジョブID> --json
```

終了済みジョブだけを対象とし、利用者の明示承認なしに再実行しない。元の記録を書き換えず、新しいジョブIDを作る。
