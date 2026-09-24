// docs/HOLO-INDIE-PLAN.md の内容から、holo Indie へ応募する作品（PicoAgent）の企画書PowerPointを生成する。
// 実行: node tools/holo-indie-deck/build.js
// 前提: pptxgenjs と sharp（アプリのビルドには不要な作業用ツール）。ミニゲームの画面は capture.mjs で撮る。
// 文面の正本は docs/HOLO-INDIE-PLAN.md（おまけのミニゲームは docs/HOLO-INDIE-MINIGAME.md）。
// 先に正本を直してからこのスクリプトへ反映する。
const fs = require("fs");
const os = require("os");
const path = require("path");
const pptxgen = require("pptxgenjs");
const sharp = require("sharp");

const ROOT = path.resolve(__dirname, "..", "..");
const SRC_IMG = path.join(ROOT, "docs", "images");
const OUT = path.join(ROOT, "docs", "HOLO-INDIE-PLAN.pptx");

// pptxgenjs は WebP を埋め込めないため、掲載する画像だけ PNG へ変換して一時領域へ置く。
const IMG = fs.mkdtempSync(path.join(os.tmpdir(), "holo-indie-deck-"));
// 本体の画面は自社キャラクター「むぎ」の表示例。公式キャラクターの素材はリポジトリに無く、企画書にも入れない。
const USED = ["layout-shot-stream", "layout-shot-pet", "layout-shot-chat", "holo-indie-result"];

async function prepareImages() {
  for (const name of USED) {
    const from = path.join(SRC_IMG, name + ".webp");
    if (!fs.existsSync(from)) throw new Error("画像が見つかりません（holo-indie-* は capture.mjs で撮影してください）: " + from);
    const head = fs.readFileSync(from, { encoding: "utf8", flag: "r" }).slice(0, 20);
    if (head.startsWith("version https")) {
      throw new Error("Git LFS のポインタのままです。git lfs pull を先に実行してください: " + from);
    }
    await sharp(from).png().toFile(path.join(IMG, name + ".png"));
  }
}

// ゲーム画面と同じ、夜の紫・ピンク・ミントの配色。
const NIGHT = "241C33";
const NIGHT2 = "33274A";
const PINK = "E55E87";
const PINK_SOFT = "FDEBF1";
const MINT = "2FB99C";
const MINT_SOFT = "E3F6F1";
const GOLD = "E0A92E";
const GOLD_SOFT = "FBF1DA";
const RED = "D9485F";
const WHITE = "FFFFFF";
const ALT = "F8F6FB";
const TEXT = "231C30";
const MUTED = "6E6680";
const LINE = "E6E0EE";
const NIGHT_MUTED = "C9BFDA";

const JP = "Meiryo";
const W = 13.3;
const M = 0.7;
const CW = W - M * 2;

const pres = new pptxgen();
pres.layout = "LAYOUT_WIDE";
pres.author = "PicoAgent";
pres.title = "PicoAgent holo Indie 応募企画書";

const shadow = () => ({ type: "outer", color: "1A1030", opacity: 0.1, blur: 12, offset: 3, angle: 90 });

function nightBg(s) {
  s.background = { color: NIGHT };
  s.addShape(pres.ShapeType.ellipse, {
    x: W - 3.2, y: -2.4, w: 6, h: 6, fill: { color: NIGHT2 }, line: { color: NIGHT2 },
  });
}

function title(s, kicker, text) {
  s.addText(kicker, {
    x: M, y: 0.5, w: CW, h: 0.26, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 12, bold: true, charSpacing: 2, color: PINK,
  });
  s.addText(text, {
    x: M, y: 0.8, w: CW, h: 0.6, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 28, bold: true, color: TEXT,
  });
}

function pageNo(s, n, dark) {
  s.addText(String(n), {
    x: W - 1.0, y: 7.0, w: 0.4, h: 0.28, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 10, color: dark ? NIGHT_MUTED : MUTED, align: "right",
  });
}

