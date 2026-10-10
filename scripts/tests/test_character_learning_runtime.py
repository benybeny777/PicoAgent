"""取得・学習を行わず、専用runtime導入の保護と原子確定を検査する。"""
import importlib.util
import json
import tempfile
import sys
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import voice_training_common as COMMON

SPEC = importlib.util.spec_from_file_location("learning_runtime", Path(__file__).parents[1] / "setup-character-learning-runtime.py")
RUNTIME = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNTIME)


class RuntimeTests(unittest.TestCase):
    def setUp(self):
        repo = Path(__file__).resolve().parents[2]
        (repo / "temp").mkdir(exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=repo / "temp")
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        gpu_probe = patch.object(RUNTIME, "detect_cuda_gpu", return_value={"CUDA_VISIBLE_DEVICES": "0", "CUDA_DEVICE_ORDER": "PCI_BUS_ID"})
        gpu_probe.start()
        self.addCleanup(gpu_probe.stop)
        self.real_deno_probe = RUNTIME.probe_deno
        deno_probe = patch.object(RUNTIME, "probe_deno")
        deno_probe.start()
        self.addCleanup(deno_probe.stop)
        self.root = self.base / "bundled/voice-training"
        self.shared = self.base / "shared/python.exe"
        self.ffmpeg = self.base / "ffmpeg.exe"
        self.uv = self.base / "uv.exe"
        self.status = self.base / "job/status.json"
        for path in (self.shared, self.ffmpeg, self.uv):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"existing")

    def invoke(self, populate, confirmed=True):
        with patch.object(RUNTIME.platform, "system", return_value="Windows"), patch.object(RUNTIME.platform, "machine", return_value="AMD64"), patch.object(RUNTIME.subprocess, "run") as command, patch.object(RUNTIME, "run"), patch.object(RUNTIME, "populate", side_effect=populate):
            RUNTIME.install(self.root, self.shared, self.ffmpeg, self.uv, self.status, confirmed)
            return command

    def test_GPU未確認なら取得もstage作成もしない(self):
        with self.assertRaises(ValueError):
            self.invoke(lambda *args: self.fail("取得へ進んだ"), confirmed=False)
        self.assertFalse(self.root.parent.exists())

    def test_既存環境を上書きしない(self):
        self.root.mkdir(parents=True)
        original = self.root / "original.aivmx"
        original.write_bytes(b"user-model")
        with self.assertRaises(ValueError):
            self.invoke(lambda *args: self.fail("取得へ進んだ"))
        self.assertEqual(original.read_bytes(), b"user-model")

    def test_probe失敗ではinstalledを確定せず診断stageだけ残す(self):
        def failure(*args):
            raise RuntimeError("CUDAカーネルが利用できません")
        with self.assertRaises(RuntimeError):
            self.invoke(failure)
        self.assertFalse(self.root.exists())
        stages = list(self.root.parent.glob(".installing-*"))
        self.assertEqual(len(stages), 1)
        self.assertFalse((stages[0] / "install.json").exists())
        self.assertEqual(json.loads(self.status.read_text(encoding="utf-8"))["state"], "failed")
        self.assertEqual(self.shared.read_bytes(), b"existing")

    def test_移動後CLI検証失敗では導入済みにしない(self):
        with patch.object(RUNTIME.platform, "system", return_value="Windows"), patch.object(RUNTIME.platform, "machine", return_value="AMD64"), patch.object(RUNTIME.subprocess, "run"), patch.object(RUNTIME, "populate"), patch.object(RUNTIME, "run", side_effect=RuntimeError("移動後CLIが起動できません")):
            with self.assertRaises(RuntimeError):
                RUNTIME.install(self.root, self.shared, self.ffmpeg, self.uv, self.status, True)
        self.assertFalse(self.root.exists())
        stage = next(self.root.parent.glob(".installing-*"))
        self.assertFalse((stage / "install.json").exists())

    def test_成功時だけrenameして環境変数は確定rootを参照する(self):
        def success(stage, *args):
            self.assertFalse(self.root.exists())
            (stage / ".venv/Scripts").mkdir(parents=True)
            (stage / ".venv/Scripts/python.exe").write_bytes(b"verified")
        self.invoke(success)
        record = json.loads((self.root / "runtime-env.json").read_text(encoding="utf-8"))
        self.assertEqual(record["state"], "installed")
        self.assertTrue(record["capabilities"]["cuda"])
        self.assertEqual(record["environment"]["HF_HOME"], str(self.root.resolve() / "cache/huggingface"))
        self.assertEqual(json.loads(self.status.read_text(encoding="utf-8"))["state"], "completed")
        self.assertFalse(list(self.root.parent.glob(".installing-*")))

    def test_並行導入先ができた場合はその資産を上書きしない(self):
        def conflicting(stage, *args):
            self.root.mkdir()
            (self.root / "owned-by-other").write_bytes(b"other")
        with self.assertRaises(OSError):
            self.invoke(conflicting)
        self.assertEqual((self.root / "owned-by-other").read_bytes(), b"other")

    def test_書庫の範囲外パスとハッシュ不一致を拒否する(self):
        archive = self.base / "source.zip"
        with zipfile.ZipFile(archive, "w") as zipped:
            zipped.writestr("archive/../outside", b"bad")
        with self.assertRaises(ValueError):
            RUNTIME.unpack_verified(archive, self.base / "source", {})
        with zipfile.ZipFile(archive, "w") as zipped:
            zipped.writestr("archive/slice.py", b"modified")
        with self.assertRaises(ValueError):
            RUNTIME.unpack_verified(archive, self.base / "source", {"slice.py": RUNTIME.git_blob(b"original")})

    def test_固定patchは文脈不一致で止まる(self):
        diff = "@@ -1,2 +1,2 @@\n before\n-old\n+new\n"
        self.assertEqual(RUNTIME.apply_patch("before\nold\n", diff), "before\nnew\n")
        with self.assertRaises(ValueError):
            RUNTIME.apply_patch("before\nother\n", diff)

    def test_依存は固定しORTの複数distributionを混在させない(self):
        frozen = RUNTIME.FROZEN.splitlines()
        self.assertGreater(len(frozen), 30)
        self.assertTrue(all("==" in line for line in frozen))
        self.assertIn("torch==2.10.0+cu128", frozen)
        self.assertIn("torchaudio==2.10.0+cu128", frozen)
        self.assertIn("faster-whisper==1.2.1", frozen)
        self.assertEqual([line for line in frozen if line.startswith("onnxruntime")], ["onnxruntime-gpu==1.24.4"])

    def test_成功後は所有確認済みの失敗stageだけを回収する(self):
        self.root.parent.mkdir(parents=True)
        own = self.root.with_name(".installing-voice-training-failed")
        other = self.root.with_name(".installing-voice-training-other")
        for folder, destination in ((own, self.root.resolve()), (other, self.base / "other")):
            folder.mkdir()
            (folder / "stage-owner.json").write_text(json.dumps({"destination": str(destination)}))
            (folder / "failure.json").write_text('{}')
        self.invoke(lambda *args: None)
        self.assertFalse(own.exists())
        self.assertTrue(other.exists())

    def test_共有Pythonを含む導入先は取得前に拒否する(self):
        self.root = self.shared.parent
        with self.assertRaises(ValueError):
            self.invoke(lambda *args: self.fail("取得へ進んだ"))
        self.assertEqual(self.shared.read_bytes(), b"existing")

    def installed_fixture(self):
        self.invoke(lambda *args: None)
        (self.root / "requirements-runtime.txt").write_text(RUNTIME.FROZEN, encoding="utf-8")
        return {str(path.relative_to(self.root)): path.read_bytes()
                for path in self.root.rglob("*") if path.is_file()}

    def test_既導入確認成功は取得せずrootを変更しない(self):
        before = self.installed_fixture()
        self.invoke(lambda *args: self.fail("既導入確認が再取得へ進んだ"))
        after = {str(path.relative_to(self.root)): path.read_bytes()
                 for path in self.root.rglob("*") if path.is_file()}
        self.assertEqual(before, after)
        self.assertEqual(json.loads(self.status.read_text(encoding="utf-8"))["state"], "completed")
        self.assertEqual(RUNTIME.environment(self.root.resolve())["PYTHONPATH"], str(self.root.resolve()))

    def test_既導入確認probe失敗もrootを変更しない(self):
        before = self.installed_fixture()
        with patch.object(RUNTIME.platform, "system", return_value="Windows"), patch.object(RUNTIME.platform, "machine", return_value="AMD64"), patch.object(RUNTIME.subprocess, "run"), patch.object(RUNTIME, "populate") as populate, patch.object(RUNTIME, "run", side_effect=RuntimeError("CUDA確認失敗")):
            with self.assertRaises(RuntimeError):
                RUNTIME.install(self.root, self.shared, self.ffmpeg, self.uv, self.status, True)
            populate.assert_not_called()
        self.assertEqual(before, {str(path.relative_to(self.root)): path.read_bytes()
                                  for path in self.root.rglob("*") if path.is_file()})
        self.assertEqual(json.loads(self.status.read_text(encoding="utf-8"))["state"], "failed")

    def test_既導入固定版不一致では検証も取得もしない(self):
        self.installed_fixture()
        record_path = self.root / "install.json"
        record = json.loads(record_path.read_text(encoding="utf-8"))
        record["source"]["commit"] = "不一致"
        record_path.write_text(json.dumps(record), encoding="utf-8")
        before = record_path.read_bytes()
        with self.assertRaises(ValueError):
            self.invoke(lambda *args: self.fail("不一致環境が取得へ進んだ"))
        self.assertEqual(record_path.read_bytes(), before)

    def test_所有済み未確定rootは診断退避して新規stageで再試行する(self):
        self.root.mkdir(parents=True)
        (self.root / "stage-owner.json").write_text(json.dumps({"destination": str(self.root.resolve()), "pid": 123}), encoding="utf-8")
        (self.root / "partial-source").write_bytes(b"interrupted")
        def failure(stage, *args):
            self.assertFalse(self.root.exists())
            diagnostics = [path for path in self.root.parent.glob(".installing-*") if (path / "partial-source").exists()]
            self.assertEqual(len(diagnostics), 1)
            raise RuntimeError("新規probe失敗")
        with self.assertRaises(RuntimeError):
            self.invoke(failure)
        self.assertFalse(self.root.exists())
        self.assertEqual(len(list(self.root.parent.glob(".installing-*"))), 2)

    def test_所有済みでも利用者データがある未確定rootは移動しない(self):
        self.root.mkdir(parents=True)
        (self.root / "stage-owner.json").write_text(json.dumps({"destination": str(self.root.resolve()), "pid": 123}), encoding="utf-8")
        data = self.root / "Data/user/audio.wav"
        data.parent.mkdir(parents=True)
        data.write_bytes(b"user-audio")
        with self.assertRaises(ValueError):
            self.invoke(lambda *args: self.fail("利用者データがあるのに取得へ進んだ"))
        self.assertEqual(data.read_bytes(), b"user-audio")
        self.assertFalse(list(self.root.parent.glob(".installing-*")))

    def test_所有記録の導入先不一致では未確定rootを移動しない(self):
        self.root.mkdir(parents=True)
        owner = self.root / "stage-owner.json"
        owner.write_text(json.dumps({"destination": str(self.base / "別領域"), "pid": 123}), encoding="utf-8")
        before = owner.read_bytes()
        with self.assertRaises(ValueError):
            self.invoke(lambda *args: self.fail("所有不一致なのに取得へ進んだ"))
        self.assertEqual(owner.read_bytes(), before)
        self.assertFalse(list(self.root.parent.glob(".installing-*")))

    def cli_args(self):
        return ["setup-character-learning-runtime.py", "--root", str(self.root),
                "--shared-python", str(self.shared), "--status", str(self.status),
                "--ffmpeg-path", str(self.ffmpeg), "--uv", str(self.uv), "--nvidia-confirmed"]

    def test_CLI事前検証失敗は前回完了状態を失敗理由で置き換える(self):
        RUNTIME.atomic_json(self.status, {"state": "completed"})
        self.ffmpeg.unlink()
        with patch.object(RUNTIME.sys, "argv", self.cli_args()), patch.object(RUNTIME.platform, "system", return_value="Windows"), patch.object(RUNTIME.platform, "machine", return_value="AMD64"), patch.object(RUNTIME.sys, "stderr"):
            self.assertEqual(RUNTIME.main(), 1)
        status = json.loads(self.status.read_text(encoding="utf-8"))
        self.assertEqual(status["state"], "failed")
        self.assertEqual(status["phase"], "setup")
        self.assertIn("管理実体の実行ファイルがありません", status["error"])
        self.assertFalse(self.root.exists())

    def test_CLI既導入probe失敗はrootを変更せず具体的理由を保存する(self):
        before = self.installed_fixture()
        with patch.object(RUNTIME.sys, "argv", self.cli_args()), patch.object(RUNTIME.platform, "system", return_value="Windows"), patch.object(RUNTIME.platform, "machine", return_value="AMD64"), patch.object(RUNTIME.subprocess, "run"), patch.object(RUNTIME, "run", side_effect=RuntimeError("CUDA確認失敗")), patch.object(RUNTIME.sys, "stderr"):
            self.assertEqual(RUNTIME.main(), 1)
        self.assertEqual(json.loads(self.status.read_text(encoding="utf-8"))["error"], "CUDA確認失敗")
        self.assertEqual(before, {str(path.relative_to(self.root)): path.read_bytes()
                                  for path in self.root.rglob("*") if path.is_file()})

    def test_既知minor互換とdriver境界を判定する(self):
        for capability in ("7.0", "7.5", "8.0", "8.6", "8.7", "8.9", "9.0", "10.0", "10.3", "12.0", "12.1"):
            with self.subTest(capability=capability):
                self.assertEqual(RUNTIME.select_cuda_gpu(f"2, {capability}, 570.65")["CUDA_VISIBLE_DEVICES"], "2")
        for output in ("0, 6.1, 590.00", "0, 12.0, 570.64", "0, 13.0, 590.00", "0, N/A, 590.00", "不正", ""):
            with self.subTest(output=output), self.assertRaises(ValueError):
                RUNTIME.select_cuda_gpu(output)

    def test_複数GPUは対応先頭一枚に固定し後続不正行も拒否する(self):
        self.assertEqual(RUNTIME.select_cuda_gpu("0, 6.1, 590.00\n1, 8.9, 590.00\n2, 12.0, 590.00"), {"CUDA_VISIBLE_DEVICES": "1", "CUDA_DEVICE_ORDER": "PCI_BUS_ID"})
        with self.assertRaises(ValueError):
            RUNTIME.select_cuda_gpu("0, 12.0, 590.00\n不正行")

    def test_GPU能力不足ではstage作成も取得もしない(self):
        with patch.object(RUNTIME, "detect_cuda_gpu", side_effect=ValueError("GPU能力不足")):
            with self.assertRaisesRegex(ValueError, "GPU能力不足"):
                self.invoke(lambda *args: self.fail("能力不足なのに取得へ進んだ"))
        self.assertFalse(self.root.parent.exists())

    def test_選択GPUをstageと最終probeと導入記録へ渡す(self):
        gpu = {"CUDA_VISIBLE_DEVICES": "2", "CUDA_DEVICE_ORDER": "PCI_BUS_ID"}
        with patch.object(RUNTIME, "detect_cuda_gpu", return_value=gpu), patch.object(RUNTIME.platform, "system", return_value="Windows"), patch.object(RUNTIME.platform, "machine", return_value="AMD64"), patch.object(RUNTIME.subprocess, "run"), patch.object(RUNTIME, "populate") as populate, patch.object(RUNTIME, "run") as run:
            RUNTIME.install(self.root, self.shared, self.ffmpeg, self.uv, self.status, True)
            self.assertEqual(populate.call_args.args[-1], gpu)
            for call in run.call_args_list:
                self.assertEqual(call.args[2]["CUDA_VISIBLE_DEVICES"], "2")
        self.assertEqual(json.loads((self.root / "install.json").read_text(encoding="utf-8"))["environment"]["CUDA_VISIBLE_DEVICES"], "2")

    def test_一時共有違反後の公開とcli失敗復元は同じrenameでstageを保持する(self):
        original = COMMON.os.rename
        seen = set()
        def rename(source, destination):
            pair = (str(source), str(destination))
            if pair not in seen:
                seen.add(pair)
                error = PermissionError("一時共有違反")
                error.winerror = 32
                raise error
            return original(source, destination)
        def populate(stage, *args):
            (stage / "proof").write_text("new-runtime", encoding="utf-8")
        with patch.object(COMMON.sys, "platform", "win32"), patch.object(COMMON.os, "rename", side_effect=rename) as calls, patch.object(COMMON.time, "sleep"), patch.object(RUNTIME.platform, "system", return_value="Windows"), patch.object(RUNTIME.platform, "machine", return_value="AMD64"), patch.object(RUNTIME.subprocess, "run"), patch.object(RUNTIME, "populate", side_effect=populate), patch.object(RUNTIME, "run", side_effect=RuntimeError("最終CLI失敗")):
            with self.assertRaisesRegex(RuntimeError, "最終CLI失敗"):
                RUNTIME.install(self.root, self.shared, self.ffmpeg, self.uv, self.status, True)
            self.assertEqual(calls.call_count, 4)
        self.assertFalse(self.root.exists())
        stage = next(self.root.parent.glob(".installing-*"))
        self.assertEqual((stage / "proof").read_text(encoding="utf-8"), "new-runtime")
        self.assertFalse((stage / "install.json").exists())
        self.assertTrue((stage / "failure.json").is_file())

    def test_Deno書庫は固定ハッシュと安全な単一実体だけ許可する(self):
        archive = self.base / "deno.zip"
        for name, succeeds in (("deno.exe", True), ("../deno.exe", False)):
            with zipfile.ZipFile(archive, "w") as zipped:
                zipped.writestr(name, b"official-fixture")
            destination = self.base / ("valid-deno" if succeeds else "invalid-deno")
            with patch.object(RUNTIME, "DENO_SIZE", archive.stat().st_size), patch.object(RUNTIME, "DENO_SHA256", RUNTIME.file_digest(archive, True)):
                if succeeds:
                    RUNTIME.unpack_deno(archive, destination)
                    self.assertEqual((destination / "deno.exe").read_bytes(), b"official-fixture")
                else:
                    with self.assertRaises(ValueError):
                        RUNTIME.unpack_deno(archive, destination)
                    self.assertFalse(destination.exists())
        with self.assertRaises(ValueError):
            RUNTIME.unpack_deno(archive, self.base / "checksum-failed")

    def test_Deno実行検証は管理実体に限定しglobalPATHを使わない(self):
        executable = self.root / "tools/deno/deno.exe"
        executable.parent.mkdir(parents=True)
        executable.write_bytes(b"managed-deno")
        with patch.object(RUNTIME, "run") as run:
            self.real_deno_probe(self.root, {}, self.status.parent / "check.log")
            self.assertTrue(all(call.args[0][0] == str(executable) for call in run.call_args_list))
            self.assertIn("--no-remote", run.call_args_list[1].args[0])
        executable.unlink()
        with self.assertRaises(ValueError):
            self.real_deno_probe(self.root, {}, self.status.parent / "check.log")

    def test_YouTube依存とDeno資産は公式固定値を記録する(self):
        self.assertIn("yt-dlp-ejs==0.8.0", RUNTIME.FROZEN.splitlines())
        self.installed_fixture()
        record = json.loads((self.root / "install.json").read_text(encoding="utf-8"))
        self.assertEqual(record["tools"]["deno"]["archiveSha256"], RUNTIME.DENO_SHA256)
        self.assertEqual(record["environment"]["DENO_DIR"], str(self.root.resolve() / "cache/deno"))




