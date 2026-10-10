// 天気スキルの回答を天気カードへ組み替える。形式が違えば通常の Markdown のまま描く。
// 実物の ui/chat/markdown.js を最小の DOM で動かす。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { installFakeDom } from "./lib/fake-dom.mjs";

const document = installFakeDom();
const { renderMarkdown } = await import("../../ui/chat/markdown.js");

// skills/weather/mod.rs の `天気は見出しと今日の幅と日別の表に分けて見せる` と同じ出力。
const WEATHER = [
  "**☀️ 横浜市（神奈川県）の天気：快晴**",
  "いまは 21°C（体感 20°C）、湿度 60%、風 10km/h、降水 0mm",
  "今日は 最高 23°C / 最低 14°C、降水確率 10%",
  "",
  "| 日付 | 天気 | 最高 / 最低 | 降水確率 |",
  "| --- | --- | --- | --- |",
  "| 10/10（土） | ☀️ 快晴 | 23° / 14° | 10% |",
  "| 10/11（日） | 🌧️ 雨 | 20° / 15° | 80% |",
].join("\n");

function render(text) {
  const container = document.createElement("div");
  renderMarkdown(container, text);
  return container;
}

const texts = node => node.textContent;

test("天気の回答は空模様つきのカードにし、表や太字の記号を残さない", () => {
  const view = render(WEATHER);
  const card = view.querySelector(".wx-card");
  assert.ok(card, "天気カードを描く");
  assert.equal(card.dataset.sky, "clear");
  assert.equal(texts(card.querySelector(".wx-place")), "横浜市（神奈川県）");
  assert.equal(texts(card.querySelector(".wx-cond")), "快晴");
  assert.equal(texts(card.querySelector(".wx-temp-value")), "21");
  assert.deepEqual(card.querySelectorAll(".wx-stat").map(texts), ["体感20°", "湿度60%", "風10km/h", "降水0mm"]);
  const days = card.querySelectorAll(".wx-day");
  assert.deepEqual(days.map(day => texts(day.querySelector(".wx-day-date"))), ["今日", "10/11 日"]);
  assert.equal(days[1].querySelector(".wx-day-pop").getAttribute("data-wet"), "");
  assert.equal(days[0].querySelector(".wx-day-pop").getAttribute("data-wet"), null);
  // 週の幅 14〜23° に対する帯。2日目は 15〜20°。
  const fill = days[1].querySelector(".wx-bar-fill").style;
  assert.equal(Math.round(parseFloat(fill.left)), 11);
  assert.equal(Math.round(parseFloat(fill.width)), 56);
  assert.equal(view.querySelector(".md-table-wrap"), null);
  assert.doesNotMatch(texts(view), /\*\*|\|/);
});

test("予報の無い回答も現在の天気だけのカードにする", () => {
  const card = render("**☁️ 札幌市の天気：曇り**\nいまは 18°C、湿度 60%").querySelector(".wx-card");
  assert.ok(card);
  assert.equal(card.dataset.sky, "cloud");
  assert.deepEqual(card.querySelectorAll(".wx-stat").map(texts), ["湿度60%"]);
  assert.equal(card.querySelector(".wx-days"), null);
});

test("おでかけ前チェックの途中にある天気もカードにし、前後の文はそのまま描く", () => {
  const view = render(`おでかけ前チェックだよ。\n\n${WEATHER}\n\n**控えている通知**\n- 09:00 会議`);
  assert.ok(view.querySelector(".wx-card"));
  assert.match(texts(view), /^おでかけ前チェックだよ。/);
  assert.match(texts(view), /控えている通知/);
});

test("天気の形でない回答や壊れた表はカードにしない", () => {
  assert.equal(render("エラー: 地点が見つかりません: あああ。").querySelector(".wx-card"), null);
  // 見出しだけ似ていて「いまは」の行が無い（モデルが書いた文など）。
  assert.equal(render("**☀️ 東京の天気：晴れ**\nいい天気だね！").querySelector(".wx-card"), null);
  const broken = WEATHER.replace("| 10/11（日） | 🌧️ 雨 | 20° / 15° | 80% |", "| 10/11（日） | 🌧️ 雨 |");
  assert.equal(render(broken).querySelector(".wx-card"), null);
});

test("天気スキルの出力形式とカードの読み取りが同じ形を指す", () => {
  const skill = readFileSync(new URL("../../skills/weather/mod.rs", import.meta.url), "utf8");
  const card = readFileSync(new URL("../../ui/chat/weather-card.js", import.meta.url), "utf8");
  assert.match(skill, /format!\("\*\*\{icon\} \{name\}の天気：\{label\}\*\*\\n"\)/);
  assert.ok(skill.includes("| 日付 | 天気 | 最高 / 最低 | 降水確率 |"));
  assert.ok(card.includes('"| 日付 | 天気 | 最高 / 最低 | 降水確率 |"'));
  assert.ok(skill.includes('"いまは {temp}°C"') && skill.includes('"今日は 最高 {}°C / 最低 {}°C、降水確率 {}%\\n"'));
});
