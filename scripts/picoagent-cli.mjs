#!/usr/bin/env node
// picoagent-cli.mjs - PicoAgent のローカル API を叩く最小 CLI（Ollama の CLI 風）。
//
// 前提: PicoAgent が起動していて、設定でローカル API が有効なこと
//       （設定画面「共通」タブ →「ローカルAPI」→ 有効化して再起動）。
//
// 使い方: npm run cli -- <コマンド> [引数…]  または  node scripts/picoagent-cli.mjs <コマンド>
//   status                 API の死活とバージョンを表示
//   models                 選択中モデルの一覧（/api/tags）
//   chat <メッセージ…>      1問1答で送信して応答を表示（/api/chat）
//   ask <プロンプト…>       生成（/api/generate。chat と同等の1問1答）
//   repl                   対話モード（会話文脈を保持。exit / Ctrl+C で終了）
//
// 接続先の変更: PICOAGENT_API_PORT（既定 11500）/ PICOAGENT_API_KEY（設定時は必須）

import readline from "node:readline";

const PORT = process.env.PICOAGENT_API_PORT || "11500";
const KEY = process.env.PICOAGENT_API_KEY || "";
const BASE = `http://127.0.0.1:${PORT}`;

function headers() {
  const base = { "content-type": "application/json" };
  if (KEY) base.authorization = `Bearer ${KEY}`;
  return base;
}

async function api(method, path, body) {
  let res;
  try {
    res = await fetch(`${BASE}${path}`, {
      method,
      headers: headers(),
      body: body ? JSON.stringify(body) : undefined,
    });
  } catch {
    console.error(`接続できません: ${BASE}`);
    console.error("PicoAgent を起動し、設定の「ローカルAPI」を有効にして再起動してください。ポートが違う場合は PICOAGENT_API_PORT を設定します。");
    process.exit(1);
  }
  const json = await res.json().catch(() => ({}));
  if (!res.ok) {
    const message = json.error?.message || json.error || `HTTP ${res.status}`;
    console.error(`エラー: ${message}`);
    process.exit(1);
  }
  return json;
}

async function cmdStatus() {
  const version = await api("GET", "/api/version");
  const tags = await api("GET", "/api/tags");
  const model = tags.models?.[0];
  console.log(`PicoAgent v${version.version} (${BASE})`);
  console.log(`モデル: ${model?.name || "(未選択)"} / プロバイダ: ${model?.details?.provider || "?"}`);
}

async function cmdModels() {
  const tags = await api("GET", "/api/tags");
  for (const model of tags.models || []) {
    console.log(`${model.name}\t(${model.details?.provider || "?"})`);
  }
}

async function cmdChat(text) {
  const result = await api("POST", "/api/chat", { messages: [{ role: "user", content: text }] });
  console.log(result.message?.content ?? "");
}

async function cmdAsk(text) {
  const result = await api("POST", "/api/generate", { prompt: text });
  console.log(result.response ?? "");
}

async function cmdRepl() {
  await cmdStatus();
  console.log("対話モード（exit / Ctrl+C で終了）");
  const rl = readline.createInterface({ input: process.stdin, output: process.stdout, prompt: "> " });
  const messages = [];
  rl.prompt();
  rl.on("line", async (line) => {
    const text = line.trim();
    if (!text) { rl.prompt(); return; }
    if (text === "exit" || text === "quit") { rl.close(); return; }
    messages.push({ role: "user", content: text });
    // 応答待ちの間は入力を受け付けない（連投で会話が入れ違うのを防ぐ）。
    rl.pause();
    try {
      const result = await api("POST", "/api/chat", { messages });
      const reply = result.message?.content ?? "";
      messages.push({ role: "assistant", content: reply });
      console.log(reply);
    } catch (error) {
      console.error(String(error));
    }
    rl.resume();
    rl.prompt();
  });
  rl.on("close", () => process.exit(0));
}

const [command, ...rest] = process.argv.slice(2);
const text = rest.join(" ").trim();
switch (command) {
  case "status":
  case "version":
    await cmdStatus();
    break;
  case "models":
  case "list":
    await cmdModels();
    break;
  case "chat":
    if (!text) { console.error("使い方: npm run cli -- chat こんにちは"); process.exit(1); }
    await cmdChat(text);
    break;
  case "ask":
  case "generate":
    if (!text) { console.error("使い方: npm run cli -- ask 今日の天気は？"); process.exit(1); }
    await cmdAsk(text);
    break;
  case "repl":
  case "run":
    await cmdRepl();
    break;
  default:
    console.log(`使い方: npm run cli -- <コマンド> [引数…]
  status               API の死活とバージョン
  models               モデル一覧
  chat <メッセージ…>    1問1答（スキルも使える）
  ask <プロンプト…>     生成（chat と同等）
  repl                 対話モード（会話文脈を保持）
環境変数: PICOAGENT_API_PORT（既定 11500）/ PICOAGENT_API_KEY`);
    process.exit(command ? 1 : 0);
}
