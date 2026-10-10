// ui-store.js - UI保存領域への唯一の入口。保存値と例外本文を診断へ出さない。
const SAFE_ERRORS = new Set(["SecurityError", "QuotaExceededError", "InvalidStateError", "NotSupportedError"]);
function failure(operation, error) {
  let kind = "保存領域を利用できません";
  try {
    const name = error?.name;
    if (SAFE_ERRORS.has(name)) {
      kind = { SecurityError: "保存領域へのアクセスが拒否されました", QuotaExceededError: "保存領域の容量が足りません", InvalidStateError: "保存領域が利用可能な状態ではありません", NotSupportedError: "保存領域が対応していません" }[name];
    }
  } catch { /* 例外のプロパティ自体を読めない場合も本文を取り出さない。 */ }
  return { ok: false, reason: `${operation}: ${kind}` };
}
export function read(key) {
  try {
    const value = globalThis.localStorage.getItem(key);
    if (value !== null && typeof value !== "string") return { ok: false, reason: "保存値を読めません: 保存領域の返却形式が不正です" };
    return { ok: true, value };
  } catch (error) { return failure("保存値を読めません", error); }
}
export function readJson(key, validate) {
  const result = read(key);
  if (!result.ok) return { ...result, corrupted: false };
  if (result.value === null) return result;
  let value;
  try { value = JSON.parse(result.value); }
  catch { return { ok: false, reason: "保存値を読めません: JSONが壊れています", corrupted: true }; }
  try {
    if (validate(value) !== true) return { ok: false, reason: "保存値を読めません: 保存値の形式が不正です", corrupted: true };
  } catch { return { ok: false, reason: "保存値を読めません: 保存値の形式を確認できません", corrupted: true }; }
  return { ok: true, value };
}
export function write(key, value) {
  if (typeof value !== "string") return { ok: false, reason: "保存できません: 保存する値の形式が不正です" };
  try { globalThis.localStorage.setItem(key, value); return { ok: true }; }
  catch (error) { return failure("保存できません", error); }
}
export function remove(key) {
  try { globalThis.localStorage.removeItem(key); return { ok: true }; }
  catch (error) { return failure("保存値を消せません", error); }
}
