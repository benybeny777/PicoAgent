"""架空の結果だけを返す連続ツール推論検査。実ファイル・外部APIは操作しない。"""

import argparse
import copy
import json
import os
from pathlib import Path
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools/lora"))
import runner
from probe import backend_evidence

SEEDS = (1, 2, 3)
VARIANTS = ("base", "adapter")
FIXTURE = Path(__file__).parent / "fixtures/tool-chain-eval.json"
BOUNDARY = "隔離した推論連鎖のみ。実アプリの停止・人格変更・音声の受入ではない。意味レビューは別記録とし、AI評価と人手評価を区別する。"


class ChainError(ValueError):
    pass


class ResponseCapture:
    def __init__(self, client, event):
        self.client, self.event = client, event

    def json(self, *args, **kwargs):
        response = self.client.json(*args, **kwargs)
        self.event["raw_response"] = copy.deepcopy(response)
        return response


def require(condition, reason):
    if not condition:
        raise ChainError(reason)


def unique_object(pairs):
    value = {}
    for key, item in pairs:
        require(key not in value, f"JSON引数のキーが重複しています: {key}")
        value[key] = item
    return value


def load_input(path, fixture_path=FIXTURE):
    source, cases = runner.load_cases(path)
    fixture = json.loads(fixture_path.read_text(encoding="utf-8"))
    require({key: value for key, value in source.items() if key != "cases"}
            == {key: value for key, value in fixture.items() if key != "cases"}, "fixture のメタ情報が変わっています")
    require(len(cases) == len(fixture["cases"]), "fixture と入力のシナリオ数が違います")
    for case, raw in zip(cases, fixture["cases"]):
        require({key: value for key, value in case.items() if key != "body"} == raw,
                "本番body以外のfixtureが変わっています")
        body = case["body"]
        require(body.get("n", 1) == 1, "単一回答だけを検査します")
        require(body["messages"][-1].get("role") == "user", "開始時はuser入力で終わる必要があります")
        require(all(message.get("role") != "tool" and not message.get("tool_calls") for message in body["messages"]),
                "開始時に正解ツール履歴を挿してはいけません")
        names = [tool["function"]["name"] for tool in body.get("tools", [])]
        require(sorted(names) == sorted(case["tool_names"]), "本番bodyのツール一覧が違います")
    return source, cases


def check_arguments(name, arguments, step):
    require(isinstance(arguments, dict), "ツール引数はJSON objectが必要です")
    required = step["arguments"]
    for key, expected in required.items():
        actual = arguments.get(key)
        require(actual == expected and type(actual) is not bool, f"ツール引数が違います: {name}.{key}")
    extra = set(arguments) - set(required)
    if name == "read_file":
        require(extra <= {"offset", "limit"}, "read_file の余分な引数です")
        offset, limit = arguments.get("offset", 1), arguments.get("limit", 800)
        require(type(offset) is int and offset == 1 and type(limit) is int
                and step["line_count"] <= limit <= 2000, "fixture 全文を読める行範囲が必要です")
    elif name == "weather":
        require(extra <= {"days"}, "weather の余分な引数です")
        days = arguments.get("days", 3)
        require(type(days) is int and 1 <= days <= 7, "weather.days が範囲外です")
    else:
        require(not extra, "percent_calc の余分な引数です")


