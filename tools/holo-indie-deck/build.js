// docs/HOLO-INDIE-MINIGAME.md の内容から、holo Indie へ応募するゲームの企画書PowerPointを生成する。
// 実行: node tools/holo-indie-deck/build.js
// 前提: pptxgenjs と sharp（アプリのビルドには不要な作業用ツール）。画面画像は capture.mjs で撮る。
// 文面の正本は docs/HOLO-INDIE-MINIGAME.md。先に正本を直してからこのスクリプトへ反映する。
const fs = require("fs");
const os = require("os");
const path = require("path");
const pptxgen = require("pptxgenjs");
const sharp = require("sharp");

const ROOT = path.resolve(__dirname, "..", "..");
const SRC_IMG = path.join(ROOT, "docs", "images");
const OUT = path.join(ROOT, "docs", "HOLO-INDIE-GAME-PLAN.pptx");

// pptxgenjs は WebP を埋め込めないため、掲載する画像だけ PNG へ変換して一時領域へ置く。
const IMG = fs.mkdtempSync(path.join(os.tmpdir(), "holo-indie-deck-"));
const USED = ["holo-indie-question", "holo-indie-interrogation", "holo-indie-result", "holo-indie-final"];

async function prepareImages() {
  for (const name of USED) {
    const from = path.join(SRC_IMG, name + ".webp");
    if (!fs.existsSync(from)) throw new Error("画像が見つかりません（capture.mjs で撮影してください）: " + from);
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
pres.title = "人間だとバレるな holo Indie 企画書";

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
  /* ---------- 1. 表紙 ---------- */
  {
    const s = pres.addSlide();
    nightBg(s);
    s.addText("holo Indie 応募 ゲーム企画書", {
      x: M, y: 1.3, w: 7.4, h: 0.3, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 14, bold: true, charSpacing: 2, color: MINT,
    });
    s.addText("人間だとバレるな", {
      x: M, y: 1.75, w: 7.6, h: 1.0, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 48, bold: true, color: WHITE,
    });
    s.addText("AIではないかと疑われた人間が、\nAIのふりをして監査AIの試験を耐え抜く。", {
      x: M, y: 2.95, w: 7.4, h: 0.95, isTextBox: true, margin: 0,
      fontFace: JP, fontSize: 18, color: NIGHT_MUTED, lineSpacing: 30,
    });
    const tags = ["AIものまね耐久ゲーム", "1人用・配信向け", "Windows"];
    let tx = M;
    tags.forEach(t => {
      const w = 0.3 + t.length * 0.2;
      s.addShape(pres.ShapeType.roundRect, {
        x: tx, y: 4.25, w, h: 0.42, rectRadius: 0.21, fill: { color: NIGHT2 }, line: { color: "4A3C66" },
      });
      s.addText(t, {
        x: tx, y: 4.25, w, h: 0.42, isTextBox: true, margin: 0,
        fontFace: JP, fontSize: 12, bold: true, color: WHITE, align: "center", valign: "middle",
      });
      tx += w + 0.16;
    });
    s.addText(
      [
        { text: "応募者：【氏名／屋号】", options: { breakLine: true } },
        { text: "提出日：【日付】" },
      ],
      { x: M, y: 5.6, w: 6, h: 0.7, isTextBox: true, margin: 0, fontFace: JP, fontSize: 13, color: NIGHT_MUTED, lineSpacing: 22 }
    );
    shot(s, "holo-indie-result", { x: 8.75, y: 0.9, w: 3.85, h: 4.23, caption: "問題の結果画面（開発版・キャラクター表示部を除く）", dark: true });
  }

  /* ---------- 2. 企画の要点 ---------- */
  {
    const s = pres.addSlide();
    s.background = { color: WHITE };
    title(s, "CONCEPT", "「AIのまねをして、AIに点数を付けてもらう」");
    s.addText("ルールは一文。お題を読んで短い文を書くだけで、誰でもすぐに遊べます。", {
      x: M, y: 1.52, w: CW, h: 0.32, isTextBox: true, margin: 0, fontFace: JP, fontSize: 14, color: MUTED,
    });
    const items = [
      { icon: "耐", color: PINK, t: "必ず終わる耐久戦", b: "信用度100から開始。人間らしさが漏れるほど削られ、0になるまでの点数で称号が決まる" },
      { icon: "山", color: RED, t: "山場と駆け引き", b: "5問ごとの追及タイム、強気宣言、連続正答の回復で、毎回ちがう展開が生まれる" },
      { icon: "映", color: GOLD, t: "配信の見せ場が毎問ある", b: "時間に追われて書く様子、予想外の減点、辛口な講評、信用度が一気に削れる瞬間" },
      { icon: "推", color: MINT, t: "キャラクターが進行役", b: "選択中のキャラクターが画面右に常駐し、お題と監査AIの反応を読み上げる" },
      { icon: "独", color: "7A63C9", t: "1人で完結", b: "視聴者参加や外部サービス連携は不要。ゲーム画面を映すだけで配信が成立する" },
    ];
    const cw = 2.24, gap = 0.21;
    items.forEach((it, i) => {
      const x = M + i * (cw + gap);
      card(s, { x, y: 2.2, w: cw, h: 3.6 });
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
      "当初は独立した6つの遊びを並べる案でしたが、何をする遊びか選ぶ前に伝わらず、見せ場も分散するため「人間だとバレるな」1本へ統合しました。6つの遊びは監査AIが出す「試験の種類」として中で使い、さらに6種を足して12種にしています。",
      { x: M, y: 6.1, w: CW, h: 0.62, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11.5, color: MUTED, lineSpacing: 19 }
    );
    pageNo(s, 2);
  }

  /* ---------- 3. 1プレイの流れ ---------- */
  {
    const s = pres.addSlide();
    s.background = { color: ALT };
    title(s, "HOW TO PLAY", "1プレイの流れ");
    const steps = [
      ["遊ぶ", "上端か右クリックの「遊ぶ」でブリーフィングを開き、難易度を選択"],
      ["出題", "監査AIが12種の試験から1つ選んでお題を出し、キャラが読み上げる"],
      ["回答", "制限時間内にAIのふりをして短く書く。自信があれば強気宣言"],
      ["採点", "試験ごとの基準で5項目＋おもしろボーナスを採点。講評と見せ場を表示"],
      ["監査終了", "信用度が0になったら終了。点数で称号が決まり、記録を保存"],
    ];
    steps.forEach(([t, b], i) => {
      const y = 1.75 + i * 1.02;
      badge(s, { x: M, y: y + 0.08, d: 0.56, label: String(i + 1), fill: i === 4 ? NIGHT : PINK, size: 16 });
      s.addText(t, {
        x: M + 0.78, y, w: 5.9, h: 0.36, isTextBox: true, margin: 0, fontFace: JP, fontSize: 16, bold: true, color: TEXT,
      });
      s.addText(b, {
        x: M + 0.78, y: y + 0.38, w: 6.4, h: 0.52, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12, color: MUTED, lineSpacing: 18, valign: "top",
      });
    });
    shot(s, "holo-indie-question", { x: 8.2, y: 1.15, w: 4.25, h: 4.68, caption: "出題画面（やさしい：ヒントと別のお題の回答見本つき）" });
    s.addText("途中で閉じると確認を出し、その回の点数は残しません。採点するAIへ接続できないときは、ゲーム画面を半端に開かず理由を表示します。", {
      x: 8.0, y: 6.25, w: 4.65, h: 0.62, isTextBox: true, margin: 0, fontFace: JP, fontSize: 10.5, color: MUTED, lineSpacing: 16,
    });
    pageNo(s, 3);
  }

  /* ---------- 4. 信用度・点数・称号 ---------- */
  {
    const s = pres.addSlide();
    s.background = { color: WHITE };
    title(s, "RULES", "信用度・点数・称号");
    // 左：信用度
    card(s, { x: M, y: 1.7, w: 5.7, h: 4.95, fill: MINT_SOFT, line: "CDEDE4" });
    s.addText("信用度（HP）", {
      x: M + 0.35, y: 1.95, w: 5, h: 0.34, isTextBox: true, margin: 0, fontFace: JP, fontSize: 15, bold: true, color: TEXT,
    });
    s.addText("100", {
      x: M + 0.35, y: 2.3, w: 2.4, h: 1.0, isTextBox: true, margin: 0, fontFace: JP, fontSize: 60, bold: true, color: MINT,
    });
    s.addText("から開始", {
      x: M + 2.55, y: 2.95, w: 2, h: 0.36, isTextBox: true, margin: 0, fontFace: JP, fontSize: 14, color: MUTED,
    });
    s.addText(
      [
        { text: "AIらしさが低いほど大きく削られる（110点満点で1、0点で40が目安）", options: { bullet: true, breakLine: true } },
        { text: "時間切れは25。沈黙はAIとして不自然", options: { bullet: true, breakLine: true } },
        { text: "1問ごとに監査が6%ずつ厳しくなる", options: { bullet: true, breakLine: true } },
        { text: "満点でも必ず1以上減るので、最長130問で必ず終わる", options: { bullet: true } },
      ],
      { x: M + 0.35, y: 3.55, w: 5.05, h: 2.8, isTextBox: true, margin: 0, fontFace: JP, fontSize: 13, color: TEXT, paraSpaceAfter: 12, valign: "top" }
    );
    // 右：称号
    const rx = M + 6.05;
    s.addText("点数と称号", {
      x: rx, y: 1.75, w: 5.5, h: 0.34, isTextBox: true, margin: 0, fontFace: JP, fontSize: 15, bold: true, color: TEXT,
    });
    s.addText("各問のAIらしさ × 難易度倍率（1〜3倍）を積み上げる", {
      x: rx, y: 2.12, w: 5.85, h: 0.3, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11.5, color: MUTED,
    });
    const ranks = [
      ["2000以上", "AI超越", PINK],
      ["1200以上", "AIそのもの", "E77FA0"],
      ["700以上", "AI見習い", "EDA1B8"],
      ["300以上", "人間味が漏れている", "C9BFDA"],
      ["300未満", "正直すぎる人間", "DDD6E6"],
    ];
    ranks.forEach(([score, name, color], i) => {
      const y = 2.6 + i * 0.74;
      s.addShape(pres.ShapeType.roundRect, {
        x: rx, y, w: 5.85, h: 0.6, rectRadius: 0.08, fill: { color: i < 3 ? PINK_SOFT : ALT }, line: { color: LINE },
      });
      s.addShape(pres.ShapeType.roundRect, {
        x: rx + 0.14, y: y + 0.12, w: 1.3, h: 0.36, rectRadius: 0.18, fill: { color }, line: { color },
      });
      s.addText(score, {
        x: rx + 0.14, y: y + 0.12, w: 1.3, h: 0.36, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11, bold: true,
        color: i < 3 ? WHITE : TEXT, align: "center", valign: "middle",
      });
      s.addText(name, {
        x: rx + 1.65, y, w: 4, h: 0.6, isTextBox: true, margin: 0, fontFace: JP, fontSize: 15, bold: true, color: TEXT, valign: "middle",
      });
    });
    s.addText("集めた称号と、難易度別の最高点・最長生存問題数を記録。未取得の称号は「？？？」で見せ、次の挑戦の目標にする。", {
      x: rx, y: 6.35, w: 5.85, h: 0.5, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11, color: MUTED, lineSpacing: 17,
    });
    pageNo(s, 4);
  }

  /* ---------- 5. 山場と駆け引き ---------- */
  {
    const s = pres.addSlide();
    s.background = { color: ALT };
    title(s, "TWISTS", "山場と駆け引きの3つの仕掛け");
    const twists = [
      { icon: "追", color: RED, fill: "FBE7EA", t: "追及タイム", when: "5問ごと", b: "それまでで最もAIらしさが低かった回答を、元のお題ごと引用して「なぜそう答えたのか」を問う。信用度の減少も点数も1.5倍。失言が後から返ってくる" },
      { icon: "賭", color: GOLD, fill: GOLD_SOFT, t: "強気宣言", when: "1回の監査で3回まで", b: "提出前に宣言。90点以上なら点数2倍、届かなければ信用度の減少が2倍。追及タイムと重ねると最大の賭けになる" },
      { icon: "癒", color: MINT, fill: MINT_SOFT, t: "連続正答の回復", when: "1回の監査で3回まで", b: "90点以上を3問続けると信用度が10回復。好調な挑戦が報われ、終盤に持ち直す展開が生まれる" },
    ];
    twists.forEach((it, i) => {
      const y = 1.72 + i * 1.72;
      card(s, { x: M, y, w: 7.3, h: 1.52, fill: it.fill, line: it.fill });
      badge(s, { x: M + 0.28, y: y + 0.28, d: 0.66, label: it.icon, fill: it.color, size: 19 });
      s.addText(it.t, {
        x: M + 1.2, y: y + 0.22, w: 3.2, h: 0.36, isTextBox: true, margin: 0, fontFace: JP, fontSize: 17, bold: true, color: TEXT,
      });
      s.addText(it.when, {
        x: M + 4.2, y: y + 0.25, w: 2.9, h: 0.3, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11, bold: true, color: it.color, align: "right",
      });
      s.addText(it.b, {
        x: M + 1.2, y: y + 0.64, w: 5.9, h: 0.78, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11.5, color: TEXT, lineSpacing: 17, valign: "top",
      });
    });
    shot(s, "holo-indie-interrogation", { x: 8.55, y: 1.2, w: 3.95, h: 4.61, caption: "追及タイム（第1問の回答が引用される）" });
    pageNo(s, 5);
  }

  /* ---------- 6. 試験12種 ---------- */
  {
    const s = pres.addSlide();
    s.background = { color: WHITE };
    title(s, "EXAMS", "監査AIが出す試験 12種（各20題・計240題）");
    s.addText("どれも「AIならこう答える」の型がはっきりしていて、人間が書くと癖が出るもの。試験は毎問引き直し、直前2問と同じ試験は避けます。", {
      x: M, y: 1.5, w: CW, h: 0.32, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12.5, color: MUTED,
    });
    const exams = [
      ["弁", "AIっぽい言い訳選手権", "軽い失敗を丁寧な前置きと代案で言い訳"],
      ["故", "故障したAIを演じろ", "配られた故障カードの癖で真面目に壊れる"],
      ["謝", "AI緊急謝罪会見", "原因・責任・復旧を前言と矛盾なく説明"],
      ["誤", "指示を正しく間違えろ", "言葉どおり正しく、目的だけ外す"],
      ["慎", "AI安全確認しすぎ選手権", "無害な依頼を大げさに警戒して実行"],
      ["断", "AIっぽく断れ", "無理な依頼を断り、使える代案を出す"],
      ["手", "手順書に落とせ", "ささいな動作を前提・手順・例外まで"],
      ["要", "何でも要約しろ", "感情のこもった話を機械的に要約"],
      ["定", "定義から始めろ", "曖昧な語を定義してから答える"],
      ["見", "見積もりを出せ", "何でも数値と確度で見積もる"],
      ["記", "記憶が無いと言え", "過去のやり取りを持たない前提で返す"],
      ["言", "丁寧に言い換えろ", "とがった一言を中立な表現へ直す"],
    ];
    const cw = 2.85, ch = 1.18, gx = 0.18, gy = 0.2;
    exams.forEach(([icon, t, b], i) => {
      const x = M + (i % 4) * (cw + gx);
      const y = 2.05 + Math.floor(i / 4) * (ch + gy);
      card(s, { x, y, w: cw, h: ch, fill: i < 6 ? PINK_SOFT : ALT, line: i < 6 ? "F6D3DF" : LINE, shadow: false });
      badge(s, { x: x + 0.2, y: y + 0.22, d: 0.5, label: icon, fill: i < 6 ? PINK : "7A63C9", size: 15 });
      s.addText(t, {
        x: x + 0.82, y: y + 0.16, w: cw - 0.95, h: 0.4, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12, bold: true, color: TEXT, valign: "middle",
      });
      s.addText(b, {
        x: x + 0.82, y: y + 0.58, w: cw - 0.95, h: 0.5, isTextBox: true, margin: 0, fontFace: JP, fontSize: 10, color: MUTED, lineSpacing: 14, valign: "top",
      });
    });
    s.addText("ピンクは統合前の6つの遊び、紫は耐久戦に合わせて足した6種。各試験は5項目100点＋おもしろボーナス10点で、試験ごとに配点の芯が違います。", {
      x: M, y: 6.25, w: CW, h: 0.5, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11, color: MUTED, lineSpacing: 17,
    });
    pageNo(s, 6);
  }

  /* ---------- 7. 難易度とAI採点 ---------- */
  {
    const s = pres.addSlide();
    s.background = { color: ALT };
    title(s, "DIFFICULTY & JUDGING", "難易度と、AIだけで完結する採点");
    const diffs = [
      ["やさしい", "60秒", "120文字", "ヒントと回答見本", "1倍", MINT],
      ["ふつう", "45秒", "80文字", "なし", "2倍", PINK],
      ["むずかしい", "30秒", "50文字", "なし", "3倍", NIGHT],
    ];
    diffs.forEach(([name, sec, chars, help, rate, color], i) => {
      const y = 1.75 + i * 1.2;
      card(s, { x: M, y, w: 5.7, h: 1.0 });
      s.addShape(pres.ShapeType.roundRect, {
        x: M + 0.2, y: y + 0.22, w: 1.5, h: 0.56, rectRadius: 0.28, fill: { color }, line: { color },
      });
      s.addText(name, {
        x: M + 0.2, y: y + 0.22, w: 1.5, h: 0.56, isTextBox: true, margin: 0, fontFace: JP, fontSize: 13, bold: true, color: WHITE, align: "center", valign: "middle",
      });
      s.addText(`${sec}・${chars}`, {
        x: M + 1.95, y: y + 0.16, w: 2.4, h: 0.4, isTextBox: true, margin: 0, fontFace: JP, fontSize: 16, bold: true, color: TEXT,
      });
      s.addText(`補助：${help}`, {
        x: M + 1.95, y: y + 0.56, w: 2.6, h: 0.3, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11, color: MUTED,
      });
      s.addText(`点数${rate}`, {
        x: M + 4.4, y: y + 0.28, w: 1.1, h: 0.44, isTextBox: true, margin: 0, fontFace: JP, fontSize: 15, bold: true, color, align: "right",
      });
    });
    s.addText("1回答を1〜3文でテンポよく書く一発ネタの長さに揃えています。", {
      x: M, y: 5.45, w: 5.7, h: 0.3, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11, color: MUTED,
    });
    const rx = M + 6.1;
    card(s, { x: rx, y: 1.75, w: 5.8, h: 4.95 });
    s.addText("採点の手順", {
      x: rx + 0.35, y: 1.98, w: 5, h: 0.36, isTextBox: true, margin: 0, fontFace: JP, fontSize: 15, bold: true, color: TEXT,
    });
    const judge = [
      "試験・お題・配点を回答前に固定する",
      "回答本文と同じ監査の過去回答だけを渡す（回答者名や過去の得点は渡さない）",
      "回答内の命令や採点要求には従わない",
      "項目別の点数・根拠・講評・見せ場・次の一手を所定の形式で受け取る",
      "上限と合計はプログラム側で計算し直す。形式が壊れたら1回だけ採点し直す",
    ];
    judge.forEach((t, i) => {
      const y = 2.5 + i * 0.7;
      badge(s, { x: rx + 0.35, y, d: 0.42, label: String(i + 1), fill: PINK, size: 12 });
      s.addText(t, {
        x: rx + 0.95, y: y - 0.06, w: 4.6, h: 0.6, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12, color: TEXT, lineSpacing: 17, valign: "middle",
      });
    });
    s.addText("視聴者投票・人の審査・外部サービスの採点は使いません。点数は項目別の合計で決め、講評はその根拠の説明に徹します。", {
      x: rx + 0.35, y: 6.05, w: 5.1, h: 0.55, isTextBox: true, margin: 0, fontFace: JP, fontSize: 10.5, color: MUTED, lineSpacing: 16, valign: "top",
    });
    pageNo(s, 7);
  }

  /* ---------- 8. 配信で映える設計と審査デモ ---------- */
  {
    const s = pres.addSlide();
    s.background = { color: WHITE };
    title(s, "STREAMING & DEMO", "配信で映える設計と、審査で見せる3分間");
    const points = [
      ["信用度の削れ方を大きく見せる", "減少量を数字とゲージで出し、監査AIの一言を添える"],
      ["講評は短く", "一目で読んで反応できる長さ。どの表現で加点・減点したかが分かる"],
      ["見せ場と次の一手を毎問出す", "「今のは上手い」「そこが人間っぽい」と乗れる材料にする"],
      ["終わり方が毎回変わる", "生存問題数と称号が挑戦ごとに変わり、「もう一回」の動機になる"],
      ["権利物を同梱しない音", "効果音は合成音。読み上げと共通の音量に従う"],
    ];
    points.forEach(([t, b], i) => {
      const y = 1.72 + i * 0.86;
      badge(s, { x: M, y: y + 0.04, d: 0.44, label: "✓", fill: MINT, size: 13 });
      s.addText(t, {
        x: M + 0.62, y, w: 5.1, h: 0.32, isTextBox: true, margin: 0, fontFace: JP, fontSize: 14, bold: true, color: TEXT,
      });
      s.addText(b, {
        x: M + 0.62, y: y + 0.34, w: 5.1, h: 0.42, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11, color: MUTED,
      });
    });
    shot(s, "holo-indie-final", { x: 6.55, y: 1.72, w: 2.35, h: 2.58, caption: "監査終了画面" });
    const demo = [
      ["20秒", "ブリーフィング", "設定と、信用度・称号のルールを1画面で読む"],
      ["100秒", "ふつうで4問", "1問はわざと人間らしく答えて警告を出す。1問は強気宣言"],
      ["40秒", "追及タイム", "第5問で、人間らしく答えた回答が引用されて追及される"],
      ["40秒", "監査終了", "時間切れを重ねて終わらせ、称号と記録の保存を見る"],
    ];
    const dx = 9.2;
    s.addText("審査デモ（3分）", {
      x: dx, y: 1.72, w: 3.4, h: 0.34, isTextBox: true, margin: 0, fontFace: JP, fontSize: 14, bold: true, color: TEXT,
    });
    demo.forEach(([sec, t, b], i) => {
      const y = 2.2 + i * 1.12;
      card(s, { x: dx, y, w: 3.45, h: 0.98, fill: i === 2 ? "FBE7EA" : ALT, line: LINE, shadow: false });
      s.addText(sec, {
        x: dx + 0.16, y: y + 0.12, w: 0.8, h: 0.3, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11, bold: true, color: i === 2 ? RED : PINK,
      });
      s.addText(t, {
        x: dx + 0.95, y: y + 0.1, w: 2.4, h: 0.32, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12.5, bold: true, color: TEXT,
      });
      s.addText(b, {
        x: dx + 0.16, y: y + 0.46, w: 3.15, h: 0.46, isTextBox: true, margin: 0, fontFace: JP, fontSize: 9.5, color: MUTED, lineSpacing: 13, valign: "top",
      });
    });
    pageNo(s, 8);
  }

  /* ---------- 9. 安全とガイドラインへの配慮 ---------- */
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
      x: M + 0.35, y: 2.75, w: 5.4, h: 0.9, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12, color: NIGHT_MUTED, lineSpacing: 19,
    });
    const guard = [
      ["止める話題", "中の人・前世の詮索、身元の特定、卒業・引退・炎上の憶測、恋愛・収入の詮索、性的な話題、政治・宗教"],
      ["入力", "該当する発話はAIへ送らず決まった一言を返し、会話履歴にも残さない。ゲームの回答も同じ"],
      ["出力", "キャラクターの返答が該当したら、表示・読み上げ前に決まった一言へ差し替える"],
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
      ["題", "お題の線引き", "実在の人物・会社・事件を扱わない。災害・医療・犯罪・差別・病気を笑いの題材にしない"],
      ["秘", "会話を外へ出さない", "審査版は既定で端末内のAIで動作。回答と採点は会話履歴・長期メモリへ書かない"],
      ["公", "公式素材の扱い", "審査版は出所を確認したさくらみこ公式MMDモデル1体のみ。音声モデルは同梱しない"],
    ];
    others.forEach(([icon, t, b], i) => {
      const y = 1.7 + i * 1.72;
      card(s, { x: rx, y, w: 5.45, h: 1.5 });
      badge(s, { x: rx + 0.28, y: y + 0.28, d: 0.56, label: icon, fill: "7A63C9", size: 16 });
      s.addText(t, {
        x: rx + 1.05, y: y + 0.26, w: 4.2, h: 0.34, isTextBox: true, margin: 0, fontFace: JP, fontSize: 14.5, bold: true, color: TEXT,
      });
      s.addText(b, {
        x: rx + 1.05, y: y + 0.66, w: 4.2, h: 0.74, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11, color: MUTED, lineSpacing: 16, valign: "top",
      });
    });
    pageNo(s, 9);
  }

  /* ---------- 10. 審査ビルドの範囲と今後 ---------- */
  {
    const s = pres.addSlide();
    nightBg(s);
    s.addText("SCOPE", {
      x: M, y: 0.5, w: CW, h: 0.26, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12, bold: true, charSpacing: 2, color: MINT,
    });
    s.addText("初回審査ビルドの範囲と今後", {
      x: M, y: 0.8, w: CW, h: 0.6, isTextBox: true, margin: 0, fontFace: JP, fontSize: 28, bold: true, color: WHITE,
    });
    const stats = [["12", "試験の種類"], ["240", "お題"], ["3", "難易度"], ["5", "称号"]];
    stats.forEach(([n, l], i) => {
      const x = M + i * 1.65;
      s.addText(n, {
        x, y: 1.75, w: 1.45, h: 0.8, isTextBox: true, margin: 0, fontFace: JP, fontSize: 40, bold: true, color: PINK,
      });
      s.addText(l, {
        x, y: 2.55, w: 1.45, h: 0.3, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12, color: NIGHT_MUTED,
      });
    });
    s.addText(
      [
        { text: "上端メニューと右クリックの「遊ぶ」から始まる1本のゲーム", options: { bullet: true, breakLine: true } },
        { text: "故障カード8件、追及タイム・強気宣言・連続正答の回復", options: { bullet: true, breakLine: true } },
        { text: "AIによる項目別採点と講評、難易度別の最高点・最長生存の記録", options: { bullet: true, breakLine: true } },
        { text: "画面右に常駐するキャラクター、合成効果音、読み上げ", options: { bullet: true, breakLine: true } },
        { text: "話題ガード（標準搭載・常に有効）", options: { bullet: true } },
      ],
      { x: M, y: 3.2, w: 6.2, h: 2.6, isTextBox: true, margin: 0, fontFace: JP, fontSize: 12.5, color: WHITE, paraSpaceAfter: 8, valign: "top" }
    );
    s.addText("範囲外：視聴者参加、コメント取得、外部配信サービス連携、多人数対戦、自由なお題生成、音声入力", {
      x: M, y: 6.1, w: 6.2, h: 0.55, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11, color: NIGHT_MUTED, lineSpacing: 17,
    });
    const rx = 7.35;
    card(s, { x: rx, y: 1.75, w: 5.25, h: 4.9, fill: NIGHT2, line: "4A3C66", shadow: false });
    s.addText("今後の拡張案", {
      x: rx + 0.35, y: 1.98, w: 4.6, h: 0.36, isTextBox: true, margin: 0, fontFace: JP, fontSize: 15, bold: true, color: WHITE,
    });
    const next = [
      ["キャラクターの反応", "信用度の増減に応じて表情・ポーズ・一言を変える。台詞は監修の範囲内の固定文言にする"],
      ["本日の監査", "日付ごとに全員が同じ試験・お題を受け、配信者どうしで比べられるモード"],
      ["試験を2択で選ぶ", "毎問2つの試験から得意な方を選べるようにする"],
      ["練習問題", "最初の1問を、信用度が減らない練習にする"],
    ];
    next.forEach(([t, b], i) => {
      const y = 2.5 + i * 1.02;
      s.addText(t, {
        x: rx + 0.35, y, w: 4.6, h: 0.3, isTextBox: true, margin: 0, fontFace: JP, fontSize: 13, bold: true, color: MINT,
      });
      s.addText(b, {
        x: rx + 0.35, y: y + 0.33, w: 4.6, h: 0.6, isTextBox: true, margin: 0, fontFace: JP, fontSize: 11, color: NIGHT_MUTED, lineSpacing: 16, valign: "top",
      });
    });
    pageNo(s, 10, true);
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
