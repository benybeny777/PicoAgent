"""専用のローカルllama-serverで本番入力の評価とトークン検査を行う。"""

from __future__ import annotations

import argparse
import copy
import hashlib
import http.client
import json
import math
import os
from pathlib import Path
import secrets
import socket
import subprocess
import sys
import threading
import time
from contextlib import contextmanager
from datetime import datetime, timezone
from urllib import error

# 原子的配置のWindows共有違反対策は、配布スクリプトの共通実装を使う。
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from scripts.voice_training_common import atomic_move


def utc_now():
    return datetime.now(timezone.utc).isoformat()


def file_record(path):
    path = Path(path).resolve(strict=True)
    if not path.is_file():
        raise ValueError(f"通常ファイルを指定してください: {path}")
    with path.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    return {"path": str(path), "sha256": digest}


def save_result(path, document):
    """新規作成済みの自分の結果だけを、途中JSONを見せず更新する。"""
    pending = path.with_name(path.name + ".pending")
    try:
        with pending.open("x", encoding="utf-8", newline="\n") as stream:
            json.dump(document, stream, ensure_ascii=False, indent=2, allow_nan=False)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        atomic_move(pending, path)
    finally:
        # 作成を拒否された既存ファイルは回収しない。
        if "stream" in locals():
            pending.unlink(missing_ok=True)


def result_key(result):
    return result.get("case_id"), result.get("seed")


def read_journal(path, planned, *, require_complete, key_function=result_key):
    """欠損・重複・途中行を許さず、実行順に保存した結果だけを復元する。"""
    results = []
    seen = set()
    with path.open(encoding="utf-8") as stream:
        for line_number, line in enumerate(stream, 1):
            if not line.endswith("\n"):
                raise ValueError(f"結果ジャーナルの{line_number}行目が未完です")
            result = json.loads(line)
            if not isinstance(result, dict) or result.get("status") not in {"completed", "failed", "interrupted"}:
                raise ValueError(f"結果ジャーナルの{line_number}行目が不正です")
            key = key_function(result)
            if key in seen or len(results) >= len(planned) or key != planned[len(results)]:
                raise ValueError(f"結果ジャーナルに欠損・重複・順序違いがあります: {line_number}行目")
            if require_complete and result["status"] != "completed":
                raise ValueError(f"結果ジャーナルに完了していないケースがあります: {key}")
            seen.add(key)
            results.append(result)
    if require_complete and len(results) != len(planned):
        raise ValueError(f"結果ジャーナルの件数が不足しています: {len(results)}/{len(planned)}")
    return results


def append_result(journal, result):
    # 全結果を毎回書き直さず、終了した1ケースだけを追記してOSへ渡す。
    journal.write(json.dumps(result, ensure_ascii=False, separators=(",", ":"), allow_nan=False) + "\n")
    journal.flush()


class LocalClient:
    def __init__(self, port, api_key, process):
        self.base = f"http://127.0.0.1:{port}"
        self.port = port
        self.api_key = api_key
        self.process = process

    @contextmanager
    def open(self, endpoint, body=None, timeout=120):
        """JSONとSSEで同じ専用接続を使い、応答が少しずつ届いても総時間を制限する。"""
        if endpoint not in {"/props", "/v1/chat/completions", "/apply-template", "/tokenize"}:
            raise ValueError("許可していない評価APIです")
        if self.process.poll() is not None:
            raise RuntimeError(f"評価サーバが終了しました: {self.process.returncode}")
        data = None if body is None else json.dumps(body, ensure_ascii=False, allow_nan=False).encode("utf-8")
        headers = {
            "Content-Type": "application/json",
            "Authorization": f"Bearer {self.api_key}",
        }
        started = time.monotonic()
        # http.clientは環境変数のHTTPプロキシ・認証・リダイレクトを使わない。
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=timeout)
        expired = threading.Event()
        timer = None
        try:
            connection.connect()
            own_socket = connection.sock

            def abort():
                expired.set()
                try:
                    own_socket.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass

            remaining = timeout - (time.monotonic() - started)
            if remaining <= 0:
                raise TimeoutError("評価APIの総時間が上限を超えました")
            timer = threading.Timer(remaining, abort)
            timer.daemon = True
            timer.start()
            connection.request("GET" if body is None else "POST", endpoint, body=data, headers=headers)
            with connection.getresponse() as response:
                if 300 <= response.status < 400:
                    raise ValueError("ローカル評価サーバからのリダイレクトは許可しません")
                if response.status >= 400:
                    raise error.HTTPError(self.base + endpoint, response.status, response.reason, response.headers, None)
                yield response
                if expired.is_set():
                    raise TimeoutError("評価APIの総時間が上限を超えました")
        except (OSError, http.client.HTTPException) as exc:
            if expired.is_set():
                raise TimeoutError("評価APIの総時間が上限を超えました") from exc
            raise
        finally:
            if timer is not None:
                timer.cancel()
                timer.join()
            connection.close()

    def json(self, endpoint, body=None, timeout=120):
        with self.open(endpoint, body, timeout) as response:
            raw = response.read(32_000_001)
        if len(raw) > 32_000_000:
            raise ValueError("評価APIの応答が上限32MBを超えています")
        result = json.loads(raw)
        if not isinstance(result, dict) or "error" in result:
            raise ValueError(f"評価APIの応答が不正です: {result}")
        return result


