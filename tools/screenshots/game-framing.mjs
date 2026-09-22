// 全同梱キャラで、開始形式と保存構図に左右されないゲーム表示と復元を検査する。
import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";

export async function validateGameFraming(openScene, root) {
  const output = join(root, "temp", "game-framing-qa");
  await mkdir(output, { recursive: true });
  const registry = JSON.parse(await readFile(join(root, "ui/assets/characters/index.json"), "utf8"));
  const layouts = ["pet", "mascot", "widget", "chat", "bar", "workspace", "stream"];
  const reports = [];
  for (const { id } of registry.characters) {
    const saved = Object.fromEntries(layouts.map(layout => [layout, { preset: "custom", zoom: 3, x: -200, y: 350 }]));
    const page = await openScene({ characterId: id, width: 1000, height: 760, characterFraming: { [id]: saved } });
    try {
      const result = await page.evaluate(layouts => {
        const snapshot = () => {
          const view = getComputedStyle(document.getElementById("characterView"));
          const node = document.querySelector("#character canvas");
          const canvas = node ? getComputedStyle(node) : null;
          return {
            zoom: character.style.getPropertyValue("--char-zoom"),
            x: character.style.getPropertyValue("--char-x"), y: character.style.getPropertyValue("--char-y"),
            view: [view.transform, view.scale, view.width, view.height],
            canvas: canvas && [canvas.width, canvas.height, canvas.transform, canvas.left, canvas.top],
            live2d: live2dModel && [live2dModel.scale.x, live2dModel.scale.y, live2dModel.position.x, live2dModel.position.y],
          };
        };
        const original = JSON.stringify(characterFraming);
        const results = layouts.map(layout => {
          applyLayout(layout);
          const before = snapshot();
          window.PicoGames.open();
          const game = snapshot();
          window.PicoGames.close();
          return { layout, appliedLayout: currentLayout, before, game, after: snapshot() };
        });
        return { id: characterManifest.id, kind: character.dataset.renderKind, results, savedUnchanged: original === JSON.stringify(characterFraming) };
      }, layouts);
      assert.equal(result.id, id, `${id}: 別キャラへ変わっています`);
      assert.equal(result.savedUnchanged, true, `${id}: 保存構図が変更されています`);
      for (const item of result.results) {
        assert.equal(item.appliedLayout, item.layout, `${id}: 指定した表示形式が適用されていません`);
        assert.deepEqual(item.game, result.results[0].game, `${id}/${item.layout}: ゲーム構図が開始形式に依存しています`);
        assert.deepEqual(item.after, item.before, `${id}/${item.layout}: 通常構図へ戻りません`);
      }
      await page.evaluate(() => window.PicoGames.open());
      await page.screenshot({ path: join(output, `${id}.png`) });
      reports.push(result);
      console.log(`ゲーム構図: ${id} / 7形式と復元を確認`);
    } finally {
      await page.close();
    }
  }
  await writeFile(join(output, "report.json"), JSON.stringify(reports, null, 2));
  return output;
}
