// docs/IP-PROPOSAL-VTUBER.md の内容から、VTuber事務所へ送付する提案書PowerPointを生成する。
// 実行: node tools/ip-proposal-deck/build.js
// 前提: npm i -g pptxgenjs sharp （アプリのビルドには不要な作業用ツール）
// 文面の正本は docs/IP-PROPOSAL-VTUBER.md。先に正本を直してからこのスクリプトへ反映する。
const fs = require("fs");
const os = require("os");
const path = require("path");
const pptxgen = require("pptxgenjs");
const sharp = require("sharp");

const ROOT = path.resolve(__dirname, "..", "..");
const SRC_IMG = path.join(ROOT, "docs", "images");
const OUT = path.join(ROOT, "docs", "IP-PROPOSAL-VTUBER.pptx");

// pptxgenjs は WebP を埋め込めないため、掲載する画像だけ PNG へ変換して一時領域へ置く。
const IMG = fs.mkdtempSync(path.join(os.tmpdir(), "picoagent-deck-"));
const USED = ["layout-shot-stream", "layout-shot-workspace", "layout-shot-pet"];

async function prepareImages() {
  for (const name of USED) {
    const from = path.join(SRC_IMG, name + ".webp");
    if (!fs.existsSync(from)) throw new Error("画像が見つかりません: " + from);
    const head = fs.readFileSync(from, { encoding: "utf8", flag: "r" }).slice(0, 20);
    if (head.startsWith("version https")) {
      throw new Error("Git LFS のポインタのままです。git lfs pull を先に実行してください: " + from);
    }
    await sharp(from).png().toFile(path.join(IMG, name + ".png"));
  }
}

const INK = "191633";
const INK2 = "272150";
const BRAND = "5B4BD6";
const BRAND_SOFT = "EFECFD";
const MINT = "00A88F";
const WHITE = "FFFFFF";
const ALT = "F6F5FB";
const TEXT = "221F33";
const MUTED = "6B6780";
const LINE = "E2DEF2";
const INK_MUTED = "B9B4D6";

const JP = "Meiryo";
const W = 13.3;
const M = 0.75;
const CW = W - M * 2;

const pres = new pptxgen();
pres.layout = "LAYOUT_WIDE";
pres.author = "PicoAgent";
pres.title = "PicoAgent 公式キャラクターパック 実装のご提案";

const shadow = () => ({ type: "outer", color: "1A1440", opacity: 0.1, blur: 14, offset: 3, angle: 90 });

function darkBg(s, cy) {
  s.background = { color: INK };
  s.addShape(pres.ShapeType.ellipse, {
    x: W - 2.7, y: cy === undefined ? -2.6 : cy, w: 5.6, h: 5.6,
    fill: { color: INK2 }, line: { color: INK2 },
  });
}

function title(s, kicker, text, dark) {
  s.addText(kicker, {
    x: M, y: 0.6, w: CW, h: 0.26, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 12, bold: true, charSpacing: 2,
    color: dark ? MINT : BRAND,
  });
  s.addText(text, {
    x: M, y: 0.92, w: CW, h: 0.6, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 29, bold: true, color: dark ? WHITE : TEXT,
  });
}

function lead(s, text, dark) {
  s.addText(text, {
    x: M, y: 1.68, w: CW, h: 0.32, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 13.5, color: dark ? INK_MUTED : MUTED,
  });
}

function pageNo(s, n, dark) {
  s.addText(String(n), {
    x: W - 1.05, y: 6.98, w: 0.4, h: 0.28, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 10, color: dark ? INK_MUTED : MUTED, align: "right",
  });
}

function card(s, o) {
  s.addShape(pres.ShapeType.roundRect, {
    x: o.x, y: o.y, w: o.w, h: o.h, rectRadius: 0.09,
    fill: { color: o.fill || WHITE },
    line: { color: o.line || LINE, width: 1 },
    shadow: o.shadow === false ? undefined : shadow(),
  });
}

