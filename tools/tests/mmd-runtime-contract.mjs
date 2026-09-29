import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const source = readFileSync(new URL('../../ui/mmd-renderer.js', import.meta.url), 'utf8');
// vm内で作った値は原型が別になるため、構造比較の前に素のオブジェクトへ写す。
const plain = value => JSON.parse(JSON.stringify(value));
function profileApi() {
  const window = {};
  const context = vm.createContext({ window, document: { baseURI: 'http://localhost/', currentScript: null }, URL, DOMException });
  vm.runInContext(source, context);
  return window.PicoMMD;
}

test('MMDの全構図は頭頂から指定下端までを切り出し、細い保存枠でも左右を収める', () => {
  const { calculateFraming, framingAspect } = profileApi();
  const model = { low: 0, high: 20, eyeY: 16.8, eyeX: 1, eyeZ: 2, fov: Math.PI / 4 };
  for (const kind of ['bust', 'upper', 'full']) {
    for (const canvasAspect of [0.4, 0.9, framingAspect(kind), 2]) {
      const frame = calculateFraming({ ...model, kind, canvasAspect });
      assert.ok(frame.top > model.high, `${kind}: 頭頂の余白`);
      const expectedDepth = kind === 'bust' ? 0.4 : kind === 'upper' ? 0.65 : 1;
      assert.ok(Math.abs(frame.bottom - (kind === 'full' ? model.low : model.high - 20 * expectedDepth)) < 1e-9,
        `${kind}: 下端`);
      assert.ok(frame.viewWidth + 1e-9 >= frame.frameWidth, `${kind}/${canvasAspect}: 左右が切れる`);
      assert.ok(frame.viewHeight + 1e-9 >= frame.frameHeight, `${kind}/${canvasAspect}: 上下が切れる`);
      assert.ok(Math.abs(frame.viewHeight / (2 * Math.tan(model.fov / 2)) - frame.radius) < 1e-9);
      assert.deepEqual(plain(frame.target), { x: 1, y: (frame.top + frame.bottom) / 2, z: 2 });
    }
  }
  const saved = calculateFraming({ ...model, kind: 'bust', canvasAspect: 639.9 / 711 });
  assert.ok(saved.viewWidth >= saved.frameWidth);
  assert.ok(saved.viewHeight > saved.frameHeight);
});

test('低頭身MMDは胸元と腰まで深く映し、標準頭身の範囲は維持する', () => {
  const { calculateFraming } = profileApi();
  const model = { low: 0, high: 20, eyeY: 16.8, canvasAspect: 1.2, fov: Math.PI / 4 };
  for (const [kind, standardRange, compactDepth] of [['bust', [0.38, 0.4], 0.62], ['upper', [0.6, 0.68], 0.82]]) {
    const standard = calculateFraming({ ...model, kind });
    const compact = calculateFraming({ ...model, kind, proportion: 'compact' });
    assert.ok((model.high - standard.bottom) / 20 >= standardRange[0]);
    assert.ok((model.high - standard.bottom) / 20 <= standardRange[1]);
    assert.equal(compact.bottom, model.high - 20 * compactDepth);
  }
});

test('MMDの状態別ポーズはモデル固有値を正規化し、基底姿勢へ加算する', () => {
  const api = profileApi();
  const poses = api.normalizeProfiles({
    happy: { '右腕': [10, 20, 200], broken: [1, 2] },
    success: { '右腕': [4, 5, 6] },
  }, api.normalizePoseMap);
  assert.deepEqual(JSON.parse(JSON.stringify(poses)), {
    happy: { '右腕': [10, 20, 180] },
    success: { '右腕': [4, 5, 6] },
  });
  assert.deepEqual(JSON.parse(JSON.stringify(api.profileFor(poses, 'success'))), { '右腕': [4, 5, 6] });
  assert.deepEqual(JSON.parse(JSON.stringify(api.profileFor(poses, 'pleased'))), { '右腕': [10, 20, 180] });
  assert.deepEqual(JSON.parse(JSON.stringify(api.profileFor(poses, 'shy'))), { '右腕': [10, 20, 180] });
  assert.deepEqual(JSON.parse(JSON.stringify([...api.combinePose({ '右腕': [0, 0, 35] }, poses.happy)])), [['右腕', [10, 20, 180]]]);
});

test('MMDの複合表情はウェイトを制限し、自動まばたき停止を保持する', () => {
  const api = profileApi();
  const profile = api.normalizeExpressionProfile({
    morphs: { 'にこり': 0.6, '赤面': 2, invalid: '1' },
    autoBlink: false,
  });
  assert.deepEqual(JSON.parse(JSON.stringify(profile)), {
    morphs: { 'にこり': 0.6, '赤面': 1 },
    autoBlink: false,
  });
});

test('MMDの表示確定は目標の姿勢と表情へ一度で到達する', () => {
  const api = profileApi();
  // 撮影・表示確定は補間を待たない。1フレームだけ進める既定値では目標へ届かないことも併せて固定する。
  assert.equal(api.blendFactor(1 / 30, true), 1);
  assert.equal(api.blendFactor(10, true), 1);
  assert.ok(api.blendFactor(1 / 30, false) < 0.5);
  assert.ok(api.blendFactor(10, false) < 0.65);
  // 目を閉じる自動まばたきの山に当たっていても、確定フレームは半目・閉眼で残さない。
  assert.equal(api.blinkWeight({ time: 0.08, motion: true, settle: true }), 0);
  assert.ok(api.blinkWeight({ time: 0.08, motion: true }) > 0.9);
});

