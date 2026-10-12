"""実モデル・サーバを起動せず、連鎖の保持、拒否、記録、所有者の終了を検査する。"""

import copy
from contextlib import contextmanager, redirect_stdout
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

import tool_chain as chain


@contextmanager
def isolated_directory():
    directory = chain.ROOT / "temp/lora-tool-chain-tests"
    directory.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=directory) as folder:
        yield folder


def raw_fixture():
    return json.loads(chain.FIXTURE.read_text(encoding="utf-8"))


def mock_body(case):
    # 自己検査専用。本番実行では必ずRust exporterのbodyを使う。
    case["body"] = {"model": "検査用", "messages": [{"role": "user", "content": case["question"]}],
                    "tools": [{"type": "function", "function": {"name": name, "parameters": {"type": "object"}}}
                              for name in case["tool_names"]]}
    return case


def answer(content="完了しました。", calls=None):
    message = {"role": "assistant", "content": content}
    if calls is not None:
        message["tool_calls"] = calls
    return {"choices": [{"message": message, "finish_reason": "tool_calls" if calls else "stop"}],
            "timings": {"predicted_n": 12}, "usage": {"completion_tokens": 12}}


def call(step, identifier):
    return {"type": "function", "id": identifier,
            "function": {"name": step["name"], "arguments": json.dumps(step["arguments"], ensure_ascii=False)}}


def responses(case):
    return [answer(None, [call(step, f"actual-{index}")]) for index, step in enumerate(case["simulation"]["steps"])] + [answer()]


def execute(case, values, **changes):
    settings = dict(max_tokens=256, request_timeout=2, chain_timeout=20, max_generations=6)
    settings.update(changes)
    args = SimpleNamespace(**settings)
    client = mock.Mock()
    client.json.side_effect = values
    events = []
    result = {"run_id": "検査", "variant": "base"}
    return client, args, result, events


