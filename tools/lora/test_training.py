"""誤答履歴を正解として学習しないことと、本番テンプレートの境界を検査する。"""

import copy
import argparse
from contextlib import contextmanager, redirect_stdout
import io
import json
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock
import warnings

from training import (TARGET_MODULES, check_cuda_gradients, cuda_trainable_parameters,
                      digest, lora_config, prepare_case, run, template_messages, validate_args, write_json)
from scripts import voice_training_common


def arguments(**changes):
    values = dict(limit=0, rank=16, max_length=4096, epochs=2, accumulation=4,
                  learning_rate=2e-4, warmup=10, max_steps=0, seed=17,
                  prepare_only=False, initial_adapter=None, initial_manifest=None)
    values.update(changes)
    return argparse.Namespace(**values)


@contextmanager
def isolated_training(case_count=3, **changes):
    """ファイルは実際に保存し、GPU演算だけを差し替えて工程と失敗を検査する。"""
    scratch = Path(__file__).resolve().parents[2] / "temp/lora-training"
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=scratch) as root:
        root = Path(root)
        base = root / "base"
        base.mkdir()
        (base / "model.safetensors").write_bytes(b"test")
        source = root / "input.json"
        source.write_text(json.dumps({"cases": [
            {"id": f"case-{index}", "split": "train"} for index in range(case_count)
        ]}))
        template = root / "template.jinja"
        template.write_text("test")
        args = arguments(input=source, base=base, template=template, output=root / "output",
                         **changes)
        torch = mock.MagicMock()
        torch.cuda.is_available.return_value = True
        torch.cuda.is_bf16_supported.return_value = True
        torch.cuda.get_device_name.return_value = "検査用GPU"
        torch.cuda.max_memory_allocated.return_value = 1000
        torch.cuda.max_memory_reserved.return_value = 2000
        torch.version.cuda = "検査用CUDA"
        torch.isfinite.return_value = True
        torch.count_nonzero.return_value.item.return_value = 1
        torch.nn.utils.clip_grad_norm_.return_value.item.return_value = 0.25
        model = mock.MagicMock()
        model.to.return_value = model
        model.named_modules.return_value = [(f"layer.{name}", object()) for name in TARGET_MODULES]
        parameter = mock.Mock(requires_grad=True)
        parameter.device.type = parameter.grad.device.type = "cuda"
        parameter.numel.return_value = 2
        model.named_parameters.return_value = [("layer.lora_B.weight", parameter)]
        loss = mock.MagicMock(requires_grad=True)
        loss.device.type = "cuda"
        loss.detach.return_value.item.return_value = 1.0
        loss.__truediv__.return_value = loss
        model.return_value.loss = loss
        def save(path, **kwargs):
            path.mkdir(parents=True)
            (path / "adapter_config.json").write_text("{}")
            (path / "adapter_model.safetensors").write_bytes(b"test")
        model.save_pretrained.side_effect = save
        tokenizer, loader = mock.Mock(), mock.Mock()
        loader.from_pretrained.return_value = model
        def config(**values):
            defaults = dict(peft_type="LORA", inference_mode=False, use_dora=False,
                            use_rslora=False, rank_pattern={}, alpha_pattern={},
                            modules_to_save=None, fan_in_fan_out=False, lora_bias=False,
                            layers_to_transform=None, target_parameters=None)
            defaults.update(values)
            return SimpleNamespace(to_dict=lambda: copy.deepcopy(defaults))
        peft = SimpleNamespace(LoraConfig=mock.Mock(side_effect=config),
                               get_peft_model=mock.Mock(return_value=model),
                               PeftModel=mock.Mock())
        def load_adapter(base_model, path, **kwargs):
            base_model.initial_weight_bytes = (Path(path) / "adapter_model.safetensors").read_bytes()
            return base_model
        peft.PeftModel.from_pretrained.side_effect = load_adapter
        modules = {
            "torch": torch,
            "transformers": SimpleNamespace(AutoTokenizer=tokenizer, AutoModelForCausalLM=loader),
            "peft": peft,
        }
        def prepare(case, *unused):
            return {"id": case["id"], "category": "確認", "input_ids": [1, 2, 3],
                    "labels": [-100, 2, 3], "prefix_length": 1}
        with mock.patch.dict(sys.modules, modules), \
             mock.patch("training.prepare_case", side_effect=prepare), \
             mock.patch("training.importlib.metadata.version", return_value="検査用版"), \
             redirect_stdout(io.StringIO()):
            yield SimpleNamespace(args=args, torch=torch, model=model, tokenizer=tokenizer,
                                  loader=loader, peft=peft)


