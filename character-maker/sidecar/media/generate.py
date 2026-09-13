"""PicoAgentが所有するComfyUIを一回だけ起動し、成果物を永続領域へ移す。"""
import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time
import urllib.request

import psutil


NEGATIVE = "low quality, worst quality, deformed, distorted, disfigured, motion smear, artifacts, text, watermark"


@contextmanager
def gpu_lock(path):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a+b") as handle:
        if handle.seek(0, 2) == 0:
            handle.write(b"0")
            handle.flush()
        handle.seek(0)
        if os.name == "nt":
            import msvcrt
            msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        try:
            yield
        finally:
            handle.seek(0)
            if os.name == "nt":
                msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(handle, fcntl.LOCK_UN)


def request_json(url, payload=None):
    data = None if payload is None else json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(url, data=data, headers={"Content-Type": "application/json"} if data else {})
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def wait_for_server(url, process, timeout):
    started = time.monotonic()
    while time.monotonic() - started < timeout:
        if process.poll() is not None:
            raise RuntimeError(f"ComfyUIが起動前に終了しました: {process.returncode}")
        try:
            request_json(url + "/system_stats")
            return
        except Exception:
            time.sleep(0.5)
    raise TimeoutError("ComfyUIの起動が完了しませんでした")


def stop_owned(process):
    if process.poll() is not None:
        process.wait()
        return
    try:
        parent = psutil.Process(process.pid)
        members = parent.children(recursive=True) + [parent]
    except psutil.NoSuchProcess:
        process.wait()
        return
    for member in reversed(members):
        try:
            member.terminate()
        except psutil.NoSuchProcess:
            pass
    _, alive = psutil.wait_procs(members, timeout=10)
    for member in alive:
        try:
            member.kill()
        except psutil.NoSuchProcess:
            pass
    psutil.wait_procs(alive, timeout=10)
    process.wait()


def qwen(prompt, seed, width, height, input_image):
    workflow = {
        "1":{"class_type":"UNETLoader","inputs":{"unet_name":"qwen_image_edit_2511_bf16.safetensors","weight_dtype":"default"}},
        "2":{"class_type":"CLIPLoader","inputs":{"clip_name":"qwen_2.5_vl_7b.safetensors","type":"qwen_image","device":"default"}},
        "3":{"class_type":"VAELoader","inputs":{"vae_name":"qwen_image_vae.safetensors"}},
        "5":{"class_type":"ModelSamplingAuraFlow","inputs":{"model":["1",0],"shift":3.1}},
        "6":{"class_type":"CFGNorm","inputs":{"model":["5",0],"strength":1}},
        "7":{"class_type":"TextEncodeQwenImageEditPlus","inputs":{"clip":["2",0],"prompt":prompt,"vae":["3",0]}},
        "8":{"class_type":"TextEncodeQwenImageEditPlus","inputs":{"clip":["2",0],"prompt":"","vae":["3",0]}},
        "9":{"class_type":"EmptySD3LatentImage","inputs":{"width":width,"height":height,"batch_size":1}},
        "10":{"class_type":"KSampler","inputs":{"model":["6",0],"positive":["7",0],"negative":["8",0],"latent_image":["9",0],"seed":seed,"steps":20,"cfg":4,"sampler_name":"euler","scheduler":"simple","denoise":1}},
        "12":{"class_type":"VAEDecode","inputs":{"samples":["10",0],"vae":["3",0]}},
        "13":{"class_type":"SaveImage","inputs":{"images":["12",0],"filename_prefix":"PicoAgent/image"}},
    }
    # 参照画像は任意入力。無い場合は同じ導入済みモデルで文章だけから新規生成する。
    if input_image:
        workflow["4"] = {"class_type":"LoadImage","inputs":{"image":"input.png"}}
        workflow["7"]["inputs"]["image1"] = ["4",0]
        workflow["8"]["inputs"]["image1"] = ["4",0]
    return workflow


