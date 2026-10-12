"""少量学習LoRAの非ゼロ適用をCPU/GPUで確認する隔離した検査入口。

runnerと同じファイル引数を受け、--adapterを必須にする。trainの先頭1件
（--limitで最大8件）だけを、CPU基底・CPUアダプタ・GPU基底・GPUアダプタの
順に実行する。temperature=0、seed=1、max_tokens=1、top_logprobs=5は固定。
生成はASCIIのaに制約するが、比較するのは制約前のraw確率だけ。
この検査は品質評価でも製品の追加推論でもない。
"""

from __future__ import annotations

import copy
import json
import math
from pathlib import Path
import re
import sys

import runner


MIN_LOGPROB_DELTA = 1e-5
MAX_CASES = 8
MAX_TOKENS = 1
PROBE_TEXT = "a"
PROBE_GRAMMAR = 'root ::= "a"'
RUNS = (("cpu-base", 0, False), ("cpu-adapter", 0, True),
        ("gpu-base", 99, False), ("gpu-adapter", 99, True))


class ProbeClient:
    """回答回収はrunnerを共用し、検査専用の生成条件だけAPI直前に固定する。"""

    def __init__(self, client):
        self.client = client

    def json(self, endpoint, body=None, timeout=120):
        if endpoint != "/v1/chat/completions" or not isinstance(body, dict):
            raise ValueError("LoRA適用検査ではchat completionsだけを使います")
        # evaluate_caseが返すrequestも実送信条件を記録する。同関数はbodyを複製済み。
        set_probe_parameters(body)
        return self.client.json(endpoint, body, timeout=timeout)


def set_probe_parameters(body):
    body.update(temperature=0, seed=1, max_tokens=MAX_TOKENS, stream=False,
                cache_prompt=False, logprobs=True, top_logprobs=5,
                post_sampling_probs=False, verbose=True, return_tokens=True, grammar=PROBE_GRAMMAR)


def probe_case(client, case, args):
    """制約前後の入力を検証してから、生成1位置目のraw確率を取得する。"""
    body = case["body"]
    if body.get("tools") or any(key in body for key in ("grammar", "json_schema", "response_format")):
        raise ValueError("固定ASCII制約の適用検査にはツール・出力制約なしのtrain入力が必要です")
    controlled = copy.deepcopy(case)
    controlled["body"] = runner.generation_body(case, 1, MAX_TOKENS)
    set_probe_parameters(controlled["body"])
    evidence = {
        "original": runner.inspect_case(client, case, args),
        "controlled": runner.inspect_case(client, controlled, args),
    }
    verified_prompt({"prompt_evidence": evidence})
    result = runner.evaluate_case(ProbeClient(client), case, 1, args)
    result["prompt_evidence"] = evidence
    return result


def verified_prompt(result):
    evidence = result.get("prompt_evidence", {})
    values = []
    for key in ("original", "controlled"):
        record = evidence.get(key, {})
        prompt, tokens = record.get("prompt"), record.get("tokens")
        if not isinstance(prompt, str) or not prompt or not isinstance(tokens, list) or not tokens or any(type(token) is not int or token < 0 for token in tokens):
            raise ValueError("制約前後の本番promptとtoken列の検査記録がありません")
        values.append((prompt, tokens))
    if values[0] != values[1]:
        raise ValueError("ASCII制約によって本番promptまたはtoken列が変わりました")
    return values[0]


