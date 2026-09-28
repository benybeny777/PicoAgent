// 人が画面を見て判断するための一覧。画像と検査結果は temp/ 内だけに置く。
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const output = path.join(root, "temp", "visual-review");
const captures = path.join(root, "temp", "ui-check");
const nativeCaptures = path.join(output, "native");
const layouts = ["pet", "mascot", "stream"];
const layoutNames = { pet: "ペット", mascot: "ミニ", stream: "配信" };
const bundled = fs.readdirSync(path.join(root, "characters"), { withFileTypes: true })
  .filter(entry => entry.isDirectory() && fs.existsSync(path.join(root, "characters", entry.name, "character.json")))
  .map(entry => JSON.parse(fs.readFileSync(path.join(root, "characters", entry.name, "character.json"), "utf8")))
  .map(({ id, name }) => ({ id, name, source: "bundled" }));
const userDirectory = process.env.APPDATA ? path.join(process.env.APPDATA, "PicoAgent", "characters") : "";
const userCharacters = userDirectory && fs.existsSync(userDirectory) ? fs.readdirSync(userDirectory, { withFileTypes: true })
  .filter(entry => entry.isDirectory() && fs.existsSync(path.join(userDirectory, entry.name, "character.json")))
  .map(entry => JSON.parse(fs.readFileSync(path.join(userDirectory, entry.name, "character.json"), "utf8")))
  .map(({ id, name }) => ({ id, name, source: "user" })) : [];
const characters = [...bundled, ...userCharacters.filter(item => !bundled.some(base => base.id === item.id))]
  .sort((a, b) => a.id.localeCompare(b.id));
const capture = process.argv.includes("--capture");
const failures = [];
const reports = [];
fs.mkdirSync(output, { recursive: true });
const summaryFile = path.join(output, "capture-report.json");
if (!capture && fs.existsSync(summaryFile)) {
  try { failures.push(...JSON.parse(fs.readFileSync(summaryFile, "utf8")).failures); }
  catch { console.warn("前回の検査結果を読み取れません"); }
}

if (capture) {
  for (const { id } of bundled) {
    for (const layout of layouts) fs.rmSync(path.join(captures, `${id}-${layout}.png`), { force: true });
    fs.rmSync(path.join(captures, "report.json"), { force: true });
    // 1体ずつ実行し、失敗したキャラがいても残りの撮影を続ける。
    const run = spawnSync(process.execPath, [path.join(root, "tools", "screenshots", "ui-check.mjs")], {
      cwd: root, env: { ...process.env, UI_CHECK_CHARACTERS: id, UI_CHECK_LAYOUTS: layouts.join(",") },
      encoding: "utf8", stdio: ["ignore", "pipe", "pipe"],
    });
    const reportFile = path.join(captures, "report.json");
    if (fs.existsSync(reportFile)) {
      try { reports.push({ id, ...JSON.parse(fs.readFileSync(reportFile, "utf8")) }); }
      catch { failures.push({ id, detail: "検査結果を読み取れません" }); }
    }
    if (run.status !== 0) failures.push({ id, detail: (run.stderr || run.stdout || run.error?.message || "検査失敗").slice(-1600) });
    console.log(`${id}: ${run.status === 0 ? "撮影完了" : "検査失敗"}`);
  }
  fs.writeFileSync(summaryFile, JSON.stringify({ generatedAt: new Date().toISOString(), reports, failures }, null, 2));
}

