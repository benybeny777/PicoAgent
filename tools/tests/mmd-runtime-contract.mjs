import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const source = readFileSync(new URL('../../ui/mmd-renderer.js', import.meta.url), 'utf8');
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
