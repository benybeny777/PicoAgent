"""管理下ComfyUIのJSON通信と起動待ち。旧表情生成には依存しない。"""
import json
import subprocess
import time
import urllib.error
import urllib.request


def request_json(url: str, payload: dict | None = None, timeout: float = 30) -> dict:
    data = None if payload is None else json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(
        url, data=data, headers={"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.load(response)


def wait_for_server(base_url: str, process: subprocess.Popen, timeout: int) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"ComfyUI exited during startup: {process.returncode}")
        try:
            request_json(f"{base_url}/system_stats", timeout=2)
            return
        except (urllib.error.URLError, TimeoutError):
            time.sleep(1)
    raise TimeoutError(f"ComfyUI did not start within {timeout} seconds")
