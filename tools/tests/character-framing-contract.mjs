import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const styles = readFileSync(new URL("../../ui/styles.css", import.meta.url), "utf8");
const renderer = readFileSync(new URL("../../ui/renderer.js", import.meta.url), "utf8");

test("native-bustのメッシュ描画へ全身絵用クロップを適用しない", () => {
  const general = styles.indexOf(
    '.character[data-render-kind="mesh-rig"] .character-view > canvas',
  );
  const selector =
    '.character[data-render-kind="mesh-rig"][data-source-framing="native-bust"] .character-view > canvas';
  const override = styles.indexOf(selector);
  assert.ok(general >= 0, "メッシュ描画の一般指定が必要です");
  assert.ok(override > general, "native-bustの上書きは一般指定より後に置いてください");

  const rule = styles.slice(override, styles.indexOf("}", override) + 1);
  assert.match(rule, /width:auto!important/);
  assert.match(rule, /height:min\(calc\(76vh \* var\(--stream-mesh-scale,2\.6\) \/ 2\.6\),var\(--mesh-source-css-height/);
  assert.match(rule, /left:50%/);
  assert.match(rule, /translateX\(-50%\)/);
  assert.match(renderer, /isNativeBustImage && usesStreamImageCrop[\s\S]*FRAMING_PRESETS\.full/);
});

test("低頭身メッシュは配信でも低頭身用の胸上構図を保つ", () => {
  const general = styles.indexOf(
    '.character[data-render-kind="mesh-rig"] .character-view > canvas',
  );
  const selector =
    '.character[data-render-kind="mesh-rig"][data-body-proportion="compact"] .character-view > canvas';
  const override = styles.indexOf(selector);
  assert.ok(override > general, "低頭身メッシュの上書きは一般指定より後に置いてください");
  const rule = styles.slice(override, styles.indexOf("}", override) + 1);
  assert.match(rule, /height:min\(calc\(76vh \* var\(--stream-mesh-scale,2\.6\) \/ 2\.6\),var\(--mesh-source-css-height/);
  assert.match(rule, /left:50%/);
  assert.match(renderer, /character\.dataset\.renderKind === "mesh-rig"/);
  assert.match(renderer, /isCompactImage/);
});

test("Live2Dの小型アイコンはモデル固有の顔位置を適用できる", () => {
  assert.match(renderer, /function applyLive2DFraming\(\)/);
  assert.match(renderer, /activeLive2dConfig\.iconFraming/);
  assert.match(renderer, /activeLive2dConfig\.layoutFraming\?\.\[currentLayout\]/);
  assert.match(renderer, /applyLive2DFraming\(\);/);
});

test("native-fullの独自2.5Dを配信画面で過大クロップしない", () => {
  assert.match(renderer, /\["native-bust", "native-full"\]\.includes\(manifest\.sourceFraming\)/);
  const selector =
    '.character[data-render-kind="mesh-rig"][data-source-framing="native-full"] .character-view > canvas';
  const override = styles.indexOf(selector);
  assert.ok(override >= 0, "native-fullの上書きが必要です");
  const rule = styles.slice(override, styles.indexOf("}", override) + 1);
  assert.match(rule, /width:auto!important/);
  assert.match(rule, /height:100%!important/);
  assert.match(rule, /left:50%/);
  assert.match(rule, /translateX\(-50%\)/);
  assert.doesNotMatch(rule, /260%/);
});

const meshRig = readFileSync(new URL("../../ui/mesh-rig.js", import.meta.url), "utf8");

test("配信メッシュは縦基準で縦横比を維持する", () => {
  const selector =
    '.pet-app[data-layout="stream"]:not(.is-booting) .character[data-render-kind="mesh-rig"] .character-view > canvas';
  const start = styles.indexOf(selector);
  assert.ok(start >= 0, "メッシュ描画の一般指定が必要です");
  const rule = styles.slice(start, styles.indexOf("}", start) + 1);
  assert.match(rule, /width:auto!important/);
  assert.match(rule, /height:min\(calc\(76vh \* var\(--stream-mesh-scale,2\.6\) \/ 2\.6\),var\(--mesh-source-css-height/);
  assert.doesNotMatch(rule, /width:var\(--stream-render-size/);
  assert.match(renderer, /--stream-mesh-scale/);
  assert.match(renderer, /applyMeshRigFraming\(\)/);
  assert.match(renderer, /setFraming\(scale\)/);
  assert.match(meshRig, /visibleSourceHeight = this\.app\.screen\.height \/ this\.textureFit \/ this\.framingScale/);
  assert.match(meshRig, /Math\.min\(visibleSourceHeight, bufferHeight\) \/ dpr/);
  assert.match(meshRig, /addEventListener\("resize", this\.refreshSourceLimit\)/);
  assert.match(meshRig, /removeEventListener\("resize", this\.refreshSourceLimit\)/);
});import { runInNewContext } from "node:vm";

test("メッシュ構図を変更・DPI移動しても原画とバッファの上限を超えず破棄時に解除する", () => {
  const listeners = new Map();
  const root = { devicePixelRatio: 1,
    addEventListener: (name, fn) => listeners.set(name, fn),
    removeEventListener: (name, fn) => { if (listeners.get(name) === fn) listeners.delete(name); },
  };
  runInNewContext(meshRig.replace("Object.freeze({ create })", "Object.freeze({ create, MeshRigController })"),
    { window: root, performance: { now: () => 0 } });
  const properties = new Map();
  const transforms = {};
  const app = { screen: { width: 285, height: 315 }, renderer: { resolution: 4.5 },
    stage: Object.fromEntries(["pivot", "position", "scale"].map(key => [key, { set: (...v) => { transforms[key] = v; } }])),
    ticker: { add() {}, stop() {}, remove() {} }, destroy() {},
  };
  const canvas = { style: { setProperty: (k, v) => properties.set(k, v) }, remove() {} };
  const controller = new root.PicoMeshRig.MeshRigController(app, canvas, {}, [], null, null, 0.1);
  assert.equal(properties.get("--mesh-source-css-height"), "1417.5px");
  controller.setFraming(2.6);
  assert.ok(Math.abs(parseFloat(properties.get("--mesh-source-css-height")) - 3150 / 2.6) < 1e-8);
  root.devicePixelRatio = 2;
  listeners.get("resize")();
  assert.ok(Math.abs(parseFloat(properties.get("--mesh-source-css-height")) - 3150 / 2.6 / 2) < 1e-8);
  controller.setFraming(1);
  assert.equal(transforms.scale[0], 1);
  assert.equal(properties.get("--mesh-source-css-height"), "708.75px");
  controller.destroy();
  assert.equal(listeners.size, 0);
});

test("Live2Dは形式別構図を優先して未指定座標を継承し明示0を維持する", () => {
  const functionBody = renderer.slice(renderer.indexOf("function applyLive2DFraming()"), renderer.indexOf("function desiredLive2DResolution()"));
  const result = {};
  const context = { currentLayout: "chat", LIVE2D_RENDER_WIDTH: 285, LIVE2D_RENDER_HEIGHT: 315,
    activeLive2dConfig: { scale: 2, offsetX: -20, offsetY: 30, iconFraming: { scale: 3, offsetY: 80 }, layoutFraming: { chat: { scale: 4, offsetY: 0 } } },
    live2dModel: { internalModel: { originalWidth: 285, originalHeight: 315 },
      scale: { set: v => { result.scale = v; } }, anchor: { set() {} },
      position: { set: (x, y) => { result.x = x; result.y = y; } } },
  };
  runInNewContext(functionBody + "applyLive2DFraming();", context);
  assert.deepEqual(result, { scale: 4, x: 122.5, y: 315 });
  context.currentLayout = "bar";
  runInNewContext(functionBody + "applyLive2DFraming();", context);
  assert.deepEqual(result, { scale: 3, x: 122.5, y: 395 });
});

test("native-fullのメッシュも配信では保存中の上半身倍率を適用する", () => {
  const source = renderer.slice(renderer.indexOf("function applyMeshRigFraming()"), renderer.indexOf("async function prepareMeshRig("));
  let scale;
  runInNewContext(source + "applyMeshRigFraming();", { currentLayout: "stream", characterZoom: 1.55, character: { dataset: { sourceFraming: "native-full" } }, meshRigController: { setFraming: v => { scale = v; } } });
  assert.equal(scale, 1.55);
});

test("メッシュ休止時に差分絵がなければ定義済み静止フレームへ退避する", () => {
  const source = renderer.slice(renderer.indexOf("function setAnimation("), renderer.indexOf("function frameDuration("));
  const calls = [];
  const context = { animations: { idle: { framePaths: ["source.png"] } }, animationName: "idle", frame: 0,
    lastFrameAt: 0, performance: { now: () => 0 }, character: { dataset: {} }, live2dActive: false, mmdController: null,
    meshRigController: {}, meshRigCanRender: () => false, rigAvailable: true, rigLayersAvailable: false,
    wakeAnimationLoop() {}, setMeshRigVisible: v => calls.push(["mesh",v]), preloadAnimation() {},
    startRig: () => { throw new Error("存在しない差分絵を表示してはいけない"); }, stopRig() {},
    displayPreloadedFrame: (name,index) => calls.push([name,index]),
  };
  runInNewContext(source + 'setAnimation("idle", true);', context);
  assert.deepEqual(calls, [["mesh",false],["idle",0]]);
});

test("マスコットのLive2D canvasは縮んだキャラ枠の高さを超えない", () => {
  // 入力欄を開くとキャラ枠は窓の下45%に縮む。固定398pxのまま下端へ置くと、はみ出した頭が
  // フキダシの下に潜った（操作部品の収まりだけを見る検査では合格していた）。
  const rule = styles.match(/\[data-layout="mascot"\] \.character-view > canvas \{([^}]*)\}/)?.[1];
  assert.ok(rule, "マスコットのcanvas規則が見つかりません");
  assert.match(rule, /height:min\(398px,100%\)!important/);
  assert.match(rule, /width:auto!important/);
  assert.doesNotMatch(rule, /height:\d+px!important/);
});

test("キャラを重ねて置く形式では入力と返答の文字・ボタンの居場所を空ける", () => {
  // 作業形式は通知ペインを畳むと、その下のキャラが中央の入力欄と送信・音声ボタンを覆った。
  const narrow = styles.match(/@media \(max-width:1100px\) \{([\s\S]*?)\n\}/)?.[1] || "";
  assert.match(narrow, /\[data-layout="workspace"\] \.bubble \{ padding-right:230px; \}/);
  assert.match(narrow, /\[data-layout="workspace"\] \.character \{[^}]*width:200px;/);
  // ウィジェットはキャラを入力欄の右下へ重ねるため、入力・返答の文字が潜らないよう右を空ける。
  assert.match(styles, /\[data-layout="widget"\] \.composer textarea \{[^}]*padding-right:108px;/);
  assert.match(styles, /\[data-layout="widget"\] \.reply \{[^}]*padding-right:100px;/);
  // 最小付近のバーは回答操作を入力欄の右上へ重ねる。その下へ文字を潜らせない。
  const tiny = styles.match(/@media \(max-height:280px\) \{([\s\S]*?)\n\}/)?.[1] || "";
  assert.match(tiny, /\[data-layout="bar"\] \.composer textarea \{ padding-right:min\(290px,50%\); \}/);
});

test("長身の画像キャラは既定構図を全身にしない（胸上の規則）", async () => {
  // 規則: 低頭身2Dは上半身、長身・実写・Live2Dは胸上が初期構図（README/SPEC）。
  // あおい・通常頭身むぎが defaultFraming.zoom=1（全身）を持ち、ペット形式で太ももまで映っていた。
  const { readdirSync, existsSync } = await import("node:fs");
  const root = new URL("../../characters/", import.meta.url);
  const offenders = [];
  for (const id of readdirSync(root)) {
    const file = new URL(`${id}/character.json`, root);
    if (!existsSync(file)) continue;
    const manifest = JSON.parse(readFileSync(file, "utf8"));
    if (manifest.live2d || manifest.mmd || manifest.bodyProportion === "compact") continue;
    const zoom = Number(manifest.defaultFraming?.zoom);
    if (Number.isFinite(zoom) && zoom <= 1) offenders.push(`${id}: zoom ${zoom}`);
  }
  assert.deepEqual(offenders, []);
});

test("ゲーム中のキャラは胸上の枠で切り取り、拡大を打ち消さない", () => {
  // 以前は拡大を scale(1) で打ち消し、縦長の枠から全身画像をはみ出させていたため、
  // 仕様の「横切れのないバストアップ」が全身表示になっていた。
  const box = styles.match(/\.pet-app\.is-playing-game\[data-layout\] \.character \{([^}]*)\}/)?.[1] || "";
  assert.match(box, /overflow:hidden;/);
  assert.match(box, /aspect-ratio:430 \/ 540;/);
  const view = styles.match(/body \.pet-app\.is-playing-game\[data-layout\] \.character \.character-view \{([^}]*)\}/)?.[1] || "";
  assert.match(view, /scale\(var\(--char-zoom,1\.55\)\)!important/);
  assert.doesNotMatch(view, /height:139%/);
  // 下の通常画面のボタンや返答がゲームの見出しの後ろへ透けないよう、背景は不透明にする。
  const overlay = styles.match(/\.game-overlay \{([^}]*)\}/)?.[1] || "";
  assert.doesNotMatch(overlay, /rgba\([^)]*,\s*\.\d+\)/);
  assert.match(styles, /\.pet-app\.is-playing-game \.online \{ display:none; \}|\.pet-app\.is-playing-game \.online \{/ );
});

test("配信の胸上原画メッシュは下端基準で置き、字幕の上で体を途切れさせない", () => {
  // 原画の可視画素で高さが頭打ちになると、上端基準では胸で切れた下端が舞台の途中に浮いた。
  const rule = styles.match(/\.pet-app\[data-layout="stream"\]:not\(\.is-booting\) \.character\[data-render-kind="mesh-rig"\]\[data-source-framing="native-bust"\] \.character-view > canvas \{([^}]*)\}/)?.[1] || "";
  assert.match(rule, /bottom:0;/);
  assert.match(rule, /transform-origin:50% 100%;/);
  assert.match(rule, /var\(--mesh-source-css-height,9999px\)/);
});

test("キャラ枠の中のハートと接続点は枠の内側に置き、枠はスクロールさせない", () => {
  // チャットのハート（left:88px）と接続点は62pxの丸枠の外にあり、切り取られて押せなかった。
  // hidden の枠はfocusやクリックで中身がスクロールし、丸アイコンの顔が横へずれたまま戻らなかった。
  const chatHeart = styles.match(/\[data-layout="chat"\] \.relationship-button \{([^}]*)\}/)?.[1] || "";
  assert.match(chatHeart, /right:2px; bottom:2px;/);
  const widgetHeart = styles.match(/\[data-layout="widget"\] \.relationship-button \{ right:auto;([^}]*)\}/)?.[1] || "";
  assert.match(widgetHeart, /bottom:2px;/);
  assert.match(styles, /\[data-layout="widget"\] \.character \{ display:block; overflow:hidden; overflow:clip; \}/);
  assert.match(styles, /border-radius:50%; overflow:hidden; overflow:clip;/);
  // 配信の拡大でキャラを操作UIより前へ出さない（「拡大」ボタンを覆って戻せなくなる）。
  assert.match(styles, /\[data-layout="stream"\] \.bubble\.fullscreen ~ \.character \{ z-index:2; \}/);
  assert.match(renderer, /relationshipButton\?\.focus\(\{ preventScroll: true \}\)/);
});
