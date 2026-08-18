const characters = [
  ["mugi", "むぎ", "2D"],
  ["rin", "りん", "2D"],
  ["mai", "まい", "2D"],
  ["koharu", "こはる", "2D"],
  ["sena", "セナ", "2D"],
  ["noa", "ノア", "2D"],
  ["rin-tha3", "りん（自作THA3）", "2D"],
  ["photoreal-prototype", "実写・物理演算テスト", "実写2D"],
  ["hiyori", "ひより", "Live2D"],
  ["haru", "はる", "Live2D"],
  ["epsilon", "イプシロン", "Live2D"],
  ["mao", "まお", "Live2D"],
  ["rice", "ライス", "Live2D"],
  ["wanko", "わんこ", "Live2D"],
];

const items = characters.flatMap(([id, name, kind]) => [
  { id, name, kind, mode: "standard", modeLabel: "通常表示", src: `../../docs/images/character-${id}.webp` },
  { id, name, kind, mode: "stream", modeLabel: "配信表示", src: `../../docs/images/character-${id}-stream.webp` },
]);

const gallery = document.querySelector("#gallery");
const visibleCount = document.querySelector("#visibleCount");
const search = document.querySelector("#search");
const detail = document.querySelector("#detail");
const detailImage = document.querySelector("#detailImage");
const detailTitle = document.querySelector("#detailTitle");
const detailMeta = document.querySelector("#detailMeta");
let filter = "all";
let visibleItems = items;
let detailIndex = 0;

function matches(item) {
  const query = search.value.trim().toLocaleLowerCase("ja");
  const modeMatches = filter === "all" || item.mode === filter;
  const textMatches = !query || `${item.name} ${item.id} ${item.kind} ${item.modeLabel}`.toLocaleLowerCase("ja").includes(query);
  return modeMatches && textMatches;
}

function openDetail(item) {
  detailIndex = visibleItems.indexOf(item);
  detailImage.src = item.src;
  detailImage.alt = `${item.name}の${item.modeLabel}動画`;
  detailTitle.textContent = item.name;
  detailMeta.textContent = `${item.kind}・${item.modeLabel}・6秒 / 16fps`;
  detail.showModal();
}

function render() {
  visibleItems = items.filter(matches);
  visibleCount.textContent = String(visibleItems.length);
  gallery.replaceChildren();
  if (!visibleItems.length) {
    const empty = document.createElement("p");
    empty.className = "empty";
    empty.textContent = "条件に合う動画がありません。";
    gallery.append(empty);
    return;
  }
  for (const item of visibleItems) {
    const article = document.createElement("article");
    article.className = "card";
    const button = document.createElement("button");
    button.type = "button";
    button.setAttribute("aria-label", `${item.name}の${item.modeLabel}を拡大`);
    button.innerHTML = `<div class="media"><img src="${item.src}" alt="${item.name}の${item.modeLabel}動画" loading="lazy"><span class="badge ${item.mode}">${item.modeLabel}</span></div><div class="caption"><strong>${item.name}</strong><span>${item.kind}</span></div>`;
    button.querySelector("img").addEventListener("error", () => article.classList.add("missing"), { once: true });
    button.addEventListener("click", () => openDetail(item));
    article.append(button);
    gallery.append(article);
  }
}

function stepDetail(offset) {
  if (!visibleItems.length) return;
  detailIndex = (detailIndex + offset + visibleItems.length) % visibleItems.length;
  openDetail(visibleItems[detailIndex]);
}

document.querySelectorAll("[data-filter]").forEach(button => {
  button.addEventListener("click", () => {
    filter = button.dataset.filter;
    document.querySelectorAll("[data-filter]").forEach(node => node.classList.toggle("is-active", node === button));
    render();
  });
});
search.addEventListener("input", render);
document.querySelector("#closeDetail").addEventListener("click", () => detail.close());
document.querySelector("#previous").addEventListener("click", () => stepDetail(-1));
document.querySelector("#next").addEventListener("click", () => stepDetail(1));
detail.addEventListener("click", event => { if (event.target === detail) detail.close(); });
document.addEventListener("keydown", event => {
  if (!detail.open) return;
  if (event.key === "ArrowLeft") stepDetail(-1);
  if (event.key === "ArrowRight") stepDetail(1);
});

render();