def previous_adapter(fixture):
    """実学習を行わず、保存完了した前runの最小成果を隔離領域へ作る。"""
    previous = fixture.args.output.parent / "previous"
    adapter = previous / "checkpoint-epoch-1" / "adapter"
    adapter.mkdir(parents=True)
    config = lora_config(fixture.args.rank).to_dict()
    config.update(inference_mode=True, peft_version="保存時の版",
                  base_model_name_or_path="以前の配置")
    config["target_modules"].reverse()
    write_json(adapter / "adapter_config.json", config)
    (adapter / "adapter_model.safetensors").write_bytes(b"previous-lora-weights")
    manifest = previous / "manifest.json"
    write_json(manifest, {
        "status": "学習完了・品質未判定", "base_model_sha256": digest(fixture.args.base / "model.safetensors"),
        "arguments": {"rank": fixture.args.rank},
        "checkpoints": [{"adapter": "checkpoint-epoch-1/adapter", "status": "保存完了・品質未判定"}],
    })
    fixture.args.initial_adapter = adapter
    fixture.args.initial_manifest = manifest


class 学習条件の検査(unittest.TestCase):
    def test_初期adapterと前manifestは対でのみ指定できる(self):
        for changes in ({"initial_adapter": Path("adapter")}, {"initial_manifest": Path("manifest.json")}):
            with self.subTest(changes=changes), self.assertRaisesRegex(ValueError, "対で指定"):
                validate_args(arguments(**changes))
        validate_args(arguments())
        validate_args(arguments(initial_adapter=Path("adapter"), initial_manifest=Path("manifest.json")))

    def test_負の件数やゼロの学習条件と非有限の学習率を拒否する(self):
        for name, value in (
            ("limit", -1), ("rank", 0), ("rank", -2), ("max_length", 0),
            ("max_length", 1), ("warmup", -1), ("epochs", 0),
            ("accumulation", 0), ("max_steps", -1),
            ("learning_rate", 0), ("learning_rate", -1),
            ("learning_rate", float("inf")), ("learning_rate", float("nan")),
            ("learning_rate", True), ("rank", 1.5),
        ):
            with self.subTest(name=name, value=value), self.assertRaises(ValueError):
                validate_args(arguments(**{name: value}))
        validate_args(arguments(limit=0, warmup=0))
        validate_args(arguments(epochs=1, max_steps=1))
        with self.assertRaises(ValueError):
            validate_args(arguments(epochs=2, max_steps=1))

    def test_凍結重みも含めてcpu残留と更新対象の逸脱を拒否する(self):
        def parameter(device="cuda", trainable=True):
            return SimpleNamespace(device=SimpleNamespace(type=device), requires_grad=trainable)
        valid = [("base.weight", parameter(trainable=False)),
                 ("layer.lora_A.weight", parameter()), ("layer.lora_B.weight", parameter())]
        model = SimpleNamespace(named_parameters=lambda: iter(valid))
        self.assertEqual(len(cuda_trainable_parameters(model)), 2)
        for invalid in (
            [("base.weight", parameter(device="cpu", trainable=False)), valid[1]],
            [("base.weight", parameter())], [("base.weight", parameter(trainable=False))], [],
        ):
            with self.subTest(parameters=invalid), self.assertRaises(RuntimeError):
                cuda_trainable_parameters(SimpleNamespace(named_parameters=lambda: iter(invalid)))

    def test_勾配がない層やcpu上の勾配を更新しない(self):
        for gradient in (None, SimpleNamespace(device=SimpleNamespace(type="cpu"))):
            with self.subTest(gradient=gradient), self.assertRaises(RuntimeError):
                check_cuda_gradients([("layer.lora_B.weight", SimpleNamespace(grad=gradient))])
        check_cuda_gradients([("layer.lora_B.weight", SimpleNamespace(
            grad=SimpleNamespace(device=SimpleNamespace(type="cuda"))))])


