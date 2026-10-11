"""本番の入力を保持して、正しい回答部分だけをGPUで追加学習する。"""

from __future__ import annotations

import argparse
import copy
import gc
import hashlib
import importlib.metadata
import json
import math
import os
from pathlib import Path
import random
import sys
import time

# Windows共有違反の有限再試行は、既存の原子的配置へ集約する。
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from scripts.voice_training_common import atomic_move


TARGET_MODULES = ("q_proj", "k_proj", "v_proj", "out_proj", "w1", "w2", "w3")


def read_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def write_json(path: Path, value):
    temporary = path.with_suffix(path.suffix + ".incoming")
    try:
        with temporary.open("x", encoding="utf-8", newline="\n") as stream:
            json.dump(value, stream, ensure_ascii=False, indent=2, allow_nan=False)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        atomic_move(temporary, path)
    finally:
        if "stream" in locals():
            temporary.unlink(missing_ok=True)


def save_adapter(model, path):
    model.save_pretrained(path, safe_serialization=True, save_embedding_layers=False)


def save_epoch_checkpoint(model, output, manifest, epoch, step):
    relative = Path(f"checkpoint-epoch-{epoch}") / "adapter"
    manifest["phase"] = f"epoch {epoch} のアダプタ保存"
    write_json(output / "manifest.json", manifest)
    save_adapter(model, output / relative)
    # 保存が返るまで完了epochへ加えない。途中の保存物も成功扱いしない。
    manifest["checkpoints"].append({
        "epoch": epoch, "step": step, "adapter": relative.as_posix(),
        "status": "保存完了・品質未判定",
    })
    manifest.update(completed_epochs=epoch, phase="追加学習")
    write_json(output / "manifest.json", manifest)