def server_arguments(args, port, api_key):
    command = [str(args.server.resolve()), "--model", str(args.model.resolve()),
               "--host", "127.0.0.1", "--port", str(port),
               "--ctx-size", str(args.context), "--gpu-layers", str(args.gpu_layers),
               "--parallel", "1", "--cache-ram", "0", "--fit", "off",
               "--jinja", "--chat-template-file", str(args.template.resolve()),
               "--api-key", api_key, "--no-webui", "--no-agent", "--perf"]
    if args.adapter is not None:
        command += ["--lora", str(args.adapter.resolve())]
    if getattr(args, "verbosity", None) is not None:
        command += ["--verbosity", str(args.verbosity)]
    if args.gpu_layers == 0:
        command += ["--device", "none", "--no-kv-offload", "--no-op-offload"]
    elif args.device:
        command += ["--device", args.device]
    return command


def stop_process(process):
    """自分のPopenだけを対象に、停止後のwaitまで必ず行う。"""
    if process.poll() is None:
        try:
            process.terminate()
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
    else:
        process.wait()


@contextmanager
def local_server(args, log_path):
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as port_socket:
        port_socket.bind(("127.0.0.1", 0))
        port = port_socket.getsockname()[1]
    # ポート解放から起動までに他プロセスが確保しても、固有キーが一致しない。
    api_key = secrets.token_hex(32)
    command = server_arguments(args, port, api_key)
    environment = {key: os.environ[key] for key in (
        "SystemRoot", "WINDIR", "PATH", "TEMP", "TMP", "HOME", "USERPROFILE",
    ) if key in os.environ}
    with log_path.open("xb") as log:
        process = subprocess.Popen(
            command, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
            cwd=args.server.resolve().parent, env=environment,
            creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
        )
        try:
            client = LocalClient(port, api_key, process)
            deadline = time.monotonic() + args.startup_timeout
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError(f"評価サーバの起動が時間切れです: {log_path}")
                try:
                    # /healthは認証対象外なので、固有キーを検証する/propsを使う。
                    props = client.json("/props", timeout=min(1, remaining))
                    break
                except error.HTTPError as exc:
                    if exc.code != 503:
                        raise
                except (error.URLError, TimeoutError, ConnectionError):
                    pass
                time.sleep(min(0.1, max(0, deadline - time.monotonic())))
            yield client, {
                "pid": process.pid, "port": port, "props": props,
                "arguments": ["<実行ごとの認証キー>" if item == api_key else item for item in command],
                "log": str(log_path),
            }
        finally:
            stop_process(process)


def load_cases(path, split=None, limit=None):
    if path.stat().st_size > 32_000_000:
        raise ValueError("入力教材が上限32MBを超えています")
    document = json.loads(path.read_text(encoding="utf-8-sig"))
    cases = document.get("cases") if isinstance(document, dict) else None
    if not isinstance(cases, list) or not cases:
        raise ValueError("入力には空でないcases配列が必要です")
    ids = set()
    for case in cases:
        if not isinstance(case, dict):
            raise ValueError("各ケースはJSONオブジェクトにしてください")
        case_id = case.get("id")
        if not isinstance(case_id, str) or not case_id.strip() or case_id in ids:
            raise ValueError("ケースのidが空か重複しています")
        ids.add(case_id)
        body = case.get("body")
        if not isinstance(body, dict) or not isinstance(body.get("messages"), list) or not body["messages"]:
            raise ValueError(f"本番変換済みのbody.messagesがありません: {case_id}")
        if "tools" in body and not isinstance(body["tools"], list):
            raise ValueError(f"body.toolsは配列にしてください: {case_id}")
        if case.get("expected") is None and case.get("target") is None:
            raise ValueError(f"意味判定用のexpectedまたはtargetがありません: {case_id}")
    selected = [case for case in cases if split is None or case.get("split") == split]
    if limit is not None:
        selected = selected[:limit]
    if not selected:
        raise ValueError("指定条件に合うケースがありません")
    return document, selected