class AtomicMoveTests(unittest.TestCase):
    def setUp(self):
        repo = Path(__file__).resolve().parents[2]
        (repo / "temp").mkdir(exist_ok=True)
        temporary = tempfile.TemporaryDirectory(dir=repo / "temp")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    @staticmethod
    def sharing_error(code):
        error = PermissionError("原子的配置の共有拒否")
        error.winerror = code
        return error

    def test_共有違反だけ同じ操作で再試行し成功後に一時物を残さない(self):
        for replace in (False, True):
            for code in (5, 32, 33):
                source, target = self.root / "incoming", self.root / "target"
                source.write_text("new", encoding="utf-8")
                if replace:
                    target.write_text("old", encoding="utf-8")
                operation_name = "replace" if replace else "rename"
                operation = getattr(COMMON.os, operation_name)
                count = 0
                def move(*args):
                    nonlocal count
                    count += 1
                    if count < 3:
                        raise self.sharing_error(code)
                    return operation(*args)
                with patch.object(COMMON.sys, "platform", "win32"), patch.object(COMMON.os, operation_name, side_effect=move) as calls, patch.object(COMMON.time, "sleep") as sleep:
                    COMMON.atomic_move(source, target, replace=replace)
                    self.assertEqual(calls.call_count, 3)
                    self.assertEqual(sleep.call_count, 2)
                    self.assertTrue(all(call.args == (source, target) for call in calls.call_args_list))
                self.assertFalse(source.exists())
                self.assertEqual(target.read_text(encoding="utf-8"), "new")
                target.unlink()

    def test_共有違反の上限は元例外と旧版を保持しjson一時物を回収する(self):
        target = self.root / "status.json"
        target.write_text("old", encoding="utf-8")
        error = self.sharing_error(5)
        with patch.object(COMMON.sys, "platform", "win32"), patch.object(COMMON.os, "replace", side_effect=error) as calls, patch.object(COMMON.time, "sleep") as sleep:
            with self.assertRaises(PermissionError) as raised:
                RUNTIME.atomic_json(target, {"state": "completed"})
            self.assertIs(raised.exception, error)
            self.assertEqual(calls.call_count, len(COMMON.ATOMIC_MOVE_DELAYS) + 1)
            self.assertEqual(sleep.call_count, len(COMMON.ATOMIC_MOVE_DELAYS))
        self.assertEqual(target.read_text(encoding="utf-8"), "old")
        self.assertEqual(list(self.root.glob("*.incoming")), [])

    def test_長期lockのrenameは未確定stageを保持しその他osエラーは即時返す(self):
        stage, destination = self.root / "stage", self.root / "runtime"
        stage.mkdir()
        (stage / "value").write_text("new", encoding="utf-8")
        for platform, code, attempts in (("win32", 32, len(COMMON.ATOMIC_MOVE_DELAYS) + 1), ("win32", 87, 1), ("linux", 5, 1)):
            error = self.sharing_error(code)
            with patch.object(COMMON.sys, "platform", platform), patch.object(COMMON.os, "rename", side_effect=error) as calls, patch.object(COMMON.time, "sleep") as sleep:
                with self.assertRaises(PermissionError) as raised:
                    COMMON.atomic_move(stage, destination, replace=False)
                self.assertIs(raised.exception, error)
                self.assertEqual(calls.call_count, attempts)
                self.assertEqual(sleep.call_count, attempts - 1)
            self.assertTrue(stage.is_dir())
            self.assertFalse(destination.exists())
            self.assertEqual((stage / "value").read_text(encoding="utf-8"), "new")


if __name__ == "__main__":
    unittest.main()
