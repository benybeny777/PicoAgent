"""GPUを使わず、専用HTTPサーバの評価・中断・回収を検査する。"""

import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from contextlib import contextmanager
from unittest import mock


SPEC = importlib.util.spec_from_file_location("lora_runner", Path(__file__).with_name("runner.py"))
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


MOCK_SERVER = r'''
import json
import os
import sys
import time
from http.server import BaseHTTPRequestHandler, HTTPServer

port, key, mode = int(sys.argv[1]), sys.argv[2], sys.argv[3]
if mode == "起動待機":
    time.sleep(60)
if mode == "即終了":
    raise SystemExit(7)
if mode in ("NVIDIA", "Intel", "複数GPU"):
    names = ["Intel(R) Graphics"] if mode == "Intel" else ["NVIDIA GeForce RTX 5060 Laptop GPU"]
    if mode == "複数GPU":
        names.append("Intel(R) Graphics")
    for index, name in enumerate(names):
        print(f"llama_prepare_model_devices: using device Vulkan{index} ({name}) (unknown id) - 100 MiB free", flush=True)

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, status, value):
        body = json.dumps(value, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def authorized(self):
        if self.headers.get("Authorization") != "Bearer " + key:
            self.reply(401, {"error": "認証失敗"})
            return False
        return True

    def do_GET(self):
        if self.authorized():
            self.reply(200, {"default_generation_settings": {"n_ctx": 4096}, "pid": os.getpid()})

    def do_POST(self):
        if not self.authorized():
            return
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if self.path == "/tokenize":
            self.reply(200, {"tokens": [1, 2, 3] if body["add_special"] and body["parse_special"] else []})
            return
        question = body["messages"][-1]["content"]
        if question == "失敗":
            self.reply(500, {"error": "意図した検査エラー"})
        elif question == "リダイレクト":
            self.send_response(302)
            self.send_header("Location", "https://example.invalid/forbidden")
            self.end_headers()
        elif question == "待機":
            time.sleep(5)
            self.reply(200, {})
        elif self.path == "/apply-template":
            self.reply(200, {"prompt": "<|startoftext|>" + json.dumps(body, ensure_ascii=False)})
        else:
            self.reply(200, {"choices": [{"message": {"content": "私は麦茶が好きです。",
                "tool_calls": [{"function": {"name": "calculate", "arguments": "{}"}}]},
                "finish_reason": "stop"}], "timings": {"predicted_ms": 4.0},
                "usage": {"prompt_tokens": 5, "completion_tokens": 3}})

HTTPServer(("127.0.0.1", port), Handler).serve_forever()
'''