def generation_body(case, seed, max_tokens):
    body = copy.deepcopy(case["body"])
    body.update(temperature=0.6, seed=seed, stream=False, cache_prompt=False, max_tokens=max_tokens)
    return body


def evaluate_case(client, case, seed, args):
    body = generation_body(case, seed, args.max_tokens)
    started = time.perf_counter()
    response = client.json("/v1/chat/completions", body, timeout=args.request_timeout)
    elapsed = time.perf_counter() - started
    choices = response.get("choices")
    if not isinstance(choices, list) or not choices or not isinstance(choices[0], dict) or not isinstance(choices[0].get("message"), dict):
        raise ValueError("chat completionsに回答messageがありません")
    message = choices[0]["message"]
    return {
        "request": body, "raw_response": response, "content": message.get("content"),
        "tool_calls": message.get("tool_calls", []), "timings": response.get("timings"),
        "usage": response.get("usage"), "elapsed_seconds": elapsed,
        "finish_reason": choices[0].get("finish_reason"), "human_judgment": None,
    }


def inspect_case(client, case, args):
    # 本番body全体を渡し、toolsやchat_template_kwargsの境界も共有する。
    body = copy.deepcopy(case["body"])
    started = time.perf_counter()
    applied = client.json("/apply-template", body, timeout=args.request_timeout)
    prompt = applied.get("prompt")
    if not isinstance(prompt, str):
        raise ValueError("テキストモデルのprompt文字列がありません")
    # llama.cppのchat completionsが内部で使う特殊トークン設定とそろえる。
    tokenize_body = {"content": prompt, "add_special": True, "parse_special": True}
    tokenized = client.json("/tokenize", tokenize_body, timeout=args.request_timeout)
    tokens = tokenized.get("tokens")
    if not isinstance(tokens, list) or not tokens or not all(type(token) is int for token in tokens):
        raise ValueError("tokenizeに空でない整数token ID配列がありません")
    return {
        "request": body, "prompt": prompt, "tokens": tokens,
        "tokenize_request": tokenize_body, "elapsed_seconds": time.perf_counter() - started,
        "parity_judgment": None,
    }