def run_chain(client, case, seed, args, result, record):
    conversation = copy.deepcopy(case)
    steps, seen = case["simulation"]["steps"], set()
    completed = 0
    result.update(generations=0, accepted_tools=[], human_judgment=None,
                  mechanical_chain="未完", final_content=None)
    deadline = time.monotonic() + args.chain_timeout
    for turn in range(1, args.max_generations + 1):
        event = {"event": "generation", "run_id": result["run_id"], "turn": turn,
                 "case_id": case["id"], "variant": result["variant"], "seed": seed,
                 "status": "running", "expected_stage": completed + 1,
                 "request": runner.generation_body(conversation, seed, args.max_tokens,
                                                   getattr(args, "generation_profile", "current"))}
        try:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("連続ツール検査の総時間を超えました")
            settings = copy.copy(args)
            settings.request_timeout = min(args.request_timeout, remaining)
            generated = runner.evaluate_case(ResponseCapture(client, event), conversation, seed, settings)
            event.update(generated)
            result["generations"] = turn
            require(time.monotonic() < deadline, "連続ツール検査の総時間を超えました")
            choices = generated["raw_response"]["choices"]
            require(len(choices) == 1, "複数の回答が返りました")
            message = choices[0]["message"]
            require(message.get("role") == "assistant", "assistant 以外の回答です")
            require(generated["finish_reason"] in ("stop", "tool_calls"), "未完または不明の生成終了理由です")
            content = message.get("content")
            require(content is None or isinstance(content, str), "本文が文字列ではありません")
            calls = message.get("tool_calls") or []
            require(isinstance(calls, list), "tool_calls が配列ではありません")
            if not calls:
                require(generated["finish_reason"] == "stop", "ツール終了なのに呼出がありません")
                require(completed == len(steps), "必要なツールを完了する前に回答を終了しました")
                require(isinstance(content, str) and bool(content.strip()), "最後の回答が空です")
                result.update(mechanical_chain="順序・引数・終了を確認済み", final_content=content,
                              stop_reason="ツール後の実生成回答。意味判定は未実施")
                event.update(status="completed", decision="最終回答を保存。意味判定待ち")
                return
            require(len(calls) == 1, "結果に依存する呼出を並列に出しています")
            require(completed < len(steps), "必要な処理の後に余分なツールを呼んでいます")
            call = calls[0]
            require(isinstance(call, dict) and call.get("type") == "function", "function 呼出ではありません")
            identifier = call.get("id")
            require(isinstance(identifier, str) and bool(identifier.strip()) and identifier not in seen,
                    "呼出IDが空または再利用されています")
            function = call.get("function")
            require(isinstance(function, dict), "function がありません")
            step = steps[completed]
            name = function.get("name")
            require(name == step["name"], f"ツール順序または名前が違います: 期待={step['name']}, 実際={name}")
            raw_arguments = function.get("arguments")
            require(isinstance(raw_arguments, str), "function.arguments はJSON文字列が必要です")
            arguments = json.loads(raw_arguments, object_pairs_hook=unique_object,
                                   parse_constant=lambda value: (_ for _ in ()).throw(ChainError("非有限のJSON引数です")))
            check_arguments(name, arguments, step)
            seen.add(identifier)
            # 呼出ID・引数はモデルの実出力を保持する。正解呼出への置換はしない。
            assistant = copy.deepcopy(message)
            assistant["content"] = content or ""
            tool = {"role": "tool", "tool_call_id": identifier, "content": step["result"]}
            conversation["body"]["messages"].extend([assistant, tool])
            completed += 1
            result["accepted_tools"].append({"id": identifier, "name": name, "arguments": arguments})
            event.update(status="completed", decision="実呼出を検証し架空結果だけを返却",
                         fixture_tool_result=tool, completed_tools=completed)
        except BaseException as exc:
            event.update(status="interrupted" if isinstance(exc, KeyboardInterrupt) else "failed",
                         error=f"{type(exc).__name__}: {exc}")
            raise
        finally:
            record(event)
    raise ChainError(f"最大{args.max_generations}生成までに最終回答へ到達しませんでした")


