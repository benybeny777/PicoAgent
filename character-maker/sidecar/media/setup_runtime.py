"""共通Python本体からNVFP4専用仮想環境を作り、旧生成環境を保持する。"""
import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import zipfile

from model_catalog import LICENSE_REVISION, check_hardware
from setup_models import digest, download, safe_path

COMFY_COMMIT = "b0b743566f65daafc423b4fea8a2fbda94b3384a"
COMFY_SHA = "c8997f6781e2b53861697fab23b77a7fdb556ec0640f21653a89689556816b54"
COMFY_SIZE = 13_350_160


def checked_path(path):
    """resolveより先に全親のリンクを調べ、管理領域からの迂回を拒否する。"""
    path = Path(os.path.abspath(path))
    for part in (path, *path.parents):
        if part.is_symlink() or part.is_junction():
            raise ValueError("リンクされたランタイム・キャッシュ配置は使用できません")
    return path.resolve()


def remove_runtime_tree(path, parent):
    path = checked_path(path)
    if path.parent != parent or not (path.name == "media-runtime-old" or path.name.startswith(".installing-media-runtime-")):
        raise ValueError("削除対象が専用ランタイムの作業領域外です")
    if path.exists():
        shutil.rmtree(path)


def verify_runtime(path):
    python = checked_path(path / "venv/Scripts/python.exe")
    if not python.is_file() or not checked_path(path / "ComfyUI/main.py").is_file():
        raise RuntimeError("専用ランタイムの実行ファイルが欠損しています")
    run([str(python), "-c", "import torch,comfy_kitchen,comfy_aimdo; assert torch.cuda.is_available(); assert torch.cuda.get_device_capability()[0] >= 12; assert tuple(map(int, torch.version.cuda.split('.')[:2])) >= (13, 0)"])


