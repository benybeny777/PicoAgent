"""モデルを起動せず、SSEの境界・絶対期限・保存・比較の根拠を検査する。"""

import argparse
from collections import deque
from contextlib import contextmanager, redirect_stdout
import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
import json
from pathlib import Path
import tempfile
import threading
import time
import unittest
from unittest import mock

import benchmark
import test_runner


def sse(*events):
    return b"".join(b"data: " + (event if isinstance(event, str) else json.dumps(event, ensure_ascii=False)).encode("utf-8")
                    + b"\r\n\r\n" for event in events)


def response_events(tokens=4, cache=0):
    return [{"choices": [{"index": 0, "delta": {"role": "assistant"}}]},
            {"choices": [{"delta": {"reasoning_content": "思考"}}]},
            {"choices": [{"delta": {"content": "　"}}]},
            {"choices": [{"delta": {"content": "答えです。"}}]},
            {"choices": [{"delta": {}, "finish_reason": "stop"}]},
            {"choices": [], "timings": {"cache_n": cache, "prompt_n": 12 - cache, "prompt_ms": 20,
                                        "predicted_n": tokens, "predicted_ms": 100},
             "usage": {"prompt_tokens": 12, "completion_tokens": tokens}}, "[DONE]"]


class PartsResponse:
    def __init__(self, parts):
        self.parts = deque(parts)

    def getheader(self, name, default):
        return "text/event-stream; charset=utf-8"

    def read1(self, count):
        return self.parts.popleft() if self.parts else b""


class PartsClient:
    def __init__(self, parts):
        self.parts = parts

    @contextmanager
    def open(self, *args, **kwargs):
        yield PartsResponse(self.parts)


