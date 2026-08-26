#!/usr/bin/env node

import fs from "node:fs";

const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const text = Buffer.concat(chunks).toString("utf8").trim();
if (text && process.env.CLAUDE_ENV_FILE) {
  const input = JSON.parse(text);
  const sessionId = input.session_id;
  if (typeof sessionId === "string" && /^[A-Za-z0-9._-]{8,160}$/.test(sessionId)) {
    fs.appendFileSync(
      process.env.CLAUDE_ENV_FILE,
      `export WINDOWS_PROCESS_JOBS_OWNER_THREAD_ID='${sessionId}'\n`,
      "utf8",
    );
  }
}
