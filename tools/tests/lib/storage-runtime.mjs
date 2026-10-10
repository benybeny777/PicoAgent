import vm from "node:vm";
import { uiFile } from "./ui-source.mjs";

// 本番のストアと通知集約を同じVMで実行し、例外を握り潰す代用品を使わない。
export function installStorageRuntime(context) {
  vm.runInContext(uiFile("app/ui-store.js").replace(/export function /g, "function "), context);
  const toast = uiFile("app/toast.js");
  vm.runInContext(toast.slice(toast.indexOf("export function createBackgroundFailureReporter")).replace("export function ", "function "), context);
}
