"""明示したdev入力で基底とLoRAのSSE応答時間を測る。品質の自動採点は行わない。"""

from __future__ import annotations

import argparse
import copy
import json
import math
import os
import statistics
import sys
import time

import runner
from probe import backend_evidence


REPETITIONS = 3
MAX_CASES = 8


class StreamError(ValueError):
    def __init__(self, reason, message):
        super().__init__(message)
        self.reason = reason


def case_ids(value):
    values = [item.strip() for item in value.split(",")]
    if not 1 <= len(values) <= MAX_CASES or not all(values) or len(set(values)) != len(values):
        raise argparse.ArgumentTypeError("重複のないdevのcase IDを1〜8件、カンマ区切りで明示してください")
    return values


def selected_cases(path, identifiers):
    source, cases = runner.load_cases(path)
    lookup = {case["id"]: case for case in cases}
    if not identifiers or len(identifiers) > MAX_CASES or len(set(identifiers)) != len(identifiers):
        raise ValueError("devのcase IDを重複なく1〜8件指定してください")
    if any(identifier not in lookup for identifier in identifiers):
        raise ValueError("入力に存在しないcase IDがあります")
    selected = [lookup[identifier] for identifier in identifiers]
    if any(case.get("split") != "dev" for case in selected):
        raise ValueError("速度測定に使えるのは明示したdev入力だけです")
    return source, selected


def stream_body(case, max_tokens, cache_prompt, profile="current"):
    body = runner.generation_body(case, 1, max_tokens, profile)
    if body.get("n", 1) != 1:
        raise ValueError("速度測定は単一の回答だけを対象にします")
    body.update(stream=True, stream_options={"include_usage": True}, cache_prompt=cache_prompt, id_slot=0)
    return body


def check_metrics(result):
    timings = result.get("timings")
    usage = result.get("usage")
    if not isinstance(timings, dict) or not isinstance(usage, dict):
        raise StreamError("missing_metrics", "最終timingsまたはusageがありません")
    for key in ("cache_n", "prompt_n", "predicted_n"):
        value = timings.get(key)
        if type(value) is not int or value < 0:
            raise StreamError("invalid_metrics", f"timings.{key}が非負の整数ではありません")
    for key in ("prompt_ms", "predicted_ms"):
        value = timings.get(key)
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
            raise StreamError("invalid_metrics", f"timings.{key}が非負の有限数ではありません")
    for key in ("prompt_tokens", "completion_tokens"):
        if type(usage.get(key)) is not int or usage[key] < 0:
            raise StreamError("invalid_metrics", f"usage.{key}が非負の整数ではありません")
    result.update(generated_tokens=timings["predicted_n"], cache_n=timings["cache_n"],
                  generation_tokens_per_second=(timings["predicted_n"] * 1000 / timings["predicted_ms"]
                                                if timings["predicted_ms"] > 0 else None),
                  token_counts_agree=timings["predicted_n"] == usage["completion_tokens"])


def measure_stream(client, body, timeout, result):
    """途中結果を呼出元の記録へ残し、EOFや期限切れを成功に変えない。"""
    started = time.perf_counter()
    deadline = time.monotonic() + timeout
    result.update(request=body, raw_events=[], content="", ttft_seconds=None,
                  finish_reason=None, done_received=False, timings=None, usage=None)
    total_bytes = 0
    pending = b""
    data_lines = []

    def event(payload):
        result["raw_events"].append(payload)
        if payload == "[DONE]":
            result["done_received"] = True
            return
        try:
            chunk = json.loads(payload)
        except ValueError as exc:
            raise StreamError("invalid_event", "SSEのdataがJSONではありません") from exc
        if not isinstance(chunk, dict) or "error" in chunk:
            raise StreamError("server_error", "SSEがエラーまたは不正なオブジェクトを返しました")
        if chunk.get("timings") is not None:
            result["timings"] = chunk["timings"]
        if chunk.get("usage") is not None:
            result["usage"] = chunk["usage"]
        choices = chunk.get("choices")
        if not isinstance(choices, list) or len(choices) > 1:
            raise StreamError("invalid_event", "SSEのchoicesが単一回答の形式ではありません")
        if not choices:
            return
        choice = choices[0]
        if not isinstance(choice, dict) or choice.get("index", 0) != 0:
            raise StreamError("invalid_event", "SSEの回答位置が不正です")
        delta = choice.get("delta", {})
        if not isinstance(delta, dict):
            raise StreamError("invalid_event", "SSEのdeltaがオブジェクトではありません")
        content = delta.get("content")
        if content is not None:
            if not isinstance(content, str):
                raise StreamError("invalid_event", "SSEのcontentが文字列ではありません")
            if content.strip() and result["ttft_seconds"] is None:
                result["ttft_seconds"] = time.perf_counter() - started
            result["content"] += content
        if choice.get("finish_reason") is not None:
            reason = choice["finish_reason"]
            if not isinstance(reason, str) or not reason:
                raise StreamError("invalid_event", "SSEの終了理由が不正です")
            result["finish_reason"] = reason

    try:
        with client.open("/v1/chat/completions", body, timeout=timeout) as response:
            if response.getheader("Content-Type", "").split(";", 1)[0].strip() != "text/event-stream":
                raise StreamError("invalid_content_type", "SSE応答ではありません")
            while not result["done_received"]:
                if time.monotonic() >= deadline:
                    raise TimeoutError("SSE応答の総時間が上限を超えました")
                received = response.read1(4096)
                if time.monotonic() >= deadline:
                    raise TimeoutError("SSE応答の総時間が上限を超えました")
                if not received:
                    raise StreamError("missing_done", "[DONE]より前にSSEがEOFになりました")
                total_bytes += len(received)
                if total_bytes > 32_000_000:
                    raise StreamError("response_too_large", "SSE応答が上限32MBを超えています")
                pending += received
                while b"\n" in pending and not result["done_received"]:
                    line, pending = pending.split(b"\n", 1)
                    line = line.rstrip(b"\r")
                    if not line:
                        if data_lines:
                            event(b"\n".join(data_lines).decode("utf-8", errors="strict"))
                            data_lines.clear()
                    elif line.startswith(b"data:"):
                        data_lines.append(line[5:].removeprefix(b" "))
        if result["finish_reason"] is None:
            raise StreamError("missing_finish_reason", "最終finish_reasonがありません")
        check_metrics(result)
        result.update(stop_reason=result["finish_reason"], truncated=result["finish_reason"] == "length",
                      no_visible_content=result["ttft_seconds"] is None)
    except BaseException as exc:
        result["stop_reason"] = ("interrupted" if isinstance(exc, KeyboardInterrupt) else
                                 "timeout" if isinstance(exc, TimeoutError) else
                                 exc.reason if isinstance(exc, StreamError) else "transport_error")
        raise
    finally:
        result["elapsed_seconds"] = time.perf_counter() - started
        result["received_bytes"] = total_bytes