class BenchmarkTests(unittest.TestCase):
    def test_SSEのUTF8と改行の分割境界を越えて可視文字だけをTTFTにする(self):
        events = response_events()
        raw = sse(*events)
        result = {}
        # 1byteごとの分割で日本語・CRLF・data行をすべて横切る。
        with mock.patch.object(benchmark.time, "perf_counter", side_effect=[10, 10.25, 10.75]):
            benchmark.measure_stream(PartsClient([raw[index:index + 1] for index in range(len(raw))]), {}, 10, result)
        self.assertEqual(result["content"], "　答えです。")
        self.assertEqual(result["ttft_seconds"], 0.25)
        self.assertEqual(result["elapsed_seconds"], 0.75)
        self.assertEqual(len(result["raw_events"]), len(events))
        self.assertTrue(result["done_received"])
        self.assertEqual(result["generation_tokens_per_second"], 40)

    def test_SSEの複数data行とコメントを解釈する(self):
        first = b': keepalive\n\ndata: {"choices": [\ndata: {"delta": {"content": "A"}}]}\n\n'
        result = {}
        benchmark.measure_stream(PartsClient([first + sse(*response_events()[4:])]), {}, 10, result)
        self.assertEqual(result["content"], "A")

    def test_DONE終了理由計測値の欠落と壊れた応答を成功にしない(self):
        events = response_events()
        variants = [
            (events[:-1], "missing_done"),
            (events[:4] + events[5:], "missing_finish_reason"),
            (events[:5] + events[6:], "missing_metrics"),
            (events[:4] + ["{壊れたJSON"], "invalid_event"),
            (events[:4] + [{"error": "停止"}], "server_error"),
        ]
        broken_metrics = copy.deepcopy(events)
        broken_metrics[-2]["timings"]["cache_n"] = True
        variants.append((broken_metrics, "invalid_metrics"))
        for chunks, reason in variants:
            with self.subTest(reason=reason):
                result = {}
                with self.assertRaises(benchmark.StreamError):
                    benchmark.measure_stream(PartsClient([sse(*chunks)]), {}, 10, result)
                self.assertEqual(result["stop_reason"], reason)
                self.assertEqual(result["content"], "　答えです。")
                self.assertGreaterEqual(result["elapsed_seconds"], 0)

    def test_可視文字なしと生成上限終了を品質合格に変えない(self):
        events = response_events()
        events[3]["choices"][0]["delta"] = {"tool_calls": [{"function": {"name": "calculate", "arguments": "{}"}}]}
        events[4]["choices"][0]["finish_reason"] = "length"
        result = {}
        benchmark.measure_stream(PartsClient([sse(*events)]), {}, 10, result)
        self.assertIsNone(result["ttft_seconds"])
        self.assertTrue(result["no_visible_content"])
        self.assertTrue(result["truncated"])
        self.assertIn("tool_calls", result["raw_events"][3])

    def test_途切れず届く応答にも共有HTTP入口の総時間上限が効く(self):
        disconnected = threading.Event()

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                self.rfile.read(int(self.headers["Content-Length"]))
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                try:
                    for _ in range(200):
                        self.wfile.write(b": keepalive\n\n")
                        self.wfile.flush()
                        time.sleep(0.01)
                except OSError:
                    disconnected.set()

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.01})
        thread.start()
        try:
            process = mock.Mock()
            process.poll.return_value = None
            client = benchmark.runner.LocalClient(server.server_port, "test-only-key", process)
            result = {}
            with self.assertRaises(TimeoutError):
                benchmark.measure_stream(client, {}, 0.15, result)
            self.assertEqual(result["stop_reason"], "timeout")
            self.assertLess(result["elapsed_seconds"], 1)
            self.assertTrue(disconnected.wait(1))
        finally:
            server.shutdown()
            server.server_close()
            thread.join(1)
        self.assertFalse(thread.is_alive())

    @contextmanager
    def environment(self, fail_at=None):
        directory = Path(__file__).resolve().parents[2] / "temp" / "lora-benchmark-tests"
        directory.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=directory) as temporary:
            root = Path(temporary)
            files = {}
            for name in ("server", "model", "adapter", "template"):
                files[name] = root / name
                files[name].write_text("検査専用", encoding="utf-8")
            cases = [{"id": name, "split": "dev", "expected": "意味で判定", "target": {"content": "送信禁止の正解"},
                      "body": {"messages": [{"role": "user", "content": name}], "tools": [],
                               "chat_template_kwargs": {"enable_thinking": False}}} for name in ("dev-A", "dev-B")]
            cases.append({**cases[0], "id": "final-A", "split": "final"})
            files["input"] = root / "cases.json"
            files["input"].write_text(json.dumps({"cases": cases}), encoding="utf-8")
            args = benchmark.parser().parse_args([
                *[item for name, path in files.items() for item in ("--" + name, str(path))],
                "--output", str(root / "results.json"), "--gpu-layers", "0", "--case-ids", "dev-A,dev-B"])
            requests = []
            closed = []

            @contextmanager
            def server(settings, log_path):
                variant = "adapter" if settings.adapter else "base"
                log_path.write_text("offloaded 0/17 layers to GPU\n", encoding="utf-8")

                class Client:
                    @contextmanager
                    def open(self, endpoint, body, timeout):
                        requests.append((variant, body))
                        events = response_events(tokens=4 if variant == "base" else 2,
                                                 cache=11 if body["cache_prompt"] else 0)
                        if len(requests) == fail_at:
                            events = events[:-1]
                        yield PartsResponse([sse(*events)])

                try:
                    yield Client(), {"log": str(log_path), "pid": 123}
                finally:
                    closed.append(variant)

            with mock.patch.object(benchmark.runner, "local_server", side_effect=server), redirect_stdout(io.StringIO()):
                yield args, requests, closed

    def test_明示devだけを選びfinal欠落重複を拒否する(self):
        with self.environment() as (args, _, _):
            for identifiers in (["final-A"], ["missing"], ["dev-A", "dev-A"], []):
                with self.subTest(ids=identifiers), self.assertRaises(ValueError):
                    benchmark.selected_cases(args.input, identifiers)
            source, cases = benchmark.selected_cases(args.input, ["dev-B", "dev-A"])
            self.assertEqual([case["id"] for case in cases], ["dev-B", "dev-A"])
        for value in ("", "a,a", ",".join(str(i) for i in range(9))):
            with self.assertRaises(argparse.ArgumentTypeError):
                benchmark.case_ids(value)

    def test_同一入力を準備と測定に分け短い回答を高速改善にしない(self):
        with self.environment() as (args, requests, closed):
            args.generation_profile = "official"
            document = benchmark.run(args)
            self.assertEqual(document["status"], "completed")
            self.assertEqual(closed, ["base", "adapter"])
            self.assertEqual(len(requests), 32)
            self.assertEqual(document["journal"]["verified_records"], 32)
            self.assertEqual(sum(item["warmup"] for item in document["results"]), 8)
            self.assertEqual(document["parameters"]["generation_profile"], "official")
            for base, adapter in zip(requests[:16], requests[16:]):
                self.assertEqual(base[1], adapter[1])
                for key, value in benchmark.runner.generation_settings("official").items():
                    self.assertEqual(base[1][key], value)
                    self.assertEqual(document["parameters"][key], value)
                self.assertNotIn("送信禁止の正解", json.dumps(base[1], ensure_ascii=False))
                self.assertTrue(base[1]["stream"])
                self.assertEqual(base[1]["seed"], 1)
                self.assertFalse(base[1]["chat_template_kwargs"]["enable_thinking"])
            summary = document["summary"]
            self.assertEqual(len(summary["groups"]), 8)
            for group in summary["groups"]:
                self.assertEqual(group["measured_samples"], 3)
                self.assertEqual(group["metrics"]["generated_tokens"]["count"], 3)
                self.assertEqual(group["metrics"]["cache_n"]["median"], 11 if group["cache_prompt"] else 0)
            for comparison in summary["comparisons"]:
                self.assertFalse(comparison["same_generated_counts"])
                self.assertIsNone(comparison["speed_improvement_judgment"])
                self.assertIsNone(comparison["quality_judgment"])
            self.assertEqual(benchmark.distribution([4, 100, 2]), {"count": 3, "median": 4, "p95": 100})
            original = args.output.read_bytes()
            with self.assertRaises(FileExistsError):
                benchmark.run(args)
            self.assertEqual(args.output.read_bytes(), original)

    def test_失敗の途中回答をジャーナルへ保存し所有サーバのfinallyを通る(self):
        with self.environment(fail_at=3) as (args, requests, closed):
            with self.assertRaisesRegex(benchmark.StreamError, "EOF"):
                benchmark.run(args)
            self.assertEqual(closed, ["base"])
            document = json.loads(args.output.read_text(encoding="utf-8"))
            self.assertEqual(document["status"], "failed")
            self.assertEqual(document["journal"]["verified_records"], 3)
            self.assertEqual(document["results"][-1]["content"], "　答えです。")
            self.assertEqual(document["results"][-1]["stop_reason"], "missing_done")
            self.assertEqual(document["summary"]["groups"][0]["measured_samples"], 1)
            self.assertFalse(document["summary"]["groups"][0]["complete"])

    def test_共用した実プロセス管理は応答時間切れ不正SSE中断でも回収する(self):
        for question, interrupt, exception in (("待機", False, TimeoutError),
                                                ("質問", False, benchmark.StreamError),
                                                ("質問", True, KeyboardInterrupt)):
            with self.subTest(question=question, interrupt=interrupt):
                with test_runner.RunnerTests().environment(questions=(question,), seeds="1") as (args, _, created):
                    args.adapter = args.model
                    args.case_ids = ["case-0"]
                    args.request_timeout = 0.15
                    original = benchmark.measure_stream
                    with mock.patch.object(benchmark, "runner", test_runner.runner), \
                         mock.patch.object(benchmark, "measure_stream", side_effect=KeyboardInterrupt if interrupt else original), \
                         redirect_stdout(io.StringIO()):
                        with self.assertRaises(exception):
                            benchmark.run(args)
                    self.assertEqual(len(created), 1)
                    self.assertIsNotNone(created[0][0].returncode)
                    saved = json.loads(args.output.read_text(encoding="utf-8"))
                    self.assertEqual(saved["status"], "interrupted" if interrupt else "failed")

    def test_準備測定を集計から除きケースごとの分位を保存する(self):
        with self.environment() as (args, _, _):
            results = benchmark.run(args)["results"]
            for result in results:
                result["elapsed_seconds"] = (9999 if result["warmup"] else
                                             result["repetition"] + (100 if result["case_id"] == "dev-B" else 0))
            for group in benchmark.summarize(results)["groups"]:
                expected = 102 if group["case_id"] == "dev-B" else 2
                self.assertEqual(group["metrics"]["elapsed_seconds"]["median"], expected)
                self.assertEqual(group["metrics"]["elapsed_seconds"]["p95"], expected + 1)


if __name__ == "__main__":
    unittest.main()
