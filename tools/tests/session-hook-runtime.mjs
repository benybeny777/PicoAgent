import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, chmodSync, rmSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { spawnSync } from 'node:child_process';

const root = resolve(import.meta.dirname, '../..');
const bash = process.env.BASH_TEST_BINARY || (process.platform === 'win32' ? 'C:/Program Files/Git/bin/bash.exe' : 'bash');
const entries = ['.claude/hooks/session-start.sh', '.codex/hooks/session-start.sh'];
const preparation = readFileSync(join(root, 'tools/session-start.sh'), 'utf8');

// 実hookを隔離配置し、外部コマンドだけ偽物にする。取得・apt・Cargoは実行しない。
function run(entry, { fail = '', missing = false, remote = 'true', skip = '', projectUnset = false, source = preparation } = {}) {
  mkdirSync(join(root, 'temp'), { recursive: true });
  const directory = mkdtempSync(join(root, 'temp/session-hook-'));
  try {
    for (const part of ['tools', '.claude/hooks', '.codex/hooks', 'bin']) mkdirSync(join(directory, part), { recursive: true });
    writeFileSync(join(directory, 'tools/session-start.sh'), source);
    for (const path of entries) writeFileSync(join(directory, path), readFileSync(join(root, path)));
    const mock = `#!/bin/bash
if [ "$PWD" != "$EXPECTED_PROJECT_DIR" ]; then exit 31; fi
case "$MOCK_NAME" in
 id) printf '0\\n'; exit 0;;
 dpkg-query) if [ "$MISSING" = 1 ]; then exit 1; fi; printf 'install ok installed'; exit 0;;
esac
printf '%s %s\\n' "$MOCK_NAME" "$*" >> "$TRACE_FILE"
if [ "$MOCK_NAME $*" = "$FAIL_STEP" ]; then exit 23; fi
if [ "$MOCK_NAME" = cargo ] && [ "$*" = "$FAIL_STEP" ]; then exit 23; fi
if [ "$MOCK_NAME" = apt-get ] && [ "$1" = "$FAIL_STEP" ]; then exit 23; fi
`;
    writeFileSync(join(directory, 'mock.sh'), mock);
    for (const name of ['cargo', 'apt-get', 'id', 'dpkg-query']) {
      const path = join(directory, 'bin', name);
      writeFileSync(path, `#!/bin/bash\nexport MOCK_NAME=${name}\nexec bash "$MOCK_SCRIPT" "$@"\n`);
      chmodSync(path, 0o755);
    }
    const trace = join(directory, 'trace');
    writeFileSync(trace, '');
    const posix = path => process.platform === 'win32' ? path.replaceAll('\\', '/').replace(/^([A-Za-z]):/, (_, drive) => `/${drive.toLowerCase()}`) : path;
    const result = spawnSync(bash, ['-c', 'export PATH="$MOCK_BIN:$PATH"; exec bash "$HOOK"'], {
      cwd: directory,
      encoding: 'utf8',
      env: { ...process.env, CLAUDE_CODE_REMOTE: remote, CLAUDE_PROJECT_DIR: projectUnset ? '' : posix(directory), SKIP_PREBUILD: skip,
        EXPECTED_PROJECT_DIR: posix(directory),
        MOCK_BIN: posix(join(directory, 'bin')), MOCK_SCRIPT: posix(join(directory, 'mock.sh')),
        TRACE_FILE: posix(trace), HOOK: posix(join(directory, entry)), FAIL_STEP: fail, MISSING: missing ? '1' : '0' },
    });
    if (result.error) throw result.error;
    return { status: result.status, output: result.stdout + result.stderr, trace: readFileSync(trace, 'utf8').trim().split('\n').filter(Boolean) };
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

const expectedPreparation = [
  'cargo xtask sync-characters', 'cargo xtask setup pixi', 'cargo xtask build-ui',
  'cargo fetch --manifest-path src-tauri/Cargo.toml --quiet',
  'cargo test --lib --manifest-path src-tauri/Cargo.toml --no-run --quiet',
];
function assertPrepared(result) {
  assert.equal(result.status, 0, result.output);
  assert.deepEqual(result.trace, expectedPreparation);
  assert.match(result.output, /準備完了/);
}
function assertStopped(result) {
  assert.equal(result.status, 23, result.output);
  assert.match(result.output, /準備に失敗.*終了コード: 23/);
  assert.doesNotMatch(result.output, /準備完了/);
}

for (const entry of entries) {
  test(`${entry}はローカルで何も導入しない`, () => {
    const result = run(entry, { remote: '' });
    assert.equal(result.status, 0);
    assert.deepEqual(result.trace, []);
  });
  test(`${entry}は同期から固定PIXIを経て配信物を生成する`, () => {
    const result = run(entry);
    assertPrepared(result);
  });
  test(`${entry}は専用環境変数なしでもそのworktreeだけを準備する`, () => {
    assertPrepared(run(entry, { projectUnset: true }));
  });
  test(`${entry}は現行仕様の素材同期失敗だけ理由を示して続行する`, () => {
    const result = run(entry, { fail: 'xtask sync-characters' });
    assert.equal(result.status, 0);
    assert.match(result.output, /素材の同期に失敗/);
    assert.equal(result.trace[1], 'cargo xtask setup pixi');
  });
  for (const [fail, missing] of [['xtask setup pixi', false], ['xtask build-ui', false],
    ['fetch --manifest-path src-tauri/Cargo.toml --quiet', false],
    ['test --lib --manifest-path src-tauri/Cargo.toml --no-run --quiet', false], ['update', true], ['install', true]]) {
    test(`${entry}は${fail}失敗後の依存工程へ進まない`, () => {
      const result = run(entry, { fail, missing });
      assertStopped(result);
      const last = result.trace.at(-1);
      assert.ok(last.includes(fail), last);
    });
  }
  test(`${entry}の明示事前ビルド省略は他の準備を飛ばさない`, () => {
    const result = run(entry, { skip: '1' });
    assert.equal(result.status, 0);
    assert.equal(result.trace.length, 4);
    assert.match(result.output, /SKIP_PREBUILD=1/);
  });
}

test('PIXIを準備しない旧順序を実行検査が検出する', () => {
  const result = run(entries[0], { source: preparation.replace('cargo xtask setup pixi\n', '') });
  assert.throws(() => assertPrepared(result), assert.AssertionError);
});

test('旧事前ビルドの失敗握り潰しを実行検査が検出する', () => {
  const source = preparation.replace('cargo test --lib --manifest-path src-tauri/Cargo.toml --no-run --quiet',
    'cargo test --lib --manifest-path src-tauri/Cargo.toml --no-run --quiet || true');
  const result = run(entries[0], { source, fail: 'test --lib --manifest-path src-tauri/Cargo.toml --no-run --quiet' });
  assert.throws(() => assertStopped(result), assert.AssertionError);
});

test('登録用入口へ準備手順を再複製しない', () => {
  for (const entry of entries) {
    const source = readFileSync(join(root, entry), 'utf8');
    assert.match(source, /exec bash .*\/tools\/session-start\.sh/);
    assert.doesNotMatch(source, /cargo|apt-get|APT_PACKAGES/);
  }
});
