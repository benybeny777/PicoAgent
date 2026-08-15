const LAYER_NAMES = [
  "base", "blink_half", "blink_closed", "mouth_closed", "mouth_open",
  "gaze_left", "gaze_right", "expression_happy", "expression_sad",
  "expression_angry", "expression_shy", "expression_surprised", "expression_confused",
];
const TYPE_LABELS = { preview: "基本画像", layer: "表情レイヤー", frame: "連番フレーム", live2d: "Live2D" };

const gallery = document.querySelector("#partsGallery");
const visibleCount = document.querySelector("#visibleCount");
const characterFilter = document.querySelector("#characterFilter");
const search = document.querySelector("#search");
const loading = document.querySelector("#loading");
const detail = document.querySelector("#detail");
const detailImage = document.querySelector("#detailImage");
const detailTitle = document.querySelector("#detailTitle");
const detailMeta = document.querySelector("#detailMeta");
let items = [];
let visibleItems = [];
let filter = "all";
let detailIndex = 0;

function characterRoot(entry) {
  return new URL(`../../characters/${entry.manifest}`, location.href);
}

function addCandidate(candidates, entry, path, type, label) {
  if (!path || /^(?:https?:|data:)/i.test(path)) return;
  const manifestUrl = characterRoot(entry);
  const url = new URL(path, manifestUrl).href;
  if (candidates.some(item => item.url === url)) return;
  candidates.push({ id: entry.id, name: entry.name, path, url, type, label });
}

async function live2dCandidates(candidates, entry, manifest) {
  const definitions = manifest.live2d?.models || (manifest.live2d?.model ? [{ path: manifest.live2d.model }] : []);
  for (const definition of definitions) {
    const modelPath = definition.path || definition.model;
    if (!modelPath) continue;
    try {
      const modelUrl = new URL(modelPath, characterRoot(entry));
      const model = await fetch(modelUrl).then(response => {
        if (!response.ok) throw new Error(`${response.status}`);
        return response.json();
      });
      for (const texture of model.FileReferences?.Textures || []) {
        const url = new URL(texture, modelUrl).href;
        if (!candidates.some(item => item.url === url)) {
          candidates.push({ id: entry.id, name: entry.name, path: `${modelPath} → ${texture}`, url, type: "live2d", label: "texture" });
        }
      }
    } catch (error) {
      console.warn(`${entry.id}: Live2Dモデル定義を読めません`, error);
    }
  }
}

async function exists(item) {
  try {
    const response = await fetch(item.url, { method: "HEAD" });
    return response.ok;
  } catch {
    return false;
  }
}

async function loadItems() {
  const index = await fetch("../../characters/index.json").then(response => {
    if (!response.ok) throw new Error(`characters/index.json: ${response.status}`);
    return response.json();
  });
  const candidates = [];
  for (const entry of index.characters || []) {
    const option = document.createElement("option");
    option.value = entry.id;
    option.textContent = entry.name;
    characterFilter.append(option);
    try {
      const manifest = await fetch(characterRoot(entry)).then(response => {
        if (!response.ok) throw new Error(`${response.status}`);
        return response.json();
      });
      addCandidate(candidates, entry, "preview.png", "preview", "preview");
      addCandidate(candidates, entry, "preview.jpg", "preview", "preview");
      addCandidate(candidates, entry, manifest.boot?.sleeping, "preview", "boot");
      addCandidate(candidates, entry, "sleeping.png", "preview", "sleeping");
      for (const layer of LAYER_NAMES) {
        addCandidate(candidates, entry, `layers/${layer}.png`, "layer", layer);
        addCandidate(candidates, entry, `expression-patches/${layer}.png`, "layer", `${layer} patch`);
      }
      for (const [state, animation] of Object.entries(manifest.animations || {})) {
        for (const path of animation.framePaths || []) addCandidate(candidates, entry, path, "frame", state);
      }
      await live2dCandidates(candidates, entry, manifest);
    } catch (error) {
      console.warn(`${entry.id}: character.jsonを読めません`, error);
    }
  }
  const checks = await Promise.all(candidates.map(async item => ({ item, ok: await exists(item) })));
  items = checks.filter(result => result.ok).map(result => result.item);
}

function matches(item) {
  const query = search.value.trim().toLocaleLowerCase("ja");
  const typeMatches = filter === "all" || item.type === filter;
  const characterMatches = characterFilter.value === "all" || item.id === characterFilter.value;
  const text = `${item.name} ${item.id} ${item.label} ${item.path} ${TYPE_LABELS[item.type]}`.toLocaleLowerCase("ja");
  return typeMatches && characterMatches && (!query || text.includes(query));
}

function openDetail(item) {
  detailIndex = visibleItems.indexOf(item);
  detailImage.src = item.url;
  detailImage.alt = `${item.name}の${item.label}`;
  detailTitle.textContent = `${item.name}・${item.label}`;
  detailMeta.textContent = `${TYPE_LABELS[item.type]}・${item.path}`;
  detail.showModal();
}

function render() {
  visibleItems = items.filter(matches);
  visibleCount.textContent = String(visibleItems.length);
  gallery.replaceChildren();
  if (!visibleItems.length) {
    const empty = document.createElement("p");
    empty.className = "empty";
    empty.textContent = "条件に合う画像パーツがありません。";
    gallery.append(empty);
    return;
  }
  for (const item of visibleItems) {
    const article = document.createElement("article");
    article.className = "card";
    const button = document.createElement("button");
    button.type = "button";
    button.setAttribute("aria-label", `${item.name}の${item.label}を原寸確認`);
    const media = document.createElement("div");
    media.className = "media";
    const image = document.createElement("img");
    image.src = item.url;
    image.alt = `${item.name}の${item.label}`;
    image.loading = "lazy";
    const badge = document.createElement("span");
    badge.className = `badge ${item.type}`;
    badge.textContent = TYPE_LABELS[item.type];
    media.append(image, badge);
    const caption = document.createElement("div");
    caption.className = "caption";
    const title = document.createElement("strong");
    title.textContent = item.name;
    const label = document.createElement("span");
    label.textContent = item.label;
    caption.append(title, label);
    const path = document.createElement("code");
    path.className = "path";
    path.textContent = item.path;
    button.append(media, caption, path);
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
characterFilter.addEventListener("change", render);
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

loadItems()
  .then(() => {
    loading.hidden = true;
    render();
  })
  .catch(error => {
    loading.textContent = `画像パーツを読み込めません: ${error.message}`;
    console.error(error);
  });
