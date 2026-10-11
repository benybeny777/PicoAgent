// weather-card.js - 天気スキルの回答（Markdown）を天気カードへ組み替える。
//
// 天気スキルは履歴・書き出し・読み上げ・モデルへの文脈に共通で流れる Markdown を返す
// （`skills/weather/mod.rs` の `summarize_weather`）。見た目だけをここで整え、本文の形式は変えない。
// 形式が想定どおりでなければ null を返し、通常の Markdown 描画へ任せる（壊れた半端なカードを見せない）。

import { el } from "../app/util.js";

const TITLE = /^\*\*(\S+)\s+(.+)の天気：(.+)\*\*$/;
const TABLE_HEADER = "| 日付 | 天気 | 最高 / 最低 | 降水確率 |";

/** 天気の名前（`weather_icon` のラベル）から、カードの空模様を決める。 */
function skyOf(label) {
  if (/雷/.test(label)) return "storm";
  if (/雪/.test(label)) return "snow";
  if (/雨/.test(label)) return "rain";
  if (/快晴|^晴れ/.test(label)) return "clear";
  if (/薄曇り/.test(label)) return "partly";
  return /曇り|霧/.test(label) ? "cloud" : "unknown";
}

// 数値の欠測「-」と、不正な値を別扱いにする。表示できない値で本文を捨てない。
const number = (value, { missing = false, min = -Infinity, max = Infinity } = {}) => {
  if (missing && value === "-") return null;
  if (!/^-?\d+$/.test(value ?? "")) return undefined;
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) && parsed >= min && parsed <= max ? parsed : undefined;
};
const LABEL = /^(快晴|晴れ|薄曇り|曇り|霧|霧雨|雨|雪|にわか雨|にわか雪|雷雨|—)$/;

function parseNow(line) {
  const match = line?.match(/^いまは(?: (-?\d+)°C)?(?:（体感 (-?\d+)°C）)?(?:、湿度 (\d+)%)?(?:、風 (\d+)km\/h)?(?:、降水 (\d+)mm)?$/);
  if (!match) return null;
  const fields = ["temp", "feel", "humidity", "wind", "rain"];
  const now = {};
  for (const [index, field] of fields.entries()) {
    const raw = match[index + 1];
    const options = field === "humidity" ? { min: 0, max: 100 } : ["wind", "rain"].includes(field) ? { min: 0 } : {};
    now[field] = raw === undefined ? null : number(raw, options);
    if (now[field] === undefined) return null;
  }
  return now;
}

function parseRange(high, low, pop) {
  const result = { high: number(high, { missing: true }), low: number(low, { missing: true }), pop: number(pop, { missing: true, min: 0, max: 100 }) };
  if (Object.values(result).includes(undefined) || (result.high !== null && result.low !== null && result.high < result.low)) return null;
  return result;
}

function parseDay(line) {
  const match = line.match(/^\| (\d{1,2}\/\d{1,2}（[日月火水木金土]）|-) \| (\S+) (.+) \| (-?\d+|-)° \/ (-?\d+|-)° \| (\d+|-)% \|$/);
  if (!match || !LABEL.test(match[3])) return null;
  const range = parseRange(match[4], match[5], match[6]);
  if (!range) return null;
  if (match[1] !== "-") {
    const [month, day] = match[1].split(/[\/（]/).map(Number);
    if (month < 1 || month > 12 || day < 1 || day > new Date(2000, month, 0).getDate()) return null;
  }
  return { date: match[1], icon: match[2], label: match[3], ...range };
}

/** 天気の正規形全体が一致した場合だけカード化し、非該当本文は通常描画へ返す。 */
export function parseWeatherBlock(lines, start) {
  const title = lines[start]?.match(TITLE);
  const now = parseNow(lines[start + 1]);
  if (!title || !LABEL.test(title[3]) || !now) return null;
  let index = start + 2;
  let today = null;
  const days = [];
  if (lines[index]?.startsWith("今日は")) {
    const match = lines[index].match(/^今日は 最高 (-?\d+|-)°C \/ 最低 (-?\d+|-)°C、降水確率 (\d+|-)%$/);
    if (!match || !(today = parseRange(match[1], match[2], match[3]))) return null;
    index += 1;
  }
  const tableIndex = lines[index]?.trim() === "" ? index + 1 : index;
  if (lines[tableIndex]?.trim() === TABLE_HEADER) {
    if (lines[tableIndex + 1]?.trim() !== "| --- | --- | --- | --- |") return null;
    index = tableIndex + 2;
    while (index < lines.length && lines[index].trim().startsWith("|")) {
      const day = parseDay(lines[index].trim());
      if (!day || days.length === 7) return null;
      days.push(day);
      index += 1;
    }
    if (!days.length) return null;
    if (today && ["high", "low", "pop"].some(key => today[key] !== days[0][key])) return null;
  }
  const [, icon, place, label] = title;
  return { node: weatherCard({ icon, place, label, now, today, days }), next: index };
}