def first_position(result):
    """UTF-8除外がない固定ASCIIの1tokenを確認し、常に生成位置0だけを使う。"""
    if result.get("content") != PROBE_TEXT:
        raise ValueError("固定ASCIIの1文字が生成されていません")
    choices = result.get("raw_response", {}).get("choices")
    if not isinstance(choices, list) or len(choices) != 1:
        raise ValueError("単一の回答choiceがありません")
    logprobs = choices[0].get("logprobs")
    content = logprobs.get("content") if isinstance(logprobs, dict) else None
    if not isinstance(content, list) or len(content) != 1 or not isinstance(content[0], dict):
        raise ValueError("生成1token分のlogprobがありません")
    first = content[0]
    tokens = result.get("raw_response", {}).get("__verbose", {}).get("tokens")
    selected_id = first.get("id")
    if type(selected_id) is not int or selected_id < 0 or tokens != [selected_id] or any(type(token) is not int for token in tokens):
        raise ValueError("生成1位置目の実token IDを確認できません")
    raw_bytes = first.get("bytes")
    if raw_bytes != list(PROBE_TEXT.encode("ascii")) or any(type(value) is not int for value in raw_bytes):
        raise ValueError("生成1位置目のbytesが固定ASCIIと一致しません")
    return first, {"index": 0, "prefix_token_ids": [],
                   "selected_token_id": selected_id, "emitted_bytes": raw_bytes}


def candidates(first):
    """固定エンジンのtoken IDで候補を照合する。文字化けした表示文字で同一視しない。"""
    top = first.get("top_logprobs")
    if not isinstance(top, list) or not top or len(top) > 5:
        raise ValueError("候補tokenのtop_logprobsがありません")
    found = {}
    for entry in [first, *top]:
        if not isinstance(entry, dict):
            raise ValueError("logprobの候補形式が不正です")
        value = entry.get("logprob")
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value > 0:
            raise ValueError("候補tokenのlogprobが有限の非正数ではありません")
        key = entry.get("id")
        if type(key) is not int or key < 0:
            raise ValueError("候補tokenのIDがありません")
        raw_bytes = entry.get("bytes")
        if not isinstance(raw_bytes, list) or not raw_bytes or any(type(v) is not int or not 0 <= v <= 255 for v in raw_bytes):
            raise ValueError("候補tokenのbytesが不正です")
        if key in found and not math.isclose(found[key]["logprob"], value, rel_tol=0, abs_tol=1e-12):
            raise ValueError("同じ候補tokenに矛盾するlogprobがあります")
        found[key] = {"id": key, "token": entry.get("token"), "bytes": raw_bytes, "logprob": value}
    return found


def compare(base, adapted):
    """CPUとGPUの差でなく、同じバックエンド内で同じ候補tokenを比較する。"""
    if base.get("request") != adapted.get("request"):
        raise ValueError("基底とアダプタの生成要求が一致しません")
    request = base.get("request", {})
    if request.get("post_sampling_probs") is not False or request.get("grammar") != PROBE_GRAMMAR or request.get("max_tokens") != 1:
        raise ValueError("固定ASCII 1tokenの制約前raw確率という比較条件がありません")
    if verified_prompt(base) != verified_prompt(adapted):
        raise ValueError("基底とアダプタの本番promptまたはtoken列が一致しません")
    first, base_position = first_position(base)
    second, adapter_position = first_position(adapted)
    if base_position["index"] != adapter_position["index"] or base_position["prefix_token_ids"] != adapter_position["prefix_token_ids"]:
        raise ValueError("比較する生成位置または先行token列が一致しません")
    left, right = candidates(first), candidates(second)
    common = left.keys() & right.keys()
    if not common:
        raise ValueError("同じ生成位置に共通の候補tokenがありません")
    differences = []
    for key in sorted(common):
        delta = right[key]["logprob"] - left[key]["logprob"]
        differences.append({"id": key, "token": left[key]["token"], "bytes": left[key]["bytes"],
                            "base_logprob": left[key]["logprob"],
                            "adapter_logprob": right[key]["logprob"], "delta": delta})
    maximum = max(abs(item["delta"]) for item in differences)
    return {"passed": maximum > MIN_LOGPROB_DELTA,
            "outcome": "effect_detected" if maximum > MIN_LOGPROB_DELTA else "no_effect_detected",
            "max_abs_logprob_delta": maximum,
            "threshold": MIN_LOGPROB_DELTA, "common_candidates": differences,
            "base_position": base_position, "adapter_position": adapter_position,
            "reason": "同じ候補tokenに閾値を超える非ゼロ差があります" if maximum > MIN_LOGPROB_DELTA else "この入力の候補差は閾値以下です。アダプタ全体の無効を証明するものではありません"}


