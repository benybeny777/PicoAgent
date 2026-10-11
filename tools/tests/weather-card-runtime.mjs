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


test("不正な現在値と予報値と欠けた表は本文を欠落させず通常表示する", () => {
  for (const text of [
    WEATHER.replace("湿度 60%", "湿度 101%"),
    WEATHER.replace("いまは 21°C", "いまは 21°Cに見えます"),
    WEATHER.replace("いまは 21°C", "いまは予報を読んでね"),
    WEATHER.replace("いまは 21°C", `いまは ${"9".repeat(400)}°C`),
    WEATHER.replace("80% |", "800% |"),
    WEATHER.replace("20° / 15°", "10° / 15°"),
    WEATHER.replace("| --- | --- | --- | --- |", "| --- | --- |"),
    WEATHER.replace("今日は 最高 23°C / 最低 14°C、降水確率 10%", "今日は傘も忘れないでね"),
    WEATHER.slice(0, WEATHER.lastIndexOf("|")),
  ]) {
    const view = render(text);
    assert.equal(view.querySelector(".wx-card"), null, text);
    assert.ok(view.textContent.includes("いまは"));
  }
});

test("今日の行だけがある場合も最高最低と降水確率を失わない", () => {
  const view = render(WEATHER.split("\n").slice(0, 3).join("\n"));
  assert.match(view.textContent, /最高 23°.*最低 14°/);
  assert.match(view.textContent, /降水確率10%/);
});

test("欠測は未知のまま保持し日付を勝手に今日へ読み替えない", () => {
  const text = WEATHER.replace("今日は 最高 23°C / 最低 14°C、降水確率 10%\n", "")
    .replace("23° / 14°", "-° / -°").replace("10% |", "-% |");
  const card = render(text).querySelector(".wx-card");
  assert.ok(card);
  assert.match(card.querySelector(".wx-day-date").textContent, /10\/10/);
  assert.match(card.querySelector(".wx-day").getAttribute("aria-label"), /未確認/);
});

test("地点や説明へHTMLを含めても実行せず文字として描く", () => {
  const attack = '<img src=x onerror=alert(1)>';
  const view = render(WEATHER.replace("横浜市（神奈川県）", attack));
  assert.equal(view.querySelectorAll("img").length, 0);
  assert.ok(view.textContent.includes(attack));
});


test("天気カードは狭幅で降水列を消さず複数段表示し明るい空模様でも本文のコントラストを保つ", () => {
  const css = readFileSync(new URL("../../ui/components/chat.css", import.meta.url), "utf8");
  const block = css.slice(css.indexOf("/* ── 天気カード"), css.indexOf(".copy-code"));
  assert.doesNotMatch(block, /\.wx-day-pop[^}]*display:\s*none/);
  assert.match(block, /\.wx-day-high[^}]*grid-row:\s*3/);
  assert.match(block, /\.wx-place[^}]*white-space:\s*normal/);
  assert.match(block, /prefers-reduced-motion: reduce/);
  const alpha = Number(block.match(/linear-gradient\(rgba\(0, 0, 0, ([\d.]+)\)/)?.[1]);
  assert.ok(Number.isFinite(alpha));
  const linear = channel => channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  for (const match of block.matchAll(/--wx-(?:from|to): #([\da-f]{6})/g)) {
    const channels = [0, 2, 4].map(offset => parseInt(match[1].slice(offset, offset + 2), 16));
    // 白い光沢が最大の角でも、前面の暗幕と白文字の比を確認する。
    const [r, g, b] = channels.map(channel => linear(((channel * 0.72 + 255 * 0.28) * (1 - alpha)) / 255));
    const luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    assert.ok(1.05 / (luminance + 0.05) >= 4.5, `背景#${match[1]}の本文コントラスト`);
  }
});
