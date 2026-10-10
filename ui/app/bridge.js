// bridge.js - メイン画面からバックエンド（Tauri）を呼ぶ唯一の入口。
//
// 約束の正本は docs/UI-CONTRACT.md。ここは `window.picoagent`（ui/picoagent.js、設定画面と共有）の
// 上に名前付きの関数を並べるだけで、業務処理を持たない。他のモジュールが `window.picoagent` や
// `window.__TAURI__` を直接触ると tools/tests/ui-architecture-contract.mjs が落とす。

const api = window.picoagent;

export const frontendReady = () => api.frontendReady();
export const bootProgress = () => api.bootProgress();
export const init = () => api.init();
export const finishBoot = () => api.finishBoot();
export const revealMain = () => api.revealMain();

export const send = text => api.send(text);
export const sendImage = (text, images) => api.sendImage(text, images);
export const cancel = () => api.cancel();
export const captureScreen = () => api.captureScreen();
export const readFileContent = (name, dataUrl) => api.readFileContent(name, dataUrl);
export const readDroppedPath = path => api.readDroppedPath(path);
export const transcribeAudio = dataB64 => api.transcribeAudio(dataB64);
export const exportHistory = markdown => api.exportHistory(markdown);

export const speak = text => api.speak(text);
export const aivisStatus = () => api.aivisStatus();
export const prepareVoiceModel = () => api.prepareVoiceModel();

export const idleChatter = () => api.idleChatter();
export const dailyGreeting = () => api.dailyGreeting();
export const dailyEvents = () => api.dailyEvents();
export const alarms = () => api.alarms();
export const alarmWake = (id, consumed = false) => api.alarmWake(id, consumed);

export const setCharacterProfile = (characterId, profile, greeting, voice) =>
  api.setCharacterProfile(characterId, profile, greeting, voice);
export const userCharacters = () => api.userCharacters();
export const assetUrl = path => api.assetUrl(path);
export const saveScene = scenes => api.saveScene(scenes);
export const saveUiPref = (key, value) => api.saveUiPref(key, value);
export const setCharacterRenderer = (characterId, renderer) =>
  api.setCharacterRenderer(characterId, renderer);

export const menu = (context, x, y) => api.menu(context, x, y);
export const layoutMenu = (x, y) => api.layoutMenu(x, y);
export const characterMenu = (x, y, characters) => api.characterMenu(x, y, characters);

export const startWindowDrag = () => api.startWindowDrag();
export const startWindowResize = direction => api.startWindowResize(direction);
export const setWindowSize = (width, height) => api.setWindowSize(width, height);
export const moveAndResizeWindow = (leftDelta, topDelta, width, height) =>
  api.moveAndResizeWindow(leftDelta, topDelta, width, height);
export const setFullscreen = on => api.setFullscreen(on);
export const minimizeWindow = () => api.minimizeWindow();
export const toggleMaximizeWindow = () => api.toggleMaximizeWindow();
export const closeWindow = () => api.closeWindow();
export const resetLayout = () => api.resetLayout();

export const openSettings = (tab = null) => api.openSettings(tab);
export const openExternalUrl = url => api.openExternalUrl(url);
export const openGeneratedMedia = path => api.openGeneratedMedia(path);
export const configPresence = keys => api.configPresence(keys);

export const relationshipState = () => api.relationshipState();
export const relationshipEvent = (kind, detail = "") => api.relationshipEvent(kind, detail);
export const pinRelationshipMoment = (momentId, pinned) => api.pinRelationshipMoment(momentId, pinned);
export const deleteRelationshipMoment = momentId => api.deleteRelationshipMoment(momentId);
export const clearRelationship = () => api.clearRelationship();
export const completeOnboarding = () => api.completeOnboarding();

export const scheduleRuns = () => api.scheduleRuns();
export const rerunScheduleTask = taskId => api.rerunScheduleTask(taskId);
export const durableJobs = () => api.durableJobs();
export const retryDurableJob = id => api.retryDurableJob(id);
export const cancelDurableJob = id => api.cancelDurableJob(id);
export const undoProjectRule = rule => api.undoProjectRule(rule);

export const requestGpuCacheReset = reason => api.requestGpuCacheReset(reason);
export const gameReady = () => api.gameReady();
export const gameJudge = request => api.gameJudge(request);
export const quit = () => api.quit();
export const engineStatus = () => api.engineStatus();
export const componentStatus = () => api.settings.componentStatus();
export const installComponents = (ids, enableTts = false) => api.settings.installComponents(ids, enableTts);

/** バックエンドからのイベント（`pet:*` など決まった名前）を購読する。 */
export const on = (channel, callback) => api.on(channel, callback);
/** 画面固有の購読（ストリーミングなど）。解除関数を返す Promise。 */
export const listen = (channel, callback) => api.listen(channel, callback);
