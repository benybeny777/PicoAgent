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
  return "cloud";
}

const number = value => (/^-?\d+(?:\.\d+)?$/.test(value ?? "") ? Number(value) : null);
const pick = (line, pattern) => line.match(pattern)?.[1] ?? null;

function parseNow(line) {
  if (!line?.startsWith("いまは")) return null;
  return {
    temp: number(pick(line, /^いまは\s*(-?\d+)°C/)),
    feel: number(pick(line, /体感\s*(-?\d+)°C/)),
    humidity: pick(line, /湿度\s*(\d+)%/),
    wind: pick(line, /風\s*(\d+)km\/h/),
    rain: pick(line, /降水\s*([\d.]+)mm/),
  };
}

function parseDay(line) {
  const cells = line.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map(cell => cell.trim());
  if (cells.length !== 4) return null;
  const [date, weather, range, pop] = cells;
  const [icon, ...words] = weather.split(/\s+/);
  const [high, low] = range.split("/").map(part => number(part.replace("°", "").trim()));
  return { date, icon, label: words.join(" "), high, low, pop: number(pop.replace("%", "")) };
}

/**
 * `lines[start]` から天気の回答を読み、カードと次に読む行番号を返す。天気の回答でなければ null。
 * 見出し → いまの行 →（今日の行 → 空行 → 日別の表）の順。予報が無い回答は見出しと「いま」だけで作る。
 */
export function parseWeatherBlock(lines, start) {
  const title = lines[start]?.match(TITLE);
  const now = parseNow(lines[start + 1]);
  if (!title || !now) return null;
  let index = start + 2;
  const days = [];
  if (lines[index]?.startsWith("今日は")) index += 1;
  if (lines[index]?.trim() === "" && lines[index + 1]?.trim() === TABLE_HEADER) index += 1;
  if (lines[index]?.trim() === TABLE_HEADER && /^\|[\s|:-]+\|$/.test(lines[index + 1]?.trim() ?? "")) {
    index += 2;
    while (index < lines.length && /^\|.+\|$/.test(lines[index].trim())) {
      const day = parseDay(lines[index]);
      if (!day) return null;
      days.push(day);
      index += 1;
    }
  }
  const [, icon, place, label] = title;
  return { node: weatherCard({ icon, place, label, now, days }), next: index };
}

const degrees = value => (value === null ? "—" : `${value}°`);

function stat(name, value, unit) {
  if (value === null) return null;
  return el("span", { className: "wx-stat" }, [el("span", { className: "wx-stat-name", textContent: name }), `${value}${unit}`]);
}

/** 週の最低〜最高の幅に対する、その日の気温の帯（天気アプリの温度バー）。 */
function rangeBar(day, floor, span) {
  const bar = el("span", { className: "wx-bar" });
  if (day.low !== null && day.high !== null && span > 0) {
    bar.append(el("span", {
      className: "wx-bar-fill",
      style: { left: `${((day.low - floor) / span) * 100}%`, width: `${Math.max(((day.high - day.low) / span) * 100, 4)}%` },
    }));
  }
  return bar;
}

function forecast(days) {
  const lows = days.map(day => day.low).filter(value => value !== null);
  const highs = days.map(day => day.high).filter(value => value !== null);
  const floor = Math.min(...lows, ...highs);
  const span = Math.max(...lows, ...highs) - floor;
  return el("div", { className: "wx-days" }, days.map((day, position) => el("div", { className: "wx-day" }, [
    el("span", { className: "wx-day-date", textContent: position === 0 ? "今日" : day.date.replace(/（(.)）/, " $1") }),
    el("span", { className: "wx-day-icon", title: day.label, textContent: day.icon }),
    el("span", { className: "wx-day-pop", textContent: day.pop === null ? "" : `☂ ${day.pop}%` }, [], { "data-wet": day.pop !== null && day.pop >= 50 ? "" : null }),
    el("span", { className: "wx-day-low", textContent: degrees(day.low) }),
    rangeBar(day, floor, span),
    el("span", { className: "wx-day-high", textContent: degrees(day.high) }),
  ])));
}

function weatherCard({ icon, place, label, now, days }) {
  const today = days[0];
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
    stat("湿度", now.humidity, "%"),
    stat("風", now.wind, "km/h"),
    stat("降水", now.rain, "mm"),
  ].filter(Boolean);
  if (stats.length) card.append(el("div", { className: "wx-stats" }, stats));
  if (days.length) card.append(forecast(days));
  return card;
}