class RunnerTests(unittest.TestCase):
    def test_実GPU名の一致だけで推論を許可し違うGPUと証拠欠落は回収する(self):
        for startup in ("NVIDIA", "Intel", "通常", "複数GPU"):
            with self.subTest(startup=startup), self.environment(startup=startup, seeds="1") as (args, _, created):
                args.gpu_layers = 99
                args.expected_device_name = "NVIDIA GeForce RTX 5060 Laptop GPU"
                with mock.patch.object(runner, "evaluate_case", wraps=runner.evaluate_case) as evaluate:
                    if startup == "NVIDIA":
                        result = runner.run(args)
                        self.assertEqual(evaluate.call_count, 1)
                        evidence = result["server"]["device_name_evidence"]
                        self.assertTrue(evidence["passed"])
                        self.assertEqual(evidence["devices"][0]["name"], args.expected_device_name)
                    else:
                        with self.assertRaisesRegex(ValueError, "GPU名"):
                            runner.run(args)
                        evaluate.assert_not_called()
                        result = json.loads(args.output.read_text(encoding="utf-8"))
                        self.assertEqual(result["status"], "failed")
                        self.assertEqual(result["results"], [])
                self.assertEqual(len(created), 1)
                self.assertIsNotNone(created[0][0].returncode)

    def test_CPUへのGPU名指定と空の実GPU名は起動前に拒否する(self):
        for layers, expected in ((0, "NVIDIA"), (99, " ")):
            with self.subTest(layers=layers), self.environment() as (args, _, created):
                args.gpu_layers, args.expected_device_name = layers, expected
                with self.assertRaisesRegex(ValueError, "GPU99"):
                    runner.run(args)
                self.assertEqual(created, [])

    def test_生成profileは既定互換を保ち指定値だけを独立して上書きする(self):
        case = {"body": {"messages": [{"role": "user", "content": "質問"}],
                         "temperature": 0.8, "top_k": 30, "top_p": 0.9,
                         "repeat_penalty": 1.2, "tools": []}}
        before = copy.deepcopy(case)
        expected = {"current": (0.6, 30, 1.2), "official": (0.1, 50, 1.05), "greedy": (0, 50, 1.05)}
        for profile, values in expected.items():
            with self.subTest(profile=profile):
                body = runner.generation_body(case, 2, 128, profile)
                self.assertEqual(tuple(body[key] for key in ("temperature", "top_k", "repeat_penalty")), values)
                self.assertEqual(body["top_p"], 0.9)
                self.assertEqual(body["seed"], 2)
                self.assertEqual(body["max_tokens"], 128)
                self.assertFalse(body["cache_prompt"])
                self.assertFalse(body["stream"])
                body["messages"].clear()
                self.assertEqual(case, before)
        self.assertEqual(runner.generation_body(case, 2, 128), runner.generation_body(case, 2, 128, "current"))
        settings = runner.generation_settings("official")
        settings["temperature"] = 9
        self.assertEqual(runner.generation_settings("official")["temperature"], 0.1)
        with self.assertRaisesRegex(ValueError, "不明な生成profile"):
            runner.generation_body(case, 1, 128, "typo")

    def test_全profileを要求と結果メタ情報へ同じ条件で保存する(self):
        for profile in runner.GENERATION_PROFILES:
            with self.subTest(profile=profile), self.environment(seeds="1") as (args, _, _):
                args.generation_profile = profile
                result = runner.run(args)
                saved = json.loads(args.output.read_text(encoding="utf-8"))
                self.assertEqual(saved["parameters"]["generation_profile"], profile)
                for key, value in runner.generation_settings(profile).items():
                    self.assertEqual(saved["parameters"][key], value)
                    self.assertEqual(result["results"][0]["request"][key], value)

    def test_入力検査は生成profileを受け取らず本番bodyだけを検査する(self):
        from contextlib import redirect_stderr
        import io

        required = [item for name in ("server", "model", "template", "input", "output")
                    for item in ("--" + name, name)]
        with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            runner.parser().parse_args(["inspect", *required, "--gpu-layers", "0", "--generation-profile", "official"])

    def test_起動詳細度は既定5で明示値を優先し引数を重複させない(self):
        args = runner.parser().parse_args([
            "evaluate", "--server", "server.exe", "--model", "model.gguf",
            "--template", "template.jinja", "--input", "cases.json",
            "--output", "result.json", "--gpu-layers", "99",
        ])
        for supplied, expected in (({}, 5), ({"verbosity": None}, 5), ({"verbosity": 0}, 0),
                                   ({"verbosity": 3}, 3), ({"verbosity": 5}, 5)):
            with self.subTest(指定=supplied):
                settings = copy.copy(args)
                vars(settings).update(supplied)
                command = runner.server_arguments(settings, 12345, "test-key")
                self.assertEqual(command.count("--verbosity"), 1)
                self.assertEqual(command[command.index("--verbosity") + 1], str(expected))

    @contextmanager
    def environment(self, mode="evaluate", questions=("質問",), startup="通常", seeds="1,2"):
        directory = Path(__file__).resolve().parents[2] / "temp" / "lora-runner-tests"
        directory.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=directory) as temporary:
            root = Path(temporary)
            server = root / "mock_server.py"
            server.write_text(MOCK_SERVER, encoding="utf-8")
            model = root / "model.gguf"
            model.write_bytes(b"GGUF")
            template = root / "template.jinja"
            template.write_text("{{ messages }}", encoding="utf-8")
            cases = [{"id": f"case-{number}", "split": "dev", "expected": {"owner": "キャラ"},
                      "target": {"role": "assistant", "content": "正解"},
                      "body": {"messages": [{"role": "user", "content": question}],
                               "model": "lfm25_1_2b_jp", "temperature": 0.6,
                               "tools": [{"type": "function", "function": {"name": "calculate"}}],
                               "chat_template_kwargs": {"enable_thinking": False}}}
                     for number, question in enumerate(questions)]
            source = root / "cases.json"
            source.write_text(json.dumps({"cases": cases}), encoding="utf-8")
            # Windowsのvenvランチャーは子Pythonを作るため、実体を直接起動する。
            executable = getattr(sys, "_base_executable", sys.executable)
            args = runner.parser().parse_args([
                mode, "--server", executable, "--model", str(model), "--template", str(template),
                "--input", str(source), "--output", str(root / "result.json"),
                "--gpu-layers", "0", "--seeds", seeds,
            ])
            created = []
            original_popen = subprocess.Popen

            def spawn(*popen_args, **kwargs):
                process = original_popen(*popen_args, **kwargs)
                created.append((process, kwargs))
                return process

            def command(_args, port, key):
                return [executable, str(server), str(port), key, startup]

            with mock.patch.object(runner, "server_arguments", side_effect=command), \
                 mock.patch.object(runner.subprocess, "Popen", side_effect=spawn):
                try:
                    yield args, cases, created
                finally:
                    for process, _ in created:
                        if process.poll() is None:
                            runner.stop_process(process)
                            self.fail("検査対象のサーバが回収されていません")

    def test_本番bodyを保持し全seedの生回答と未採点状態を保存する(self):
        with self.environment() as (args, cases, created):
            original = copy.deepcopy(cases)
            with mock.patch.dict(os.environ, {"HF_TOKEN": "漏らしてはいけない", "LLAMA_ARG_MODEL_URL": "外部URL"}):
                result = runner.run(args)
            self.assertEqual(result["status"], "completed")
            self.assertEqual(result["parameters"]["generation_profile"], "current")
            self.assertEqual([item["seed"] for item in result["results"]], [1, 2])
            for item in result["results"]:
                self.assertEqual(item["request"]["tools"], cases[0]["body"]["tools"])
                self.assertFalse(item["request"]["cache_prompt"])
                self.assertFalse(item["request"]["stream"])
                self.assertEqual(item["request"]["temperature"], 0.6)
                self.assertEqual(item["timings"], {"predicted_ms": 4.0})
                self.assertEqual(item["content"], "私は麦茶が好きです。")
                self.assertEqual(item["tool_calls"][0]["function"]["name"], "calculate")
                self.assertIsNone(item["human_judgment"])
            self.assertEqual(cases, original)
            self.assertIsNotNone(created[0][0].returncode)
            self.assertEqual(result["server"]["props"]["pid"], created[0][0].pid)
            self.assertNotIn("HF_TOKEN", created[0][1]["env"])
            self.assertNotIn("LLAMA_ARG_MODEL_URL", created[0][1]["env"])
            self.assertEqual(json.loads(args.output.read_text(encoding="utf-8"))["status"], "completed")
            self.assertTrue(args.output.with_name("result.server.log").is_file())

    def test_本番のtoolsと特殊token設定で入力を検査する(self):
        with self.environment(mode="inspect") as (args, cases, _):
            result = runner.run(args)
            item = result["results"][0]
            self.assertEqual(result["expected_results"], 1)
            self.assertEqual(item["request"], cases[0]["body"])
            self.assertEqual(item["tokens"], [1, 2, 3])
            self.assertIn('"tools"', item["prompt"])
            self.assertTrue(item["tokenize_request"]["add_special"])
            self.assertTrue(item["tokenize_request"]["parse_special"])
            self.assertIsNone(item["parity_judgment"])

    def test_途中のHTTP失敗でも前の結果を保持し失敗状態で停止する(self):
        with self.environment(questions=("質問", "失敗"), seeds="1") as (args, _, created):
            with self.assertRaises(runner.error.HTTPError):
                runner.run(args)
            saved = json.loads(args.output.read_text(encoding="utf-8"))
            self.assertEqual(saved["status"], "failed")
            self.assertEqual([item["status"] for item in saved["results"]], ["completed", "failed"])
            self.assertEqual(saved["results"][1]["request"]["messages"][0]["content"], "失敗")
            self.assertIsNotNone(created[0][0].returncode)

    def test_起動時間切れと応答時間切れの両方でサーバを回収する(self):
        for startup, question, parameter in (("起動待機", "質問", "startup_timeout"),
                                             ("通常", "待機", "request_timeout")):
            with self.subTest(条件=parameter), self.environment(startup=startup, questions=(question,)) as (args, _, created):
                setattr(args, parameter, 0.2)
                with self.assertRaises(TimeoutError):
                    runner.run(args)
                saved = json.loads(args.output.read_text(encoding="utf-8"))
                self.assertEqual(saved["status"], "failed")
                self.assertIsNotNone(created[0][0].returncode)

    def test_中断状態を保存して自分のサーバを回収する(self):
        with self.environment() as (args, _, created):
            with mock.patch.object(runner, "evaluate_case", side_effect=KeyboardInterrupt):
                with self.assertRaises(KeyboardInterrupt):
                    runner.run(args)
            saved = json.loads(args.output.read_text(encoding="utf-8"))
            self.assertEqual(saved["status"], "interrupted")
            self.assertEqual(saved["results"][0]["status"], "interrupted")
            self.assertIsNotNone(created[0][0].returncode)

    def test_起動直後に終了したサーバを成功扱いしない(self):
        with self.environment(startup="即終了") as (args, _, created):
            with self.assertRaisesRegex(RuntimeError, "サーバが終了"):
                runner.run(args)
            saved = json.loads(args.output.read_text(encoding="utf-8"))
            self.assertEqual(saved["status"], "failed")
            self.assertEqual(saved["results"], [])
            self.assertEqual(created[0][0].returncode, 7)

    def test_既存結果と既存ログを上書きせずサーバも起動しない(self):
        for name in ("result.json", "result.server.log", "result.results.jsonl", "result.json.pending"):
            with self.subTest(対象=name), self.environment() as (args, _, created):
                path = args.output.with_name(name)
                path.write_text("保持", encoding="utf-8")
                with self.assertRaises(FileExistsError):
                    runner.run(args)
                self.assertEqual(path.read_text(encoding="utf-8"), "保持")
                self.assertEqual(created, [])

    def test_固有認証キーで他サーバを使わずリダイレクトも拒否する(self):
        with self.environment(questions=("リダイレクト",)) as (args, _, _):
            with self.assertRaisesRegex(ValueError, "リダイレクト"):
                runner.run(args)
        with self.environment() as (args, _, _):
            with runner.local_server(args, args.output.with_name("auth.log")) as (client, _):
                client.api_key = "wrong-key"
                with self.assertRaises(runner.error.HTTPError) as caught:
                    client.json("/props")
                self.assertEqual(caught.exception.code, 401)

    def test_終了待ちが時間切れならkill後もwaitする(self):
        process = mock.Mock()
        process.poll.return_value = None
        process.wait.side_effect = [subprocess.TimeoutExpired("検査", 5), 1]
        runner.stop_process(process)
        self.assertEqual(process.mock_calls, [mock.call.poll(), mock.call.terminate(),
                         mock.call.wait(timeout=5), mock.call.kill(), mock.call.wait(timeout=5)])

    def test_結果は一件ずつ追記し全JSONを各ケースで再保存しない(self):
        with self.environment(mode="inspect", questions=tuple("質問" for _ in range(24))) as (args, cases, _):
            args.snapshot_seconds = 3600
            with mock.patch.object(runner, "save_result", wraps=runner.save_result) as snapshots:
                result = runner.run(args)
            self.assertEqual(snapshots.call_count, 2)
            journal = args.output.with_name("result.results.jsonl")
            recorded = runner.read_journal(journal, [(case["id"], None) for case in cases], require_complete=True)
            self.assertEqual(recorded, result["results"])
            self.assertEqual(result["journal"]["verified_records"], 24)
            self.assertEqual(len(journal.read_text(encoding="utf-8").splitlines()), 24)

    def test_ジャーナルの欠損重複途中行を復元時に拒否する(self):
        with self.environment() as (args, _, _):
            journal = args.output.with_name("check.jsonl")
            planned = [("case-0", 1), ("case-1", 1), ("case-2", 1)]
            rows = [{"case_id": case_id, "seed": seed, "status": "completed"} for case_id, seed in planned]
            for selected, complete in ((rows[:-1], True), ([rows[0], rows[2]], False),
                                       ([rows[0], rows[0]], False)):
                journal.write_text("".join(json.dumps(row) + "\n" for row in selected), encoding="utf-8")
                with self.assertRaisesRegex(ValueError, "欠損|重複|不足"):
                    runner.read_journal(journal, planned, require_complete=complete)
            journal.write_text(json.dumps(rows[0]), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "未完"):
                runner.read_journal(journal, planned, require_complete=False)

    def test_Windows共有拒否は既存の共通処理で有限回だけ再試行する(self):
        from scripts.voice_training_common import ATOMIC_MOVE_DELAYS

        with self.environment() as (args, _, _):
            args.output.write_text("保持", encoding="utf-8")

            def rejected(code):
                failure = PermissionError("検査用の共有拒否")
                failure.winerror = code
                return failure

            # OS操作の成否だけを制御する。実replaceの追加共有拒否を回数検査へ混ぜない。
            # 実ファイルへの保存結果は、mockサーバを使う通常・失敗・中断検査で確認する。
            scenarios = (
                ([rejected(5), rejected(32), None], 3, None),
                (rejected(33), len(ATOMIC_MOVE_DELAYS) + 1, PermissionError),
                (rejected(87), 1, PermissionError),
            )
            for effects, attempts, expected_error in scenarios:
                with self.subTest(試行上限=attempts), \
                     mock.patch("scripts.voice_training_common.sys.platform", "win32"), \
                     mock.patch("scripts.voice_training_common.os.replace", side_effect=effects) as replace, \
                     mock.patch("scripts.voice_training_common.time.sleep") as sleep:
                    if expected_error:
                        with self.assertRaises(expected_error):
                            runner.save_result(args.output, {"完了": True})
                    else:
                        runner.save_result(args.output, {"完了": True})
                    self.assertEqual(replace.call_count, attempts)
                    self.assertEqual(sleep.call_args_list, [mock.call(delay) for delay in ATOMIC_MOVE_DELAYS[:attempts - 1]])
                    self.assertFalse(args.output.with_name("result.json.pending").exists())
                    self.assertEqual(args.output.read_text(encoding="utf-8"), "保持")

    def test_不正な入力と空の選択結果を実行前に拒否する(self):
        with self.environment() as (args, cases, created):
            with self.assertRaisesRegex(ValueError, "指定条件"):
                runner.load_cases(args.input, split="ない区分")
            for change in (lambda case: case.pop("body"), lambda case: case.update(id="")):
                invalid = copy.deepcopy(cases)
                change(invalid[0])
                args.input.write_text(json.dumps({"cases": invalid}), encoding="utf-8")
                with self.assertRaises(ValueError):
                    runner.run(args)
            self.assertEqual(created, [])


if __name__ == "__main__":
    unittest.main()