function card(s, o) {
  s.addShape(pres.ShapeType.roundRect, {
    x: o.x, y: o.y, w: o.w, h: o.h, rectRadius: 0.1,
    fill: { color: o.fill || WHITE }, line: { color: o.line || LINE, width: 1 },
    shadow: o.shadow === false ? undefined : shadow(),
  });
}

function badge(s, o) {
  s.addShape(pres.ShapeType.ellipse, {
    x: o.x, y: o.y, w: o.d, h: o.d, fill: { color: o.fill || PINK }, line: { color: o.fill || PINK },
  });
  s.addText(o.label, {
    x: o.x, y: o.y, w: o.d, h: o.d, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: o.size || 14, bold: true, color: o.color || WHITE, align: "center", valign: "middle",
  });
}

// 画面写真。縦長の画面を、角丸の枠に収めて置く（縦横比は画像に合わせて呼び出し側で決める）。
function shot(s, name, o) {
  s.addShape(pres.ShapeType.roundRect, {
    x: o.x - 0.06, y: o.y - 0.06, w: o.w + 0.12, h: o.h + 0.12, rectRadius: 0.12,
    fill: { color: NIGHT }, line: { color: NIGHT }, shadow: shadow(),
  });
  s.addImage({ path: path.join(IMG, name + ".png"), x: o.x, y: o.y, w: o.w, h: o.h });
  if (o.caption) {
    s.addText(o.caption, {
      x: o.x - 0.2, y: o.y + o.h + 0.12, w: o.w + 0.4, h: 0.26, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 10, color: o.dark ? NIGHT_MUTED : MUTED, align: "center",
    });
  }
}