def run(arguments):
    process = subprocess.Popen(arguments, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    try:
        if process.wait(timeout=7200) != 0:
            raise RuntimeError("専用生成ランタイムの準備に失敗しました。旧環境は変更していません")
    finally:
        if process.poll() is None:
            import psutil
            # 親の調査が競合で失敗しても、直接起動したプロセスの回収を必ず試みる。
            children = []
            cleanup_errors = []
            try:
                try:
                    children = psutil.Process(process.pid).children(recursive=True)
                except psutil.NoSuchProcess:
                    pass
                for child in reversed(children):
                    try:
                        child.kill()
                    except psutil.NoSuchProcess:
                        pass
                    except psutil.Error as error:
                        cleanup_errors.append(error)
            finally:
                try:
                    process.kill()
                except ProcessLookupError:
                    pass
                process.wait(timeout=10)
            _, alive = psutil.wait_procs(children, timeout=10)
            if alive or cleanup_errors:
                raise RuntimeError("生成環境の導入子プロセスを回収できませんでした")
        else:
            process.wait(timeout=10)


def install(runtime, cache, uv, accepted_license):
    if accepted_license != LICENSE_REVISION:
        raise ValueError("本人によるモデル利用条件への同意を確認できません")
    if sys.platform != "win32" or platform.machine().lower() not in ("amd64", "x86_64"):
        raise RuntimeError("この専用ランタイムの固定依存はWindows x64専用です")
    check_hardware()
    runtime = checked_path(runtime)
    cache = checked_path(cache)
    if runtime.name != "media-runtime" or runtime.parent.name != "bundled":
        raise ValueError("専用ランタイムはPicoAgent管理下のbundled/media-runtimeだけに導入できます")
    if cache.is_relative_to(runtime.parent) or runtime.is_relative_to(cache):
        raise ValueError("取得キャッシュはランタイム配置とは独立した場所を指定してください")
    lock = Path(__file__).resolve().parents[1] / "requirements-media.lock"
    signature = {"comfyCommit": COMFY_COMMIT, "lockSha256": digest(lock), "version": LICENSE_REVISION}
    marker = checked_path(runtime / "install.json")
    stage = runtime.with_name(f".installing-media-runtime-{os.getpid()}")
    backup = checked_path(runtime.with_name("media-runtime-old"))
    if backup.exists() and not runtime.exists():
        # 旧版退避直後の中断は、取得や空き容量検査より先に元の配置へ戻す。
        # 既存の本体は上書きせず、最終配置で起動できなければ復旧用の原本を保持する。
        backup.rename(runtime)
        try:
            verify_runtime(runtime)
        except BaseException:
            runtime.rename(backup)
            raise
    if marker.is_file() and json.loads(marker.read_text(encoding="utf-8")) == signature:
        verify_runtime(runtime)
        # 新版は確認済みで旧版の清掃だけ失敗した前回実行を、再取得せず完了させる。
        remove_runtime_tree(backup, runtime.parent)
        return
    runtime.parent.mkdir(parents=True, exist_ok=True)
    cache.mkdir(parents=True, exist_ok=True)
    if shutil.disk_usage(runtime.parent).free < 12_000_000_000:
        raise RuntimeError("専用ランタイム準備には12GB以上の空き容量が必要です")
    if backup.exists():
        raise RuntimeError("前回の復旧用ランタイムが残っています。復旧確認が必要です")
    if checked_path(stage).exists():
        raise RuntimeError("同じ処理の導入途中領域が残っています")
    stage.mkdir()
    try:
        item = ("comfy.zip", f"https://github.com/Comfy-Org/ComfyUI/archive/{COMFY_COMMIT}.zip", COMFY_SHA, COMFY_SIZE)
        download(cache, cache, item, 1, 1, cache / "progress.json", COMFY_SIZE, 0)
        with zipfile.ZipFile(cache / "comfy.zip") as archive:
            for entry in archive.infolist():
                parts = Path(entry.filename).parts
                if not parts or parts[0] != f"ComfyUI-{COMFY_COMMIT}":
                    raise ValueError("ComfyUI書庫の配置が固定版と一致しません")
                relative = Path(*parts[1:])
                if not parts[1:] or entry.is_dir():
                    continue
                if entry.file_size > 100_000_000 or (entry.external_attr >> 16) & 0o170000 == 0o120000:
                    raise ValueError("ComfyUI書庫の不正な項目を拒否しました")
                destination = safe_path(stage / "ComfyUI", str(relative))
                with archive.open(entry) as source, destination.open("wb") as target:
                    shutil.copyfileobj(source, target)
        # sys._base_executableは既存venvではなく共有Python本体を指す。
        run([str(uv), "venv", str(stage / "venv"), "--python", sys._base_executable, "--relocatable"])
        python = stage / "venv/Scripts/python.exe"
        run([str(uv), "pip", "sync", "--python", str(python), str(lock), "--require-hashes",
             "--index-strategy", "unsafe-best-match", "--extra-index-url", "https://download.pytorch.org/whl/cu130"])
        verify_runtime(stage)
        (stage / "install.json").write_text(json.dumps(signature, ensure_ascii=False), encoding="utf-8")
        # キャッシュ回収が失敗する可能性を、旧版を移動する前に解消する。
        for name in ("comfy.zip", "progress.json"):
            checked_path(cache / name).unlink(missing_ok=True)
        checked_path(runtime)
        checked_path(backup)
        if runtime.exists():
            runtime.replace(backup)
        try:
            stage.replace(runtime)
            # 相対配置のvenvが最終配置でも動くことを確認するまで旧版を保持する。
            verify_runtime(runtime)
        except BaseException:
            if runtime.exists():
                runtime.replace(stage)
            if backup.exists():
                backup.replace(runtime)
            raise
        try:
            remove_runtime_tree(backup, runtime.parent)
        except OSError as error:
            raise RuntimeError("新版ランタイムは検証済みですが、旧版の回収に失敗しました。再導入で回収を再試行してください") from error
    finally:
        remove_runtime_tree(stage, runtime.parent)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--uv", type=Path, required=True)
    parser.add_argument("--accept-license", required=True)
    args = parser.parse_args()
    install(args.runtime, args.cache, args.uv, args.accept_license)