def run(args):
    output = args.output.resolve()
    log_path = output.with_name(output.stem + ".server.log")
    journal_path = output.with_name(output.stem + ".results.jsonl")
    if any(path.exists() for path in (output, log_path, journal_path, output.with_name(output.name + ".pending"))):
        raise FileExistsError(f"結果またはログが既にあります。別の出力名を指定してください: {output}")
    source, cases = load_cases(args.input, args.split, args.limit)
    seeds = args.seeds if args.command == "evaluate" else [None]
    planned = [(case["id"], seed) for case in cases for seed in seeds]
    files = {name: file_record(getattr(args, name)) for name in ("server", "model", "template", "input")}
    if args.adapter is not None:
        files["adapter"] = file_record(args.adapter)
    document = {
        "schema_version": 1, "mode": args.command, "status": "running",
        "started_at": utc_now(), "files": files,
        "input_metadata": {key: value for key, value in source.items() if key != "cases"},
        "parameters": {"seeds": args.seeds if args.command == "evaluate" else [],
                       "temperature": 0.6, "cache_prompt": False, "stream": False,
                       "max_tokens": args.max_tokens, "gpu_layers": args.gpu_layers,
                       "context": args.context, "split": args.split, "limit": args.limit},
        "expected_results": len(planned),
        "journal": {"path": str(journal_path), "verified_records": 0},
        "results": [], "human_judgment": None,
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("x", encoding="utf-8") as stream:
        json.dump(document, stream, ensure_ascii=False, indent=2)
    try:
        with journal_path.open("x", encoding="utf-8", newline="\n") as journal:
            try:
                with local_server(args, log_path) as (client, server):
                    document["server"] = server
                    save_result(output, document)
                    next_snapshot = time.monotonic() + args.snapshot_seconds
                    for case in cases:
                        for seed in seeds:
                            result = {"case_id": case["id"], "seed": seed, "status": "running",
                                      "expected": case.get("expected"), "target": case.get("target"),
                                      "request": generation_body(case, seed, args.max_tokens)
                                      if args.command == "evaluate" else copy.deepcopy(case["body"]),
                                      "case_metadata": {key: value for key, value in case.items() if key != "body"}}
                            document["results"].append(result)
                            try:
                                if args.command == "evaluate":
                                    result.update(evaluate_case(client, case, seed, args))
                                else:
                                    result.update(inspect_case(client, case, args))
                                result["status"] = "completed"
                            except BaseException as exc:
                                result.update(status="interrupted" if isinstance(exc, KeyboardInterrupt) else "failed",
                                              error=f"{type(exc).__name__}: {exc}")
                                raise
                            finally:
                                append_result(journal, result)
                            if time.monotonic() >= next_snapshot:
                                os.fsync(journal.fileno())
                                save_result(output, document)
                                next_snapshot = time.monotonic() + args.snapshot_seconds
                            print(f"[{len(document['results'])}/{document['expected_results']}] "
                                  f"{case['id']} seed={seed} 完了 {result['elapsed_seconds']:.2f}秒", flush=True)
            finally:
                journal.flush()
                os.fsync(journal.fileno())
        recorded = read_journal(journal_path, planned, require_complete=True)
        if recorded != document["results"]:
            raise ValueError("結果ジャーナルと実行結果が一致しません")
        document["results"] = recorded
        document["journal"]["verified_records"] = len(recorded)
        document["status"] = "completed"
        return document
    except BaseException as exc:
        document.update(status="interrupted" if isinstance(exc, KeyboardInterrupt) else "failed",
                        error=f"{type(exc).__name__}: {exc}")
        if journal_path.exists():
            try:
                recorded = read_journal(journal_path, planned, require_complete=False)
                document["results"] = recorded
                document["journal"]["verified_records"] = len(recorded)
            except Exception as journal_error:
                document["journal"]["error"] = f"{type(journal_error).__name__}: {journal_error}"
        raise
    finally:
        document["finished_at"] = utc_now()
        save_result(output, document)


def positive_number(value):
    number = float(value)
    if not math.isfinite(number) or number <= 0:
        raise argparse.ArgumentTypeError("正の有限数を指定してください")
    return number


def positive_integer(value):
    number = int(value)
    if number <= 0:
        raise argparse.ArgumentTypeError("正の整数を指定してください")
    return number


def seeds_value(value):
    try:
        values = [int(item) for item in value.split(",")]
    except ValueError as exc:
        raise argparse.ArgumentTypeError("seedはカンマ区切りの整数にしてください") from exc
    if not values or len(set(values)) != len(values) or any(item < 0 or item >= 2**32 - 1 for item in values):
        raise argparse.ArgumentTypeError("seedは重複のない0〜4294967294にしてください")
    return values


def add_server_arguments(command, *, adapter_required=False):
    for field, help_text in (("server", "起動するllama-server実行ファイル"), ("model", "ベースGGUF"),
                             ("template", "本番Jinjaテンプレート"), ("input", "Rustがbodyを追加したcases JSON"),
                             ("output", "新規作成する結果JSON")):
        command.add_argument("--" + field, type=Path, required=True, help=help_text)
    command.add_argument("--adapter", type=Path, required=adapter_required,
                         help="比較するLoRA GGUF" + ("。必須" if adapter_required else "。省略時はベースのみ"))
    command.add_argument("--gpu-layers", type=int, choices=(0, 99), required=True, help="0はCPU、99はGPU")
    command.add_argument("--device", help="明示する製品GPUデバイス名")
    command.add_argument("--context", type=positive_integer, default=4096, help="文脈長。既定4096")
    command.add_argument("--max-tokens", type=positive_integer, default=256, help="最大生成token数。既定256")
    command.add_argument("--startup-timeout", type=positive_number, default=180, help="起動の上限秒数")
    command.add_argument("--request-timeout", type=positive_number, default=120, help="API応答の総時間上限秒数")
    command.add_argument("--snapshot-seconds", type=positive_number, default=30, help="全JSONの保存間隔。結果は各件JSONLへ追記")


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)
    for name, description in (("evaluate", "本番入力で回答を生成し、未採点の結果を保存する"),
                              ("inspect", "本番テンプレートの入力文字列とtoken IDを保存する")):
        command = commands.add_parser(name, help=description, description=description)
        add_server_arguments(command)
        command.add_argument("--seeds", type=seeds_value, default=[1, 2, 3], help="推論seed。既定1,2,3")
        command.add_argument("--limit", type=positive_integer, help="対象ケースの先頭件数")
        command.add_argument("--split", help="対象splitの完全一致名")
    return result


def main(argv=None):
    values = list(sys.argv[1:] if argv is None else argv)
    if values and values[0].startswith("--") and values[0] != "--help":
        values.insert(0, "evaluate")
    args = parser().parse_args(values)
    try:
        run(args)
    except KeyboardInterrupt:
        print("評価を中断し、取得済みの結果を保存してサーバを終了しました。", file=sys.stderr)
        return 130
    except Exception as exc:
        print(f"評価は未完了です: {exc}", file=sys.stderr)
        return 1
    print(f"結果を保存しました。回答の意味判定は未実施です: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