function badge(s, o) {
  s.addShape(pres.ShapeType.ellipse, {
    x: o.x, y: o.y, w: o.d, h: o.d, fill: { color: o.fill || BRAND }, line: { color: o.fill || BRAND },
  });
  s.addText(o.label, {
    x: o.x, y: o.y, w: o.d, h: o.d, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: o.size || 13, bold: true, color: o.color || WHITE,
    align: "center", valign: "middle",
  });
}

/* ---------- 1. 表紙 ---------- */
{
  const s = pres.addSlide();
  darkBg(s);
  s.addText("ライセンス許諾のご提案", {
    x: M, y: 1.6, w: 6.9, h: 0.3, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 13, bold: true, charSpacing: 2, color: MINT,
  });
  s.addText("推しが、デスクトップで働く。", {
    x: M, y: 2.04, w: 7.3, h: 0.78, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 33, bold: true, color: WHITE,
  });
  s.addText(
    "貴社所属タレント様の公式キャラクターを、デスクトップに常駐して\n利用者の実務を手伝う AI アシスタントとして実装するご提案です。",
    {
      x: M, y: 3.06, w: 6.9, h: 0.9, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 14, color: INK_MUTED, lineSpacing: 25,
    }
  );
  s.addText(
    [
      { text: "【貴社名】 御中", options: { bold: true, color: WHITE, fontSize: 16, breakLine: true } },
      { text: "PicoAgent ／ 【氏名・屋号】", options: { color: INK_MUTED, fontSize: 12.5, breakLine: true } },
      { text: "【YYYY年MM月DD日】", options: { color: INK_MUTED, fontSize: 12.5 } },
    ],
    { x: M, y: 4.72, w: 6.4, h: 1.4, isTextBox: true, margin: 0, fontFace: JP, lineSpacing: 24 }
  );

  s.addShape(pres.ShapeType.roundRect, {
    x: 8.05, y: 2.12, w: 4.5, h: 3.02, rectRadius: 0.1,
    fill: { color: INK2 }, line: { color: INK2 },
    shadow: { type: "outer", color: "000000", opacity: 0.35, blur: 22, offset: 5, angle: 90 },
  });
  s.addImage({ path: path.join(IMG, "layout-shot-stream.png"), x: 8.2, y: 2.27, w: 4.2, h: 2.36 });
  s.addText("配信表示（フル HD）でもそのまま使えます", {
    x: 8.2, y: 4.72, w: 4.2, h: 0.26, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 10, color: INK_MUTED, align: "center",
  });
  s.addNotes("1通目は照会文のみを送る。この提案書は窓口と受付様式の回答を得てから送付する。");
}

/* ---------- 2. ご提案の要旨 ---------- */
{
  const s = pres.addSlide();
  s.background = { color: ALT };
  title(s, "SUMMARY", "ご提案の要旨");
  lead(s, "貴社のご負担は「公式モデルのお貸し出し」と「監修」のみ。制作・開発・販売・サポートはすべて当方が行います。");

  const items = [
    { n: "01", t: "ご提供いただくもの", b: "公式モデルのお貸し出し\n人格・口調の監修\nパックの最終承認" },
    { n: "02", t: "当方が行うこと", b: "表情差分の作画\nアニメーション実装\n販売・サポート・法令対応" },
    { n: "03", t: "収益", b: "手数料控除後の受取額を\nレベニューシェア\n最低保証は求めません" },
    { n: "04", t: "貴社の金銭的リスク", b: "ありません。\n制作費・運営費の負担なし\n売上発生時のみ配分", hl: true },
  ];
  const cw = 2.86, gap = 0.18;
  items.forEach((it, i) => {
    const x = M + i * (cw + gap);
    card(s, { x, y: 2.3, w: cw, h: 3.22, fill: it.hl ? INK : WHITE, line: it.hl ? INK : LINE });
    badge(s, { x: x + 0.3, y: 2.62, d: 0.44, label: it.n, fill: it.hl ? MINT : BRAND_SOFT, color: it.hl ? INK : BRAND, size: 12 });
    s.addText(it.t, {
      x: x + 0.3, y: 3.24, w: cw - 0.56, h: 0.34, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 15, bold: true, color: it.hl ? WHITE : TEXT,
    });
    s.addText(it.b, {
      x: x + 0.3, y: 3.72, w: cw - 0.56, h: 1.5, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 12, color: it.hl ? INK_MUTED : MUTED, lineSpacing: 21,
    });
  });

  s.addShape(pres.ShapeType.roundRect, {
    x: M, y: 5.88, w: CW, h: 1.0, rectRadius: 0.09,
    fill: { color: BRAND_SOFT }, line: { color: BRAND_SOFT },
  });
  s.addText(
    [
      { text: "本体アプリケーションは全機能を無料で配布しており、当方の収益源はキャラクターパックの販売のみです。", options: { bold: true, color: TEXT, breakLine: true } },
      { text: "貴社キャラクターが生む売上は、そのすべてが分配の対象になります。お借りしたモデルデータが製品に含まれることはありません。", options: { color: MUTED } },
    ],
    { x: M + 0.32, y: 6.04, w: CW - 0.64, h: 0.7, isTextBox: true, margin: 0, fontFace: JP, fontSize: 13, lineSpacing: 22 }
  );
  pageNo(s, 2);
}

