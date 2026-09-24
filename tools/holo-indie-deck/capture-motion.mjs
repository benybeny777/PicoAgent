// 企画書用に実フロントの描画を録画する。AI応答は撮影用の固定文、音声は収録しない。
// 元モデルは読み取り配信だけとし、既存撮影ツールのモックと起動処理を再利用する。
import http from 'node:http';
import { readFile, writeFile, mkdir, mkdtemp, rm } from 'node:fs/promises';
import { join, resolve, dirname, extname, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const sourceRoot = process.env.PICO_CAPTURE_SOURCE || root;
const uiRoot = join(sourceRoot, 'ui');
const modelRoot = join(process.env.APPDATA, 'PicoAgent/characters/miko-mmd');
const output = join(root, 'docs/images');
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE
  ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright');
const run = promisify(execFile);
const ffmpeg = process.env.SHOOT_FFMPEG || 'ffmpeg';
const source = await readFile(join(sourceRoot, 'tools/screenshots/shoot.mjs'), 'utf8');
function section(start, end) {
  const first = source.indexOf(start);
  if (first < 0) throw new Error(`撮影処理の開始位置が見つかりません: ${start}`);
  const last = source.indexOf(end, first);
  if (last < 0) throw new Error(`撮影処理の終了位置が見つかりません: ${end}`);
  return source.slice(first, last);
}
// 既存撮影のひより専用合成音は使わず、今回の口パクは下の模擬振幅だけで制御する。
const originalMockSource = section('const mock =', '// ---------------------------------------------------------------- 撮影');
const hiyoriVoiceCondition = ' || ${JSON.stringify(characterId)} === "hiyori"';
if (!originalMockSource.includes(hiyoriVoiceCondition)) {
  throw new Error('既存撮影のひより合成音条件が変更されています。無音化の処理を確認してください。');
}
const mockSource = originalMockSource.replace(hiyoriVoiceCondition, '');
const mock = new Function(`${mockSource} return mock;`)();
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.png': 'image/png', '.webp': 'image/webp', '.jpg': 'image/jpeg', '.svg': 'image/svg+xml' };
const server = http.createServer(async (req, res) => {
  try {
    const requested = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
    const prefix = '/assets/characters/miko-mmd/';
    const base = requested.startsWith(prefix) ? modelRoot : uiRoot;
    const suffix = requested.startsWith(prefix) ? requested.slice(prefix.length) : requested.slice(1);
    const path = resolve(base, suffix);
    if (!path.startsWith(resolve(base) + sep)) throw new Error('配信範囲の外です');
    let body = await readFile(path);
    if (requested === '/assets/characters/index.json') {
      const catalog = JSON.parse(body);
      catalog.characters.push({ id: 'miko-mmd', name: 'さくらみこ', manifest: 'miko-mmd/character.json' });
      body = Buffer.from(JSON.stringify(catalog));
    }
    res.writeHead(200, { 'content-type': mime[extname(path)] || 'application/octet-stream' });
    res.end(body);
  } catch { res.writeHead(404); res.end(); }
});
let browser;
let temp;
try {
  await mkdir(output, { recursive: true });
  await mkdir(join(root, 'temp'), { recursive: true });
  temp = await mkdtemp(join(root, 'temp/holo-motion-'));
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  browser = await chromium.launch({ args: ['--enable-unsafe-swiftshader'] });
  const reply = 'にゃっはろ～！今日は何をする？ 予定の整理も、調べものも、いっしょに進めようにぇ。';
  const history = [];
  const openScene = new Function('browser', 'mock', 'PORT', 'HISTORY', 'REPLY', `${section('async function openScene(', 'async function validateAiGames(')} return openScene;`)(browser, mock, server.address().port, history, reply);
  for (const scene of [
    { id: 'miko', characterId: 'miko-mmd', layout: 'stream', width: 1280, height: 720, seconds: 7 },
    { id: 'miko', characterId: 'miko-mmd', layout: 'pet', width: 900, height: 600 },
    { id: 'miko', characterId: 'miko-mmd', layout: 'chat', width: 520, height: 650 },
    { id: 'hiyori', characterId: 'hiyori', layout: 'pet', width: 900, height: 600, seconds: 6, fps: 60 },
  ]) {
    console.log(`撮影準備: ${scene.id}/${scene.layout}`);
    const page = await openScene({ ...scene, voice: false,
      reply: scene.id === 'hiyori' ? 'こんにちは！予定の整理も、調べものも、いっしょに進めましょう。' : reply });
    try {
      if (scene.layout !== 'chat') await page.waitForFunction(id => id === 'miko'
        ? typeof mmdController !== 'undefined' && mmdController
        : typeof live2dActive !== 'undefined' && live2dActive, scene.id, { timeout: 30000 });
      await page.waitForTimeout(1200);
      await page.evaluate(value => {
        setInputValue(value);
        document.getElementById('composer').dispatchEvent(new Event('submit', { cancelable: true }));
      }, '今日やることを一緒に整理して');
      await page.waitForFunction(() => typeof busy !== 'undefined' && !busy, null, { timeout: 20000 });
      await page.waitForTimeout(5500);
      if (scene.id === 'miko') await page.screenshot({ path: join(output, `holo-${scene.id}-${scene.layout}.png`) });
      if (!scene.seconds) continue;
      const now = await page.evaluate(() => Date.now());
      await page.clock.pauseAt(new Date(now + 10000));
      const fps = scene.fps || 30;
      const frames = Math.round(scene.seconds * fps);
      const dir = join(temp, scene.id);
      await mkdir(dir);
      for (let index = 0; index < frames; index++) {
        await page.evaluate(({ index, id, fps }) => {
          const t = index / fps;
          setAnimation(t < 2 ? 'idle' : t < 3.5 ? 'thinking' : t < 5 ? 'success' : 'idle');
          // ひよりの口は撮影用の模擬振幅。音声データや本人の声は動画へ入れない。
          if (id === 'hiyori' && typeof live2dMouth !== 'undefined') live2dMouth = t < 2 ? Math.max(0, Math.sin(t * 13)) * 0.7 : 0;
        }, { index, id: scene.id, fps });
        await page.clock.runFor(Math.round((index + 1) * 1000 / fps) - Math.round(index * 1000 / fps));
        const frame = await page.screenshot();
        await writeFile(join(dir, `f-${String(index).padStart(4, '0')}.png`), frame);
        if (index === 0) await writeFile(join(output, `holo-${scene.id}-motion-poster.png`), frame);
        if (scene.id === 'hiyori' && [0, 7, 21, 60, 180].includes(index)) {
          await writeFile(join(root, 'temp', `holo-hiyori-check-${index}.png`), frame);
        }
        if (index % fps === 0) console.log(`録画: ${scene.id} ${index / fps}/${scene.seconds}秒`);
      }
      await run(ffmpeg, ['-y', '-loglevel', 'error', '-framerate', String(fps), '-i', join(dir, 'f-%04d.png'), '-c:v', 'libx264', '-preset', 'medium', '-crf', '18', '-pix_fmt', 'yuv420p', '-an', '-movflags', '+faststart', join(output, `holo-${scene.id}-motion.mp4`)]);
      console.log(`動画完成: holo-${scene.id}-motion.mp4`);
    } finally { await page.close(); }
  }
} finally {
  try { await browser?.close(); }
  finally {
    if (server.listening) await new Promise(resolve => server.close(resolve));
    if (temp) await rm(temp, { recursive: true, force: true });
  }
}