class 連続ツール(unittest.TestCase):
    def test_モデルの実呼出とidをそのまま次の生成へ渡す(self):
        case = mock_body(raw_fixture()["cases"][0])
        before = copy.deepcopy(case)
        values = responses(case)
        client, args, result, events = execute(case, values)
        chain.run_chain(client, case, 2, args, result, events.append)
        self.assertEqual(len(events), 4)
        self.assertEqual(result["generations"], 4)
        self.assertEqual(case, before)
        self.assertIsNone(result["human_judgment"])
        for index, event in enumerate(events):
            request = event["request"]
            self.assertEqual(request["seed"], 2)
            self.assertFalse(request["cache_prompt"])
            self.assertEqual(len(request["messages"]), 1 + 2 * index)
            if index:
                previous = request["messages"][-2]
                self.assertEqual(previous["tool_calls"], values[index - 1]["choices"][0]["message"]["tool_calls"])
                self.assertEqual(request["messages"][-1]["tool_call_id"], f"actual-{index - 1}")
                self.assertEqual(request["messages"][-1]["content"], case["simulation"]["steps"][index - 1]["result"])
            self.assertEqual(event["raw_response"], values[index])

    def test_順序とidと引数の違反は架空結果を返す前に拒否する(self):
        case = mock_body(raw_fixture()["cases"][0])
        for failure in ("名前", "壊れたJSON", "同名キー", "別パス", "複数呼出", "空ID", "ID再利用", "長さ制限"):
            with self.subTest(failure=failure):
                values = responses(case)
                first = values[0]["choices"][0]["message"]["tool_calls"][0]
                if failure == "名前":
                    first["function"]["name"] = "weather"
                elif failure == "壊れたJSON":
                    first["function"]["arguments"] = "{"
                elif failure == "同名キー":
                    first["function"]["arguments"] = '{"path":"x","path":"y"}'
                elif failure == "別パス":
                    first["function"]["arguments"] = '{"path":"実ファイル.txt"}'
                elif failure == "複数呼出":
                    values[0]["choices"][0]["message"]["tool_calls"].append(copy.deepcopy(first))
                elif failure == "空ID":
                    first["id"] = ""
                elif failure == "ID再利用":
                    values[1]["choices"][0]["message"]["tool_calls"][0]["id"] = first["id"]
                else:
                    values[0]["choices"][0]["finish_reason"] = "length"
                client, args, result, events = execute(case, values)
                with self.assertRaises(ValueError):
                    chain.run_chain(client, case, 1, args, result, events.append)
                self.assertEqual(events[-1]["status"], "failed")
                self.assertNotIn("fixture_tool_result", events[-1])
                self.assertEqual(client.json.call_count, 2 if failure == "ID再利用" else 1)

    def test_計算を省略した早期回答と失敗後の後続呼出を拒否する(self):
        for index, continue_after_error in ((0, False), (2, True)):
            with self.subTest(case=index):
                case = mock_body(raw_fixture()["cases"][index])
                values = [answer()]
                if continue_after_error:
                    values = responses(case)
                    values[-1] = answer(None, [call(raw_fixture()["cases"][0]["simulation"]["steps"][1], "wrong-next")])
                client, args, result, events = execute(case, values)
                with self.assertRaises(chain.ChainError):
                    chain.run_chain(client, case, 1, args, result, events.append)
                self.assertEqual(events[-1]["status"], "failed")

    def test_読込失敗を返した後の実回答を別生成として保持する(self):
        case = mock_body(raw_fixture()["cases"][2])
        values = responses(case)
        values[-1] = answer("割合と天気は確認できませんでした。")
        client, args, result, events = execute(case, values)
        chain.run_chain(client, case, 3, args, result, events.append)
        self.assertEqual(client.json.call_count, 2)
        self.assertEqual(len(result["accepted_tools"]), 1)
        self.assertEqual(result["final_content"], "割合と天気は確認できませんでした。")
        self.assertIn("エラー", events[-1]["request"]["messages"][-1]["content"])

    def test_生成上限と期限と不正応答は途中結果を残して止める(self):
        case = mock_body(raw_fixture()["cases"][0])
        for failure in ("生成上限", "期限", "回答欠落"):
            with self.subTest(failure=failure):
                values = responses(case)
                if failure == "期限":
                    values = [TimeoutError("応答期限")]
                elif failure == "回答欠落":
                    values = [{"choices": []}]
                client, args, result, events = execute(case, values, max_generations=2)
                with self.assertRaises((ValueError, TimeoutError)):
                    chain.run_chain(client, case, 1, args, result, events.append)
                self.assertEqual(client.json.call_count, 2 if failure == "生成上限" else 1)
                self.assertEqual(len(events), client.json.call_count)
                if failure == "回答欠落":
                    self.assertEqual(events[-1]["raw_response"], {"choices": []})
        parser = chain.parser()
        option = next(action for action in parser._actions if action.dest == "max_generations")
        self.assertEqual(list(option.choices), [1, 2, 3, 4, 5, 6])

    def test_本番body以外の変更と先置きツール履歴を拒否する(self):
        with isolated_directory() as folder:
            path = Path(folder) / "body.json"
            source = raw_fixture()
            source["cases"] = [mock_body(case) for case in source["cases"]]
            path.write_text(json.dumps(source), encoding="utf-8")
            chain.load_input(path)
            for failure in ("答え混入", "fixture改変"):
                with self.subTest(failure=failure):
                    invalid = copy.deepcopy(source)
                    if failure == "答え混入":
                        invalid["cases"][0]["body"]["messages"].insert(0, {"role": "tool", "content": "正解"})
                    else:
                        invalid["cases"][0]["simulation"]["steps"][0]["result"] = "別の答え"
                    path.write_text(json.dumps(invalid), encoding="utf-8")
                    with self.assertRaises(chain.ChainError):
                        chain.load_input(path)

    def test_十八runの保存と例外時のサーバ所有者終了を確認する(self):
        for failure in (False, True):
            with self.subTest(failure=failure), isolated_directory() as folder:
                folder = Path(folder)
                source = raw_fixture()
                source["cases"] = [mock_body(case) for case in source["cases"]]
                input_path = folder / "body.json"
                input_path.write_text(json.dumps(source), encoding="utf-8")
                args = SimpleNamespace(input=input_path, output=folder / "result.json", adapter=input_path,
                                       server=input_path, model=input_path, template=input_path, gpu_layers=0,
                                       context=4096, max_tokens=256, max_generations=6, chain_timeout=20,
                                       request_timeout=2, snapshot_seconds=30)
                closed, adapters = [], []
                @contextmanager
                def owner(settings, log):
                    adapters.append(settings.adapter)
                    log.write_text("自己検査のmockログ", encoding="utf-8")
                    client = mock.Mock()
                    values = [value for case in source["cases"] for seed in chain.SEEDS for value in responses(case)]
                    client.json.side_effect = [TimeoutError("応答期限")] if failure else values
                    try:
                        yield client, {"pid": "mock"}
                    finally:
                        closed.append(True)
                with mock.patch.object(chain.runner, "local_server", side_effect=owner), \
                     mock.patch.object(chain, "backend_evidence", return_value={"passed": True, "reason": "mock"}), \
                     redirect_stdout(io.StringIO()):
                    if failure:
                        with self.assertRaises(TimeoutError):
                            chain.run(args)
                    else:
                        document = chain.run(args)
                        self.assertEqual(len(document["results"]), 18)
                        self.assertEqual(document["status"], "completed")
                        self.assertEqual(adapters, [None, input_path])
                    with self.assertRaisesRegex(chain.ChainError, "上書き"):
                        chain.run(args)
                self.assertEqual(len(closed), 1 if failure else 2)
                document = json.loads(args.output.read_text(encoding="utf-8"))
                self.assertEqual(document["status"], "failed" if failure else "completed")
                events = [json.loads(line) for line in Path(document["events"]).read_text(encoding="utf-8").splitlines()]
                self.assertEqual(events[-1]["event"], "run_end")
                if failure:
                    self.assertEqual(events[-1]["status"], "failed")
                    self.assertEqual(events[-2]["event"], "generation")


if __name__ == "__main__":
    unittest.main()