def backend_evidence(log_path, gpu_layers):
    """GPUへの実offloadをログで確認し、GPU指定だけで成功扱いしない。"""
    text = log_path.read_text(encoding="utf-8", errors="replace")
    matches = re.findall(r"offloaded\s+(\d+)/(\d+)\s+layers\s+to GPU", text)
    counts = [{"offloaded": int(a), "total": int(b)} for a, b in matches]
    if gpu_layers:
        passed = any(item["offloaded"] > 0 and item["offloaded"] == item["total"] for item in counts)
        reason = "全層のGPU offloadを確認しました" if passed else "全層のGPU offloadをログで確認できません"
    else:
        # runnerはCPU時にdevice=noneとKV/演算offload無効を明示する。
        passed = not any(item["offloaded"] > 0 for item in counts)
        reason = "CPU専用起動でGPUへのoffloadがないことを確認しました" if passed else "CPU専用起動なのにGPUへのoffloadがあります"
    return {"passed": passed, "offload_counts": counts, "reason": reason}


def parse_args(argv=None):
    values = list(sys.argv[1:] if argv is None else argv)
    parser = runner.parser()
    parser.prog = "probe.py"
    parser.description = __doc__
    # 共通parserへ既定値を渡す。上書きは下で検証し、検査条件の変更を拒否する。
    args = parser.parse_args(["evaluate", "--gpu-layers", "0", "--split", "train",
                              "--seeds", "1", "--max-tokens", str(MAX_TOKENS), "--limit", "1", *values])
    if args.adapter is None:
        parser.error("LoRA適用検査では--adapterが必須です")
    if args.split != "train" or args.seeds != [1] or args.max_tokens != MAX_TOKENS or args.gpu_layers != 0:
        parser.error("split=train、seed=1、max_tokens=1、GPU切替順は固定です")
    if not 1 <= args.limit <= MAX_CASES:
        parser.error(f"--limitは1〜{MAX_CASES}にしてください")
    return args