class 準備失敗の記録(unittest.TestCase):
    def test_トークナイザとgpuとモデル読込の失敗をmanifestへ残す(self):
        for failure, phase in (
            ("tokenizer", "トークナイザと学習範囲の確認"),
            ("cuda", "CUDA GPUの確認"),
            ("model", "ベースモデルとLoRAの読込"),
        ):
            with self.subTest(failure=failure), isolated_training() as fixture:
                if failure == "tokenizer":
                    fixture.tokenizer.from_pretrained.side_effect = RuntimeError("トークナイザ読込の失敗")
                fixture.torch.cuda.is_available.return_value = failure != "cuda"
                fixture.loader.from_pretrained.side_effect = RuntimeError("モデル読込の失敗")
                with self.assertRaises(RuntimeError):
                    run(fixture.args)
                manifest = json.loads((fixture.args.output / "manifest.json").read_text(encoding="utf-8"))
                self.assertEqual(manifest["status"], "学習失敗・中断")
                self.assertEqual(manifest["phase"], phase)
                self.assertEqual(manifest["steps"], 0)
                self.assertEqual(manifest["arguments"]["seed"], 17)
                self.assertIn("RuntimeError", manifest["error"])
                self.assertEqual(len(manifest["base_model_sha256"]), 64)
                self.assertIn("run_elapsed_seconds", manifest)
                if failure == "model":
                    fixture.torch.cuda.empty_cache.assert_called_once()
                    self.assertEqual(manifest["gpu"], "検査用GPU")
                    self.assertIn("peak_reserved_vram_gb", manifest)
                else:
                    fixture.loader.from_pretrained.assert_not_called()
                    fixture.torch.cuda.empty_cache.assert_not_called()


class 初期adapterの引継ぎ(unittest.TestCase):
    def test_欠損とbaseや条件の不一致はモデルとgpuに触る前に拒否する(self):
        config_changes = {
            "r": 8, "lora_alpha": 16, "lora_dropout": 0.1, "bias": "all",
            "target_modules": ["q_proj"], "task_type": "SEQ_CLS", "peft_type": "IA3",
            "use_dora": True, "use_rslora": True, "rank_pattern": {"q_proj": 8},
            "alpha_pattern": {"q_proj": 16}, "modules_to_save": ["lm_head"],
            "fan_in_fan_out": True, "lora_bias": True, "layers_to_transform": [0],
            "target_parameters": ["experts.weight"],
        }
        failures = ["missing_manifest", "missing_config", "missing_weights", "base", "rank", "unsaved"]
        for failure in failures + list(config_changes):
            with self.subTest(failure=failure), isolated_training() as fixture:
                previous_adapter(fixture)
                config_path = fixture.args.initial_adapter / "adapter_config.json"
                manifest_path = fixture.args.initial_manifest
                if failure.startswith("missing_"):
                    paths = {"missing_manifest": manifest_path, "missing_config": config_path,
                             "missing_weights": fixture.args.initial_adapter / "adapter_model.safetensors"}
                    paths[failure].unlink()
                elif failure in failures:
                    data = json.loads(manifest_path.read_text(encoding="utf-8"))
                    if failure == "base":
                        data["base_model_sha256"] = "0" * 64
                    elif failure == "rank":
                        data["arguments"]["rank"] = 8
                    else:
                        data["checkpoints"] = []
                    write_json(manifest_path, data)
                else:
                    data = json.loads(config_path.read_text(encoding="utf-8"))
                    data[failure] = config_changes[failure]
                    write_json(config_path, data)
                with self.assertRaises(ValueError):
                    run(fixture.args)
                fixture.tokenizer.from_pretrained.assert_not_called()
                fixture.torch.cuda.is_available.assert_not_called()
                fixture.loader.from_pretrained.assert_not_called()
                manifest = json.loads((fixture.args.output / "manifest.json").read_text(encoding="utf-8"))
                self.assertEqual(manifest["status"], "学習失敗・中断")
                self.assertEqual(manifest["phase"], "初期adapterの由来と条件の確認")

    def test_指定した初期重みを読んでoptimizerと乱数は新runで開始する(self):
        with isolated_training(epochs=1, max_steps=1) as fixture:
            previous_adapter(fixture)
            previous_bytes = fixture.args.initial_manifest.read_bytes()
            run(fixture.args)
            self.assertEqual(fixture.model.initial_weight_bytes, b"previous-lora-weights")
            call = fixture.peft.PeftModel.from_pretrained.call_args
            self.assertIs(call.args[0], fixture.model)
            self.assertEqual(Path(call.args[1]), fixture.args.initial_adapter.resolve())
            self.assertTrue(call.kwargs["is_trainable"])
            self.assertTrue(call.kwargs["local_files_only"])
            self.assertEqual(call.kwargs["torch_device"], "cuda")
            fixture.peft.get_peft_model.assert_not_called()
            fixture.torch.optim.AdamW.assert_called_once()
            fixture.torch.optim.AdamW.return_value.load_state_dict.assert_not_called()
            fixture.torch.manual_seed.assert_called_once_with(fixture.args.seed)
            fixture.torch.cuda.manual_seed_all.assert_called_once_with(fixture.args.seed)
            self.assertEqual(fixture.args.initial_manifest.read_bytes(), previous_bytes)
            manifest = json.loads((fixture.args.output / "manifest.json").read_text(encoding="utf-8"))
            self.assertEqual(manifest["initialization"]["mode"], "adapter重みを引き継ぐ新規run")
            self.assertEqual(set(manifest["initialization"]["files"]), {"manifest", "config", "weights"})
            for entry in manifest["initialization"]["files"].values():
                self.assertEqual(entry["sha256"], digest(Path(entry["path"])))
                self.assertTrue(entry["role"])

    def test_未指定は従来の新規lora初期化を保つ(self):
        with isolated_training(epochs=1, max_steps=1) as fixture:
            run(fixture.args)
            fixture.peft.get_peft_model.assert_called_once()
            fixture.peft.PeftModel.from_pretrained.assert_not_called()
            manifest = json.loads((fixture.args.output / "manifest.json").read_text(encoding="utf-8"))
            self.assertEqual(manifest["initialization"]["mode"], "新規LoRA")

    def test_初期重みの欠損tensor警告は学習を開始せず失敗にする(self):
        with isolated_training() as fixture:
            previous_adapter(fixture)
            def missing(*args, **kwargs):
                warnings.warn("Found missing adapter keys while loading the checkpoint: ['lora_B'].")
            fixture.peft.PeftModel.from_pretrained.side_effect = missing
            with self.assertRaisesRegex(UserWarning, "missing adapter keys"):
                run(fixture.args)
            fixture.torch.optim.AdamW.assert_not_called()
            self.assertFalse((fixture.args.output / "adapter").exists())


