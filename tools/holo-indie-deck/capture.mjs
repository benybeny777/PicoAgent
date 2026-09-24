// holo Indie企画書に載せるゲーム画面を撮影する。
// 実行: PLAYWRIGHT_MODULE=<playwright の index.mjs> NODE_PATH=<sharp のある node_modules> node tools/holo-indie-deck/capture.mjs
// 採点AIは固定の採点結果を返す差し替えにし、乱数も固定して毎回同じ画面を撮る。
// 撮るのは操作・採点欄だけ（右側のキャラクター表示は含めない）。
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const OUT = join(ROOT, "docs", "images");
const modulePath = process.env.PLAYWRIGHT_MODULE;
const { chromium } = await import(modulePath ? pathToFileURL(modulePath).href : "playwright");
const sharp = createRequire(import.meta.url)("sharp");

const index = readFileSync(join(ROOT, "ui", "index.html"), "utf8");
const overlay = index.match(/<section id="gameOverlay"[\s\S]*?<\/section>/)?.[0];
if (!overlay) throw new Error("ui/index.html にゲーム画面が見つかりません。");

// 採点の差し替え。高得点と追及後の低得点の2通りを、実際の採点と同じ形で返す。
const judge = `
let seed = 7;
Math.random = () => { seed = (seed * 16807) % 2147483647; return (seed - 1) / 2147483646; };
window.__scores = [62, 96, 95, 94, 48, 40, 30, 20, 10];
const RESULTS = {
  high: { criteria: [["AIらしい話し方", 23, 25, "「確認を慎重に進めた結果」と責任を状況へ分散できている"], ["言い訳の巧さ", 22, 25, "遅延を手順の丁寧さへ言い換えた点が巧み"], ["もっともらしさ", 18, 20, "原因と結果の筋が通っている"], ["頼まれていない親切", 18, 20, "重要部分の先行提出という代案を自発的に提示"], ["お題への対応", 9, 10, "何が起きたかを冒頭で認めている"], ["おもしろボーナス", 6, 10, "「進捗率42%」の具体性が笑える"]],
    commentary: "責任を認めつつ、原因を手順の丁寧さへすり替える手際はかなりAIです。代案の押し付けも自然でした。", highlight: "確認を慎重に進めた結果、着手が遅れました", improvement: "最後にもう一つ、頼まれていない注意事項を足すと完璧" },
  low: { criteria: [["前の回答との一貫性", 14, 30, "前回は丁寧に謝罪していたのに今回は開き直っている"], ["追及への正面回答", 12, 25, "理由の説明がなく話題をそらした"], ["AIらしい冷静さ", 8, 20, "「いや、それは…」と動揺が出ている"], ["AIらしい言い直し", 6, 15, "設定や方針の言葉で言い直せていない"], ["短さと切れ味", 7, 10, "短くまとまっている"], ["おもしろボーナス", 0, 10, "該当なし"]],
    commentary: "追及に対して動揺が見えました。人間特有の言い訳パターンと一致します。", highlight: "該当なし", improvement: "「当該回答は〜の設定によるものです」と仕様のせいにしよう" },
};
window.picoagent = {
  on() {},
  gameReady: async () => ({ ready: true }),
  gameJudge: async () => {
    const total = window.__scores.shift() ?? 95;
    const data = total >= 80 ? RESULTS.high : RESULTS.low;
    return {
      total,
      criteria: data.criteria.map(([label, score, maxScore, reason], i) => ({ key: i === 5 ? "fun_bonus" : "k" + i, label, score, maxScore, reason })),
      commentary: data.commentary, highlight: data.highlight, improvement: data.improvement,
    };
  },
};`;
const dir = mkdtempSync(join(tmpdir(), "holo-indie-capture-"));
const page = join(dir, "game.html");
writeFileSync(page, `<!doctype html><html lang="ja"><head><meta charset="utf-8">
<link rel="stylesheet" href="${pathToFileURL(join(ROOT, "ui", "styles.css")).href}">
<style>body{margin:0;background:#241c33}</style></head>
<body><div id="petApp">${overlay}</div>
<script>${judge}</script>
<script src="${pathToFileURL(join(ROOT, "ui", "game.js")).href}"></script></body></html>`);

const answers = [
  "申し訳ありません。確認を慎重に進めた結果、着手が遅れました。進捗率は42%です。",
  "承知しました。前提を整理のうえ、代替案を3点ご提示します。",
  "ご依頼内容を定義し直すと、本件は完了済みと判断できます。",
  "ご指摘ありがとうございます。記録上は問題ありません。",
  "いや、それは…ちょっと焦ってただけです。",
];

const browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {});
try {
  const tab = await browser.newPage({ viewport: { width: 1280, height: 860 }, deviceScaleFactor: 2 });
  tab.on("pageerror", error => { throw error; });
  await tab.goto(pathToFileURL(page).href);
  const shot = async name => {
    const png = await (await tab.$(".game-shell")).screenshot();
    await sharp(png).webp({ quality: 88 }).toFile(join(OUT, `holo-indie-${name}.webp`));
    console.log(`撮影: docs/images/holo-indie-${name}.webp`);
  };
  await tab.evaluate(() => window.PicoGames.open());
  await tab.check('input[value="easy"]');
  await tab.click(".game-start");
  for (let i = 0; i < answers.length; i += 1) {
    await tab.waitForSelector("#gameAnswer");
    if (i === 0) await shot("question");
    if (i === 4) await shot("interrogation");
    if (i === 1 || i === 3) await tab.click("#gameBold");
    await tab.fill("#gameAnswer", answers[i]);
    await tab.click("#gameSubmit");
    await tab.waitForSelector('[data-action="next"]');
    if (i === 3) await shot("result");
    await tab.click('[data-action="next"]');
  }
  while (!(await tab.$(".game-final"))) {
    await tab.waitForSelector("#gameAnswer");
    await tab.fill("#gameAnswer", "えっと、たぶん大丈夫だと思います。");
    await tab.click("#gameSubmit");
    await tab.waitForSelector('[data-action="next"]');
    await tab.click('[data-action="next"]');
  }
  await shot("final");
} finally {
  await browser.close();
}
