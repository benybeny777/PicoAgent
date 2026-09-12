// 明示セットアップ専用。アプリ起動・通常ビルドから取得しない。
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile, writeFile, mkdir, rename, rm, stat } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import vm from 'node:vm';
const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '../..');
const output = path.join(root, 'ui/vendor/mmd');
const stage = path.join(root, 'temp/mmd-runtime-build');
await mkdir(stage, { recursive: true });
const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm';
const install = spawnSync(npm, ['ci', '--ignore-scripts', '--no-audit', '--no-fund'], {
  cwd: here, stdio: 'inherit', shell: process.platform === 'win32', windowsHide: true,
});
if (install.error || install.status !== 0) throw new Error('MMD依存の固定取得に失敗しました', { cause: install.error });
const { build } = await import('esbuild');
await build({ entryPoints: [path.join(here, 'entry.mjs')], outfile: path.join(stage, 'mmd-runtime.js'),
  bundle: true, format: 'iife', globalName: 'PicoMMDVendor', platform: 'browser', target: 'es2022',
  minify: true, legalComments: 'eof', metafile: true, logLevel: 'info' });
// npmのライフサイクルスクリプトは不要。esbuildはnpmが選ぶOS別optionalDependencyの実体を直接使う。
const bundle = await readFile(path.join(stage, 'mmd-runtime.js'), 'utf8');
if (!bundle.includes('PicoMMDVendor')) throw new Error('MMDランタイムの公開入口がありません');
new vm.Script(bundle, { filename: 'mmd-runtime.js' });
const packages = ['babylon-mmd', '@babylonjs/core', 'esbuild'];
let licenses = '';
for (const name of packages) {
  const base = path.join(here, 'node_modules', name);
  const pkg = JSON.parse(await readFile(path.join(base, 'package.json'), 'utf8'));
  let license;
  for (const file of ['LICENSE', 'license.md', 'LICENSE.md']) {
    try { license = await readFile(path.join(base, file), 'utf8'); break; } catch {}
  }
  if (!license) throw new Error(`${name}のライセンスが見つかりません`);
  licenses += `\n=== ${name} ${pkg.version} (${pkg.license}) ===\n${license}\n`;
}
// GitのLF正規化後も固定ハッシュが一致するよう、依存配布物のCRLFを揃える。
await writeFile(path.join(stage, 'LICENSES.txt'), licenses.replaceAll('\r\n', '\n'));
const digest = async file => createHash('sha256').update(await readFile(file)).digest('hex');
const metadata = { schemaVersion: 1, packages: { 'babylon-mmd': '1.3.0', '@babylonjs/core': '9.26.0', esbuild: '0.28.2' },
  lockSha256: await digest(path.join(here, 'package-lock.json')),
  files: { 'mmd-runtime.js': await digest(path.join(stage, 'mmd-runtime.js')), 'LICENSES.txt': await digest(path.join(stage, 'LICENSES.txt')) } };
await writeFile(path.join(stage, 'runtime.json'), JSON.stringify(metadata, null, 2) + '\n');
// Windowsでも既存ディレクトリへのrenameに頼らず、検証済み一式を入れ替える。
const backup = path.join(root, 'temp/mmd-runtime-previous');
const exists = async file => { try { await stat(file); return true; } catch (error) { if (error.code === 'ENOENT') return false; throw error; } };
if (await exists(backup)) throw new Error('前回のMMD退避が残っています。temp/mmd-runtime-previous の復旧状態を確認してください');
await mkdir(path.dirname(output), { recursive: true });
const hadPrevious = await exists(output);
if (hadPrevious) await rename(output, backup);
try { await rename(stage, output); }
catch (error) {
  if (hadPrevious) await rename(backup, output);
  throw error;
}
if (hadPrevious) await rm(backup, { recursive: true });
console.log('MMDランタイムを ui/vendor/mmd へ配置しました');
