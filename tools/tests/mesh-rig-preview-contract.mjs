import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const preview = readFileSync(
  new URL("../mesh-rig-preview/index.html", import.meta.url),
  "utf8",
);
const renderer = readFileSync(new URL("../../ui/mesh-rig.js", import.meta.url), "utf8");
const appRenderer = readFileSync(new URL("../../ui/character/mesh.js", import.meta.url), "utf8");
const previewLauncher = readFileSync(
  new URL("../../scripts/open-character-preview.ps1", import.meta.url),
  "utf8",
);
const config = readFileSync(new URL("../../src-tauri/src/config.rs", import.meta.url), "utf8");

test("WindowsのBAT入口はcmdの文字コードに依存しない", () => {
  for (const name of [
    "PicoAgentを起動.bat",
    "キャラクター確認画面を開く.bat",
    "関係者版インストーラをビルド.bat",
  ]) {
    const bytes = readFileSync(new URL(`../../${name}`, import.meta.url));
    assert.ok(
      bytes.every((byte) => byte < 0x80),
      `${name}はASCIIだけでPowerShell入口を呼ぶ`,
    );
    assert.match(bytes.toString("ascii"), /show-powershell7-required\.ps1/);
  }
});

test("Windowsの確認入口は2.5D・MMD・登録済みLive2Dを選べる", () => {
  assert.match(previewLauncher, /'2d', 'live2d', 'miko-mmd', 'korone-mmd'/);
  assert.match(previewLauncher, /PSObject\.Properties\['live2d'\]/);
  assert.match(previewLauncher, /PICO_PREVIEW_CHARACTER/);
  assert.match(previewLauncher, /& cargo xtask dev/);
  const override = config.indexOf('std::env::var("PICO_PREVIEW_CHARACTER")');
  const persisted = config.indexOf("self.values.lock().unwrap().get(key)", override);
  assert.ok(override >= 0 && persisted > override, "確認対象は保存値より先に、その起動中だけ解決する");
});

test("関係者版ビルドのダブルクリック入口は結果を読むまで閉じない", () => {
  const launcher = readFileSync(
    new URL("../../関係者版インストーラをビルド.bat", import.meta.url),
    "ascii",
  );
  assert.match(launcher, /set "PICOAGENT_BUILD_EXIT_CODE=%ERRORLEVEL%"/);
  assert.match(launcher, /wait-before-close\.ps1/);
  assert.match(launcher, /exit \/b %PICOAGENT_BUILD_EXIT_CODE%/);
});

test("確認画面はあおい・むぎ2種・りん・まい・セナ・実写を本番メッシュ描画で切り替える", () => {
  assert.match(preview, /value="aoi"/);
  assert.match(preview, /value="mugi"/);
  assert.match(preview, /value="mugi">むぎ（SD）<\/option>/);
  assert.match(preview, /value="mugi-normal">むぎ（通常頭身）<\/option>/);
  assert.match(preview, /value="photoreal"/);
  for (const id of ["rin", "mai", "sena"]) assert.match(preview, new RegExp(`value="${id}"`));
  assert.match(preview, /characters\/aoi/);
  assert.match(preview, /characters\/mugi/);
  assert.match(preview, /characters\/mugi-normal-lvs/);
  assert.match(preview, /characters\/photoreal-prototype/);
  assert.doesNotMatch(preview, /品質NG: 現在の生成PSD/);
  assert.match(preview, /window\.PicoMeshRig\.create/);
});