const degrees = value => (value === null ? "—" : `${value}°`);

function stat(name, value, unit) {
  if (value === null) return null;
  return el("span", { className: "wx-stat" }, [el("span", { className: "wx-stat-name", textContent: name }), `${value}${unit}`]);
}

/** 週の最低〜最高の幅に対する、その日の気温の帯（天気アプリの温度バー）。 */
function rangeBar(day, floor, span) {
  const bar = el("span", { className: "wx-bar" }, [], { "aria-hidden": "true" });
  if (day.low !== null && day.high !== null && span > 0) {
    bar.append(el("span", {
      className: "wx-bar-fill",
      style: { left: `${((day.low - floor) / span) * 100}%`, width: `${Math.min(100 - ((day.low - floor) / span) * 100, Math.max(((day.high - day.low) / span) * 100, 4))}%` },
    }));
  }
  return bar;
}

function forecast(days, today) {
  const lows = days.map(day => day.low).filter(value => value !== null);
  const highs = days.map(day => day.high).filter(value => value !== null);
  const floor = Math.min(...lows, ...highs);
  const span = Math.max(...lows, ...highs) - floor;
  return el("div", { className: "wx-days" }, days.map((day, position) => el("div", { className: "wx-day" }, [
    el("span", { className: "wx-day-date", textContent: position === 0 && today ? "今日" : day.date.replace(/（(.)）/, " $1") }),
    el("span", { className: "wx-day-icon", title: day.label, textContent: day.icon }, [], { "aria-label": day.label }),
    el("span", { className: "wx-day-pop", textContent: day.pop === null ? "" : `☂ ${day.pop}%` }, [], { "data-wet": day.pop !== null && day.pop >= 50 ? "" : null }),
    el("span", { className: "wx-day-low", textContent: degrees(day.low) }),
    rangeBar(day, floor, span),
    el("span", { className: "wx-day-high", textContent: degrees(day.high) }),
  ], { role: "group", "aria-label": `${day.date}の${day.label}、最低${day.low === null ? "未確認" : `${day.low}度`}、最高${day.high === null ? "未確認" : `${day.high}度`}、降水確率${day.pop === null ? "未確認" : `${day.pop}%`}` })));
}

function weatherCard({ icon, place, label, now, today, days }) {
  const card = el("div", { className: "wx-card", dataset: { sky: skyOf(label) } }, [], {
    role: "group", "aria-label": `${place}の天気：${label}`,
  });
  card.append(el("div", { className: "wx-head" }, [
    el("div", { className: "wx-where" }, [
      el("div", { className: "wx-place", textContent: place }),
      el("div", { className: "wx-cond", textContent: label }),
      today ? el("div", { className: "wx-hilo", textContent: `最高 ${degrees(today.high)}  最低 ${degrees(today.low)}` }) : null,
    ]),
    el("div", { className: "wx-icon", textContent: icon }, [], { "aria-hidden": "true" }),
    el("div", { className: "wx-temp" }, [
      el("span", { className: "wx-temp-value", textContent: now.temp === null ? "—" : String(now.temp) }),
      el("span", { className: "wx-temp-unit", textContent: "°C" }),
    ]),
  ]));
  const stats = [
    stat("体感", now.feel, "°"),
    today && !days.length ? stat("降水確率", today.pop, "%") : null,
    stat("湿度", now.humidity, "%"),
    stat("風", now.wind, "km/h"),
    stat("降水", now.rain, "mm"),
  ].filter(Boolean);
  if (stats.length) card.append(el("div", { className: "wx-stats" }, stats));
  if (days.length) card.append(forecast(days, today));
  return card;
}
