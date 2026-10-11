"""学習側と製品側の入力全文・全token IDを、保存済みの検査結果で照合する。"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import sys

from runner import file_record, load_cases, positive_integer, save_result, utc_now
from training import prepare_case, read_json


def value_digest(value):
    text = value if isinstance(value, str) else json.dumps(value, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def first_difference(left, right):
    for index in range(max(len(left), len(right))):
        a = left[index] if index < len(left) else None
        b = right[index] if index < len(right) else None
        if a != b:
            return {"index": index, "hf": a, "llama": b,
                    "hf_context": left[max(0, index - 12):index + 13],
                    "llama_context": right[max(0, index - 12):index + 13]}
    return None


def effective_server_prompt(hf_prompt, hf_tokens, inspection, tokenizer):
    """先頭BOSをtokenize側が補うことを証明できた場合だけ、比較用本文へ戻す。"""
    raw = inspection["prompt"]
    adjustment = {"applied": False, "reason": "原文をそのまま比較"}
    if hf_prompt == raw:
        return raw, adjustment
    bos = getattr(tokenizer, "bos_token", None)
    bos_id = getattr(tokenizer, "bos_token_id", None)
    if not isinstance(bos, str) or not bos or type(bos_id) is not int:
        adjustment["reason"] = "BOSの文字列またはtoken IDが未定義"
        return raw, adjustment
    adjustment.update(bos_token=bos, bos_token_id=bos_id)
    if not hf_prompt.startswith(bos) or hf_prompt.startswith(bos + bos) or raw.startswith(bos):
        adjustment["reason"] = "先頭BOSが1個だけ省略された形ではない"
        return raw, adjustment
    if inspection.get("tokenize_request") != {"content": raw, "add_special": True, "parse_special": True}:
        adjustment["reason"] = "特殊tokenを追加する要求の証拠がない"
        return raw, adjustment
    if tokenizer.encode(bos, add_special_tokens=False) != [bos_id]:
        adjustment["reason"] = "BOS文字列とtoken IDの対応が一致しない"
        return raw, adjustment
    if hf_tokens != inspection["tokens"]:
        adjustment["reason"] = "HFと製品側の全token IDが一致しない"
        return raw, adjustment
    raw_tokens = tokenizer.encode(raw, add_special_tokens=False)
    if raw_tokens[:1] == [bos_id] or inspection["tokens"] != [bos_id] + raw_tokens:
        adjustment["reason"] = "製品側token列が原文へBOSを1個追加した結果ではない"
        return raw, adjustment
    # llama.cppのcommon/chat.cppはadd_bos時に描画文字列の先頭BOSを除く。
    # 原文は保存したまま、tokenizeの同じBOS追加だけを比較用本文へ反映する。
    adjustment.update(applied=True, reason="原文への単一BOS追加と全token ID一致を確認")
    return bos + raw, adjustment


def inspection_rows(inspection, cases, files):
    if inspection.get("mode") != "inspect" or inspection.get("status") != "completed":
        raise ValueError("完了したrunner inspectの結果が必要です")
    for name in ("input", "template"):
        if inspection.get("files", {}).get(name, {}).get("sha256") != files[name]["sha256"]:
            raise ValueError(f"inspectで使用した{name}のSHA-256が一致しません")
    results = inspection.get("results")
    if not isinstance(results, list) or inspection.get("expected_results") != len(cases) or len(results) != len(cases):
        raise ValueError("inspectの件数が全ケース数と一致しません。split/limitなしで取得してください")
    rows = {}
    for row in results:
        if not isinstance(row, dict) or row.get("status") != "completed":
            raise ValueError("inspectに失敗または未完了のケースがあります")
        case_id = row.get("case_id")
        if not isinstance(case_id, str) or not case_id or case_id in rows:
            raise ValueError("inspectのcase_idが欠落または重複しています")
        rows[case_id] = row
    expected_ids = {case["id"] for case in cases}
    if rows.keys() != expected_ids:
        raise ValueError(f"inspectのIDが一致しません。欠落={sorted(expected_ids - rows.keys())} 余分={sorted(rows.keys() - expected_ids)}")
    for case in cases:
        row = rows[case["id"]]
        if row.get("request") != case["body"]:
            raise ValueError(f"本番bodyとinspectのrequestが一致しません: {case['id']}")
        prompt, tokens = row.get("prompt"), row.get("tokens")
        if not isinstance(prompt, str) or not isinstance(tokens, list) or not tokens or not all(type(item) is int for item in tokens):
            raise ValueError(f"inspectのpromptまたはtoken IDが不正です: {case['id']}")
        if row.get("tokenize_request") != {"content": prompt, "add_special": True, "parse_special": True}:
            raise ValueError(f"製品と同じ特殊トークン条件ではありません: {case['id']}")
    return rows


def compare(cases, inspected, tokenizer, template, max_length):
    results = []
    for case in cases:
        result = {"case_id": case["id"], "split": case.get("split"), "status": "failed"}
        try:
            # targetを含むlabelsの組み立てはローカルだけで行う。サーバへは送信しない。
            prepared = prepare_case(case, tokenizer, template, max_length)
            hf_prompt = prepared["prompt"]
            hf_tokens = prepared["input_ids"][:prepared["prefix_length"]]
            llama_prompt = inspected[case["id"]]["prompt"]
            llama_tokens = inspected[case["id"]]["tokens"]
            effective_prompt, bos_adjustment = effective_server_prompt(
                hf_prompt, hf_tokens, inspected[case["id"]], tokenizer
            )
            raw_prompt_diff = first_difference(hf_prompt, llama_prompt)
            prompt_diff = first_difference(hf_prompt, effective_prompt)
            token_diff = first_difference(hf_tokens, llama_tokens)
            result.update(
                status="matched" if prompt_diff is None and token_diff is None else "mismatched",
                prompt_match=prompt_diff is None, tokens_match=token_diff is None,
                first_prompt_difference=prompt_diff, first_token_difference=token_diff,
                raw_prompt_match=raw_prompt_diff is None, first_raw_prompt_difference=raw_prompt_diff,
                bos_adjustment=bos_adjustment,
                hf={"prompt_sha256": value_digest(hf_prompt), "tokens_sha256": value_digest(hf_tokens),
                    "characters": len(hf_prompt), "tokens": len(hf_tokens)},
                llama={"prompt_sha256": value_digest(llama_prompt), "tokens_sha256": value_digest(llama_tokens),
                       "characters": len(llama_prompt), "tokens": len(llama_tokens)},
                llama_effective={"prompt_sha256": value_digest(effective_prompt),
                                 "tokens_sha256": value_digest(llama_tokens),
                                 "characters": len(effective_prompt), "tokens": len(llama_tokens)},
            )
        except Exception as exc:
            result["error"] = f"{type(exc).__name__}: {exc}"
        results.append(result)
    return results


def load_tokenizer(base):
    from transformers import AutoTokenizer
    return AutoTokenizer.from_pretrained(base, local_files_only=True, trust_remote_code=False)


def run(args):
    output = args.output.resolve()
    if output.exists() or output.with_name(output.name + ".pending").exists():
        raise FileExistsError(f"既存の照合結果を上書きしません: {output}")
    report = {"schema_version": 1, "status": "running", "started_at": utc_now(),
              "files": {}, "results": [], "matched": 0, "mismatched": 0, "failed": 0}
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("x", encoding="utf-8"):
        pass
    try:
        save_result(output, report)
        report["files"] = {name: file_record(getattr(args, name)) for name in ("input", "inspection", "template")}
        _, cases = load_cases(args.input)
        report["cases"] = len(cases)
        inspection = read_json(args.inspection)
        if not isinstance(inspection, dict):
            raise ValueError("inspectの結果はJSON objectである必要があります")
        inspected = inspection_rows(inspection, cases, report["files"])
        base = args.base.resolve(strict=True)
        if not base.is_dir():
            raise ValueError("HFモデルのローカルディレクトリを指定してください")
        # 語彙・特殊トークン・設定の実体を記録し、重みの取得や推論は行わない。
        tokenizer_files = [base / name for name in (
            "config.json", "tokenizer.json", "tokenizer_config.json", "special_tokens_map.json",
            "added_tokens.json", "vocab.json", "merges.txt", "vocab.txt", "tokenizer.model", "spiece.model",
        ) if (base / name).is_file()]
        if not tokenizer_files:
            raise ValueError("ローカルのtokenizer資産がありません")
        report["base"] = {"path": str(base), "tokenizer_files": [file_record(path) for path in tokenizer_files]}
        report["engine_files"] = {name: inspection.get("files", {}).get(name) for name in ("server", "model", "adapter")}
        template = args.template.read_text(encoding="utf-8")
        tokenizer = load_tokenizer(base)
        report["results"] = compare(cases, inspected, tokenizer, template, args.max_length)
        for key in ("matched", "mismatched", "failed"):
            report[key] = sum(row["status"] == key for row in report["results"])
        report["status"] = "matched" if report["matched"] == len(cases) else "mismatched"
        return report
    except BaseException as exc:
        report.update(status="interrupted" if isinstance(exc, KeyboardInterrupt) else "failed",
                      error=f"{type(exc).__name__}: {exc}")
        raise
    finally:
        report["finished_at"] = utc_now()
        save_result(output, report)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for name, help_text in (("input", "Rustがbodyを追加した全cases JSON"),
                            ("inspection", "全caseに対するrunner inspectの結果JSON"),
                            ("base", "HFモデルのローカルディレクトリ"),
                            ("template", "本番Jinjaテンプレート"), ("output", "新規の照合結果JSON")):
        parser.add_argument("--" + name, type=Path, required=True, help=help_text)
    parser.add_argument("--max-length", type=positive_integer, default=4096, help="学習時の最大長")
    args = parser.parse_args(argv)
    try:
        report = run(args)
    except KeyboardInterrupt:
        print("入力照合を中断し、現在の状態を保存しました。", file=sys.stderr)
        return 130
    except Exception as exc:
        print(f"入力照合は失敗しました: {exc}", file=sys.stderr)
        return 1
    print(f"入力照合: 一致{report['matched']}件、不一致{report['mismatched']}件、検査不能{report['failed']}件。{args.output}")
    return 0 if report["status"] == "matched" else 1


if __name__ == "__main__":
    raise SystemExit(main())