/* ---------- 3. PicoAgent とは ---------- */
{
  const s = pres.addSlide();
  s.background = { color: WHITE };
  title(s, "PRODUCT", "PicoAgent について");
  lead(s, "デスクトップの最前面に常駐するキャラクターが、会話から実務を代行する Windows / macOS 向けアプリケーションです。");

  const rows = [
    { t: "AI がローカルで動く", b: "推論エンジンを同梱。会話内容は外部に送信されません" },
    { t: "74 種の実務機能", b: "Web 検索、文書作成、画像生成、メール・チャット連携を自動で実行" },
    { t: "7 つの表示形式", b: "マスコット、チャット、作業画面のほか、フル HD の配信用表示を備えています" },
    { t: "キャラクターは差し替え式", b: "外見・音声・人格・口調をキャラクターパックごとに定義します" },
    { t: "安全側の既定", b: "削除は OS のゴミ箱経由のみ。PC 操作・外部連携は既定で無効" },
  ];
  rows.forEach((r, i) => {
    const y = 2.3 + i * 0.88;
    badge(s, { x: M, y: y + 0.04, d: 0.34, label: "✓", fill: BRAND_SOFT, color: BRAND, size: 12 });
    s.addText(r.t, {
      x: M + 0.52, y: y, w: 5.9, h: 0.3, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 14, bold: true, color: TEXT,
    });
    s.addText(r.b, {
      x: M + 0.52, y: y + 0.32, w: 5.95, h: 0.5, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 11.5, color: MUTED, lineSpacing: 17,
    });
  });

  card(s, { x: 7.4, y: 2.3, w: 5.15, h: 3.92, fill: ALT, line: LINE });
  s.addImage({ path: path.join(IMG, "layout-shot-workspace.png"), x: 7.58, y: 2.52, w: 4.8, h: 2.92 });
  s.addText("作業表示。会話からファイル整理や PDF 処理まで実行します", {
    x: 7.6, y: 5.62, w: 4.75, h: 0.3, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 10, color: MUTED, align: "center",
  });
  pageNo(s, 3);
}