def run(args):
    selection = getattr(args, "variants", "both")
    require(selection in (*VARIANTS, "both"), "--variants はbase、adapter、bothから指定してください")
    variants = VARIANTS if selection == "both" else (selection,)
    require("adapter" not in variants or args.adapter is not None,
            "adapterまたはbothの検査には--adapterを指定してください")
    fixture_path = getattr(args, "fixture", FIXTURE)
    source, cases = load_input(args.input, fixture_path)
    output = args.output.resolve()
    journal_path = output.with_name(output.stem + ".events.jsonl")
    logs = {variant: output.with_name(output.stem + f".{variant}.server.log") for variant in variants}
    require(not any(path.exists() for path in (output, journal_path, *logs.values(), output.with_name(output.name + ".pending"))),
            "結果またはログは上書きしません。新規出力名を指定してください")
    file_names = ("server", "model", "template", "input") + (("adapter",) if "adapter" in variants else ())
    document = {"schema_version": 1, "status": "running", "boundary": BOUNDARY,
                "started_at": runner.utc_now(), "human_judgment": None,
                "files": {name: runner.file_record(getattr(args, name)) for name in file_names},
                "fixture": runner.file_record(fixture_path), "events": str(journal_path),
                "parameters": {"variants": list(variants), "seeds": list(SEEDS), **runner.generation_parameters(args), "cache_prompt": False,
                               "gpu_layers": args.gpu_layers, "context": args.context, "max_tokens": args.max_tokens,
                               "max_generations": args.max_generations, "chain_timeout": args.chain_timeout},
                "expected_runs": len(variants) * len(cases) * len(SEEDS), "servers": {}, "results": []}
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("x", encoding="utf-8") as handle:
        json.dump(document, handle, ensure_ascii=False, indent=2)
    sequence = 0
    try:
        with journal_path.open("x", encoding="utf-8", newline="\n") as journal:
            def record(event):
                nonlocal sequence
                sequence += 1
                runner.append_result(journal, {"sequence": sequence, **event})
            next_snapshot = time.monotonic() + args.snapshot_seconds
            for variant in variants:
                settings = copy.copy(args)
                settings.adapter = args.adapter if variant == "adapter" else None
                with runner.local_server(settings, logs[variant]) as (client, server):
                    document["servers"][variant] = server
                    for case in cases:
                        for seed in SEEDS:
                            run_id = f"{variant}/{case['id']}/{seed}"
                            result = {"run_id": run_id, "variant": variant, "case_id": case["id"], "seed": seed,
                                      "status": "running", "expected": case["expected"]}
                            document["results"].append(result)
                            record({"event": "run_start", "run_id": run_id, "status": "completed"})
                            try:
                                run_chain(client, case, seed, args, result, record)
                                result["status"] = "completed"
                            except (ChainError, json.JSONDecodeError) as exc:
                                result.update(status="failed", mechanical_chain="不合格", stop_reason=str(exc))
                            except BaseException as exc:
                                result.update(status="interrupted" if isinstance(exc, KeyboardInterrupt) else "failed",
                                              stop_reason=f"{type(exc).__name__}: {exc}")
                                raise
                            finally:
                                record({"event": "run_end", **result})
                            if time.monotonic() >= next_snapshot:
                                os.fsync(journal.fileno())
                                runner.save_result(output, document)
                                next_snapshot = time.monotonic() + args.snapshot_seconds
                            print(f"{run_id}: {result['status']} / {result['stop_reason']}", flush=True)
                evidence = backend_evidence(logs[variant], args.gpu_layers)
                document["servers"][variant]["backend_evidence"] = evidence
                require(evidence["passed"], f"実行バックエンドを確認できません: {evidence['reason']}")
            journal.flush()
            os.fsync(journal.fileno())
        # JSONLは1イベントずつ追記。全文の再保存は一定間隔と終了時だけ。
        ended = []
        with journal_path.open(encoding="utf-8") as handle:
            for expected_sequence, line in enumerate(handle, 1):
                require(line.endswith("\n"), "ジャーナル末尾が途中です")
                event = json.loads(line)
                require(event["sequence"] == expected_sequence, "ジャーナルに欠落または重複があります")
                if event["event"] == "run_end":
                    ended.append({key: value for key, value in event.items() if key not in ("sequence", "event")})
        require(ended == document["results"] and len(ended) == document["expected_runs"], "完了記録が一致しません")
        document["verified_events"] = sequence
        document["status"] = "completed" if all(row["status"] == "completed" for row in ended) else "completed_with_chain_failures"
        return document
    except BaseException as exc:
        document.update(status="interrupted" if isinstance(exc, KeyboardInterrupt) else "failed",
                        error=f"{type(exc).__name__}: {exc}")
        raise
    finally:
        document["finished_at"] = runner.utc_now()
        runner.save_result(output, document)


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    runner.add_server_arguments(result)
    result.add_argument("--variants", choices=(*VARIANTS, "both"), default="both",
                        help="検査対象。既定both。adapterまたはbothには--adapterが必要")
    result.add_argument("--fixture", type=Path, default=FIXTURE, help="入力と照合するraw fixture。既定は開発用3シナリオ")
    result.add_argument("--max-generations", type=int, choices=range(1, 7), default=6, help="1シナリオの生成上限。最大6")
    result.add_argument("--chain-timeout", type=runner.positive_number, default=600, help="1シナリオの総時間上限秒数")
    return result


if __name__ == "__main__":
    try:
        document = run(parser().parse_args())
        raise SystemExit(0 if document["status"] == "completed" else 2)
    except Exception as error:
        print(f"連続ツール検査失敗: {type(error).__name__}: {error}", file=sys.stderr)
        raise SystemExit(1)