test("LocalVTuberStudio schema 3を中間形式へ書き換えず読む", () => {
  assert.match(renderer, /sourceRig\.schema_version !== 3/);
  assert.match(renderer, /sourceRig\.profile !== "lvs-anime25d-v1"/);
  assert.match(renderer, /sourceRig\.scene_graph/);
  assert.match(renderer, /sourceRig\.layers\.mouth_closed/);
  assert.match(renderer, /eyeAperture\(sourceRig\.layers\.left_eye_base/);
  assert.match(renderer, /openness <= 0\.55 && images\.has\(`\$\{side\}_eye_closed`\)/, "半閉眼以下は原画の閉じ目素材へ戻す");
  assert.match(renderer, /openBoost = 1 \+ smoothstep\(0\.62, 1, open\) \* 0\.18/);
  assert.match(renderer, /if \(openness >= 1[^\n]+return/);
  assert.match(renderer, /ctx\.clearRect\(left, top, right - left, bottom - top\)/, "閉じ目素材の範囲だけを置換する");
  assert.match(renderer, /draw\(`\$\{side\}_eye_base`, base\)/);
  assert.match(renderer, /const aperture = base\.eye_aperture/);
  assert.match(renderer, /ctx\.clip\(\);\s*draw\(`\$\{side\}_eye_base`/);
  assert.doesNotMatch(renderer, /globalCompositeOperation = "copy"/);
  assert.match(renderer, /rootUrl\}\/\$\{lvsPartPath\(name\)\}/);
  assert.match(renderer, /validateLvsSecondaryMotion\(sourceRig\)/);
  assert.match(renderer, /secondaryMotion: secondaryHair \? \{ hair: secondaryHair \} : null/);
  assert.match(renderer, /springStep\(/);
  assert.match(renderer, /this\.secondaryHair\.frequency_hz/);
  assert.match(renderer, /this\.secondaryHair\.damping_ratio/);
  assert.match(renderer, /this\.secondaryHair\.strength/);
});

test("確認画面は目・視線・5母音・表情・全動作・腕の操作を保つ", () => {
  assert.match(preview, /data-wink="L"/);
  assert.match(preview, /data-wink="R"/);
  assert.match(preview, /id="gazeX"/);
  assert.match(preview, /id="gazeY"/);
  for (const label of ["あ", "い", "う", "え", "お", "笑顔", "困り顔", "考え顔", "照れ", "聞く"]) {
    assert.ok(preview.includes(`>${label}<`), `${label}の確認操作が必要です`);
  }
  assert.match(preview, /id="arm"/);
  assert.match(renderer, /setEyeOpen\(left, right = left\)/);
  assert.match(renderer, /apertureWidth \* 0\.08/);
  assert.match(renderer, /setExpression\(name\)/);
  // 表情の固定値は共有の `EXPRESSION_POSES` が持つ（描画ループから何度も引くため）。
  for (const expression of ["happy", "concern", "thinking"]) {
    assert.match(
      renderer,
      new RegExp(`${expression}: Object\\.freeze\\(\\{ eye: 1,`),
      `${expression}は表情選択だけで半目にしない`,
    );
  }
  // 表情ごとの値は読み取り専用で共有する（毎フレーム作り直さない）。
  assert.match(renderer, /const EXPRESSION_POSES = Object\.freeze\(\{/);
  assert.match(renderer, /expressionPose\(\) \{\s*\n\s*return EXPRESSION_POSES\[this\.expression\] \|\| EXPRESSION_POSES\.neutral;/);
  // 顔の描き直し判定は毎フレーム通る。比較のために配列や文字列を作らない。
  assert.doesNotMatch(renderer, /\.map\(value => value\.toFixed\(3\)\)\.join/);
  assert.match(renderer, /const next = face\.keyScratch \|\|= new Int32Array\(7\);/);
  assert.match(renderer, /if \(last && last\.every\(\(value, index\) => value === next\[index\]\)\) return;/);
  // 初回は必ず描く（まだ値が無いことを null で表す）。
  assert.match(renderer, /lastKey: null, keyScratch: null/);
  // 形式の判定でも毎フレーム配列を作らない。
  assert.doesNotMatch(renderer, /\["anime25d-v1", "lvs-anime25d-v1"\]\.includes/);
  assert.doesNotMatch(preview, /data-motion="(?:wake|sleep)"/, "不要な起床・睡眠動作を確認画面へ戻さない");
  assert.doesNotMatch(renderer, /this\.expression === "sleep"|\/sleep\//, "独自2.5Dへ寝顔専用分岐を戻さない");
  assert.match(renderer, /shy: Object\.freeze\(\{ eye: 1,[\s\S]{0,160}blush: 1/, "照れ表情は開眼と頬染めを使う");
  assert.match(renderer, /const eyeOpen = this\.eyeOpen \* expression\.eye;/, "旧2.5Dでも寝顔の閉眼係数を適用する");
  assert.match(renderer, /this\.rig\.expressionSadMode !== "disabled"/, "不自然な旧悲しみ差分をパック設定で無効化できる");
  assert.match(renderer, /this\.rig\.expressionHappyMode !== "disabled"/, "不自然な旧笑顔差分をパック設定で無効化できる");
  assert.match(renderer, /expressionDisplacement\(this\.rig, globalX, globalY, expression, true\)/, "旧実写リグにも眉と口角の表情を反映する");
  assert.match(renderer, /item\.role === "face" \|\| item\.role === "residual"/, "統合顔レイヤーにも眉の表情を反映する");
  assert.match(preview, /controller\.setEyeOpen\(0, 0\); await wait\(220\)/, "完全閉眼を保持する");
});

test("実画面検査は全表情と全動作を記録する", () => {
  const checker = readFileSync(new URL("../check-lvs-character.mjs", import.meta.url), "utf8");
  assert.match(checker, /expressions: \[\]/);
  assert.match(checker, /motions: \[\]/);
  assert.match(checker, /arms: \[\]/);
  assert.match(checker, /\[\["open", 0\], \["half", 0\.5\], \["closed", 1\]\]/);
  assert.match(checker, /captureUrl\.searchParams\.set\("autoBlink", "off"\)/);
  assert.match(preview, /query\.get\("autoBlink"\) !== "off"/);
  assert.match(preview, /manifest\.meshRig\.assetVersion \|\| manifest\.assetVersion/);
  assert.match(appRenderer, /assetVersion: pack\.meshRig\.assetVersion \|\| pack\.assetVersion,/);
  assert.match(renderer, /createBlushOverlay\(app, rig, originX, originY, textureFit\)/);
  assert.match(renderer, /globalCompositeOperation = "destination-out"/);
  assert.match(renderer, /armClearance = Math\.max\(2,/);
  for (const label of ["通常", "笑顔", "困り顔", "考え顔", "照れ", "待機", "会話", "考え中", "よろこび", "困り", "聞く"]) {
    assert.ok(checker.includes(`"${label}"`), `${label}を実画面検査へ含める`);
  }
  assert.match(checker, /report\.unique\.expressions < 6/);
  assert.match(checker, /report\.unique\.motions < 6/);
});

test("キャラクター切替では破棄されたcanvasを再利用しない", () => {
  const destroy = preview.indexOf("controller?.destroy()");
  const create = preview.indexOf('document.createElement("canvas")', destroy);
  const render = preview.indexOf("window.PicoMeshRig.create", create);
  assert.ok(destroy >= 0 && create > destroy && render > create);
});

test("省モーションでもメッシュを消さず受動動作だけを固定する", () => {
  assert.match(renderer, /setReducedMotion\(reduced\)/);
  assert.match(renderer, /this\.updateMesh\(this\.reducedMotion \? this\.startedAt : now, dt\)/);
  // 「アニメーションを減らす」でもメッシュ自体は止めず、描画器へ状態を渡して動きだけを抑える。
  assert.match(appRenderer, /this\.controller\.setReducedMotion\(this\.reducedMotion\);/);
  assert.match(appRenderer, /setReducedMotion\(reduced\) \{\s*\n\s*this\.reducedMotion = Boolean\(reduced\);\s*\n\s*this\.controller\?\.setReducedMotion\(this\.reducedMotion\);/);
});

test("独自2.5Dは首境界を開く局所頭部変形をせず一体で呼吸する", () => {
  assert.doesNotMatch(renderer, /const isNeck = name === "neck"/);
  assert.doesNotMatch(renderer, /motionPose\.headX \* headWeight/);
  assert.match(renderer, /y -= breath \* 2 \* faceScale/);
  assert.match(renderer, /if \(item\.role === "torso"\)/);
  assert.match(renderer, /torso: \{ x0: torso\[0\], y0: torso\[1\], x1: torso\[2\], y1: torso\[3\] \}/);
  assert.match(renderer, /this\.chestFollow\.value - this\.speechBody\.value/);
  assert.match(renderer, /verticalPosition < 0\.58/);
});
