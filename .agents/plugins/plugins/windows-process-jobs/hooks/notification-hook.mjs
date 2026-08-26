#!/usr/bin/env node

import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const MAX_INPUT_BYTES = 1024 * 1024;
const MAX_EVENTS = 10;

async function readInput() {
  const chunks = [];
  let size = 0;
  for await (const chunk of process.stdin) {
    size += chunk.length;
    if (size > MAX_INPUT_BYTES) throw new Error("フック入力が上限を超えました");
    chunks.push(chunk);
  }
  const text = Buffer.concat(chunks).toString("utf8").trim();
  return text ? JSON.parse(text) : {};
}

function stateRoot() {
  return process.env.WINDOWS_PROCESS_JOBS_HOME
    ? path.resolve(process.env.WINDOWS_PROCESS_JOBS_HOME)
    : path.join(process.env.LOCALAPPDATA ?? path.join(os.homedir(), "AppData", "Local"), "Codex", "windows-process-jobs");
}

function sessionId(input) {
  for (const value of [input.thread_id, input.session_id]) {
    if (typeof value === "string" && /^[A-Za-z0-9._-]{8,160}$/.test(value)) return value;
  }
  return null;
}

function validEvent(value, owner) {
  return value
    && value.schema_version === 1
    && value.owner_thread_id === owner
    && /^[A-Za-z0-9._-]{8,160}$/.test(value.id)
    && value.kind === "completion"
    && typeof value.message === "string"
    && value.message.length > 0
    && value.message.length <= 1000;
}

function claimEvents(owner) {
  const root = stateRoot();
  if (!fs.existsSync(root)) return [];
  const candidates = [];
  for (const job of fs.readdirSync(root, { withFileTypes: true })) {
    if (!job.isDirectory()) continue;
    const directory = path.join(root, job.name, "events");
    if (!fs.existsSync(directory)) continue;
    for (const item of fs.readdirSync(directory, { withFileTypes: true })) {
      if (item.isFile() && item.name.endsWith(".json")) candidates.push(path.join(directory, item.name));
    }
  }
  candidates.sort();
  const claimed = [];
  for (const file of candidates.slice(-MAX_EVENTS)) {
    let value;
    try {
      value = JSON.parse(fs.readFileSync(file, "utf8"));
      if (!validEvent(value, owner)) continue;
      const delivered = `${file}.delivered`;
      fs.renameSync(file, delivered);
      claimed.push(value);
    } catch {}
  }
  return claimed;
}

const input = await readInput();
const owner = sessionId(input);
if (owner) {
  const events = claimEvents(owner);
  if (events.length > 0) {
    const context = [
      "Windows Process Jobsの検証済み完了通知です。利用者への回答の冒頭で、次の各通知を簡潔に伝えてください。",
      ...events.map((event) => `- ${event.message}`),
    ].join("\n");
    process.stdout.write(JSON.stringify({
      hookSpecificOutput: {
        hookEventName: String(input.hook_event_name ?? "UserPromptSubmit"),
        additionalContext: context,
      },
    }));
  }
}