def distribution(values):
    values = sorted(value for value in values if value is not None)
    return {"count": len(values), "median": statistics.median(values) if values else None,
            "p95": values[math.ceil(len(values) * 0.95) - 1] if values else None}


def summarize(results):
    groups = {}
    for result in results:
        if result["warmup"] or result["status"] != "completed":
            continue
        key = result["case_id"], result["variant"], result["cache_prompt"]
        groups.setdefault(key, []).append(result)
    summaries = []
    for (case_id, variant, cache_prompt), samples in groups.items():
        metrics = {key: distribution([sample[key] for sample in samples]) for key in (
            "ttft_seconds", "elapsed_seconds", "generation_tokens_per_second", "generated_tokens", "cache_n")}
        metrics.update({key: distribution([sample["timings"][key] for sample in samples])
                        for key in ("prompt_n", "prompt_ms", "predicted_ms")})
        summaries.append({"case_id": case_id, "variant": variant, "cache_prompt": cache_prompt,
                          "measured_samples": len(samples), "complete": len(samples) == REPETITIONS,
                          "generated_token_counts": [sample["generated_tokens"] for sample in samples],
                          "finish_reasons": [sample["finish_reason"] for sample in samples], "metrics": metrics})
    comparisons = []
    for case_id, cache_prompt in dict.fromkeys((key[0], key[2]) for key in groups):
        base = groups.get((case_id, "base", cache_prompt), [])
        adapter = groups.get((case_id, "adapter", cache_prompt), [])
        counts = {name: [sample["generated_tokens"] for sample in samples]
                  for name, samples in (("base", base), ("adapter", adapter))}
        comparisons.append({"case_id": case_id, "cache_prompt": cache_prompt, "generated_token_counts": counts,
                            "same_generated_counts": len(base) == len(adapter) == REPETITIONS and counts["base"] == counts["adapter"],
                            "quality_judgment": None, "speed_improvement_judgment": None,
                            "review_requirement": "回答の意味と生成量を確認してから速度を比較する。短い誤答は改善と判定しない。"})
    return {"percentile_method": "nearest_rank", "warmup_excluded": True,
            "groups": summaries, "comparisons": comparisons}


def plan(cases):
    for variant in ("base", "adapter"):
        for case in cases:
            for cache_prompt in (False, True):
                for repetition in range(REPETITIONS + 1):
                    yield {"run_id": json.dumps([variant, case["id"], cache_prompt, repetition], ensure_ascii=False),
                           "variant": variant, "case_id": case["id"], "cache_prompt": cache_prompt,
                           "repetition": repetition, "warmup": repetition == 0, "seed": 1}


