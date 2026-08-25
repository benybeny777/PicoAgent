---
name: result
description: Windows Process Jobsの終了済みジョブから終了コードと容量制限済みの結果ログを取得するときに使う。
---

# Windowsジョブ結果

この`SKILL.md`の2階層上を`<plugin-root>`とする。

```powershell
pwsh -NoProfile -File "<plugin-root>/scripts/job.ps1" result <ジョブID> --json
```

`succeeded`、`failed`、`cancelled`を区別し、実行中なら結果扱いにしない。
