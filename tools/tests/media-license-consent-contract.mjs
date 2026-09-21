import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const source = path => readFileSync(new URL(`../../${path}`, import.meta.url), 'utf8');

test('新モデルの本人同意欄は初期状態で未選択になり説明と共に表示へ渡る', () => {
  const settings = source('ui/settings.js');
  const start = settings.indexOf('      const licensedModel =');
  const end = settings.indexOf('      button.addEventListener', start);
  const snippet = settings.slice(start, end);
  for (const id of ['image-krea2', 'video-ltx25', 'video-wan']) {
    const context = { item: { id }, document: {
      createElement: tag => ({ tag, checked: false, children: [], append(...children) { this.children.push(...children); } }),
      createTextNode: text => ({ text }),
    } };
    vm.createContext(context);
    vm.runInContext(`${snippet}\nthis.result = { licensedModel, licenseAccepted, licenseConsent };`, context);
    const result = context.result;
    assert.equal(result.licensedModel, id !== 'video-wan');
    if (result.licensedModel) {
      assert.equal(result.licenseAccepted.checked, false);
      assert.equal(result.licenseAccepted.type, 'checkbox');
      assert.equal(result.licenseConsent.children[0], result.licenseAccepted);
      assert.match(result.licenseConsent.children[1].text, /本人が確認/);
    }
  }
  assert.match(settings, /sourceLinks,\s*licenseConsent,/);
  assert.match(settings, /licensedModel && !licenseAccepted\?\.checked/);
  assert.match(settings, /installComponents\(\[item\.id\], false, licensedModel \? \[item\.id\] : \[\]\)/);
});

test('同意情報は取得APIへ伝わり既存呼び出しでは空になる', () => {
  assert.match(source('ui/picoagent.js'), /acceptedLicenseIds = \[\]/);
  assert.match(source('src-tauri/src/commands.rs'), /accepted_license_ids: Option<Vec<String>>/);
  assert.match(source('src-tauri/src/components.rs'), /accepted_license_ids\.contains\(id\)/);
});