def digest(path: Path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def template_messages(messages):
    """wireの引数字列だけを、製品Jinjaが受けるmappingへ戻す。"""
    result = copy.deepcopy(messages)
    for message in result:
        if not isinstance(message.get("content"), str):
            raise ValueError("メッセージ本文は文字列である必要があります")
        for call in message.get("tool_calls") or []:
            function = call["function"]
            arguments = function["arguments"]
            if isinstance(arguments, str):
                arguments = json.loads(arguments)
            if not isinstance(arguments, dict):
                raise ValueError("ツール引数はJSON objectである必要があります")
            function["arguments"] = arguments
    return result


def prepare_case(case, tokenizer, template: str, max_length: int):
    body = case["body"]
    messages = template_messages(body["messages"])
    target = case["target"]
    if target.get("role") != "assistant":
        raise ValueError("学習対象はassistantの正解だけです")
    if not target.get("content", "").strip() and not target.get("tool_calls"):
        raise ValueError("空の回答は学習できません")
    kwargs = {"chat_template": template, "tools": body.get("tools", []), "tokenize": False}
    prompt = tokenizer.apply_chat_template(messages, add_generation_prompt=True, **kwargs)
    full = tokenizer.apply_chat_template(
        messages + template_messages([target]), add_generation_prompt=False, **kwargs
    )
    if not full.startswith(prompt):
        raise ValueError("正解を追加すると本番の入力が変わりました")
    prefix_ids = tokenizer.encode(prompt, add_special_tokens=False)
    input_ids = tokenizer.encode(full, add_special_tokens=False)
    prefix_length = len(prefix_ids)
    if input_ids[:prefix_length] != prefix_ids:
        raise ValueError("回答の境界で入力トークンが変わりました")
    if len(input_ids) > max_length:
        raise ValueError(f"教材{case['id']}が最大長を超えました: {len(input_ids)} > {max_length}")
    supervised = input_ids[prefix_length:]
    if not supervised or tokenizer.eos_token_id not in supervised:
        raise ValueError("正解または回答の終端が失われています")
    labels = [-100] * prefix_length + supervised
    return {
        "id": case["id"], "category": case.get("category"),
        "input_ids": input_ids, "labels": labels, "prefix_length": prefix_length,
        "prompt": prompt, "completion": full[len(prompt):],
    }


def validate_args(args):
    for name, minimum in (
        ("limit", 0), ("rank", 1), ("max_length", 2), ("epochs", 1),
        ("accumulation", 1), ("warmup", 0), ("max_steps", 0),
    ):
        value = getattr(args, name)
        if isinstance(value, bool) or not isinstance(value, int) or value < minimum:
            raise ValueError(f"{name}は{minimum}以上の整数で指定してください")
    if (isinstance(args.learning_rate, bool)
            or not isinstance(args.learning_rate, (int, float))
            or not math.isfinite(args.learning_rate) or args.learning_rate <= 0):
        raise ValueError("learning_rateは有限の正の値で指定してください")
    if args.max_steps and args.epochs != 1:
        raise ValueError("少量確認のmax-stepsはepochs=1と組み合わせてください")


def cuda_trainable_parameters(model):
    parameters = list(model.named_parameters())
    if any(value.device.type != "cuda" for _, value in parameters):
        raise RuntimeError("ベース重みまたはLoRAにCUDA以外のパラメータが残っています")
    trainable = [(name, value) for name, value in parameters if value.requires_grad]
    if not trainable or any("lora_" not in name for name, _ in trainable):
        raise RuntimeError("LoRA以外のパラメータが更新対象、または更新対象が空です")
    return trainable


def check_cuda_gradients(trainable):
    missing = [name for name, value in trainable if value.grad is None]
    if missing:
        raise RuntimeError(f"更新対象に勾配が届いていません: {missing}")
    if any(value.grad.device.type != "cuda" for _, value in trainable):
        raise RuntimeError("更新対象の勾配がCUDA上にありません")


def run(args):
    validate_args(args)
    if args.output.exists():
        raise ValueError("既存の出力先を上書きしません")
    args.output.mkdir(parents=True)
    manifest = {
        "status": "準備中", "phase": "依存と入力の確認", "seed": args.seed,
        "completed_epochs": 0, "checkpoints": [],
        "arguments": {key: str(value) if isinstance(value, Path) else value for key, value in vars(args).items()},
        "kernel": "PyTorch標準CUDA演算を明示選択。Hubカーネル取得なし",
    }
    torch = model = optimizer = scheduler = None
    inputs = labels = loss = grad = None
    trainable = []
    cuda_ready = False
    step = 0
    run_started = time.monotonic()
    started = None
    try:
        write_json(args.output / "manifest.json", manifest)
        manifest["packages"] = {}
        for name in ("torch", "transformers", "peft", "accelerate", "safetensors"):
            try:
                manifest["packages"][name] = importlib.metadata.version(name)
            except importlib.metadata.PackageNotFoundError:
                manifest["packages"][name] = None
        manifest.update(input_sha256=digest(args.input), template_sha256=digest(args.template),
                        base_model_sha256=digest(args.base / "model.safetensors"))
        data = read_json(args.input)
        cases = [case for case in data["cases"] if case.get("split") == "train"]
        if args.limit:
            cases = cases[:args.limit]
        if not cases:
            raise ValueError("学習用のケースがありません")
        from transformers import AutoModelForCausalLM, AutoTokenizer

        manifest["phase"] = "トークナイザと学習範囲の確認"
        write_json(args.output / "manifest.json", manifest)
        tokenizer = AutoTokenizer.from_pretrained(args.base, local_files_only=True)
        template = args.template.read_text(encoding="utf-8")
        prepared = [prepare_case(case, tokenizer, template, args.max_length) for case in cases]
        manifest.update(status="検査済み", cases=len(prepared), tokens=[
            {"id": row["id"], "category": row["category"], "input": row["prefix_length"],
             "target": len(row["input_ids"]) - row["prefix_length"]} for row in prepared
        ])
        if args.prepare_only:
            write_json(args.output / "prepared.json", prepared)
            manifest["phase"] = "学習範囲の確認完了"
            print(json.dumps({"状態": "学習対象の検査完了", "件数": len(prepared)}, ensure_ascii=False), flush=True)
            return

        manifest["phase"] = "CUDA GPUの確認"
        write_json(args.output / "manifest.json", manifest)
        import torch

        if not torch.cuda.is_available() or not torch.cuda.is_bf16_supported():
            raise RuntimeError("BF16対応CUDA GPUが必要です。CPUへ切り替えません")
        cuda_ready = True
        manifest.update(gpu=torch.cuda.get_device_name(), cuda=torch.version.cuda)
        random.seed(args.seed)
        torch.manual_seed(args.seed)
        torch.cuda.manual_seed_all(args.seed)
        torch.cuda.reset_peak_memory_stats()
        manifest["phase"] = "ベースモデルとLoRAの読込"
        write_json(args.output / "manifest.json", manifest)
        from peft import LoraConfig, get_peft_model

        model = AutoModelForCausalLM.from_pretrained(
            args.base, dtype=torch.bfloat16, local_files_only=True,
            attn_implementation="sdpa", use_kernels=False,
        )
        # CUDAへの移動に失敗しても、読み込んだモデルをfinallyで回収する。
        model = model.to("cuda")
        model.config.use_cache = False
        found = {name.rsplit(".", 1)[-1] for name, _ in model.named_modules()}
        if not set(TARGET_MODULES).issubset(found):
            raise RuntimeError(f"対象層が見つかりません: {sorted(set(TARGET_MODULES) - found)}")
        model = get_peft_model(model, LoraConfig(
            r=args.rank, lora_alpha=2 * args.rank, lora_dropout=0.05,
            target_modules=list(TARGET_MODULES), task_type="CAUSAL_LM", bias="none",
        ))
        model.gradient_checkpointing_enable(gradient_checkpointing_kwargs={"use_reentrant": False})
        model.train()
        trainable = cuda_trainable_parameters(model)
        total_steps = math.ceil(len(prepared) / args.accumulation) * args.epochs
        if args.max_steps:
            total_steps = min(total_steps, args.max_steps)
        warmup = min(args.warmup, max(0, total_steps - 1))
        optimizer = torch.optim.AdamW([value for _, value in trainable], lr=args.learning_rate)

        def schedule(step):
            if step < warmup:
                return (step + 1) / max(1, warmup)
            return max(0.0, (total_steps - step) / max(1, total_steps - warmup))

        scheduler = torch.optim.lr_scheduler.LambdaLR(optimizer, schedule)
        manifest.update(status="学習中", phase="追加学習",
                        trainable_parameters=sum(value.numel() for _, value in trainable),
                        total_steps=total_steps, effective_warmup=warmup)
        write_json(args.output / "manifest.json", manifest)
        started = time.monotonic()
        with (args.output / "progress.jsonl").open("w", encoding="utf-8") as log:
            for epoch in range(args.epochs):
                order = list(prepared)
                random.Random(args.seed + epoch).shuffle(order)
                completed_cases = 0
                for start in range(0, len(order), args.accumulation):
                    group = order[start:start + args.accumulation]
                    optimizer.zero_grad(set_to_none=True)
                    loss_sum = 0.0
                    for row in group:
                        inputs = torch.tensor([row["input_ids"]], device="cuda")
                        labels = torch.tensor([row["labels"]], device="cuda")
                        loss = model(input_ids=inputs, attention_mask=torch.ones_like(inputs), labels=labels, use_cache=False).loss
                        if loss.device.type != "cuda" or not loss.requires_grad:
                            raise RuntimeError("学習損失がCUDA上の計算グラフにつながっていません")
                        if not torch.isfinite(loss):
                            raise RuntimeError("損失が有限値ではありません")
                        loss_sum += loss.detach().item()
                        (loss / len(group)).backward()
                        inputs = labels = loss = None
                    check_cuda_gradients(trainable)
                    grad = torch.nn.utils.clip_grad_norm_([value for _, value in trainable], 1.0, error_if_nonfinite=True)
                    optimizer.step()
                    scheduler.step()
                    torch.cuda.synchronize()
                    step += 1
                    completed_cases += len(group)
                    progress = {"step": step, "total": total_steps, "epoch": epoch + 1, "loss": loss_sum / len(group), "grad_norm": grad.item(), "elapsed_seconds": round(time.monotonic() - started, 3), "peak_vram_gb": round(torch.cuda.max_memory_allocated() / 1e9, 3), "peak_reserved_vram_gb": round(torch.cuda.max_memory_reserved() / 1e9, 3)}
                    line = json.dumps(progress, ensure_ascii=False, allow_nan=False)
                    print(line, flush=True)
                    log.write(line + "\n")
                    log.flush()
                    if step >= total_steps:
                        break
                if completed_cases == len(order):
                    save_epoch_checkpoint(model, args.output, manifest, epoch + 1, step)
                if step >= total_steps:
                    break
        nonzero = sum(torch.count_nonzero(value.detach()).item() for name, value in trainable if "lora_B" in name)
        if not nonzero:
            raise RuntimeError("LoRA Bが更新されていません")
        manifest["phase"] = "学習済みアダプタの保存"
        write_json(args.output / "manifest.json", manifest)
        save_adapter(model, args.output / "adapter")
        manifest.update(status="学習完了・品質未判定", phase="学習完了", steps=step,
                        nonzero_lora_b=nonzero)
    except BaseException as error:
        manifest.update(status="学習失敗・中断", steps=step, error=f"{type(error).__name__}: {error}")
        raise
    finally:
        manifest["run_elapsed_seconds"] = time.monotonic() - run_started
        if started is not None:
            manifest["elapsed_seconds"] = time.monotonic() - started
        try:
            # schedulerとパラメータ一覧もモデルを参照するため、先にすべて手放す。
            inputs = labels = loss = grad = None
            scheduler = optimizer = model = None
            trainable.clear()
            gc.collect()
            if cuda_ready:
                try:
                    manifest.update(peak_vram_gb=torch.cuda.max_memory_allocated() / 1e9,
                                    peak_reserved_vram_gb=torch.cuda.max_memory_reserved() / 1e9)
                finally:
                    torch.cuda.empty_cache()
        except Exception as cleanup_error:
            manifest["cleanup_error"] = f"{type(cleanup_error).__name__}: {cleanup_error}"
            if manifest["status"] != "学習失敗・中断":
                manifest["status"] = "学習失敗・中断"
                raise
        finally:
            write_json(args.output / "manifest.json", manifest)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("input", "base", "template", "output"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--max-length", type=int, default=4096)
    parser.add_argument("--rank", type=int, default=16)
    parser.add_argument("--epochs", type=int, default=2)
    parser.add_argument("--accumulation", type=int, default=4)
    parser.add_argument("--learning-rate", type=float, default=2e-4)
    parser.add_argument("--warmup", type=int, default=10)
    parser.add_argument("--max-steps", type=int, default=0)
    parser.add_argument("--seed", type=int, default=20261012)
    run(parser.parse_args())


if __name__ == "__main__":
    main()
