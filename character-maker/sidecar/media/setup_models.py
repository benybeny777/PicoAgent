"""選択された動画モデルだけを固定版・固定SHA-256で取得する。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import time
import urllib.error
import urllib.parse
import urllib.request

from model_catalog import MODELS as MODERN_MODELS, LICENSE_REVISION, check_hardware, hf_token


MODELS = {
    "wan-2.2-5b": [
        ("diffusion_models/wan2.2_ti2v_5B_fp16.safetensors", "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/c4f60d30c55a624e35427060fdd217579a6c1d77/split_files/diffusion_models/wan2.2_ti2v_5B_fp16.safetensors?download=true", "456f901338bd9eadbded3828b819109a9b68e8a525ca5cf8d0049a69fcfeca1e", 9_999_658_848),
        ("vae/wan2.2_vae.safetensors", "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/c4f60d30c55a624e35427060fdd217579a6c1d77/split_files/vae/wan2.2_vae.safetensors?download=true", "e40321bd36b9709991dae2530eb4ac303dd168276980d3e9bc4b6e2b75fed156", 1_409_400_960),
        ("text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors", "https://huggingface.co/Comfy-Org/Wan_2.1_ComfyUI_repackaged/resolve/617a7633e636506f850e043bc4605f290a466a8e/split_files/text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors?download=true", "c3355d30191f1f066b26d93fba017ae9809dce6c627dda5f6a66eaa651204f68", 6_735_906_897),
    ],
    "ltx-0.9.8-2b-fp8": [
        ("checkpoints/ltxv-2b-0.9.8-distilled-fp8.safetensors", "https://huggingface.co/Lightricks/LTX-Video/resolve/8984fa25007f376c1a299016d0957a37a2f797bb/ltxv-2b-0.9.8-distilled-fp8.safetensors?download=true", "d6d8fa8ed3a98346787c2503ac80fb5d7cebcf80e356b79a2ba361fbadf97e15", 4_461_695_684),
        ("text_encoders/t5xxl_fp16.safetensors", "https://huggingface.co/comfyanonymous/flux_text_encoders/resolve/6af2a98e3f615bdfa612fbd85da93d1ed5f69ef5/t5xxl_fp16.safetensors?download=true", "6e480b09fae049a72d2a8c5fbccb8d3e92febeb233bbe9dfe7256958a9167635", 9_787_841_024),
    ],
}
VERSIONS = {"wan-2.2-5b": "wan-2.2-5b", "ltx-0.9.8-2b-fp8": "0.9.8"}
MODELS.update(MODERN_MODELS)
VERSIONS.update({name: LICENSE_REVISION for name in MODERN_MODELS})


class PrivateRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        redirected = super().redirect_request(req, fp, code, msg, headers, newurl)
        if redirected and urllib.parse.urlsplit(newurl).netloc != urllib.parse.urlsplit(req.full_url).netloc:
            redirected.remove_header("Authorization")
        if urllib.parse.urlsplit(newurl).scheme != "https":
            raise ValueError("HTTPS以外の取得先への転送を拒否しました")
        return redirected


def remote(url, method="GET", headers=None):
    headers = dict(headers or {})
    if urllib.parse.urlsplit(url).hostname == "huggingface.co":
        token = hf_token()
        if token:
            headers["Authorization"] = "Bearer " + token
    try:
        return urllib.request.build_opener(PrivateRedirect()).open(
            urllib.request.Request(url, headers=headers, method=method), timeout=120)
    except urllib.error.HTTPError as error:
        if error.code in (401, 403):
            raise RuntimeError("モデル提供元の認証が必要です。モデルページで利用条件へ同意し、同じアカウントで端末の hf auth login を実行してください。トークンをチャットへ貼らないでください") from None
        raise RuntimeError(f"モデル取得に失敗しました（HTTP {error.code}）") from None


def preflight(model, accepted_license, root, cache):
    if model not in MODERN_MODELS:
        return
    if accepted_license != LICENSE_REVISION:
        raise ValueError("モデル固有の利用条件を確認し、明示的に同意してから取得してください")
    check_hardware()
    missing = sum(item[3] for item in MODELS[model] if not (root / item[0]).is_file())
    # キャッシュと導入先が別ボリュームの場合も、移動中の空き容量を保守的に確保する。
    for path in (root, cache):
        if shutil.disk_usage(path).free < missing + 5_000_000_000:
            raise RuntimeError(f"モデル取得に必要な空き容量が不足しています（目安 {(missing + 5_000_000_000) / 1e9:.1f}GB）")
    # 途中まで大容量取得してから認証不足を検出しない。
    for _, url, _, _ in MODELS[model]:
        with remote(url, method="HEAD"):
            pass


def digest(path):
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def progress(path, message, downloaded, total):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps({"message": message, "downloaded": downloaded, "total": total}, ensure_ascii=False), encoding="utf-8")
    # Windowsで画面が読み取り中でも、進捗表示だけの競合で本体取得を失敗させない。
    for attempt in range(5):
        try:
            temporary.replace(path)
            return
        except PermissionError:
            if attempt < 4:
                time.sleep(0.05)
    temporary.unlink(missing_ok=True)


def checked_root(root):
    root = Path(os.path.abspath(root))
    for part in (root, *root.parents):
        if part.is_symlink() or (hasattr(part, "is_junction") and part.is_junction()):
            raise ValueError("モデル保存先のルートと親にリンクを使用できません")
    return root.resolve()


def safe_path(root, relative):
    root = checked_root(root)
    target = root / relative
    if not target.resolve().is_relative_to(root):
        raise ValueError("モデル保存先が管理ディレクトリ外です")
    for part in (target, *target.parents):
        if part == root:
            break
        if part.is_symlink() or (hasattr(part, "is_junction") and part.is_junction()):
            raise ValueError(f"モデル保存先にリンクを使用できません: {part}")
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.is_symlink() or target.resolve().parent != target.parent.resolve():
        raise ValueError(f"モデル保存先にリンクを使用できません: {target}")
    return target


def download_once(root, cache, item, index, count, progress_path, total_all, completed):
    relative, url, expected, expected_size = item
    destination = safe_path(root, relative)
    pending = safe_path(cache, relative.replace("/", "_") + ".part")
    if destination.is_file() and destination.stat().st_size == expected_size and digest(destination) == expected:
        progress(progress_path, f"モデル {index}/{count} を確認済み: {relative}", completed + expected_size, total_all)
        return expected_size
    if destination.exists():
        raise ValueError(f"既存モデルが固定値と一致しません: {relative}")
    offset = pending.stat().st_size if pending.exists() else 0
    if offset > expected_size:
        pending.unlink()
        offset = 0
    if offset == expected_size:
        if digest(pending) != expected:
            pending.unlink()
            raise ValueError(f"モデルのSHA-256検査に失敗しました: {relative}")
        pending.replace(destination)
        return expected_size
    with remote(url, headers={"Range": f"bytes={offset}-"} if offset else {}) as response:
        if offset and getattr(response, "status", 200) != 206:
            raise ValueError(f"モデル取得の再開に対応しない応答です: {relative}")
        if offset and not response.headers.get("Content-Range", "").startswith(f"bytes {offset}-"):
            raise ValueError(f"モデル取得の再開位置が一致しません: {relative}")
        with pending.open("ab" if offset else "wb") as output:
            downloaded = offset
            last = 0.0
            while block := response.read(1024 * 1024):
                if downloaded + len(block) > expected_size:
                    raise ValueError(f"取得サイズが固定値を超えました: {relative}")
                output.write(block)
                downloaded += len(block)
                now = time.monotonic()
                if now - last >= 0.2:
                    progress(progress_path, f"モデル {index}/{count} を取得中: {relative}", completed + downloaded, total_all)
                    last = now
            output.flush()
            os.fsync(output.fileno())
    if pending.stat().st_size != expected_size:
        raise OSError(f"モデルの転送が途中で終了しました。途中位置から再開します: {relative}")
    if digest(pending) != expected:
        pending.unlink()
        raise ValueError(f"モデルのSHA-256検査に失敗しました: {relative}")
    pending.replace(destination)
    progress(progress_path, f"モデル {index}/{count} を取得完了: {relative}", completed + expected_size, total_all)
    return expected_size


def download(root, cache, item, index, count, progress_path, total_all, completed):
    for attempt in range(3):
        try:
            return download_once(root, cache, item, index, count, progress_path, total_all, completed)
        except (OSError, urllib.error.URLError):
            if attempt == 2:
                raise
            progress(progress_path, f"通信が中断したため取得を再開します（{attempt + 1}/2）", completed, total_all)
            time.sleep(1)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", choices=MODELS, required=True)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--progress", type=Path, required=True)
    parser.add_argument("--accept-license", help="利用者が確認したモデル利用条件の版")
    parser.add_argument("--preflight-only", action="store_true")
    args = parser.parse_args()
    root = checked_root(args.runtime)
    cache = checked_root(args.cache)
    root.mkdir(parents=True, exist_ok=True)
    cache.mkdir(parents=True, exist_ok=True)
    args.progress.parent.mkdir(parents=True, exist_ok=True)
    preflight(args.model, args.accept_license, root, cache)
    if args.preflight_only:
        print("モデルの取得前確認が完了しました", flush=True)
        return
    items = MODELS[args.model]
    total = sum(item[3] for item in items)
    completed = 0
    for index, item in enumerate(items, 1):
        completed += download(root, cache, item, index, len(items), args.progress, total, completed)
    if args.model == "krea-2-turbo-nvfp4":
        # ライセンス3.1が指定する告知原文を維持する。
        notice = safe_path(root, "licenses/Notice.txt")
        notice.write_text("Krea 2 is licensed under the Krea 2 Community License Agreement. For more information, visit https://krea.ai/krea-2-licensing.\n", encoding="utf-8")
    marker = safe_path(root, "install.json.incoming")
    marker.write_text(json.dumps({"version": VERSIONS[args.model], "model": args.model, "licenseRevision": args.accept_license, "files": [item[0] for item in items]}, ensure_ascii=False, indent=2), encoding="utf-8")
    marker.replace(root / "install.json")


if __name__ == "__main__":
    main()