def wan(prompt, seed, width, height, frames, input_image):
    workflow = {
        "37":{"class_type":"UNETLoader","inputs":{"unet_name":"wan2.2_ti2v_5B_fp16.safetensors","weight_dtype":"default"}},
        "38":{"class_type":"CLIPLoader","inputs":{"clip_name":"umt5_xxl_fp8_e4m3fn_scaled.safetensors","type":"wan","device":"default"}},
        "39":{"class_type":"VAELoader","inputs":{"vae_name":"wan2.2_vae.safetensors"}},
        "6":{"class_type":"CLIPTextEncode","inputs":{"clip":["38",0],"text":prompt}},
        "7":{"class_type":"CLIPTextEncode","inputs":{"clip":["38",0],"text":NEGATIVE}},
        "48":{"class_type":"ModelSamplingSD3","inputs":{"model":["37",0],"shift":8}},
        "55":{"class_type":"Wan22ImageToVideoLatent","inputs":{"vae":["39",0],"width":width,"height":height,"length":frames,"batch_size":1}},
        "3":{"class_type":"KSampler","inputs":{"model":["48",0],"positive":["6",0],"negative":["7",0],"latent_image":["55",0],"seed":seed,"steps":20,"cfg":5,"sampler_name":"uni_pc","scheduler":"simple","denoise":1}},
        "8":{"class_type":"VAEDecode","inputs":{"samples":["3",0],"vae":["39",0]}},
        "57":{"class_type":"CreateVideo","inputs":{"images":["8",0],"fps":24}},
        "58":{"class_type":"SaveVideo","inputs":{"video":["57",0],"filename_prefix":"PicoAgent/video","format":"auto","codec":"auto"}},
    }
    if input_image:
        workflow["56"] = {"class_type":"LoadImage","inputs":{"image":"input.png"}}
        workflow["55"]["inputs"]["start_image"] = ["56",0]
    return workflow


def ltx(prompt, seed, width, height, frames, input_image):
    workflow = {
        "38":{"class_type":"CLIPLoader","inputs":{"clip_name":"t5xxl_fp16.safetensors","type":"ltxv","device":"default"}},
        "44":{"class_type":"CheckpointLoaderSimple","inputs":{"ckpt_name":"ltxv-2b-0.9.8-distilled-fp8.safetensors"}},
        "6":{"class_type":"CLIPTextEncode","inputs":{"clip":["38",0],"text":prompt}},
        "7":{"class_type":"CLIPTextEncode","inputs":{"clip":["38",0],"text":NEGATIVE}},
        "69":{"class_type":"LTXVConditioning","inputs":{"positive":["6",0],"negative":["7",0],"frame_rate":24}},
        "70":{"class_type":"EmptyLTXVLatentVideo","inputs":{"width":width,"height":height,"length":frames,"batch_size":1}},
        "71":{"class_type":"LTXVScheduler","inputs":{"steps":8,"max_shift":2.05,"base_shift":0.95,"stretch":True,"terminal":0.1,"latent":["70",0]}},
        "73":{"class_type":"KSamplerSelect","inputs":{"sampler_name":"euler"}},
        "72":{"class_type":"SamplerCustom","inputs":{"model":["44",0],"positive":["69",0],"negative":["69",1],"sampler":["73",0],"sigmas":["71",0],"latent_image":["70",0],"add_noise":True,"noise_seed":seed,"cfg":1}},
        "8":{"class_type":"VAEDecode","inputs":{"samples":["72",0],"vae":["44",2]}},
        "80":{"class_type":"CreateVideo","inputs":{"images":["8",0],"fps":24}},
        "81":{"class_type":"SaveVideo","inputs":{"video":["80",0],"filename_prefix":"PicoAgent/video","format":"auto","codec":"auto"}},
    }
    if input_image:
        workflow["78"] = {"class_type":"LoadImage","inputs":{"image":"input.png"}}
        workflow["77"] = {"class_type":"LTXVImgToVideo","inputs":{"positive":["6",0],"negative":["7",0],"vae":["44",2],"image":["78",0],"width":width,"height":height,"length":frames,"batch_size":1,"strength":0.15}}
        workflow["69"]["inputs"].update(positive=["77",0], negative=["77",1])
        workflow["71"]["inputs"]["latent"] = ["77",2]
        workflow["72"]["inputs"]["latent_image"] = ["77",2]
        del workflow["70"]
    return workflow


