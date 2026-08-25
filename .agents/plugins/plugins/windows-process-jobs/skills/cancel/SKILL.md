---
name: cancel
description: 利用者が明示的に求めたとき、Windows Process Jobsの管理対象ジョブだけをPID識別後に取り消すために使う。
---

# Windowsジョブ取消

この`SKILL.md`の2階層上を`<plugin-root>`とする。

```powershell
pwsh -NoProfile -File "<plugin-root>/scripts/job.ps1" cancel <ジョブID> --json
```

- 利用者が取消を明示した場合だけ使う。
- `critical`ジョブは利用者の強制取消承認後に`--force`を付ける。
- PID、開始時刻、実行ファイルが一致しない場合は停止しない。