class 学習結果の保存(unittest.TestCase):
    def test_共有読取の一時失敗は共通retryで回復し恒久失敗は原本を保つ(self):
        with isolated_training() as fixture:
            path = fixture.args.output.parent / "shared.json"
            path.write_text('{"old":true}')
            replace = os.replace
            error = PermissionError("共有読取の競合")
            error.winerror = 5
            for permanent in (False, True):
                with self.subTest(恒久失敗=permanent):
                    original = path.read_bytes()
                    attempts = 0
                    def replacement(source, destination):
                        nonlocal attempts
                        attempts += 1
                        if permanent or attempts == 1:
                            raise error
                        replace(source, destination)
                    with mock.patch.object(voice_training_common.sys, "platform", "win32"), \
                         mock.patch.object(voice_training_common.os, "replace", side_effect=replacement), \
                         mock.patch.object(voice_training_common.time, "sleep"):
                        if permanent:
                            with self.assertRaises(PermissionError):
                                write_json(path, {"new": True})
                            self.assertEqual(path.read_bytes(), original)
                            self.assertEqual(attempts, len(voice_training_common.ATOMIC_MOVE_DELAYS) + 1)
                        else:
                            write_json(path, {"new": True})
                            self.assertEqual(json.loads(path.read_text(encoding="utf-8")), {"new": True})
                            self.assertEqual(attempts, 2)
                    self.assertFalse(path.with_suffix(".json.incoming").exists())

    def test_各epochと最終adapterを保存しstep上限の途中を完了にしない(self):
        for epochs, max_steps, completed, steps in ((2, 0, 2, [2, 4]), (1, 1, 0, []), (1, 2, 1, [2])):
            with self.subTest(epochs=epochs, max_steps=max_steps), \
                 isolated_training(accumulation=2, epochs=epochs, max_steps=max_steps) as fixture:
                run(fixture.args)
                manifest = json.loads((fixture.args.output / "manifest.json").read_text(encoding="utf-8"))
                self.assertEqual(manifest["status"], "学習完了・品質未判定")
                self.assertEqual(manifest["completed_epochs"], completed)
                self.assertEqual([item["step"] for item in manifest["checkpoints"]], steps)
                expected = [f"checkpoint-epoch-{epoch}/adapter" for epoch in range(1, completed + 1)]
                self.assertEqual([item["adapter"] for item in manifest["checkpoints"]], expected)
                actual = [call.args[0].relative_to(fixture.args.output).as_posix()
                          for call in fixture.model.save_pretrained.call_args_list]
                self.assertEqual(actual, expected + ["adapter"])
                for path in actual:
                    self.assertTrue((fixture.args.output / path / "adapter_model.safetensors").is_file())

    def test_途中checkpointの保存失敗をepoch完了や最終成功にしない(self):
        with isolated_training(accumulation=2) as fixture:
            save = fixture.model.save_pretrained.side_effect
            def fail_second(path, **kwargs):
                if path.parent.name == "checkpoint-epoch-2":
                    raise OSError("2epoch目の保存失敗")
                save(path, **kwargs)
            fixture.model.save_pretrained.side_effect = fail_second
            with self.assertRaisesRegex(OSError, "2epoch目の保存失敗"):
                run(fixture.args)
            manifest = json.loads((fixture.args.output / "manifest.json").read_text(encoding="utf-8"))
            self.assertEqual(manifest["status"], "学習失敗・中断")
            self.assertEqual(manifest["completed_epochs"], 1)
            self.assertEqual([item["epoch"] for item in manifest["checkpoints"]], [1])
            self.assertEqual(manifest["phase"], "epoch 2 のアダプタ保存")
            self.assertFalse((fixture.args.output / "adapter").exists())