def output_record(history, prompt_id):
    result = history.get(prompt_id, {})
    status = result.get("status", {})
    if status.get("status_str") == "error":
        messages = status.get("messages", [])
        details = [str(message[1].get("exception_message", message[1]))
                   for message in messages if isinstance(message, (list, tuple))
                   and len(message) > 1 and message[0] in ("execution_error", "execution_interrupted")
                   and isinstance(message[1], dict)]
        raise RuntimeError("ComfyUIの生成に失敗しました: " + ("; ".join(details) or "詳細なし"))
    for node in result.get("outputs", {}).values():
        for key in ("images", "gifs"):
            values = node.get(key, [])
            if values:
                return values[0]
    return None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", choices=("qwen-edit", "wan-2.2-5b", "ltx-0.9.8-2b-fp8"), required=True)
    parser.add_argument("--prompt", required=True)
    parser.add_argument("--input", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--comfy", type=Path, required=True)
    parser.add_argument("--models", type=Path, required=True)
    parser.add_argument("--gpu-lock", type=Path, required=True)
    parser.add_argument("--port", type=int, default=58130)
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--width", type=int, required=True)
    parser.add_argument("--height", type=int, required=True)
    parser.add_argument("--frames", type=int, default=97)
    parser.add_argument("--startup-timeout", type=int, default=600)
    parser.add_argument("--generation-timeout", type=int, default=14400)
    args = parser.parse_args()
    run =args.output.parent / (".generating-" + args.output.stem)
    temporary = args.output.with_suffix(args.output.suffix + ".incoming")
    # 既存の処理や完成品を上書きせず、この呼び出しが作った途中物だけを回収する。
    if args.output.exists() or temporary.exists():
        raise FileExistsError(f"生成先が既に存在します: {args.output}")
    run.mkdir(parents=True, exist_ok=False)
    try:
        (run / "input").mkdir()
        (run / "output").mkdir()
        (run / "temp").mkdir()
        if args.input:
            shutil.copy2(args.input, run / "input/input.png")
        workflow = qwen(args.prompt, args.seed, args.width, args.height, args.input) if args.engine == "qwen-edit" else (wan(args.prompt, args.seed, args.width, args.height, args.frames, args.input) if args.engine == "wan-2.2-5b" else ltx(args.prompt, args.seed, args.width, args.height, args.frames, args.input))
        with gpu_lock(args.gpu_lock):
            with socket.socket() as probe:
                probe.bind(("127.0.0.1", args.port))
            command = [os.fspath(Path(os.sys.executable)), os.fspath(args.comfy / "main.py"), "--listen", "127.0.0.1", "--port", str(args.port), "--base-directory", os.fspath(run), "--models-directory", os.fspath(args.models), "--temp-directory", os.fspath(run / "temp"), "--disable-auto-launch", "--disable-all-custom-nodes", "--disable-api-nodes", "--preview-method", "none"]
            environment = dict(os.environ, HF_HUB_OFFLINE="1", TRANSFORMERS_OFFLINE="1", HF_HUB_DISABLE_TELEMETRY="1")
            with (run / "comfy.log").open("w", encoding="utf-8") as log:
                process = subprocess.Popen(command, cwd=args.comfy, env=environment, stdout=log, stderr=subprocess.STDOUT, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
                try:
                    url = f"http://127.0.0.1:{args.port}"
                    wait_for_server(url, process, args.startup_timeout)
                    queued = request_json(url + "/prompt", {"prompt": workflow})
                    prompt_id = queued.get("prompt_id")
                    if not prompt_id:
                        raise ValueError(f"ワークフローが拒否されました: {queued}")
                    started = time.monotonic()
                    record = None
                    while time.monotonic() - started < args.generation_timeout:
                        if process.poll() is not None:
                            raise RuntimeError(f"ComfyUIが生成中に終了しました: {process.returncode}")
                        record = output_record(request_json(url + "/history/" + prompt_id), prompt_id)
                        if record:
                            break
                        time.sleep(2)
                    if not record:
                        raise TimeoutError("生成が時間内に完了しませんでした")
                    source = run / record.get("type", "output") / record.get("subfolder", "") / record["filename"]
                    if not source.is_file():
                        raise FileNotFoundError(f"生成結果が見つかりません: {source}")
                    args.output.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(source, temporary)
                    temporary.replace(args.output)
                    print(json.dumps({"output": os.fspath(args.output), "seed": args.seed}, ensure_ascii=False), flush=True)
                finally:
                    stop_owned(process)
    finally:
        try:
            temporary.unlink(missing_ok=True)
        finally:
            shutil.rmtree(run)


if __name__ == "__main__":
    main()