/* ---------- 4. なぜ相性が良いのか ---------- */
{
  const s = pres.addSlide();
  darkBg(s);
  title(s, "WHY", "なぜタレント様の IP と相性が良いのか", true);

  s.addText("接触時間の長さです。", {
    x: M, y: 1.66, w: CW, h: 0.44, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 22, bold: true, color: MINT,
  });

  const cols = [
    { t: "グッズ", b: "棚に置かれている間だけ\n目に入ります", dim: true },
    { t: "配信", b: "ご視聴いただいている\n時間だけ目に入ります", dim: true },
    { t: "デスクトップ常駐", b: "利用者が PC で作業している\n間、ずっと画面にいます", dim: false },
  ];
  const cw = 3.86, gap = 0.16;
  cols.forEach((c, i) => {
    const x = M + i * (cw + gap);
    s.addShape(pres.ShapeType.roundRect, {
      x, y: 2.38, w: cw, h: 1.84, rectRadius: 0.09,
      fill: { color: c.dim ? INK2 : BRAND }, line: { color: c.dim ? INK2 : BRAND },
    });
    s.addText(c.t, {
      x: x + 0.3, y: 2.64, w: cw - 0.6, h: 0.36, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 16, bold: true, color: c.dim ? INK_MUTED : WHITE,
    });
    s.addText(c.b, {
      x: x + 0.3, y: 3.08, w: cw - 0.6, h: 0.86, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 12, color: c.dim ? INK_MUTED : "E9E5FF", lineSpacing: 20,
    });
  });

  s.addText("1 日数時間 × 数か月", {
    x: M, y: 4.5, w: 6.0, h: 0.58, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 30, bold: true, color: WHITE,
  });
  s.addText("という接触を、1,000〜2,000 円台のデジタル商材で作れる形式は他にありません。", {
    x: M, y: 5.1, w: CW, h: 0.34, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 14, color: INK_MUTED,
  });

  s.addShape(pres.ShapeType.roundRect, {
    x: M, y: 5.7, w: CW, h: 1.08, rectRadius: 0.09,
    fill: { color: INK2 }, line: { color: INK2 },
  });
  s.addText(
    [
      { text: "眺める対象ではなく、実際に役に立ちます。", options: { bold: true, color: WHITE, breakLine: true } },
      { text: "調べもの、文書作成、メールの要約を手伝うため、「かわいいから起動する」ではなく「使うから起動し、結果として毎日会う」関係になります。", options: { color: INK_MUTED } },
    ],
    { x: M + 0.32, y: 5.86, w: CW - 0.64, h: 0.78, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12.5, lineSpacing: 21 }
  );
  pageNo(s, 4, true);
}

/* ---------- 5. ご提供形態 ---------- */
{
  const s = pres.addSlide();
  s.background = { color: ALT };
  title(s, "PLAN", "ご提供形態");
  lead(s, "違いは「声をどうするか」だけで、外見と人格の制作工程は共通です。初回は A をご提案します。");

  const plans = [
    { k: "A", t: "公式キャラクターパック", p: "1,800〜2,200 円", b: "公式モデルをもとに当方が\n2.5D アニメーションを制作。\n発話は字幕で表示", n: "ご負担：モデルのお貸し出しと監修", hl: true },
    { k: "B", t: "本人収録ボイス付き", p: "2,980〜3,500 円", b: "A に加え、本人収録の定型\nボイス（起動時・待機時など）\nを再生", n: "ご負担：収録のご協力" },
    { k: "C", t: "公式音声モデル対応", p: "3,980〜4,800 円", b: "貴社がご用意された公式の\n音声モデルで、生成した応答\nをその声で読み上げ", n: "ご負担：音声モデルのご提供と許諾" },
  ];
  const cw = 3.86, gap = 0.16;
  plans.forEach((p, i) => {
    const x = M + i * (cw + gap);
    card(s, { x, y: 2.3, w: cw, h: 2.98, fill: WHITE, line: p.hl ? BRAND : LINE });
    badge(s, { x: x + 0.3, y: 2.58, d: 0.42, label: p.k, fill: p.hl ? BRAND : BRAND_SOFT, color: p.hl ? WHITE : BRAND, size: 14 });
    if (p.hl) {
      s.addShape(pres.ShapeType.roundRect, {
        x: x + cw - 1.2, y: 2.6, w: 0.9, h: 0.36, rectRadius: 0.07,
        fill: { color: MINT }, line: { color: MINT },
      });
      s.addText("初回提案", {
        x: x + cw - 1.2, y: 2.6, w: 0.9, h: 0.36, isTextBox: true, margin: 0,
        fontFace: JP, fontSize: 9.5, bold: true, color: WHITE, align: "center", valign: "middle",
      });
    }
    s.addText(p.t, {
      x: x + 0.3, y: 3.18, w: cw - 0.56, h: 0.32, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 15, bold: true, color: TEXT,
    });
    s.addText(p.p, {
      x: x + 0.3, y: 3.52, w: cw - 0.56, h: 0.4, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 19, bold: true, color: BRAND,
    });
    s.addText(p.b, {
      x: x + 0.3, y: 4.0, w: cw - 0.56, h: 0.82, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 11.5, color: MUTED, lineSpacing: 18,
    });
    s.addText(p.n, {
      x: x + 0.3, y: 4.86, w: cw - 0.56, h: 0.28, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 10.5, bold: true, color: MINT,
    });
  });

  s.addShape(pres.ShapeType.roundRect, {
    x: M, y: 5.66, w: CW, h: 1.1, rectRadius: 0.09,
    fill: { color: WHITE }, line: { color: LINE },
  });
  s.addText(
    [
      { text: "C では、その場で生成した応答をタレント様の声でお届けできます。", options: { bold: true, color: TEXT, breakLine: true } },
      { text: "当方は音声合成エンジンを統合済みで、公式モデルをお預かりできれば追加開発はほとんど必要ありません。記念日・ライブ連動の期間限定販売は、A〜C いずれでもお受けできます。", options: { color: MUTED } },
    ],
    { x: M + 0.32, y: 5.84, w: CW - 0.64, h: 0.78, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12.5, lineSpacing: 21 }
  );
  pageNo(s, 5);
}