test('MMDの表情はまばたきモーフを含めて補間し、笑顔モーフが無いモデルの瞬きを止めない', () => {
  const api = profileApi();
  const profiles = api.normalizeProfiles({ shy: { morphs: { 'まばたき': 1, '赤面': 0.8 } } }, api.normalizeExpressionProfile);
  const shy = api.expressionTargets('shy', { profiles, smile: '笑い' });
  assert.deepEqual(plain(shy), { morphs: { 'まばたき': 1, '赤面': 0.8 }, suppressBlink: false });
  // 設定表情から離れた直後は、閉眼値が残っている間だけ自動まばたきへ戻さない。
  assert.equal(api.blinkWeight({ expression: 1, time: 2, motion: true, configured: true }), 1);
  assert.equal(api.blinkWeight({ expression: 0.6, time: 2, motion: true }), 0.6);
  assert.equal(api.blinkWeight({ expression: 0, time: 2, motion: true }), 0);
  // 笑い目モーフだけのモデルは閉眼になるため瞬きを止め、笑顔モーフが無いモデルは止めない。
  assert.deepEqual(plain(api.expressionTargets('success', { smile: '笑い' })), { morphs: { '笑い': 1 }, suppressBlink: true });
  assert.deepEqual(plain(api.expressionTargets('success', { smileCorners: ['口角上げ'] })), { morphs: { '口角上げ': 0.65 }, suppressBlink: false });
  assert.deepEqual(plain(api.expressionTargets('success', {})), { morphs: {}, suppressBlink: false });
  // 状態名がObject.prototypeの名前でも継承値を別名として拾わない。
  assert.deepEqual(plain(api.expressionTargets('constructor', { smile: '笑い' })), { morphs: {}, suppressBlink: false });
  assert.equal(api.profileFor(profiles, '__proto__'), null);
});

test('製品CSPはMMDの復号画像とTauriのIPCを許可し外部サイトを許可しない', () => {
  const config = JSON.parse(readFileSync(new URL('../../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
  const csp = config.app.security.csp;
  const image = csp.split(';').find(value => value.trim().startsWith('img-src'));
  assert.match(image, /\bblob:/);
  assert.doesNotMatch(image, /https:|\*/);
  assert.match(csp, /connect-src[^;]*http:\/\/ipc\.localhost/);
});
function harness(ready = true, Engine = class { constructor() { throw new Error('LOCAL_ASSET_ACCEPTED'); } }) {
  const scripts = [];
  let engines = 0;
  const window = ready ? { PicoMMDVendor: { Engine: class { constructor() { engines++; return new Engine(); } } } } : {};
  const document = {
    baseURI: 'http://127.0.0.1:8080/ui/index.html',
    currentScript: { src: 'http://127.0.0.1:8080/ui/mmd-renderer.js' },
    createElement: () => ({ remove() {} }), head: { append: script => scripts.push(script) },
  };
  const context = vm.createContext({ window, document, URL, DOMException });
  vm.runInContext(source, context);
  return { window, scripts, engines: () => engines };
}

test('外部URLやクエリ付きPMXをGPU作成前に拒否する', async () => {
  const h = harness();
  for (const modelUrl of ['https://example.com/model.pmx', '/model.pmx?url=external', '/model.pmx#external', '/model.vrm']) {
    await assert.rejects(h.window.PicoMMD.create({}, { modelUrl }), /同じアプリ内のPMX/);
  }
  assert.equal(h.engines(), 0);
});

test('ランタイム不在は明示エラーにし次回の再試行を妨げない', async () => {
  const h = harness(false);
  const first = h.window.PicoMMD.create({}, { modelUrl: '/model.pmx' });
  h.scripts[0].onerror();
  await assert.rejects(first, /cargo xtask setup mmd/);
  const second = h.window.PicoMMD.create({}, { modelUrl: '/model.pmx' });
  assert.equal(h.scripts.length, 2);
  h.scripts[1].onerror();
  await assert.rejects(second, /cargo xtask setup mmd/);
});

test('Tauriが公開する限定asset URLは許可し類似外部URLを拒否する', async () => {
  const h = harness();
  for (const modelUrl of ['asset://localhost/C:/characters/みこ.pmx', 'http://asset.localhost/C:/characters/ころね.pmx']) {
    await assert.rejects(h.window.PicoMMD.create({}, { modelUrl }), /LOCAL_ASSET_ACCEPTED/);
  }
  assert.equal(h.engines(), 2);
  for (const modelUrl of ['asset://evil/model.pmx', 'file:///model.pmx', 'http://asset.localhost.evil/model.pmx', 'http://asset.localhost:8080/model.pmx', 'http://user@asset.localhost/model.pmx']) {
    await assert.rejects(h.window.PicoMMD.create({}, { modelUrl }), /同じアプリ内のPMX/);
  }
  assert.equal(h.engines(), 2);
});

test('ランタイム読込待ちの中断で後からGPUを作らない', async () => {
  const h = harness(false);
  const controller = new AbortController();
  const pending = h.window.PicoMMD.create({}, { modelUrl: '/model.pmx', signal: controller.signal });
  controller.abort();
  h.window.PicoMMDVendor = { Engine: class { constructor() { assert.fail('中断済みGPUを作成'); } } };
  h.scripts[0].onload();
  await assert.rejects(pending, { name: 'AbortError' });
});