def run(args):
    if args.adapter is None or args.split != "train" or not 1 <= args.limit <= MAX_CASES:
        raise ValueError("アダプタと1〜8件のtrain教材を指定してください")
    output = args.output.resolve()
    logs = {label: output.with_name(f"{output.stem}.{label}.server.log") for label, _, _ in RUNS}
    paths = [output, output.with_name(output.name + ".pending"), *logs.values()]
    if any(path.exists() for path in paths):
        raise FileExistsError(f"結果または専用ログが既にあります。別の出力名を指定してください: {output}")
    _, cases = runner.load_cases(args.input, "train", args.limit)
    files = {name: runner.file_record(getattr(args, name)) for name in ("server", "model", "template", "input", "adapter")}
    document = {
        "schema_version": 1, "mode": "adapter_effect_probe", "status": "running",
        "started_at": runner.utc_now(), "files": files,
        "parameters": {"split": "train", "limit": args.limit, "case_ids": [c["id"] for c in cases],
                       "temperature": 0, "seed": 1, "max_tokens": MAX_TOKENS, "logprobs": True,
                       "top_logprobs": 5, "cache_prompt": False, "stream": False,
                       "context": args.context, "min_logprob_delta": MIN_LOGPROB_DELTA,
                       "verbose": True, "return_tokens": True, "post_sampling_probs": False,
                       "server_verbosity": 5, "grammar": PROBE_GRAMMAR, "compared_generation_index": 0},
        "api_evidence": {"llama_cpp_commit": "d81235049384534c167caea52b85a694f6103d14",
                         "reason": "native /completionも未完UTF-8で同じ確率欠落が起きる。固定ASCII 1文字へsamplingを制約し、post_sampling_probs=falseで同一promptの位置0のraw logits由来softmaxを取得する。制約前後と基底/adapterのprompt全文・token列一致を要求する。",
                         "sources": ["tools/server/server-context.cpp:1978-2013", "tools/server/server-context.cpp:2097-2150", "tools/server/server-context.cpp:5415-5424", "tools/server/server-common.cpp:1591-1640", "tools/server/server-task.cpp:359-360", "common/sampling.cpp:157-164"]},
        "boundary": "少量学習アダプタの非ゼロ適用だけを確認する。学習品質・性能・製品採用の判定ではない。",
        "runs": [], "comparisons": [], "passed": False,
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(document, stream, ensure_ascii=False, indent=2, allow_nan=False)
        stream.write("\n")
    try:
        for label, gpu_layers, has_adapter in RUNS:
            settings = copy.copy(args)
            settings.gpu_layers = gpu_layers
            settings.adapter = args.adapter if has_adapter else None
            settings.max_tokens = MAX_TOKENS
            settings.verbosity = 5
            record = {"label": label, "gpu_layers": gpu_layers, "adapter_enabled": has_adapter,
                      "status": "running", "log": str(logs[label]), "results": []}
            document["runs"].append(record)
            try:
                with runner.local_server(settings, logs[label]) as (client, server):
                    record["server"] = server
                    for case in cases:
                        result = probe_case(client, case, settings)
                        result["case_id"] = case["id"]
                        record["results"].append(result)
                        runner.save_result(output, document)
                record["backend"] = backend_evidence(logs[label], gpu_layers)
                record["status"] = "completed"
            except Exception as exc:
                record.update(status="failed", error=f"{type(exc).__name__}: {exc}")
            finally:
                if logs[label].is_file():
                    record["log_file"] = runner.file_record(logs[label])
                runner.save_result(output, document)
            print(f"LoRA適用検査 {label}: {record['status']}", flush=True)
        by_label = {item["label"]: item for item in document["runs"]}
        for backend in ("cpu", "gpu"):
            base, adapted = by_label[f"{backend}-base"], by_label[f"{backend}-adapter"]
            for case in cases:
                verdict = {"backend": backend, "case_id": case["id"], "passed": None, "outcome": "inconclusive"}
                try:
                    if any(item["status"] != "completed" or not item.get("backend", {}).get("passed") for item in (base, adapted)):
                        raise ValueError("サーバの正常実行またはバックエンドを確認できません")
                    first = next(item for item in base["results"] if item["case_id"] == case["id"])
                    second = next(item for item in adapted["results"] if item["case_id"] == case["id"])
                    verdict.update(compare(first, second))
                except Exception as exc:
                    verdict["reason"] = f"{type(exc).__name__}: {exc}"
                document["comparisons"].append(verdict)
        document["passed"] = bool(document["comparisons"]) and all(item["passed"] for item in document["comparisons"])
        document["status"] = ("inconclusive" if any(item["outcome"] == "inconclusive" for item in document["comparisons"])
                              else "completed" if document["passed"] else "failed")
        return document
    except BaseException as exc:
        document.update(status="interrupted" if isinstance(exc, KeyboardInterrupt) else "failed",
                        error=f"{type(exc).__name__}: {exc}")
        if document["runs"] and document["runs"][-1]["status"] == "running":
            document["runs"][-1]["status"] = document["status"]
        raise
    finally:
        document["finished_at"] = runner.utc_now()
        runner.save_result(output, document)


def main(argv=None):
    args = parse_args(argv)
    try:
        result = run(args)
    except KeyboardInterrupt:
        print("LoRA適用検査を中断し、取得済みの結果を保存しました。", file=sys.stderr)
        return 130
    except Exception as exc:
        print(f"LoRA適用検査に失敗しました: {exc}", file=sys.stderr)
        return 2
    label = "判定不能" if result["status"] == "inconclusive" else "非ゼロ差を確認" if result["passed"] else "閾値を超える差は未検出"
    print(f"LoRA適用検査: {label} / {args.output}")
    return 2 if result["status"] == "inconclusive" else 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