/* ---------- 6. 収益条件 ---------- */
{
  const s = pres.addSlide();
  s.background = { color: WHITE };
  title(s, "TERMS", "収益条件のご提案");
  lead(s, "最低保証（ミニマムギャランティ）は求めません。売上が立たない限り、貴社に損失が発生しない構造です。");

  const terms = [
    ["配分方式", "手数料・返金を控除した当方の実受取額を基準に按分"],
    ["配分率", "貴社 30〜40%（体数・独占条件・監修範囲に応じご相談）"],
    ["最低保証", "なし"],
    ["監修費", "貴社で発生した監修工数は実費にてお支払い"],
    ["契約期間", "2 年間・自動更新、解約予告 3 か月"],
    ["独占", "非独占でお願いいたします"],
    ["販売終了時", "いつでも新規販売を停止可能。既購入者の継続利用のみお認めください"],
  ];
  terms.forEach((t, i) => {
    const y = 2.3 + i * 0.56;
    s.addShape(pres.ShapeType.rect, { x: M, y: y + 0.5, w: 7.0, h: 0, line: { color: LINE, width: 1 } });
    s.addText(t[0], {
      x: M, y: y, w: 1.55, h: 0.44, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 12, bold: true, color: TEXT, valign: "middle",
    });
    s.addText(t[1], {
      x: M + 1.62, y: y, w: 5.38, h: 0.44, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 11.5, color: MUTED, valign: "middle",
    });
  });

  card(s, { x: 8.35, y: 2.3, w: 4.2, h: 3.86, fill: ALT, line: LINE });
  s.addText("2,200 円で販売した場合の内訳", {
    x: 8.62, y: 2.56, w: 3.66, h: 0.3, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 12.5, bold: true, color: TEXT,
  });
  const rows = [
    ["税抜価格", "2,000 円", TEXT],
    ["プラットフォーム手数料", "−600 円", MUTED],
    ["当方受取額（基準額）", "1,400 円", TEXT],
  ];
  rows.forEach((r, i) => {
    const y = 3.08 + i * 0.54;
    s.addText(r[0], {
      x: 8.62, y, w: 2.3, h: 0.34, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 11, color: MUTED, valign: "middle",
    });
    s.addText(r[1], {
      x: 10.9, y, w: 1.38, h: 0.34, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 12.5, bold: true, color: r[2], align: "right", valign: "middle",
    });
  });
  s.addShape(pres.ShapeType.roundRect, {
    x: 8.62, y: 4.86, w: 3.66, h: 0.98, rectRadius: 0.08,
    fill: { color: BRAND }, line: { color: BRAND },
  });
  s.addText("貴社配分（35% の場合）", {
    x: 8.84, y: 4.98, w: 2.2, h: 0.3, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 10.5, color: "DDD7FF", valign: "middle",
  });
  s.addText("490 円 / 本", {
    x: 8.84, y: 5.3, w: 3.24, h: 0.4, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 19, bold: true, color: WHITE,
  });
  s.addText("※ 手数料 30% を想定。料率はプラットフォームの条件により変動するため、契約時に実料率で確定いたします。", {
    x: M, y: 6.48, w: CW, h: 0.34, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 10, color: MUTED,
  });
  pageNo(s, 6);
}