def run(args):
    profile = getattr(args, "generation_profile", "current")
    if args.adapter is None:
        raise ValueError("基底との比較に--adapterが必要です")
    source, cases = selected_cases(args.input, args.case_ids)
    lookup = {case["id"]: case for case in cases}
    runs = list(plan(cases))
    planned = [item["run_id"] for item in runs]
    output = args.output.resolve()
    journal_path = output.with_name(output.stem + ".results.jsonl")
    logs = {variant: output.with_name(output.stem + f".{variant}.server.log") for variant in ("base", "adapter")}
    if any(path.exists() for path in (output, journal_path, *logs.values(), output.with_name(output.name + ".pending"))):
        raise FileExistsError(f"結果またはログが既にあります。新しい出力名を指定してください: {output}")
    document = {"schema_version": 1, "mode": "benchmark", "status": "running", "started_at": runner.utc_now(),
                "files": {name: runner.file_record(getattr(args, name)) for name in ("server", "model", "adapter", "template", "input")},
                "input_metadata": {key: value for key, value in source.items() if key != "cases"},
                "parameters": {"case_ids": args.case_ids, "split": "dev", "gpu_layers": args.gpu_layers,
                               "device": args.device, "context": args.context, "max_tokens": args.max_tokens,
                               **runner.generation_parameters(args), "seed": 1, "repetitions": REPETITIONS, "warmup_per_group": 1,
                               "stream": True, "request_timeout": args.request_timeout},
                "expected_results": len(runs), "results": [], "servers": {},
                "journal": {"path": str(journal_path), "verified_records": 0}, "human_judgment": None}
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("x", encoding="utf-8") as stream:
        json.dump(document, stream, ensure_ascii=False, indent=2, allow_nan=False)
    try:
        with journal_path.open("x", encoding="utf-8", newline="\n") as journal:
            try:
                next_snapshot = time.monotonic() + args.snapshot_seconds
                for variant in logs:
                    settings = copy.copy(args)
                    settings.adapter = args.adapter if variant == "adapter" else None
                    with runner.local_server(settings, logs[variant]) as (client, server):
                        document["servers"][variant] = server
                        for item in (item for item in runs if item["variant"] == variant):
                            case = lookup[item["case_id"]]
                            result = {**item, "status": "running", "expected": case.get("expected"),
                                      "target": case.get("target"), "human_judgment": None}
                            document["results"].append(result)
                            try:
                                measure_stream(client, stream_body(case, args.max_tokens, item["cache_prompt"], profile), args.request_timeout, result)
                                result["status"] = "completed"
                            except BaseException as exc:
                                result.update(status="interrupted" if isinstance(exc, KeyboardInterrupt) else "failed",
                                              error=f"{type(exc).__name__}: {exc}")
                                raise
                            finally:
                                runner.append_result(journal, result)
                            if time.monotonic() >= next_snapshot:
                                os.fsync(journal.fileno())
                                runner.save_result(output, document)
                                next_snapshot = time.monotonic() + args.snapshot_seconds
                            print(f"[{len(document['results'])}/{len(runs)}] {variant} {case['id']} "
                                  f"cache={item['cache_prompt']} {'準備' if item['warmup'] else '測定'}{item['repetition']} "
                                  f"{result['elapsed_seconds']:.3f}秒", flush=True)
                    evidence = backend_evidence(logs[variant], args.gpu_layers)
                    document["servers"][variant]["backend_evidence"] = evidence
                    if not evidence["passed"]:
                        raise ValueError(f"実行バックエンドを確認できません: {evidence['reason']}")
            finally:
                journal.flush()
                os.fsync(journal.fileno())
        recorded = runner.read_journal(journal_path, planned, require_complete=True, key_function=lambda item: item["run_id"])
        if recorded != document["results"]:
            raise ValueError("結果ジャーナルと測定結果が一致しません")
        document["journal"]["verified_records"] = len(recorded)
        document["status"] = "completed"
        return document
    except BaseException as exc:
        document.update(status="interrupted" if isinstance(exc, KeyboardInterrupt) else "failed", error=f"{type(exc).__name__}: {exc}")
        if journal_path.exists():
            try:
                recorded = runner.read_journal(journal_path, planned, require_complete=False, key_function=lambda item: item["run_id"])
                document["results"] = recorded
                document["journal"]["verified_records"] = len(recorded)
            except Exception as journal_error:
                document["journal"]["error"] = f"{type(journal_error).__name__}: {journal_error}"
        raise
    finally:
        document["summary"] = summarize(document["results"])
        document["finished_at"] = runner.utc_now()
        runner.save_result(output, document)


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    runner.add_server_arguments(result, adapter_required=True)
    result.add_argument("--case-ids", type=case_ids, required=True, help="測定するdevのcase IDをカンマ区切りで1〜8件")
    return result


def main(argv=None):
    args = parser().parse_args(argv)
    try:
        run(args)
    except KeyboardInterrupt:
        print("速度測定を中断しました。途中結果を保存し専用サーバを終了しました。", file=sys.stderr)
        return 130
    except Exception as exc:
        print(f"速度測定は未完了です: {exc}", file=sys.stderr)
        return 1
    print(f"速度測定を保存しました。回答の意味判定と品質をそろえた速度比較は未実施です: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