const escape = value => String(value).replace(/[&<>"']/g, char => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[char]);
const cards = characters.flatMap(({ id, name, source: origin }) => layouts.map(layout => {
  const source = origin === "user" ? path.join(nativeCaptures, `${id}-${layout}.png`) : path.join(captures, `${id}-${layout}.png`);
  const image = fs.existsSync(source) ? `data:image/png;base64,${fs.readFileSync(source).toString("base64")}` : "";
  const failure = failures.find(item => item.id === id && item.detail.includes(`${id}/${layout}`));
  return `<article class="card" data-character="${escape(id)}" data-layout="${layout}" data-state="${failure ? "failed" : image ? "ready" : "missing"}">
    <header><strong>${escape(name)}</strong><small>${escape(id)} / ${layoutNames[layout]}</small></header>
    ${image ? `<button class="preview" type="button" aria-label="${escape(name)}の${layoutNames[layout]}を拡大"><img src="${image}" alt="${escape(name)}の${layoutNames[layout]}表示"></button>` : `<div class="empty">${failure ? "検査失敗" : origin === "user" ? "実機撮影待ち" : "未撮影"}</div>`}
    <label>目視結果<select aria-label="${escape(name)}の${layoutNames[layout]}の判定"><option value="">未確認</option><option value="ok">問題なし</option><option value="issue">要修正</option></select></label>
    <textarea aria-label="${escape(name)}の${layoutNames[layout]}の所見" placeholder="切れ・字幕・文字・重なりなど"></textarea>
  </article>`;
})).join("\n");
const failureHtml = failures.length ? `<details open><summary>自動検査の失敗 ${failures.length} 体</summary>${failures.map(item => `<pre>${escape(item.id)}: ${escape(item.detail)}</pre>`).join("")}</details>` : "";
const page = `<!doctype html><html lang="ja"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>PicoAgent 画面目視確認</title>
<style>
:root{font:15px system-ui,sans-serif;color:#1c2740;background:#edf2fa}*{box-sizing:border-box}body{margin:0}header.top{position:sticky;top:0;z-index:2;background:#14243e;color:white;padding:16px 24px;box-shadow:0 3px 12px #15254244}h1{font-size:22px;margin:0 0 6px}.top p{margin:0 0 12px;color:#d5e4fa}.controls{display:flex;flex-wrap:wrap;gap:8px;align-items:center}button,select{font:inherit}button,.controls select{border:1px solid #9db6d9;border-radius:8px;background:white;padding:8px 12px;cursor:pointer}.controls button{background:#dce9ff}main{padding:22px;max-width:1900px;margin:auto}h2{margin:24px 0 10px;font-size:19px}.note{background:#fff;padding:14px;border-radius:10px;line-height:1.6}.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(300px,1fr));gap:14px}.card{background:white;border-radius:12px;overflow:hidden;border:1px solid #c9d7e8;box-shadow:0 2px 7px #1c34501a}.card>header{display:flex;justify-content:space-between;align-items:baseline;gap:8px;padding:11px 12px}.card small{color:#596984}.preview{display:block;width:100%;padding:0;border:0;border-radius:0;background:repeating-conic-gradient(#e4ebf5 0 25%,#f8faff 0 50%) 50%/20px 20px}.preview img{display:block;width:100%;height:210px;object-fit:contain}.empty{height:210px;display:grid;place-items:center;background:#e9edf4;color:#53627a}.card label{display:flex;justify-content:space-between;align-items:center;margin:10px 12px}.card select{padding:5px}.card textarea{display:block;width:calc(100% - 24px);height:65px;margin:0 12px 12px;resize:vertical;border:1px solid #bdcde1;border-radius:6px;padding:7px;font:inherit}.card[data-result=issue]{outline:3px solid #d95353}.card[data-result=ok]{outline:2px solid #3d9a6b}pre{white-space:pre-wrap;overflow-wrap:anywhere;background:#fff4f4;padding:10px}dialog{border:0;border-radius:12px;padding:16px;max-width:96vw;max-height:96vh}dialog img{max-width:90vw;max-height:83vh;object-fit:contain}dialog::backdrop{background:#071126d9}dialog button{float:right}a{color:#1659a7}[hidden]{display:none!important}
</style>
<header class="top"><h1>画面目視確認</h1><p>同梱 ${bundled.length} 体・追加 ${userCharacters.length} 体 × ペット・ミニ・配信。実際の UI 描画を並べて確認します。</p><div class="controls"><select id="character"><option value="">全キャラ</option>${characters.map(item => `<option value="${escape(item.id)}">${escape(item.name)} (${escape(item.id)})</option>`).join("")}</select><select id="layout"><option value="">全形式</option>${layouts.map(id => `<option value="${id}">${layoutNames[id]}</option>`).join("")}</select><select id="state"><option value="">全状態</option><option value="missing">未撮影</option><option value="issue">要修正</option><option value="unchecked">未確認</option></select><button id="export">所見を JSON で保存</button><span id="count"></span></div></header>
<main><section class="note">各画像をクリックすると拡大できます。判定と所見はこのブラウザに保存されます。保存した JSON をレビューへ添付できます。画像はモックバックエンドで描いた結果です。MMDなど利用者が追加したキャラと Windows WebView2 固有の挙動は実機で確認してください。<br>撮影: <code>node tools/screenshots/review-board.mjs --capture</code>　再生成: <code>node tools/screenshots/review-board.mjs</code>　出力: <code>temp/visual-review/index.html</code></section>${failureHtml}<h2>全キャラの配置</h2><div class="grid">${cards}</div><h2>ほかの目視確認</h2><section class="note"><p>画像がない項目は、対応する撮影ツールを実行してから画面を再生成してください。</p><ul>${fs.existsSync(path.join(output, "capture-report.json")) ? '<li><a href="capture-report.json">全キャラの画面契約検査</a></li>' : ''}<li><a href="../ui-check/pet-conversation.png">送信・会話の表示</a></li><li><a href="../character-layout-matrix/contact-sheet.png">旧形式を含むキャラ一覧画像</a></li><li><a href="../settings-check/cli-model-select-narrow.png">設定画面の狭幅表示</a></li><li><a href="../native-state.png">Windows WebView2 の撮影</a></li></ul><p>実機の確認項目: 起動中の仮画面、タイトルバーの最小化、長い字幕とキャラの重なり、チャットの文字と送信、右クリックメニューの位置。保存済み配置は維持したまま確認してください。</p></section></main><dialog id="zoom"><button id="close">閉じる</button><div id="zoomTitle"></div><img alt="拡大表示"></dialog>
<script>
const cards=[...document.querySelectorAll('.card')], key='picoagent.visual-review.v1';let saved={};try{saved=JSON.parse(localStorage.getItem(key)||'{}')}catch{};
for(const card of cards){const id=card.dataset.character+'/'+card.dataset.layout, value=saved[id]||{};card.querySelector('select').value=value.result||'';card.querySelector('textarea').value=value.note||'';card.dataset.result=value.result||'';for(const input of card.querySelectorAll('select,textarea'))input.addEventListener('input',()=>{saved[id]={result:card.querySelector('select').value,note:card.querySelector('textarea').value};card.dataset.result=saved[id].result;localStorage.setItem(key,JSON.stringify(saved));filter()})}
function filter(){let shown=0;for(const card of cards){const state=document.querySelector('#state').value;const match=(!document.querySelector('#character').value||card.dataset.character===document.querySelector('#character').value)&&(!document.querySelector('#layout').value||card.dataset.layout===document.querySelector('#layout').value)&&(!state||(state==='unchecked'?!card.dataset.result:state==='issue'?card.dataset.result==='issue':card.dataset.state===state));card.hidden=!match;if(match)shown++}document.querySelector('#count').textContent=shown+' / '+cards.length+' 件'}for(const select of document.querySelectorAll('.controls select'))select.addEventListener('change',filter);filter();
const zoom=document.querySelector('#zoom');document.querySelectorAll('.preview').forEach(button=>button.addEventListener('click',()=>{zoom.querySelector('img').src=button.querySelector('img').src;document.querySelector('#zoomTitle').textContent=button.closest('.card').querySelector('header').textContent.trim();zoom.showModal()}));document.querySelector('#close').onclick=()=>zoom.close();
document.querySelector('#export').onclick=()=>{const data={exportedAt:new Date().toISOString(),results:saved};const link=document.createElement('a');link.href=URL.createObjectURL(new Blob([JSON.stringify(data,null,2)],{type:'application/json'}));link.download='visual-review.json';link.click();setTimeout(()=>URL.revokeObjectURL(link.href),1000)};
</script></html>`;
fs.writeFileSync(path.join(output, "index.html"), page);
console.log(`確認画面: ${path.join(output, "index.html")} (${characters.length * layouts.length} 件)`);
if (failures.length) process.exitCode = 1;