/* ---------- 7. 監修と安全性 ---------- */
{
  const s = pres.addSlide();
  s.background = { color: ALT };
  title(s, "GOVERNANCE", "監修と安全性について");
  lead(s, "AI がタレント様の名前で発言することのリスクを、最も重要な論点として認識しております。");

  const items = [
    { t: "音声", b: "当方が既存の音声から声を再現するモデルを作ることはありません。扱うのは貴社が本人同意のもとでご用意された公式の音声モデルに限ります" },
    { t: "人格・口調", b: "性格・話し方・一人称・挨拶文・システムプロンプトの全文を事前にご提出し、書面での承認をいただきます" },
    { t: "発言内容の制御", b: "禁止話題の一覧と照合し、利用者の発話と AI の出力の両方を止めるフィルタを標準搭載済み。AI の判断は使いません" },
    { t: "AI である旨の表示", b: "「AI による生成であり、ご本人の発言ではない」旨をアプリ内に常時表示します" },
    { t: "会話の秘匿", b: "既定のローカル動作では会話が外部に送信されません。当方は会話ログを一切収集しません" },
    { t: "学習への不使用", b: "ご支給素材および利用者の会話を、AI モデルの学習に使用しません" },
  ];
  const cw = 3.86, ch = 1.56, gx = 0.16, gy = 0.16;
  items.forEach((it, i) => {
    const x = M + (i % 3) * (cw + gx);
    const y = 2.3 + Math.floor(i / 3) * (ch + gy);
    card(s, { x, y, w: cw, h: ch, fill: WHITE, line: LINE });
    badge(s, { x: x + 0.26, y: y + 0.26, d: 0.34, label: "✓", fill: MINT, color: WHITE, size: 12 });
    s.addText(it.t, {
      x: x + 0.7, y: y + 0.26, w: cw - 0.96, h: 0.32, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 13.5, bold: true, color: TEXT,
    });
    s.addText(it.b, {
      x: x + 0.26, y: y + 0.68, w: cw - 0.52, h: 0.78, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 10.5, color: MUTED, lineSpacing: 16,
    });
  });

  s.addShape(pres.ShapeType.roundRect, {
    x: M, y: 5.86, w: CW, h: 1.0, rectRadius: 0.09,
    fill: { color: "FFF4E8" }, line: { color: "F0D8BC" },
  });
  s.addText(
    [
      { text: "現状のご報告：", options: { bold: true, color: "8A5A22" } },
      { text: "発言フィルタは実装済みで、次回の公開版から常に有効です（ご指定の禁止話題を一覧へ追加します）。人格ロック機能は本提案に合わせて実装する予定で、ご承認の見込みが立った段階で最優先に実装し、監修開始前に実装済みの版でご確認いただきます。", options: { color: "8A5A22" } },
    ],
    { x: M + 0.32, y: 6.02, w: CW - 0.64, h: 0.7, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11.5, lineSpacing: 19 }
  );
  pageNo(s, 7);
}

