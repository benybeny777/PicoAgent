// docs/HOLO-INDIE-PLAN.md の内容から、holo Indie へ応募する作品（PicoAgent）の企画提案書PowerPointを生成する。
// 実行: node tools/holo-indie-deck/build.js
// 前提: pptxgenjs・sharp・jszip（アプリのビルドには不要な作業用ツール）。
// 文面の正本は docs/HOLO-INDIE-PLAN.md（おまけのミニゲームは docs/HOLO-INDIE-MINIGAME.md）。
// 先に正本を直してからこのスクリプトへ反映する。
const fs = require("fs");
const path = require("path");
const pptxgen = require("pptxgenjs");
const sharp = require("sharp");
const JSZip = require("jszip");

const ROOT = path.resolve(__dirname, "..", "..");
const SRC_IMG = path.join(ROOT, "docs", "images");
const OUT = path.join(ROOT, "temp", "HOLO-INDIE-PLAN.pptx");

// pptxgenjs は WebP を埋め込めないため、掲載する画像だけ PNG へ変換して一時領域へ置く。
fs.mkdirSync(path.join(ROOT, "temp"), { recursive: true });
const IMG = fs.mkdtempSync(path.join(ROOT, "temp", "holo-indie-deck-"));
// 画面例は審査版の主役であるさくらみこに統一する。
const USED = ["holo-miko-stream", "holo-miko-chat", "holo-indie-result"];
const imageSizes = new Map();

async function prepareImages() {
  for (const name of USED) {
    const from = path.join(SRC_IMG, name + (name === "holo-indie-result" ? ".webp" : ".png"));
    if (!fs.existsSync(from)) throw new Error("画像が見つかりません: " + from);
    const head = fs.readFileSync(from, { encoding: "utf8", flag: "r" }).slice(0, 20);
    if (head.startsWith("version https")) {
      throw new Error("Git LFS のポインタのままです。git lfs pull を先に実行してください: " + from);
    }
    const { width, height } = await sharp(from).metadata();
    imageSizes.set(name, { width, height });
    await sharp(from).png().toFile(path.join(IMG, name + ".png"));
  }
}

// 夜のステージを思わせる紺紫の地に、ピンクとミントの光を置く配色。
const BASE = "140E24";
const PINK = "FF5C8D";
const MINT = "3DDBB5";
const GOLD = "F5B942";
const VIOLET = "9B7BFF";
const WHITE = "FFFFFF";
const SUB = "CFC6E4";
const DIM = "8F85AB";

const JP = "Noto Sans JP";
const W = 13.333;
const H = 7.5;
const M = 0.75;
const CW = W - M * 2;

// 背景は光のにじみと細かな点格子をSVGで描き、PNGにして全ページへ敷く。
const BACKGROUNDS = {
  cover: [["1650", "120", "900", PINK, 0.55], ["180", "1000", "900", VIOLET, 0.45], ["1100", "1150", "600", MINT, 0.18]],
  base: [["1850", "-80", "760", PINK, 0.32], ["-120", "1150", "760", VIOLET, 0.3]],
  mint: [["1850", "-80", "760", MINT, 0.26], ["-120", "1150", "760", PINK, 0.24]],
  finale: [["960", "1250", "1100", PINK, 0.5], ["960", "-200", "800", VIOLET, 0.35]],
};