class 引数変換(unittest.TestCase):
    def test_引数だけを解釈して原文を変えない(self):
        messages = [{"role": "assistant", "content": "", "tool_calls": [{"function": {"name": "weather", "arguments": '{"location":"仙台"}'}}]}]
        original = copy.deepcopy(messages)
        result = template_messages(messages)
        self.assertEqual(messages, original)
        self.assertEqual(result[0]["tool_calls"][0]["function"]["arguments"], {"location": "仙台"})

    def test_壊れた引数を空にしない(self):
        for arguments in ("{壊れ", "[]", "null", 3):
            with self.subTest(arguments=arguments), self.assertRaises((ValueError, TypeError)):
                template_messages([{"role": "assistant", "content": "", "tool_calls": [{"function": {"name": "weather", "arguments": arguments}}]}])


@unittest.skipUnless(os.environ.get("LORA_TEST_MODEL"), "本番トークナイザの配置を明示したときだけ実施する")
class 本番トークンと学習範囲(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        from transformers import AutoTokenizer
        cls.tokenizer = AutoTokenizer.from_pretrained(os.environ["LORA_TEST_MODEL"], local_files_only=True)
        cls.template = (Path(__file__).resolve().parents[2] / "src-tauri/src/providers/templates/lfm25-jp-history.jinja").read_text(encoding="utf-8")

    def case(self):
        return {"id": "範囲検査", "body": {"messages": [
            {"role": "system", "content": "名前はこはる。非公開の人格設定。"},
            {"role": "user", "content": "私の飲み物は？"},
            {"role": "assistant", "content": "あなたはコーヒー好きです。"},
            {"role": "user", "content": "今の記憶には私はほうじ茶が好きとあります。確認して。"},
        ], "tools": []}, "target": {"role": "assistant", "content": "あなたはほうじ茶が好きだね。先ほどのコーヒーは間違いだったよ。"}}

    def test_誤答と私有資料は入力に保持するが正解として学習しない(self):
        case = self.case()
        row = prepare_case(case, self.tokenizer, self.template, 4096)
        count = row["prefix_length"]
        self.assertTrue(all(label == -100 for label in row["labels"][:count]))
        self.assertEqual(row["labels"][count:], row["input_ids"][count:])
        self.assertIn("あなたはコーヒー好きです。", row["prompt"])
        self.assertIn("非公開の人格設定", row["prompt"])
        answer = self.tokenizer.decode(row["labels"][count:])
        self.assertIn(case["target"]["content"], answer)
        self.assertNotIn("あなたはコーヒー好きです。", answer)
        self.assertNotIn("非公開の人格設定", answer)
        self.assertIn(self.tokenizer.eos_token_id, row["labels"][count:])

    def test_切り詰めと空回答を学習へ流さない(self):
        with self.assertRaises(ValueError):
            prepare_case(self.case(), self.tokenizer, self.template, 10)
        case = self.case()
        case["target"]["content"] = ""
        with self.assertRaises(ValueError):
            prepare_case(case, self.tokenizer, self.template, 4096)

    def test_ツール呼出自体を正しい境界で学習できる(self):
        case = self.case()
        case["target"] = {"role": "assistant", "content": "", "tool_calls": [{"id": "天気確認", "type": "function", "function": {"name": "weather", "arguments": '{"location":"仙台"}'}}]}
        row = prepare_case(case, self.tokenizer, self.template, 4096)
        self.assertIn("<|tool_call_start|>[weather(location='仙台')]<|tool_call_end|>", row["completion"])
        self.assertTrue(all(label == -100 for label in row["labels"][:row["prefix_length"]]))


if __name__ == "__main__":
    unittest.main()