/* ---------- 8. 素材と監修フロー ---------- */
{
  const s = pres.addSlide();
  s.background = { color: WHITE };
  title(s, "PROCESS", "お借りする素材と監修の流れ");

  s.addText("お借りしたい素材", {
    x: M, y: 1.76, w: 5.6, h: 0.32, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 15, bold: true, color: TEXT,
  });
  const mats = [
    ["3D モデル（VRM 等）", "こちらで表情とポーズをレンダリングし 2.5D パックに起こします"],
    ["Live2D モデル", "同じ方式でお受けします。そのまま動かす方式は上位プランとして後日"],
    ["設定資料", "口調、一人称、呼称、NG 事項"],
    ["音声", "形態 B は音声ファイルと台本、形態 C は公式の音声モデル"],
  ];
  mats.forEach((m, i) => {
    const y = 2.34 + i * 0.8;
    badge(s, { x: M, y: y + 0.02, d: 0.3, label: "●", fill: BRAND_SOFT, color: BRAND, size: 8 });
    s.addText(m[0], {
      x: M + 0.46, y, w: 5.2, h: 0.3, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 12.5, bold: true, color: TEXT,
    });
    s.addText(m[1], {
      x: M + 0.46, y: y + 0.31, w: 5.2, h: 0.46, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 10.5, color: MUTED, lineSpacing: 16,
    });
  });
  s.addShape(pres.ShapeType.roundRect, {
    x: M, y: 5.72, w: 5.7, h: 0.94, rectRadius: 0.09,
    fill: { color: ALT }, line: { color: LINE },
  });
  s.addText(
    [
      { text: "お借りしたモデルは製品に同梱しません。", options: { bold: true, color: TEXT, breakLine: true } },
      { text: "出荷物はレンダリング済みの 2.5D パックで、元モデルが利用者の端末へ渡ることはありません。", options: { color: MUTED } },
    ],
    { x: M + 0.28, y: 5.86, w: 5.14, h: 0.66, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11, lineSpacing: 18 }
  );

  card(s, { x: 6.75, y: 1.76, w: 5.8, h: 4.9, fill: ALT, line: LINE });
  s.addText("監修の流れ（想定 8〜12 週間）", {
    x: 7.05, y: 2.04, w: 5.2, h: 0.32, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 14, bold: true, color: TEXT,
  });
  const steps = [
    ["素材のご支給、仕様のすり合わせ", "2 週"],
    ["表情差分・アニメーションの初稿をご提出", "3 週"],
    ["人格設定・台詞・挨拶文のご監修", "2 週"],
    ["実機版でのご確認（動く状態でご確認）", "2 週"],
    ["最終承認、ストアページのご確認、配信", "1 週"],
  ];
  steps.forEach((st, i) => {
    const y = 2.64 + i * 0.68;
    badge(s, { x: 7.05, y, d: 0.38, label: String(i + 1), fill: BRAND, color: WHITE, size: 12 });
    s.addText(st[0], {
      x: 7.58, y: y + 0.02, w: 3.5, h: 0.34, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 11.5, color: TEXT, valign: "middle",
    });
    s.addText(st[1], {
      x: 11.2, y: y + 0.02, w: 1.05, h: 0.34, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 11.5, bold: true, color: BRAND, align: "right", valign: "middle",
    });
  });
  s.addText("修正は各段階 2 回までを想定しています（貴社の監修工数を予測可能にするためです。回数・期日はご相談に応じます）。", {
    x: 7.05, y: 6.06, w: 5.2, h: 0.42, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 10, color: MUTED, lineSpacing: 15,
  });
  pageNo(s, 8);
}

/* ---------- 9. 体制と現状 ---------- */
{
  const s = pres.addSlide();
  s.background = { color: ALT };
  title(s, "ABOUT US", "当方の体制と現状");
  lead(s, "透明性のため、現状をありのままご報告します。");

  const facts = [
    ["体制", "【個人事業者／法人名】。開発から販売・サポートまで一貫"],
    ["製品の状況", "v1.0.5 Alpha。無償版を公開中／有償版は準備中"],
    ["販売予定の経路", "BOOTH、Steam を想定しています"],
    ["技術", "Rust / Tauri による実装。会話と設定は端末内で完結します"],
    ["ご確認いただける実物", "【配布 URL】より無償版をダウンロードいただけます"],
  ];
  facts.forEach((f, i) => {
    const y = 2.3 + i * 0.72;
    card(s, { x: M, y, w: 7.4, h: 0.62, fill: WHITE, line: LINE, shadow: false });
    s.addText(f[0], {
      x: M + 0.26, y, w: 1.85, h: 0.62, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 11.5, bold: true, color: BRAND, valign: "middle",
    });
    s.addText(f[1], {
      x: M + 2.15, y, w: 5.15, h: 0.62, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 11, color: MUTED, valign: "middle",
    });
  });

  s.addShape(pres.ShapeType.roundRect, {
    x: M, y: 6.0, w: 7.4, h: 0.92, rectRadius: 0.09,
    fill: { color: INK }, line: { color: INK },
  });
  s.addText(
    [
      { text: "実績がない相手とのお取引であることは承知しております。", options: { bold: true, color: WHITE, breakLine: true } },
      { text: "そのため最低保証を求めず、貴社のリスクがゼロとなる条件でご提案します。", options: { color: INK_MUTED } },
    ],
    { x: M + 0.3, y: 6.16, w: 6.84, h: 0.64, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12, lineSpacing: 20 }
  );

  card(s, { x: 8.5, y: 2.3, w: 4.05, h: 4.58, fill: WHITE, line: LINE });
  s.addImage({ path: path.join(IMG, "layout-shot-pet.png"), x: 8.72, y: 2.54, w: 3.61, h: 2.41 });
  s.addText("無償版で実際の常駐表示をご確認いただけます", {
    x: 8.72, y: 5.1, w: 3.61, h: 0.3, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 10.5, bold: true, color: TEXT, align: "center",
  });
  s.addText("常駐表示と会話、実務機能、キャラクターの差し替えまで、購入前にそのまま試せます。制作をご依頼いただく前に、完成イメージをご確認いただけます。", {
    x: 8.78, y: 5.5, w: 3.5, h: 1.1, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 10, color: MUTED, lineSpacing: 17,
  });
  pageNo(s, 9);
}

