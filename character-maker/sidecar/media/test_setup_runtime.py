"""専用ランタイムの同意、分離、公開前検証を実取得なしで固定する。"""
from pathlib import Path
from contextlib import contextmanager
import tempfile
import unittest
from unittest.mock import Mock, patch
import zipfile

import setup_runtime as runtime


class 専用ランタイム検査(unittest.TestCase):
    @contextmanager
    def 準備(self, root):
        target = root / "bundled/media-runtime"
        cache = root / "cache"
        target.mkdir(parents=True)
        (target / "旧環境.txt").write_text("旧環境を保持")

        def download(*args):
            with zipfile.ZipFile(cache / "comfy.zip", "w") as archive:
                archive.writestr(f"ComfyUI-{runtime.COMFY_COMMIT}/main.py", "# テスト用")
            (cache / "progress.json").write_text("{}")

        def run(arguments):
            if arguments[1] == "venv":
                python = Path(arguments[2]) / "Scripts/python.exe"
                python.parent.mkdir(parents=True)
                python.write_bytes(b"isolated-python")

        with patch.object(runtime, "check_hardware"), patch.object(runtime, "download", side_effect=download) as download_mock, \
                patch.object(runtime, "run", side_effect=run), \
                patch.object(runtime.shutil, "disk_usage", return_value=type("容量", (), {"free": 100_000_000_000})()), \
                patch.object(runtime.sys, "platform", "win32"), patch.object(runtime.platform, "machine", return_value="AMD64"):
            yield target, cache, download_mock

    def test_同意なしではハード検査も取得もしない(self):
        with patch.object(runtime, "check_hardware") as hardware, patch.object(runtime, "download") as download:
            with self.assertRaises(ValueError):
                runtime.install(Path("bundled/media-runtime"), Path("cache"), Path("uv"), None)
            hardware.assert_not_called()
            download.assert_not_called()

    def test_非windowsはハード検査前に拒否する(self):
        with patch.object(runtime.sys, "platform", "linux"), patch.object(runtime, "check_hardware") as hardware:
            with self.assertRaisesRegex(RuntimeError, "Windows"):
                runtime.install(Path("bundled/media-runtime"), Path("cache"), Path("uv"), runtime.LICENSE_REVISION)
            hardware.assert_not_called()

    def test_解決前にrootと親とcacheのリンクを拒否する(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for suffix in ("bundled/media-runtime", "bundled", "cache", "bundled/media-runtime-old"):
                linked = root / suffix
                with patch.object(Path, "is_junction", lambda path: path == linked), \
                        patch.object(runtime, "check_hardware"), \
                        patch.object(runtime, "download") as download, \
                        patch.object(runtime.sys, "platform", "win32"), patch.object(runtime.platform, "machine", return_value="AMD64"):
                    with self.assertRaisesRegex(ValueError, "リンク"):
                        runtime.install(root / "bundled/media-runtime", root / "cache", Path("uv"), runtime.LICENSE_REVISION)
                    download.assert_not_called()

    def test_最終配置で失敗したら旧版へ戻して候補を回収する(self):
        with tempfile.TemporaryDirectory() as temporary, self.準備(Path(temporary)) as (target, cache, _):
            original = runtime.verify_runtime

            def verify(path):
                if path == target:
                    raise RuntimeError("最終配置の起動に失敗")
                original(path)

            with patch.object(runtime, "verify_runtime", side_effect=verify):
                with self.assertRaisesRegex(RuntimeError, "最終配置"):
                    runtime.install(target, cache, Path("uv"), runtime.LICENSE_REVISION)
            self.assertTrue((target / "旧環境.txt").is_file())
            self.assertFalse((target / "install.json").exists())
            self.assertFalse(target.with_name("media-runtime-old").exists())
            self.assertEqual(list(target.parent.glob(".installing-*")), [])

    def test_cache削除失敗では旧版を公開前に保持する(self):
        with tempfile.TemporaryDirectory() as temporary, self.準備(Path(temporary)) as (target, cache, _):
            original = Path.unlink

            def unlink(path, *args, **kwargs):
                if path == cache / "progress.json":
                    raise PermissionError("一時的な読み取り競合")
                return original(path, *args, **kwargs)

            with patch.object(Path, "unlink", unlink):
                with self.assertRaises(PermissionError):
                    runtime.install(target, cache, Path("uv"), runtime.LICENSE_REVISION)
            self.assertTrue((target / "旧環境.txt").is_file())
            self.assertEqual(list(target.parent.glob(".installing-*")), [])

    def test_旧版清掃だけ失敗したら次回再取得なしで回収する(self):
        with tempfile.TemporaryDirectory() as temporary, self.準備(Path(temporary)) as (target, cache, download):
            original = runtime.remove_runtime_tree

            def remove(path, parent):
                if path.name == "media-runtime-old":
                    raise PermissionError("旧版を開いている")
                original(path, parent)

            with patch.object(runtime, "remove_runtime_tree", side_effect=remove):
                with self.assertRaisesRegex(RuntimeError, "新版ランタイムは検証済み"):
                    runtime.install(target, cache, Path("uv"), runtime.LICENSE_REVISION)
            self.assertTrue((target / "install.json").is_file())
            self.assertTrue(target.with_name("media-runtime-old").exists())
            runtime.install(target, cache, Path("uv"), runtime.LICENSE_REVISION)
            self.assertFalse(target.with_name("media-runtime-old").exists())
            self.assertEqual(download.call_count, 1)

    def test_タイムアウトで子と親を回収して終了を待つ(self):
        # プロセス回収の契約だけを検査し、実プロセスや追加パッケージに依存しない。
        class プロセスなし(Exception):
            pass

        psutil = Mock(NoSuchProcess=プロセスなし, Error=Exception)
        process = Mock(pid=12345)
        process.wait.side_effect = [runtime.subprocess.TimeoutExpired("uv", 7200), 0]
        process.poll.return_value = None
        child = Mock()
        parent = Mock()
        parent.children.return_value = [child]
        with patch.dict(runtime.sys.modules, {"psutil": psutil}), \
                patch.object(runtime.subprocess, "Popen", return_value=process), \
                patch.object(psutil, "Process", return_value=parent), \
                patch.object(psutil, "wait_procs", return_value=([child], [])):
            with self.assertRaises(runtime.subprocess.TimeoutExpired):
                runtime.run(["uv"])
        child.kill.assert_called_once()
        process.kill.assert_called_once()
        self.assertEqual(process.wait.call_args.kwargs["timeout"], 10)

    def test_旧版だけ残った中断は空き容量不足でも再取得せず復旧する(self):
        with tempfile.TemporaryDirectory() as temporary, self.準備(Path(temporary)) as (target, cache, download):
            runtime.install(target, cache, Path("uv"), runtime.LICENSE_REVISION)
            backup = target.with_name("media-runtime-old")
            target.rename(backup)
            with patch.object(runtime.shutil, "disk_usage", return_value=type("容量", (), {"free": 0})()), \
                    patch.object(runtime, "verify_runtime", wraps=runtime.verify_runtime) as verify:
                runtime.install(target, cache, Path("uv"), runtime.LICENSE_REVISION)
            self.assertTrue((target / "install.json").is_file())
            self.assertFalse(backup.exists())
            self.assertEqual(download.call_count, 1)
            verify.assert_any_call(target)

    def test_復旧先で起動できなければ退避版を残し取得を止める(self):
        with tempfile.TemporaryDirectory() as temporary, self.準備(Path(temporary)) as (target, cache, download):
            backup = target.with_name("media-runtime-old")
            target.rename(backup)
            with patch.object(runtime, "verify_runtime", side_effect=RuntimeError("復旧先の起動失敗")):
                with self.assertRaisesRegex(RuntimeError, "復旧先"):
                    runtime.install(target, cache, Path("uv"), runtime.LICENSE_REVISION)
            self.assertFalse(target.exists())
            self.assertTrue((backup / "旧環境.txt").is_file())
            download.assert_not_called()

    def test_本体と退避版の両方がある場合は本体を上書きしない(self):
        with tempfile.TemporaryDirectory() as temporary, self.準備(Path(temporary)) as (target, cache, download):
            backup = target.with_name("media-runtime-old")
            backup.mkdir()
            (backup / "退避版.txt").write_text("退避版を保持", encoding="utf-8")
            with self.assertRaisesRegex(RuntimeError, "復旧確認"):
                runtime.install(target, cache, Path("uv"), runtime.LICENSE_REVISION)
            self.assertTrue((target / "旧環境.txt").is_file())
            self.assertTrue((backup / "退避版.txt").is_file())
            download.assert_not_called()

    def test_旧環境を変更せず独立venvを公開する(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = root / "bundled/media-runtime"
            cache = root / "cache"
            old = root / "bundled/python/Scripts/python.exe"
            old.parent.mkdir(parents=True)
            old.write_bytes(b"shared-python")
            calls = []

            def download(*args):
                with zipfile.ZipFile(cache / "comfy.zip", "w") as archive:
                    archive.writestr(f"ComfyUI-{runtime.COMFY_COMMIT}/main.py", "# テスト用")
                (cache / "progress.json").write_text("{}")
                (cache / "unrelated.txt").write_text("利用者の無関係なファイル")

            def run(arguments):
                calls.append(arguments)
                if arguments[1] == "venv":
                    python = Path(arguments[2]) / "Scripts/python.exe"
                    python.parent.mkdir(parents=True)
                    python.write_bytes(b"isolated-python")

            with patch.object(runtime, "check_hardware"), patch.object(runtime, "download", side_effect=download), \
                    patch.object(runtime, "run", side_effect=run), \
                    patch.object(runtime.shutil, "disk_usage", return_value=type("容量", (), {"free": 100_000_000_000})()), \
                    patch.object(runtime.sys, "platform", "win32"), patch.object(runtime.platform, "machine", return_value="AMD64"):
                runtime.install(target, cache, Path("uv.exe"), runtime.LICENSE_REVISION)
            self.assertEqual(old.read_bytes(), b"shared-python")
            self.assertTrue((target / "venv/Scripts/python.exe").is_file())
            self.assertTrue((target / "ComfyUI/main.py").is_file())
            self.assertTrue((target / "install.json").is_file())
            self.assertTrue((cache / "unrelated.txt").is_file())
            self.assertIn("--require-hashes", calls[1])
            self.assertNotIn(str(old), calls[1])
            self.assertEqual(list(target.parent.glob(".installing-*")), [])


if __name__ == "__main__":
    unittest.main()