function build() {
  const SAMPLE = "画面は自社キャラクター「むぎ」での表示例。審査版ではさくらみこ公式MMDモデルを表示します";

  /* ---------- 1. 表紙 ---------- */
  {
    const s = pres.addSlide();
    nightBg(s);
    s.addText("holo Indie 応募企画書", {
      x: M, y: 1.25, w: 6.6, h: 0.3, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 14, bold: true, charSpacing: 2, color: MINT,
    });
    s.addText("PicoAgent", {
      x: M, y: 1.7, w: 6.6, h: 0.95, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 48, bold: true, color: WHITE,
    });
    s.addText("ホロライブのキャラクターが、デスクトップに常駐して、\n話し相手と作業の相棒になる。", {
      x: M, y: 2.85, w: 6.6, h: 0.95, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 18, color: NIGHT_MUTED, lineSpacing: 30,
    });
    const tags = ["デスクトップ常駐AI", "端末内で動くAI", "おまけミニゲームつき"];
    let tx = M;
    tags.forEach(t => {
      const w = 0.34 + t.length * 0.19;
      s.addShape(pres.ShapeType.roundRect, {
        x: tx, y: 4.15, w, h: 0.42, rectRadius: 0.21, fill: { color: NIGHT2 }, line: { color: "4A3C66" },
      });
      s.addText(t, {
        x: tx, y: 4.15, w, h: 0.42, isTextBox: true, margin: 0,
        fontFace: JP, fontSize: 12, bold: true, color: WHITE, align: "center", valign: "middle",
      });
      tx += w + 0.16;
    });
    s.addText(
      [
        { text: "対象：Windows 10／11", options: { breakLine: true } },
        { text: "応募者：【氏名／屋号】", options: { breakLine: true } },
        { text: "提出日：【日付】" },
      ],
      { x: M, y: 5.35, w: 6, h: 1.0, isTextBox: true, margin: 0, fontFace: JP, fontSize: 13, color: NIGHT_MUTED, lineSpacing: 22 }
    );
    shot(s, "layout-shot-stream", { x: 7.55, y: 2.0, w: 5.05, h: 2.84, caption: SAMPLE, dark: true });
  }

  /* ---------- 2. 企画の要点 ---------- */
  {
    const s = pres.addSlide();
    s.background = { color: WHITE };
    title(s, "CONCEPT", "推しが、いつもデスクトップにいる");
    s.addText("PCで作業している間ずっとそばにいて、話しかければ答え、頼めば手伝ってくれるキャラクターです。", {
      x: M, y: 1.52, w: CW, h: 0.32, isTextBox: true, margin: 0, fontFace: JP, fontSize: 14, color: MUTED,
    });
    const items = [
      { icon: "居", color: PINK, t: "いつも画面にいる", b: "透明なウィンドウでデスクトップに常駐。作業の邪魔をしない大きさから配信用の表示まで選べる" },
      { icon: "話", color: RED, t: "キャラクターとして答える", b: "名前・一人称・口調・性格を持ち、話の流れから推定した感情で表情が変わる" },
      { icon: "助", color: GOLD, t: "頼むと手伝う", b: "調べもの、文書やファイルの整理、リマインダーや集中タイマーを会話から頼める" },
      { icon: "端", color: MINT, t: "AIは端末の中", b: "推論エンジンを同梱し、会話を外部へ送らない。作者も会話ログを収集しない" },
      { icon: "遊", color: "7A63C9", t: "おまけのゲーム", b: "AIのふりをして監査AIの試験を耐え抜くミニゲームを「遊ぶ」から遊べる" },
    ];
    const cw = 2.24, gap = 0.21;
    items.forEach((it, i) => {
      const x = M + i * (cw + gap);
      card(s, { x, y: 2.2, w: cw, h: 3.55 });
      badge(s, { x: x + 0.25, y: 2.45, d: 0.62, label: it.icon, fill: it.color, size: 18 });
      s.addText(it.t, {
        x: x + 0.25, y: 3.27, w: cw - 0.5, h: 0.66, isTextBox: true, margin: 0,
        fontFace: JP, fontSize: 15, bold: true, color: TEXT, valign: "top",
      });
      s.addText(it.b, {
        x: x + 0.25, y: 4.0, w: cw - 0.5, h: 1.6, isTextBox: true, margin: 0,
        fontFace: JP, fontSize: 11.5, color: MUTED, lineSpacing: 19, valign: "top",
      });
    });
    s.addText(
      "holo Indieのガイドラインは、入力に応じて表示・挙動が変わるアプリケーションも「デジタルゲーム」に含めています。会話の入力でキャラクターの返答・表情・行動が変わるPicoAgentは、この定義に当てはまると考えています。",
      { x: M, y: 6.05, w: CW, h: 0.65, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11.5, color: MUTED, lineSpacing: 19, valign: "top" }
    );
    pageNo(s, 2);
  }

  /* ---------- 3. 1日の流れ ---------- */
  {
    const s = pres.addSlide();
    s.background = { color: ALT };
    title(s, "EXPERIENCE", "キャラクターと過ごす1日");
    const steps = [
      ["起動", "PCを起動すると、キャラクターがデスクトップに現れて挨拶する"],
      ["会話", "作業の合間に話しかける。雑談にも、キャラクターとして答える"],
      ["手伝い", "「この資料を要約して」「25分集中タイマー」のように頼む"],
      ["息抜き", "メニューの「遊ぶ」から、おまけのミニゲームを遊ぶ"],
      ["配信", "配信用の表示に切り替え、会話する様子をそのまま映す"],
    ];
    steps.forEach(([t, b], i) => {
      const y = 1.75 + i * 1.02;
      badge(s, { x: M, y: y + 0.08, d: 0.56, label: String(i + 1), fill: PINK, size: 16 });
      s.addText(t, {
        x: M + 0.78, y, w: 5.6, h: 0.36, isTextBox: true, margin: 0, fontFace: JP, fontSize: 16, bold: true, color: TEXT,
      });
      s.addText(b, {
        x: M + 0.78, y: y + 0.38, w: 5.6, h: 0.52, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12, color: MUTED, lineSpacing: 18, valign: "top",
      });
    });
    shot(s, "layout-shot-pet", { x: 7.35, y: 1.9, w: 5.2, h: 3.47, caption: SAMPLE });
    s.addText("ペット形式：左にフキダシ、右にキャラクター。いちばん普段づかいの表示です。", {
      x: 7.35, y: 5.95, w: 5.2, h: 0.3, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11, color: MUTED,
    });
    pageNo(s, 3);
  }

  /* ---------- 4. キャラクターの表現 ---------- */
  {
    const s = pres.addSlide();
    s.background = { color: WHITE };
    title(s, "CHARACTER", "キャラクターの表現");
    const cards = [
      { icon: "動", color: PINK, fill: PINK_SOFT, t: "動き", b: "審査版のMMDモデルは、自然な待機姿勢、上半身と首の控えめな待機動作、まばたき、表情の切り替えを端末内で描画します。元のモデルデータは加工しません。" },
      { icon: "情", color: GOLD, fill: GOLD_SOFT, t: "感情", b: "話の流れから推定した感情（うれしい・悲しい・考え中など）に合わせて表情を変えます。" },
      { icon: "映", color: MINT, fill: MINT_SOFT, t: "表示形式", b: "7種類を再起動なしで切り替えられます。フルHDの配信用表示では、背景つきの画面にキャラクターと会話を並べます。" },
    ];
    cards.forEach((it, i) => {
      const y = 1.72 + i * 1.62;
      card(s, { x: M, y, w: 6.2, h: 1.42, fill: it.fill, line: it.fill });
      badge(s, { x: M + 0.28, y: y + 0.28, d: 0.62, label: it.icon, fill: it.color, size: 18 });
      s.addText(it.t, {
        x: M + 1.15, y: y + 0.2, w: 4.8, h: 0.36, isTextBox: true, margin: 0, fontFace: JP, fontSize: 16, bold: true, color: TEXT,
      });
      s.addText(it.b, {
        x: M + 1.15, y: y + 0.6, w: 4.85, h: 0.76, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11.5, color: TEXT, lineSpacing: 17, valign: "top",
      });
    });
    const layouts = ["ミニ", "ペット", "チャット", "ウィジェット", "バー", "作業", "配信"];
    const lx = 7.3;
    s.addText("7つの表示形式", {
      x: lx, y: 1.75, w: 5.2, h: 0.34, isTextBox: true, margin: 0, fontFace: JP, fontSize: 15, bold: true, color: TEXT,
    });
    layouts.forEach((t, i) => {
      const x = lx + (i % 4) * 1.32;
      const y = 2.25 + Math.floor(i / 4) * 0.56;
      s.addShape(pres.ShapeType.roundRect, {
        x, y, w: 1.2, h: 0.42, rectRadius: 0.21, fill: { color: t === "配信" ? PINK : ALT }, line: { color: t === "配信" ? PINK : LINE },
      });
      s.addText(t, {
        x, y, w: 1.2, h: 0.42, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12, bold: true,
        color: t === "配信" ? WHITE : TEXT, align: "center", valign: "middle",
      });
    });
    shot(s, "layout-shot-stream", { x: 7.35, y: 3.62, w: 5.15, h: 2.9, caption: "配信用の表示（" + SAMPLE.replace("画面は", "") + "）" });
    pageNo(s, 4);
  }

  /* ---------- 5. 会話から頼めること ---------- */
  {
    const s = pres.addSlide();
    s.background = { color: ALT };
    title(s, "ASSIST", "会話から頼めること（一部）");
    const genres = [
      ["調", "調べもの", "Web検索して、参照元つきで答える"],
      ["文", "文書・ファイル", "要約、PDFの抜き出し、ファイル名の整理"],
      ["時", "日常の補助", "リマインダー、25分の集中タイマー、定期的な声かけ"],
      ["録", "記録", "会話や完了した作業を端末内に記録し、「続きをやる」で依頼を戻す"],
    ];
    genres.forEach(([icon, t, b], i) => {
      const x = M + (i % 2) * 3.55;
      const y = 1.75 + Math.floor(i / 2) * 2.1;
      card(s, { x, y, w: 3.35, h: 1.95 });
      badge(s, { x: x + 0.28, y: y + 0.28, d: 0.58, label: icon, fill: i % 2 ? MINT : PINK, size: 17 });
      s.addText(t, {
        x: x + 0.28, y: y + 0.94, w: 2.8, h: 0.34, isTextBox: true, margin: 0, fontFace: JP, fontSize: 15, bold: true, color: TEXT,
      });
      s.addText(b, {
        x: x + 0.28, y: y + 1.3, w: 2.85, h: 0.52, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11, color: MUTED, lineSpacing: 15, valign: "top",
      });
    });
    s.addText("PC操作や外部サービスとの連携は、安全のため既定では無効です。", {
      x: M, y: 6.1, w: 6.9, h: 0.3, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11, color: MUTED,
    });
    shot(s, "layout-shot-chat", { x: 8.65, y: 1.25, w: 3.4, h: 4.97, caption: "チャット形式（審査版の初期表示。自社キャラでの表示例）" });
    pageNo(s, 5);
  }

  /* ---------- 6. おまけ：ミニゲーム ---------- */
  {
    const s = pres.addSlide();
    s.background = { color: WHITE };
    title(s, "BONUS", "おまけ：ミニゲーム「人間だとバレるな」");
    s.addText("AIではないかと疑われた人間として、AIのふりで監査AIの試験に答え続ける1人用の耐久ゲーム。メニューの「遊ぶ」から始めます。", {
      x: M, y: 1.52, w: 7.3, h: 0.62, isTextBox: true, margin: 0, fontFace: JP, fontSize: 13, color: MUTED, lineSpacing: 20, valign: "top",
    });
    const facts = [
      ["信用度", "100から開始し、人間らしさが漏れるほど減る。0で監査終了"],
      ["試験", "言い訳、謝罪会見、手順書など12種（計240題）"],
      ["山場", "5問ごとの追及タイム、強気宣言、連続正答の回復"],
      ["採点", "ゲーム内のAIだけで項目別に採点し、講評を返す"],
      ["称号", "監査終了までの点数で「正直すぎる人間」〜「AI超越」の5種"],
    ];
    facts.forEach(([t, b], i) => {
      const y = 2.4 + i * 0.8;
      s.addShape(pres.ShapeType.roundRect, {
        x: M, y, w: 1.35, h: 0.5, rectRadius: 0.25, fill: { color: i === 2 ? RED : PINK_SOFT }, line: { color: i === 2 ? RED : "F6D3DF" },
      });
      s.addText(t, {
        x: M, y, w: 1.35, h: 0.5, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12.5, bold: true,
        color: i === 2 ? WHITE : PINK, align: "center", valign: "middle",
      });
      s.addText(b, {
        x: M + 1.6, y, w: 5.8, h: 0.5, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12.5, color: TEXT, valign: "middle",
      });
    });
    s.addText("配信では、時間に追われて書く様子、辛口な講評、信用度が一気に削れる瞬間が見せ場になります。設計の詳細は別紙（HOLO-INDIE-MINIGAME.md）にまとめています。", {
      x: M, y: 6.45, w: 7.3, h: 0.5, isTextBox: true, margin: 0, fontFace: JP, fontSize: 10.5, color: MUTED, lineSpacing: 16, valign: "top",
    });
    shot(s, "holo-indie-result", { x: 8.75, y: 1.2, w: 3.8, h: 4.18, caption: "問題の結果画面（キャラクター表示部を除く）" });
    pageNo(s, 6);
  }

  /* ---------- 7. 安全とガイドラインへの配慮 ---------- */
  {
    const s = pres.addSlide();
    s.background = { color: ALT };
    title(s, "SAFETY", "安全とガイドラインへの配慮");
    card(s, { x: M, y: 1.7, w: 6.1, h: 5.05, fill: NIGHT, line: NIGHT });
    badge(s, { x: M + 0.35, y: 1.98, d: 0.62, label: "守", fill: PINK, size: 18 });
    s.addText("話題ガード（標準搭載・常に有効）", {
      x: M + 1.15, y: 2.02, w: 4.8, h: 0.5, isTextBox: true, margin: 0, fontFace: JP, fontSize: 16, bold: true, color: WHITE, valign: "middle",
    });
    s.addText("キャラクターに話させない話題の一覧を内蔵し、AIの判断を使わない照合で止めます。どのAIを選んでも同じように働き、利用者は変更も無効化もできません。", {
      x: M + 0.35, y: 2.75, w: 5.4, h: 0.9, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12, color: NIGHT_MUTED, lineSpacing: 19, valign: "top",
    });
    const guard = [
      ["止める話題", "中の人・前世の詮索、身元の特定、卒業・引退・炎上の憶測、恋愛・収入の詮索、性的な話題、政治・宗教"],
      ["入力", "該当する発話はAIへ送らず決まった一言を返し、会話履歴にも残さない"],
      ["出力", "キャラクターの返答が該当したら、表示前に決まった一言へ差し替える"],
    ];
    guard.forEach(([t, b], i) => {
      const y = 3.8 + i * 0.95;
      s.addText(t, {
        x: M + 0.35, y, w: 1.3, h: 0.3, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12, bold: true, color: MINT,
      });
      s.addText(b, {
        x: M + 1.65, y, w: 4.1, h: 0.82, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11, color: WHITE, lineSpacing: 16, valign: "top",
      });
    });
    const rx = M + 6.45;
    const others = [
      ["秘", "会話を外へ出さない", "審査版は既定で端末内のAIで動作。作者は会話ログを収集しない"],
      ["公", "公式素材の扱い", "同梱は出所と利用条件を確認したさくらみこ公式MMDモデル1体のみ。音声モデルは同梱せず、読み上げは無効"],
      ["次", "一般配布までに対応", "公式キャラクターの人格を上書きできない「人格ロック」と、AIによる生成である旨の常時表示"],
    ];
    others.forEach(([icon, t, b], i) => {
      const y = 1.7 + i * 1.72;
      card(s, { x: rx, y, w: 5.45, h: 1.5 });
      badge(s, { x: rx + 0.28, y: y + 0.28, d: 0.56, label: icon, fill: i === 2 ? GOLD : "7A63C9", size: 16 });
      s.addText(t, {
        x: rx + 1.05, y: y + 0.26, w: 4.2, h: 0.34, isTextBox: true, margin: 0, fontFace: JP, fontSize: 14.5, bold: true, color: TEXT,
      });
      s.addText(b, {
        x: rx + 1.05, y: y + 0.66, w: 4.2, h: 0.74, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11, color: MUTED, lineSpacing: 16, valign: "top",
      });
    });
    pageNo(s, 7);
  }

  /* ---------- 8. 審査ビルドと審査デモ ---------- */
  {
    const s = pres.addSlide();
    s.background = { color: WHITE };
    title(s, "REVIEW BUILD", "審査ビルドと、審査で見せる3分間");
    const rows = [
      ["対象OS", "Windows 10／11（x64）"],
      ["キャラクター", "さくらみこ公式MMDモデル1体のみ"],
      ["AI", "同梱エンジンで端末内動作（初回にモデルを取得）"],
      ["音声", "なし（字幕表示のみ）"],
      ["初期表示", "チャット形式"],
      ["導入先", "通常版とは別の領域。通常版の設定や履歴に触れない"],
    ];
    rows.forEach(([k, v], i) => {
      const y = 1.75 + i * 0.78;
      s.addShape(pres.ShapeType.roundRect, {
        x: M, y, w: 6.2, h: 0.64, rectRadius: 0.08, fill: { color: i % 2 ? WHITE : ALT }, line: { color: LINE },
      });
      s.addText(k, {
        x: M + 0.25, y, w: 1.6, h: 0.64, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12.5, bold: true, color: PINK, valign: "middle",
      });
      s.addText(v, {
        x: M + 1.9, y, w: 4.15, h: 0.64, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12, color: TEXT, valign: "middle",
      });
    });
    const demo = [
      ["30秒", "起動と挨拶", "キャラクターがデスクトップに現れて挨拶する"],
      ["40秒", "会話", "雑談に答え、表情が変わる様子を見せる"],
      ["40秒", "作業の手伝い", "調べものか集中タイマーを頼む"],
      ["20秒", "話題ガード", "中の人を尋ねると、AIへ送らずに決まった一言が返る"],
      ["50秒", "おまけのゲーム", "「遊ぶ」から1〜2問遊び、採点と講評を見せる"],
    ];
    const dx = 7.3;
    demo.forEach(([sec, t, b], i) => {
      const y = 1.75 + i * 0.97;
      card(s, { x: dx, y, w: 5.3, h: 0.84, fill: i === 3 ? "FBE7EA" : ALT, line: LINE, shadow: false });
      s.addText(sec, {
        x: dx + 0.2, y: y + 0.12, w: 0.8, h: 0.3, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11.5, bold: true, color: i === 3 ? RED : PINK,
      });
      s.addText(t, {
        x: dx + 1.05, y: y + 0.1, w: 4.1, h: 0.32, isTextBox: true, margin: 0, fontFace: JP, fontSize: 13.5, bold: true, color: TEXT,
      });
      s.addText(b, {
        x: dx + 1.05, y: y + 0.46, w: 4.1, h: 0.3, isTextBox: true, margin: 0, fontFace: JP, fontSize: 10.5, color: MUTED,
      });
    });
    pageNo(s, 8);
  }

  /* ---------- 9. 今後 ---------- */
  {
    const s = pres.addSlide();
    nightBg(s);
    s.addText("ROADMAP", {
      x: M, y: 0.5, w: CW, h: 0.26, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12, bold: true, charSpacing: 2, color: MINT,
    });
    s.addText("今後の予定", {
      x: M, y: 0.8, w: CW, h: 0.6, isTextBox: true, margin: 0, fontFace: JP, fontSize: 28, bold: true, color: WHITE,
    });
    const next = [
      ["鍵", "人格ロック", "公式キャラクターの人格・口調を、利用者が上書きできなくする"],
      ["表", "AI生成である旨の常時表示", "「AIによる生成であり、ご本人の発言ではない」旨をアプリ内に常に表示する"],
      ["顔", "キャラクターの反応を増やす", "会話やゲームの展開に応じた表情・ポーズ・固定の一言を、監修の範囲内で追加する"],
      ["声", "音声での会話を実現したい", "存在感を高める主要機能。許諾が得られればYouTube音声のAI学習も候補。利用・配布条件を協議。審査版は字幕のみ"],
    ];
    next.forEach(([icon, t, b], i) => {
      const x = M + (i % 2) * 6.05;
      const y = 1.85 + Math.floor(i / 2) * 2.2;
      card(s, { x, y, w: 5.8, h: 1.95, fill: NIGHT2, line: "4A3C66", shadow: false });
      badge(s, { x: x + 0.3, y: y + 0.3, d: 0.62, label: icon, fill: i < 2 ? PINK : MINT, size: 18 });
      s.addText(t, {
        x: x + 1.15, y: y + 0.3, w: 4.4, h: 0.62, isTextBox: true, margin: 0, fontFace: JP, fontSize: 16, bold: true, color: WHITE, valign: "middle",
      });
      s.addText(b, {
        x: x + 0.3, y: y + 1.08, w: 5.2, h: 0.75, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12, color: NIGHT_MUTED, lineSpacing: 18, valign: "top",
      });
    });
    s.addText("人格ロックとAI生成の常時表示は、審査側の要件を受けて一般配布前に実装します。", {
      x: M, y: 6.4, w: CW, h: 0.3, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12, color: NIGHT_MUTED,
    });
    s.addText("収益化を目指します。本体の有償・無償、課金対象、価格、販売方式は未定です。", {
      x: M, y: 6.8, w: CW, h: 0.3, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12, color: NIGHT_MUTED,
    });
    pageNo(s, 9, true);
  }
}

(async () => {
  try {
    await prepareImages();
    build();
    await pres.writeFile({ fileName: OUT });
    console.log("生成しました: " + OUT);
  } finally {
    fs.rmSync(IMG, { recursive: true, force: true });
  }
})().catch(error => {
  console.error(error);
  process.exit(1);
});