/* ---------- 10. ご確認事項 ---------- */
{
  const s = pres.addSlide();
  darkBg(s, -3.3);
  title(s, "NEXT", "ご確認いただきたい事項", true);

  const qs = [
    "本件が貴社のライセンス許諾の受付対象に含まれるか",
    "お貸し出しいただけるモデルの形式（Live2D / 3D）",
    "監修体制とご対応可能な期間",
    "タレント様ご本人の同意取得が必要な範囲",
    "公式の音声モデルの有無、または今後のご予定",
    "配分率と契約条件に関するご意向",
    "対象としてご検討いただけるタレント様の範囲",
  ];
  qs.forEach((q, i) => {
    const x = M + (i % 2) * 6.0;
    const y = 1.96 + Math.floor(i / 2) * 0.68;
    badge(s, { x, y: y + 0.02, d: 0.36, label: String(i + 1), fill: BRAND, color: WHITE, size: 12 });
    s.addText(q, {
      x: x + 0.52, y, w: 5.2, h: 0.4, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 13, color: WHITE, valign: "middle",
    });
  });

  s.addShape(pres.ShapeType.roundRect, {
    x: M, y: 4.74, w: CW, h: 1.76, rectRadius: 0.1,
    fill: { color: INK2 }, line: { color: INK2 },
  });
  s.addText("お問い合わせ先", {
    x: M + 0.36, y: 4.98, w: 4.0, h: 0.3, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 12, bold: true, color: MINT,
  });
  const contact = [
    ["氏名／屋号", "【　　　　　　　】"],
    ["所在地", "【　　　　　　　】"],
    ["連絡先", "【メールアドレス】"],
    ["製品サイト", "【URL】"],
  ];
  contact.forEach((c, i) => {
    const x = M + 0.36 + (i % 2) * 5.6;
    const y = 5.44 + Math.floor(i / 2) * 0.46;
    s.addText(c[0], {
      x, y, w: 1.6, h: 0.34, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 11, color: INK_MUTED, valign: "middle",
    });
    s.addText(c[1], {
      x: x + 1.65, y, w: 3.7, h: 0.34, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 11.5, bold: true, color: WHITE, valign: "middle",
    });
  });

  s.addText("本提案書に記載の価格・配分率・期間はいずれも交渉の出発点であり、確定条件ではありません。", {
    x: M, y: 6.72, w: CW, h: 0.3, isTextBox: true, margin: 0,
    fontFace: JP, fontSize: 10, color: INK_MUTED,
  });
}

prepareImages()
  .then(() => pres.writeFile({ fileName: OUT }))
  .then((f) => {
    fs.rmSync(IMG, { recursive: true, force: true });
    console.log("生成しました: " + f);
  })
  .catch((e) => {
    fs.rmSync(IMG, { recursive: true, force: true });
    console.error(e.message);
    process.exit(1);
  });