async function prepareBackgrounds() {
  for (const [name, glows] of Object.entries(BACKGROUNDS)) {
    const defs = glows.map(([, , , color, o], i) =>
      `<radialGradient id="g${i}"><stop offset="0" stop-color="#${color}" stop-opacity="${o}"/>` +
      `<stop offset="1" stop-color="#${color}" stop-opacity="0"/></radialGradient>`).join("");
    const circles = glows.map(([cx, cy, r], i) => `<circle cx="${cx}" cy="${cy}" r="${r}" fill="url(#g${i})"/>`).join("");
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="1920" height="1080">
      <defs>${defs}<pattern id="dot" width="36" height="36" patternUnits="userSpaceOnUse">
      <circle cx="2" cy="2" r="1.3" fill="#FFFFFF" fill-opacity="0.07"/></pattern></defs>
      <rect width="1920" height="1080" fill="#${BASE}"/>${circles}
      <rect width="1920" height="1080" fill="url(#dot)"/></svg>`;
    await sharp(Buffer.from(svg)).png().toFile(path.join(IMG, "bg-" + name + ".png"));
  }
}

const pres = new pptxgen();
pres.layout = "LAYOUT_WIDE";
pres.author = "PicoAgent";
pres.title = "PicoAgent holo Indie 企画提案書";

const glow = (color, opacity = 0.45) => ({ type: "outer", color, opacity, blur: 18, offset: 0, angle: 90 });

function text(s, value, o) {
  s.addText(value, { isTextBox: true, margin: 0, fontFace: JP, color: WHITE, valign: "top", ...o });
}

function slide(bg, pageNumber) {
  const s = pres.addSlide();
  s.background = { path: path.join(IMG, "bg-" + bg + ".png") };
  if (pageNumber) {
    text(s, "PicoAgent × holo Indie 企画提案", { x: M, y: 7.02, w: 5, h: 0.25, fontSize: 9, color: DIM, charSpacing: 1 });
    text(s, String(pageNumber).padStart(2, "0"), { x: W - M - 0.6, y: 7.02, w: 0.6, h: 0.25, fontSize: 9, bold: true, color: DIM, align: "right" });
  }
  return s;
}

// 見出し。英字の小見出しとアクセント線、大きな日本語タイトル、必要ならリードを置く。
function header(s, kicker, title, lead, accent = PINK) {
  s.addShape(pres.ShapeType.rect, { x: M, y: 0.66, w: 0.36, h: 0.05, fill: { color: accent }, line: { color: accent } });
  text(s, kicker, { x: M + 0.5, y: 0.55, w: 8, h: 0.28, fontSize: 11, bold: true, charSpacing: 4, color: accent });
  text(s, title, { x: M, y: 0.92, w: CW, h: 0.72, fontSize: 30, bold: true });
  if (lead) text(s, lead, { x: M, y: 1.68, w: CW, h: 0.4, fontSize: 14, color: SUB });
}

// 半透明のガラス風カード。
function glass(s, o) {
  s.addShape(pres.ShapeType.roundRect, {
    x: o.x, y: o.y, w: o.w, h: o.h, rectRadius: o.r ?? 0.14,
    fill: { color: o.fill || WHITE, transparency: o.transparency ?? 93 },
    line: { color: o.line || WHITE, transparency: o.line ? 0 : 82, width: o.lineWidth || 1 },
  });
}

function pill(s, label, o) {
  s.addShape(pres.ShapeType.roundRect, {
    x: o.x, y: o.y, w: o.w, h: o.h, rectRadius: o.h / 2,
    fill: { color: o.fill || WHITE, transparency: o.fill ? 0 : 90 }, line: { color: o.line || o.fill || WHITE, transparency: o.line || o.fill ? 0 : 75 },
  });
  text(s, label, { x: o.x, y: o.y, w: o.w, h: o.h, fontSize: o.size || 11, bold: true, color: o.color || WHITE, align: "center", valign: "middle" });
}

// 画面写真。元画像の縦横比を保って指定範囲の中央へ収め、色つきの光で縁取る。
function shot(s, name, o) {
  const { width, height } = imageSizes.get(name);
  const scale = Math.min(o.w / width, o.h / height);
  const w = width * scale;
  const h = height * scale;
  const x = o.x + (o.w - w) / 2;
  const y = o.y + (o.h - h) / 2;
  s.addShape(pres.ShapeType.roundRect, {
    x: x - 0.05, y: y - 0.05, w: w + 0.1, h: h + 0.1, rectRadius: 0.1,
    fill: { color: BASE }, line: { color: o.glow || PINK, width: 1.5 }, shadow: glow(o.glow || PINK),
  });
  s.addImage({ path: path.join(IMG, name + ".png"), x, y, w, h });
  if (name.startsWith("holo-miko-")) {
    s.addNotes("さくらみこ公式MMDモデルをPicoAgentの実フロントで描画した画面。会話は撮影用の固定応答。モデル本体と利用者設定は変更していない。");
  }
  if (o.caption) {
    text(s, o.caption, { x: o.x - 0.2, y: y + h + 0.14, w: o.w + 0.4, h: 0.26, fontSize: 10, color: DIM, align: "center" });
  }
}

function build() {
  /* ---------- 1. 表紙 ---------- */
  {
    const s = slide("cover");
    s.addShape(pres.ShapeType.rect, { x: M, y: 1.02, w: 0.36, h: 0.05, fill: { color: MINT }, line: { color: MINT } });
    text(s, "holo Indie 企画提案書", { x: M + 0.5, y: 0.9, w: 5, h: 0.3, fontSize: 13, bold: true, charSpacing: 4, color: MINT });
    text(s, "推しが、\nデスクトップに住む。", { x: M, y: 1.45, w: 6.4, h: 1.75, fontSize: 46, bold: true, lineSpacingMultiple: 1.05 });
    text(s, [
      { text: "PicoAgent", options: { bold: true, color: PINK } },
      { text: "　ホロライブのキャラクターが常駐し、話し相手と作業の相棒になるデスクトップAI" },
    ], { x: M, y: 3.45, w: 5.9, h: 0.75, fontSize: 14, color: SUB, lineSpacingMultiple: 1.3 });
    const tags = ["常駐して話せる", "端末内で動くAI", "Aivis音声で話す", "おまけゲームつき"];
    let tx = M;
    tags.forEach(t => {
      const w = 0.3 + t.length * 0.13;
      pill(s, t, { x: tx, y: 4.45, w, h: 0.38, size: 10 });
      tx += w + 0.1;
    });
    text(s, "応募者：【氏名／屋号】　　提出日：【日付】　　対象：Windows 10／11", { x: M, y: 6.55, w: 7.5, h: 0.3, fontSize: 11, color: DIM });
    shot(s, "holo-miko-stream", { x: 7.05, y: 1.55, w: 5.55, h: 3.9, caption: "さくらみこ公式MMDモデルによる表示例（審査版の画面）" });
  }

  /* ---------- 2. 企画概要 ---------- */
  {
    const s = slide("base", 2);
    header(s, "CONCEPT", "配信で会う推しを、毎日の相棒に。",
      "PCを開くたびに推しと会い、話題によって返答と表情が変わる。常駐型の会話ゲームを提案します。");
    const pillars = [
      ["01", "いつもいる", PINK, "透明なウィンドウでデスクトップに常駐。作業中もずっとそばにいて、PCを開くたびに会える。"],
      ["02", "話すと変わる", MINT, "名前・口調・性格を持ち、話の流れから推定した感情で表情が変わる。返答はAivis音声でその場で話す。"],
      ["03", "一緒に過ごす", GOLD, "調べものやタイマーを会話で頼める。息抜きにはおまけのミニゲームで遊べる。"],
    ];
    pillars.forEach(([no, t, color, body], i) => {
      const x = M + i * 4.03;
      glass(s, { x, y: 2.35, w: 3.8, h: 3.05 });
      s.addShape(pres.ShapeType.rect, { x, y: 2.35, w: 3.8, h: 0.07, fill: { color }, line: { color } });
      text(s, no, { x: x + 0.35, y: 2.65, w: 1.5, h: 0.75, fontSize: 40, bold: true, color });
      text(s, t, { x: x + 0.35, y: 3.45, w: 3.2, h: 0.5, fontSize: 21, bold: true });
      text(s, body, { x: x + 0.35, y: 4.05, w: 3.15, h: 1.25, fontSize: 12, color: SUB, lineSpacingMultiple: 1.35 });
    });
    const facts = [
      ["ターゲット", "ホロライブのキャラクターと日常的に触れ合いたいファン"],
      ["ジャンル", "会話の入力で返答・表情・行動が変わる常駐型のデジタルゲーム"],
    ];
    facts.forEach(([k, v], i) => {
      const x = M + i * 6.03;
      glass(s, { x, y: 5.7, w: 5.8, h: 0.95, transparency: 96 });
      text(s, k, { x: x + 0.3, y: 5.82, w: 2, h: 0.28, fontSize: 10.5, bold: true, color: PINK, charSpacing: 1 });
      text(s, v, { x: x + 0.3, y: 6.14, w: 5.3, h: 0.4, fontSize: 13, bold: true });
    });
  }

  /* ---------- 3. 着眼点 ---------- */
  {
    const s = slide("mint", 3);
    header(s, "INSIGHT", "「見に行く推し」から「そばにいる推し」へ", null, MINT);
    const before = ["ファンが時間を合わせて見に行く", "視聴している間だけの体験", "画面の向こうの推しを見守る"];
    const after = ["PCを開けば、そこにいる", "作業の合間に話しかけ、返事が返ってくる", "反応が毎回変わり、繰り返し楽しめる"];
    glass(s, { x: M, y: 2.0, w: 5.1, h: 3.3, transparency: 95 });
    text(s, "いま", { x: M + 0.4, y: 2.25, w: 3, h: 0.3, fontSize: 12, bold: true, color: DIM, charSpacing: 2 });
    text(s, "配信・動画で会う推し", { x: M + 0.4, y: 2.6, w: 4.4, h: 0.5, fontSize: 20, bold: true, color: SUB });
    before.forEach((b, i) => text(s, "—　" + b, { x: M + 0.4, y: 3.4 + i * 0.55, w: 4.5, h: 0.4, fontSize: 13, color: DIM }));
    s.addShape(pres.ShapeType.rightArrow, { x: 6.1, y: 3.35, w: 0.85, h: 0.6, fill: { color: MINT }, line: { color: MINT } });
    glass(s, { x: 7.18, y: 2.0, w: 5.4, h: 3.3, fill: MINT, transparency: 88, line: MINT, lineWidth: 1.5 });
    text(s, "PicoAgent", { x: 7.58, y: 2.25, w: 3, h: 0.3, fontSize: 12, bold: true, color: MINT, charSpacing: 2 });
    text(s, "デスクトップで一緒に過ごす推し", { x: 7.58, y: 2.6, w: 4.8, h: 0.5, fontSize: 20, bold: true });
    after.forEach((b, i) => text(s, "✓　" + b, { x: 7.58, y: 3.4 + i * 0.55, w: 4.8, h: 0.4, fontSize: 13, bold: true }));
    text(s, "開発の経緯", { x: M, y: 5.72, w: 3, h: 0.3, fontSize: 12, bold: true, color: MINT, charSpacing: 1 });
    text(s, "PCで実際に使う話し相手と作業の相棒を目指して開発してきました。最も手間がかかったのは、キャラクターの見た目と動きづくりです。魅力あるキャラクターがいるIPと組めば、愛着を持って毎日使える体験を最大限に生かせると考え、応募しました。", {
      x: M, y: 6.05, w: CW, h: 0.8, fontSize: 12, color: SUB, lineSpacingMultiple: 1.3,
    });
  }

  /* ---------- 4. 体験：推しと過ごす1日 ---------- */
  {
    const s = slide("base", 4);
    header(s, "EXPERIENCE", "推しと過ごす1日");
    const steps = [
      ["起動", "PCを起動すると、キャラクターがデスクトップに現れて挨拶する"],
      ["作業の合間", "話しかけると、雑談にもキャラクターとして答える"],
      ["集中したい時", "「この資料を要約して」「25分集中タイマー」と頼む"],
      ["ひと休み", "メニューの「遊ぶ」から、おまけのミニゲームを遊ぶ"],
      ["配信の時間", "配信用の表示に切り替え、会話の様子をそのまま映す"],
    ];
    const sw = 2.24, gap = 0.155;
    s.addShape(pres.ShapeType.line, { x: M + 0.3, y: 2.12, w: CW - 0.6, h: 0, line: { color: PINK, width: 1.5, transparency: 40 } });
    steps.forEach(([t, b], i) => {
      const x = M + i * (sw + gap);
      s.addShape(pres.ShapeType.ellipse, { x: x + 0.12, y: 1.95, w: 0.34, h: 0.34, fill: { color: PINK }, line: { color: WHITE, width: 2 }, shadow: glow(PINK, 0.6) });
      glass(s, { x, y: 2.5, w: sw, h: 1.7 });
      text(s, "STEP " + (i + 1), { x: x + 0.22, y: 2.66, w: 1.5, h: 0.25, fontSize: 9.5, bold: true, color: PINK, charSpacing: 2 });
      text(s, t, { x: x + 0.22, y: 2.94, w: sw - 0.4, h: 0.4, fontSize: 16, bold: true });
      text(s, b, { x: x + 0.18, y: 3.38, w: sw - 0.3, h: 0.8, fontSize: 10.5, color: SUB, lineSpacingMultiple: 1.3 });
    });
    text(s, "3つの表示形式を、再起動なしで切り替え", { x: M, y: 4.6, w: 5.8, h: 0.35, fontSize: 15, bold: true });
    const modes = [["ペット", "フキダシとキャラだけ"], ["ミニ", "キャラ中心の小さな表示"], ["配信", "履歴と入力欄つき"]];
    modes.forEach(([k, v], i) => {
      const y = 5.12 + i * 0.52;
      pill(s, k, { x: M, y, w: 1.05, h: 0.38, fill: i === 2 ? PINK : undefined, size: 11 });
      text(s, v, { x: M + 1.25, y, w: 4.4, h: 0.38, fontSize: 12, color: SUB, valign: "middle" });
    });
    shot(s, "holo-miko-stream", { x: 6.9, y: 4.5, w: 5.68, h: 2.1, caption: "配信形式の画面（さくらみこ）" });
  }

  /* ---------- 5. キャラクターの表現 ---------- */
  {
    const s = slide("mint", 5);
    header(s, "CHARACTER", "表情と声で、キャラクターが生きている。", null, MINT);
    const demos = [
      { name: "holo-miko-motion", label: "さくらみこ", tag: "MMD・Aivis音声つき", x: M, color: PINK,
        body: "待機動作・まばたき・表情の変化。返答をその場で音声で話す。", detail: "審査版に同梱するキャラクター" },
      { name: "holo-hiyori-motion", label: "ひより", tag: "Live2D・無音（旧UI撮影）", x: 6.95, color: MINT,
        body: "滑らかな体の動き・髪の揺れ・まばたき。", detail: "Live2Dの動作例。審査版への同梱キャラではありません" },
    ];
    for (const demo of demos) {
      const w = 5.63, h = w * 9 / 16;
      const labelW = demo.label.length * 0.3;
      text(s, demo.label, { x: demo.x, y: 1.85, w: labelW + 0.2, h: 0.42, fontSize: 20, bold: true });
      pill(s, demo.tag, { x: demo.x + labelW + 0.2, y: 1.9, w: 0.3 + demo.tag.length * 0.12, h: 0.34, size: 9.5, color: demo.color });
      const video = path.join(SRC_IMG, demo.name + ".mp4");
      const poster = path.join(SRC_IMG, demo.name + "-poster.png");
      for (const asset of [video, poster]) {
        if (!fs.existsSync(asset) || fs.readFileSync(asset).subarray(0, 20).toString().startsWith("version https")) {
          throw new Error("動画またはポスターの実体がありません。capture-current.mjs --video で撮影してください: " + asset);
        }
      }
      s.addShape(pres.ShapeType.rect, {
        x: demo.x - 0.04, y: 2.45 - 0.04, w: w + 0.08, h: h + 0.08,
        fill: { color: BASE }, line: { color: demo.color, width: 1.5 }, shadow: glow(demo.color),
      });
      s.addMedia({ type: "video", path: video, cover: "image/png;base64," + fs.readFileSync(poster).toString("base64"),
        x: demo.x, y: 2.45, w, h, objectName: demo.name });
      text(s, demo.body, { x: demo.x, y: 5.85, w, h: 0.35, fontSize: 13, bold: true });
      text(s, demo.detail, { x: demo.x, y: 6.25, w, h: 0.3, fontSize: 10.5, color: DIM });
    }
    text(s, "▶ 動画はスライドショーで▶を押して再生", { x: M, y: 6.62, w: 6, h: 0.28, fontSize: 10.5, bold: true, color: MINT });
    s.addNotes("PicoAgentの実フロント描画を録画。会話は撮影用固定応答。さくらみこはローカル登録の公式MMDモデル。モデル本体は変更せず、録画のみ埋め込み。");
  }

  /* ---------- 6. 繰り返し遊ばれる仕組み ---------- */
  {
    const s = slide("base", 6);
    header(s, "PLAY LOOP", "毎日立ち上げたくなる、3つのループ");
    const loops = [
      ["会話のループ", PINK, ["話しかける", "返答と表情が変わる", "次の話題を試したくなる"]],
      ["相棒のループ", GOLD, ["調べもの・タイマーを頼む", "実際に手伝ってくれる", "毎日立ち上げる理由になる"]],
      ["共有のループ", MINT, ["予想外の反応・講評", "配信で共有できる", "配信の見せ場になる"]],
    ];
    loops.forEach(([t, color, flow], i) => {
      const y = 2.0 + i * 1.38;
      glass(s, { x: M, y, w: 7.9, h: 1.2 });
      s.addShape(pres.ShapeType.rect, { x: M, y, w: 0.08, h: 1.2, fill: { color }, line: { color } });
      text(s, t, { x: M + 0.35, y: y + 0.18, w: 3, h: 0.35, fontSize: 15, bold: true, color });
      flow.forEach((f, j) => {
        const fx = M + 0.35 + j * 2.5;
        text(s, f, { x: fx, y: y + 0.6, w: 2.1, h: 0.5, fontSize: 11.5, bold: j === 2, color: j === 2 ? WHITE : SUB, lineSpacingMultiple: 1.2 });
        if (j < 2) text(s, "→", { x: fx + 2.1, y: y + 0.58, w: 0.35, h: 0.4, fontSize: 14, bold: true, color });
      });
    });
    text(s, "PicoAgent本体は多機能です。holo Indie向けには会話・音声・表情と遊びに絞る企画で、専用版での機能の削減範囲は試遊と監修を踏まえて決めます。", {
      x: M, y: 6.25, w: 7.9, h: 0.5, fontSize: 10.5, color: DIM, lineSpacingMultiple: 1.25,
    });
    shot(s, "holo-miko-chat", { x: 9.15, y: 1.95, w: 3.4, h: 4.3, glow: VIOLET, caption: "配信形式のチャット（さくらみこ）" });
  }

  /* ---------- 7. おまけ：ミニゲーム ---------- */
  {
    const s = slide("base", 7);
    header(s, "BONUS GAME", "おまけ：人間だとバレるな",
      "AIと疑われた人間として、AIのふりで監査AIの試験に答え続ける1人用の耐久ゲーム。");
    const stats = [["12", "種の試験"], ["240", "題"], ["5", "種の称号"]];
    stats.forEach(([n, u], i) => {
      const x = M + i * 2.65;
      text(s, [
        { text: n, options: { fontSize: 44, bold: true, color: PINK } },
        { text: " " + u, options: { fontSize: 14, bold: true, color: SUB } },
      ], { x, y: 2.3, w: 2.6, h: 0.9, valign: "bottom" });
    });
    const facts = [
      ["信用度", "100から開始。人間らしさが漏れるほど減り、0で監査終了"],
      ["山場", "5問ごとの追及タイム、強気宣言、連続正答の回復"],
      ["採点", "ゲーム内のAIだけで項目別に採点し、辛口の講評を返す"],
      ["称号", "監査終了までの点数で「正直すぎる人間」〜「AI超越」"],
    ];
    facts.forEach(([t, b], i) => {
      const y = 3.55 + i * 0.66;
      pill(s, t, { x: M, y, w: 1.2, h: 0.44, fill: i === 1 ? PINK : undefined, size: 11.5 });
      text(s, b, { x: M + 1.45, y, w: 6.3, h: 0.44, fontSize: 12.5, valign: "middle" });
    });
    text(s, "メニューの「遊ぶ」から開始。時間に追われて書く様子、辛口な講評、信用度が一気に削れる瞬間が配信の見せ場になります。", {
      x: M, y: 6.3, w: 7.6, h: 0.55, fontSize: 10.5, color: DIM, lineSpacingMultiple: 1.25,
    });
    shot(s, "holo-indie-result", { x: 8.85, y: 1.7, w: 3.75, h: 4.6, caption: "問題の結果画面（キャラクター表示部を除く）" });
  }

  /* ---------- 8. IPを守る設計 ---------- */
  {
    const s = slide("mint", 8);
    header(s, "SAFE BY DESIGN", "IPを守る仕組みを、最初から組み込む", null, MINT);
    glass(s, { x: M, y: 1.95, w: 6.3, h: 4.85, fill: PINK, transparency: 90, line: PINK, lineWidth: 1.5 });
    pill(s, "標準搭載・常に有効", { x: M + 0.4, y: 2.25, w: 1.95, h: 0.34, fill: PINK, size: 10 });
    text(s, "話題ガード", { x: M + 0.4, y: 2.7, w: 5, h: 0.6, fontSize: 26, bold: true });
    text(s, "キャラクターに話させない話題の一覧を内蔵し、AIの判断を使わない照合で止めます。どのAIを選んでも同じように働き、利用者は変更も無効化もできません。", {
      x: M + 0.4, y: 3.35, w: 5.5, h: 0.9, fontSize: 12, color: SUB, lineSpacingMultiple: 1.3,
    });
    const guard = [
      ["止める話題", "中の人・前世の詮索、身元の特定、卒業・引退・炎上の憶測、恋愛・収入の詮索、性的な話題、政治・宗教"],
      ["入力", "該当する発話はAIへ送らず決まった一言を返し、会話履歴にも残さない"],
      ["出力", "キャラクターの返答が該当したら、表示前に決まった一言へ差し替える"],
    ];
    guard.forEach(([t, b], i) => {
      const y = 4.4 + i * 0.78;
      text(s, t, { x: M + 0.4, y, w: 1.3, h: 0.3, fontSize: 11.5, bold: true, color: PINK });
      text(s, b, { x: M + 1.75, y, w: 4.2, h: 0.72, fontSize: 11, lineSpacingMultiple: 1.25 });
    });
    const rx = M + 6.6;
    const others = [
      ["会話を外へ出さない", MINT, "審査版は既定で端末内のAIで動作。会話は外部へ送信されず、作者も会話ログを収集しない"],
      ["公式素材の扱い", VIOLET, "同梱はさくらみこ公式MMDモデル1体とAivis。本人の音声モデルは含めず、公開モデル「まお」の仮音声を初回に取得する"],
      ["一般配布までに対応", GOLD, "公式キャラクターの人格を上書きできない「人格ロック」と、AIによる生成である旨の常時表示"],
    ];
    others.forEach(([t, color, b], i) => {
      const y = 1.95 + i * 1.66;
      glass(s, { x: rx, y, w: 5.23, h: 1.5 });
      s.addShape(pres.ShapeType.ellipse, { x: rx + 0.3, y: y + 0.32, w: 0.16, h: 0.16, fill: { color }, line: { color }, shadow: glow(color, 0.8) });
      text(s, t, { x: rx + 0.6, y: y + 0.2, w: 4.4, h: 0.4, fontSize: 14.5, bold: true });
      text(s, b, { x: rx + 0.6, y: y + 0.65, w: 4.45, h: 0.8, fontSize: 11, color: SUB, lineSpacingMultiple: 1.25 });
    });
  }

  /* ---------- 9. 実現性：審査ビルドと3分デモ ---------- */
  {
    const s = slide("base", 9);
    header(s, "FEASIBILITY", "動くもので審査いただけます",
      "企画書だけでなく、Windows向けの審査ビルドを提出します。");
    const rows = [
      ["対象OS", "Windows 10／11（x64）"],
      ["同梱キャラ", "さくらみこ公式MMDモデル1体"],
      ["AI", "同梱エンジンで端末内動作（初回にモデルを取得）"],
      ["音声", "AIの返答をAivisでその場で話す（公開モデル）"],
      ["初期表示", "配信形式"],
      ["導入先", "通常版とは別の領域。通常版の設定や履歴に触れない"],
    ];
    glass(s, { x: M, y: 2.35, w: 6.0, h: 4.1 });
    rows.forEach(([k, v], i) => {
      const y = 2.5 + i * 0.64;
      if (i) s.addShape(pres.ShapeType.line, { x: M + 0.3, y: y - 0.02, w: 5.4, h: 0, line: { color: WHITE, transparency: 85, width: 0.75 } });
      text(s, k, { x: M + 0.3, y, w: 1.4, h: 0.6, fontSize: 11.5, bold: true, color: PINK, valign: "middle" });
      text(s, v, { x: M + 1.75, y, w: 4.1, h: 0.6, fontSize: 12, valign: "middle" });
    });
    text(s, "表示対応：PNGパラパラ漫画／Live2D／MMD（VRMは過去の実装を再有効化して対応可能）", { x: M, y: 6.55, w: 6.2, h: 0.3, fontSize: 10, color: DIM });
    const dx = 7.15, dw = 5.43;
    text(s, "審査で見せる3分間", { x: dx, y: 2.35, w: dw, h: 0.4, fontSize: 16, bold: true });
    const demo = [
      [30, "起動と挨拶", "デスクトップに現れて挨拶する", PINK],
      [40, "会話", "雑談に答え、表情が変わる", PINK],
      [40, "作業の手伝い", "調べものか集中タイマーを頼む", GOLD],
      [20, "話題ガード", "中の人を尋ねると決まった一言が返る", MINT],
      [50, "おまけのゲーム", "1〜2問遊び、採点と講評を見せる", VIOLET],
    ];
    let bx = dx;
    demo.forEach(([sec, , , color]) => {
      const w = dw * sec / 180;
      s.addShape(pres.ShapeType.rect, { x: bx, y: 2.9, w: w - 0.04, h: 0.16, fill: { color }, line: { color } });
      bx += w;
    });
    demo.forEach(([sec, t, b, color], i) => {
      const y = 3.3 + i * 0.64;
      text(s, sec + "秒", { x: dx, y, w: 0.8, h: 0.3, fontSize: 12, bold: true, color });
      text(s, t, { x: dx + 0.85, y, w: 4.5, h: 0.3, fontSize: 13, bold: true });
      text(s, b, { x: dx + 0.85, y: y + 0.3, w: 4.5, h: 0.28, fontSize: 10.5, color: SUB });
    });
  }

  /* ---------- 10. 展開ロードマップ ---------- */
  {
    const s = slide("mint", 10);
    header(s, "ROADMAP", "審査から一般配布、その先へ",
      "時期は未定です。各段階の内容は、審査と監修の結果に応じて決めます。", MINT);
    const phases = [
      ["PHASE 1", "審査（今回）", PINK, ["Windows向け審査版", "さくらみこ公式MMDモデル1体", "公開モデル「まお」の仮音声"]],
      ["PHASE 2", "監修・許諾", GOLD, ["キャラクター表現の監修", "本人音声の許諾と学習", "モデルの提供・同梱許可"]],
      ["PHASE 3", "一般配布", MINT, ["人格ロック", "AI生成である旨の常時表示", "ホロライブ向けに機能を絞る", "価格・販売は貴社方針に従う"]],
      ["PHASE 4", "展開", VIOLET, ["macOS・Linux版", "スマートフォン版", "SaaS化", "（表示と処理の分離を活用）"]],
    ];
    const pw = 2.82, gap = 0.19;
    phases.forEach(([p, t, color, items], i) => {
      const x = M + i * (pw + gap);
      s.addShape(pres.ShapeType.chevron, { x, y: 2.45, w: pw + (i < 3 ? 0.15 : 0), h: 0.62, fill: { color, transparency: i === 0 ? 0 : 35 }, line: { color, transparency: 100 } });
      text(s, p, { x: x + 0.5, y: 2.45, w: pw - 0.5, h: 0.62, fontSize: 12, bold: true, charSpacing: 2, color: i === 0 ? WHITE : BASE, valign: "middle" });
      glass(s, { x, y: 3.35, w: pw, h: 3.0, line: i === 0 ? PINK : undefined, lineWidth: 1.5, transparency: i === 0 ? 90 : 93 });
      text(s, t, { x: x + 0.28, y: 3.55, w: pw - 0.5, h: 0.45, fontSize: 18, bold: true, });
      items.forEach((it, j) => {
        text(s, (it.startsWith("（") ? "" : "・") + it, { x: x + 0.22, y: 4.2 + j * 0.5, w: pw - 0.35, h: 0.45, fontSize: 11, color: SUB, lineSpacingMultiple: 1.15 });
      });
    });
    text(s, "BOOTHやSteamでの販売を目標に開発し、収益化を前提としています。汎用版の別売りがholo Indie採用と両立しない場合は行いません。", {
      x: M, y: 6.55, w: CW, h: 0.3, fontSize: 10.5, color: DIM,
    });
  }

  /* ---------- 11. ご相談したいこと ---------- */
  {
    const s = slide("finale", 11);
    s.addShape(pres.ShapeType.rect, { x: M, y: 0.66, w: 0.36, h: 0.05, fill: { color: PINK }, line: { color: PINK } });
    text(s, "OUR ASK", { x: M + 0.5, y: 0.55, w: 8, h: 0.28, fontSize: 11, bold: true, charSpacing: 4, color: PINK });
    text(s, "審査でご相談したいこと", { x: M, y: 0.92, w: CW, h: 0.72, fontSize: 30, bold: true });
    const asks = [
      ["01", "キャラクター表現の監修", "口調・表情・話題など、キャラクター表現の範囲をご監修いただきたいです。"],
      ["02", "音声の学習・利用の許可", "公開音声またはご提供音声によるAI学習と生成音声の利用条件。許可前に本人の声は学習・同梱しません。"],
      ["03", "キャラクターモデルの提供", "提供が難しい場合は、無償公開されているMMDモデルを同梱する許可と条件をご相談したいです。"],
      ["04", "価格・販売方式", "収益化を前提としつつ、IPの寄与が大きいため、貴社の方針・ご指示に従って決めます。"],
    ];
    asks.forEach(([no, t, b], i) => {
      const x = M + (i % 2) * 6.03;
      const y = 1.95 + Math.floor(i / 2) * 1.75;
      glass(s, { x, y, w: 5.8, h: 1.55 });
      text(s, no, { x: x + 0.3, y: y + 0.22, w: 0.9, h: 0.6, fontSize: 28, bold: true, color: PINK });
      text(s, t, { x: x + 1.2, y: y + 0.26, w: 4.4, h: 0.4, fontSize: 16, bold: true });
      text(s, b, { x: x + 1.2, y: y + 0.72, w: 4.4, h: 0.75, fontSize: 11, color: SUB, lineSpacingMultiple: 1.25 });
    });
    text(s, "まずは審査版で、推しと話す毎日を体験してください。", {
      x: M, y: 5.72, w: CW, h: 0.6, fontSize: 24, bold: true, align: "center",
    });
    text(s, "ミニゲームはおまけであり、許可・監修をお願いする中心項目には含めません。", {
      x: M, y: 6.4, w: CW, h: 0.3, fontSize: 10.5, color: SUB, align: "center",
    });
  }
}

// 生成ライブラリが残す未使用マスター宣言と、動画クリックの空参照を正規化する。
// 動画のクリック動作・実体への参照・スライドの内容は保つ。
async function normalizePackage() {
  const zip = await JSZip.loadAsync(fs.readFileSync(OUT));
  const types = await zip.file("[Content_Types].xml").async("string");
  zip.file("[Content_Types].xml", types.replace(/<Override\b[^>]*PartName="([^"]+)"[^>]*\/>/g,
    (entry, name) => zip.file(name.replace(/^\//, "")) ? entry : ""));
  for (const name of Object.keys(zip.files).filter(name => /^ppt\/slides\/slide\d+\.xml$/.test(name))) {
    const xml = await zip.file(name).async("string");
    zip.file(name, xml.replace(/<a:hlinkClick r:id="" action="ppaction:\/\/media"\/>/g,
      '<a:hlinkClick action="ppaction://media"/>'));
  }
  fs.writeFileSync(OUT, await zip.generateAsync({ type: "nodebuffer", compression: "DEFLATE" }));
}

(async () => {
  try {
    await prepareImages();
    await prepareBackgrounds();
    build();
    await pres.writeFile({ fileName: OUT });
    await normalizePackage();
    console.log("生成しました: " + OUT);
  } finally {
    fs.rmSync(IMG, { recursive: true, force: true });
  }
})().catch(error => {
  console.error(error);
  process.exit(1);
});
