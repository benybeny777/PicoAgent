#!/usr/bin/env python3
"""Style-Bert-VITS2の学習用音声断片を検査し、ノイズの少ない断片だけで別名の新データセットを作る。

Style-Bert-VITS2の `.venv` のPythonで実行する。元データセットと元音声は読み取りだけで、書き換えない。

  # 1. 断片ごとの測定（GPUでWhisperと話者埋め込みを使う）
  python scripts/select-voice-training-clips.py measure \
      --sbv2-root temp/voice-models/Style-Bert-VITS2 --dataset sakura-miko \
      --report temp/voice-models/clean-v2/sakura-miko/clip-metrics.jsonl

  # 2. 基準で選別し、正規化済みの新データセットを作る（出力先が既にあれば停止する）
  python scripts/select-voice-training-clips.py select \
      --sbv2-root temp/voice-models/Style-Bert-VITS2 --dataset sakura-miko \
      --report temp/voice-models/clean-v2/sakura-miko/clip-metrics.jsonl \
      --output-dataset sakura-miko-clean-v2

候補は `esd.unfiltered.list` があればそれを、なければ `esd.list` を使う。
"""

from __future__ import annotations

import argparse
import json
import math
import os
import sys
from dataclasses import dataclass
from pathlib import Path

import numpy as np
import soundfile as sf

# 取得済みキャッシュだけを使い、測定のために新しいモデルを取得しない。
os.environ.setdefault("HF_HUB_OFFLINE", "1")

TARGET_LUFS = -23.0
PEAK_CEILING_DBFS = -1.0
HALLUCINATIONS = {
    "日本語の雑談です。",
    "ご視聴ありがとうございました。",
    "ご視聴ありがとうございました",
    "チャンネル登録よろしくお願いします。",
}


# ---------------------------------------------------------------- 共通


def load_mono(path: Path) -> tuple[np.ndarray, int]:
    audio, sample_rate = sf.read(path, dtype="float64", always_2d=True)
    return audio.mean(axis=1), sample_rate


def db(power: float) -> float:
    return 10.0 * math.log10(max(power, 1e-12))


def frame_power(x: np.ndarray, hop: int) -> np.ndarray:
    count = len(x) // hop
    if count == 0:
        return np.array([np.mean(x**2) if len(x) else 0.0])
    return np.mean(x[: count * hop].reshape(count, hop) ** 2, axis=1)


def resample(x: np.ndarray, source_rate: int, target_rate: int) -> np.ndarray:
    from scipy.signal import resample_poly

    if source_rate == target_rate:
        return x
    divisor = math.gcd(source_rate, target_rate)
    return resample_poly(x, target_rate // divisor, source_rate // divisor)


def read_candidates(dataset_root: Path) -> list[tuple[str, str, str, str]]:
    source_list = dataset_root / "esd.unfiltered.list"
    if not source_list.exists():
        source_list = dataset_root / "esd.list"
    entries = []
    for line in source_list.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        file_name, speaker, language, text = line.split("|", 3)
        entries.append((file_name, speaker, language, text.strip()))
    return entries


def kana(text: str) -> str:
    import pyopenjtalk

    converted = pyopenjtalk.g2p(text, kana=True)
    return "".join(ch for ch in converted if "゠" <= ch <= "ヿ" and ch != "・")


def edit_distance(a: str, b: str) -> int:
    previous = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        current = [i]
        for j, cb in enumerate(b, 1):
            current.append(min(previous[j] + 1, current[j - 1] + 1, previous[j - 1] + (ca != cb)))
        previous = current
    return previous[-1]


# ---------------------------------------------------------------- 測定


@dataclass
class Models:
    vad: object
    get_speech_timestamps: object
    whisper: object
    speaker: object


def load_models(device: str, vad_only: bool = False) -> Models:
    import torch
    from faster_whisper import WhisperModel
    from pyannote.audio import Inference, Model

    hub_dir = Path(torch.hub.get_dir()) / "litagin02_silero-vad_master"
    if not hub_dir.exists():
        raise SystemExit(f"silero-vadのローカルキャッシュがありません: {hub_dir}")
    vad, utils = torch.hub.load(str(hub_dir), "silero_vad", source="local", onnx=False, trust_repo=True)
    if vad_only:
        return Models(vad=vad, get_speech_timestamps=utils[0], whisper=None, speaker=None)
    whisper = WhisperModel("large-v3", device=device, compute_type="float16" if device == "cuda" else "int8")
    speaker_model = Model.from_pretrained("pyannote/wespeaker-voxceleb-resnet34-LM")
    speaker = Inference(speaker_model, window="whole")
    speaker.to(torch.device(device))
    return Models(vad=vad, get_speech_timestamps=utils[0], whisper=whisper, speaker=speaker)


def speaker_embedding(models: Models, audio16: np.ndarray) -> np.ndarray:
    import torch

    waveform = torch.from_numpy(audio16.astype(np.float32)).unsqueeze(0)
    vector = np.asarray(models.speaker({"waveform": waveform, "sample_rate": 16000}), dtype=np.float64).reshape(-1)
    return vector / (np.linalg.norm(vector) + 1e-12)


def measure_clip(models: Models, wav_path: Path, text: str, previous: dict | None = None) -> dict:
    import pyloudnorm
    import torch
    from scipy.signal import butter, sosfilt, welch

    x, sr = load_mono(wav_path)
    duration = len(x) / sr
    result: dict = {"duration": round(duration, 3), "sample_rate": sr}
    abs_x = np.abs(x)
    result["peak_dbfs"] = round(20 * math.log10(max(abs_x.max(), 1e-9)), 2)
    clipped = abs_x >= 0.999
    result["clip_samples"] = int(clipped.sum())
    longest = current = 0
    for flag in clipped:
        current = current + 1 if flag else 0
        longest = max(longest, current)
    result["clip_longest_run"] = longest
    result["dc_offset"] = round(float(x.mean()), 6)

    x = x - x.mean()
    try:
        loudness = pyloudnorm.Meter(sr).integrated_loudness(x)
    except ValueError:
        loudness = float("-inf")
    result["loudness_lufs"] = round(loudness, 2) if math.isfinite(loudness) else None
    gain_db = (TARGET_LUFS - loudness) if math.isfinite(loudness) else 0.0
    result["normalize_gain_db"] = round(gain_db, 2)
    result["normalized_peak_dbfs"] = round(result["peak_dbfs"] + gain_db, 2)

    # 発話区間（VAD）
    audio16 = resample(x, sr, 16000).astype(np.float32)
    stamps = models.get_speech_timestamps(
        torch.from_numpy(audio16), models.vad, sampling_rate=16000,
        min_silence_duration_ms=120, speech_pad_ms=30,
    )
    speech_mask = np.zeros(len(x), dtype=bool)
    guard_mask = np.zeros(len(x), dtype=bool)
    guard = int(0.06 * sr)
    for stamp in stamps:
        start = int(stamp["start"] * sr / 16000)
        end = int(stamp["end"] * sr / 16000)
        speech_mask[start:end] = True
        guard_mask[max(0, start - guard): min(len(x), end + guard)] = True
    result["speech_ratio"] = round(float(speech_mask.mean()), 3)
    if stamps:
        result["leading_silence"] = round(stamps[0]["start"] / 16000, 3)
        result["trailing_silence"] = round(duration - stamps[-1]["end"] / 16000, 3)
        result["speech_start"] = round(stamps[0]["start"] / 16000, 3)
        result["speech_end"] = round(stamps[-1]["end"] / 16000, 3)
    else:
        result["leading_silence"] = result["trailing_silence"] = round(duration, 3)
        result["speech_start"] = result["speech_end"] = 0.0

    hop = int(0.02 * sr)
    frames_db = np.array([db(p) for p in frame_power(x, hop)]) + gain_db
    speech_frames = frame_power(x[speech_mask], hop) if speech_mask.any() else np.array([1e-12])
    speech_median = float(np.median([db(p) for p in speech_frames])) + gain_db
    result["speech_median_db"] = round(speech_median, 2)

    # 発話の合間のノイズ床。背景BGMや定常ノイズがここに残る。
    gap = x[~guard_mask]
    result["gap_seconds"] = round(len(gap) / sr, 3)
    if len(gap) >= int(0.25 * sr):
        # 語尾の余韻や息は合間の平均を押し上げるため、合間で最も静かな側（下位10%）を定常ノイズとみなす。
        gap_frames = np.array([db(p) for p in frame_power(gap, hop)])
        result["gap_mean_db"] = round(db(float(np.mean(gap**2))) + gain_db, 2)
        floor = float(np.percentile(gap_frames, 10)) + gain_db
        result["noise_floor_method"] = "vad-gap-p10"
    else:
        floor = float(np.percentile(frames_db, 5))
        result["noise_floor_method"] = "frame-p5"
    result["noise_floor_db"] = round(floor, 2)
    result["snr_db"] = round(speech_median - floor, 2)

    # 合間の周波数特性: 音楽や電子音は平坦度が低く、特定帯域にエネルギーが残る。
    if len(gap) >= int(0.25 * sr):
        freqs, psd = welch(gap * 10 ** (gain_db / 20), sr, nperseg=min(4096, len(gap)))
        band = (freqs >= 100) & (freqs <= 8000)
        result["gap_flatness"] = round(float(np.exp(np.mean(np.log(psd[band] + 1e-20))) / (np.mean(psd[band]) + 1e-20)), 4)
        for low, high in ((50, 300), (300, 2000), (2000, 6000), (6000, 16000)):
            sel = (freqs >= low) & (freqs < high)
            result[f"gap_band_{low}_{high}_db"] = round(db(float(np.sum(psd[sel]) * (freqs[1] - freqs[0]))), 2)
    else:
        result["gap_flatness"] = None

    # 前処理で残す発話前120ms・発話後150ms（VADの余白30msを除く）の音量。境界のノイズやクリックを見る。
    head = x[max(0, int((result["speech_start"] - 0.12) * sr)): max(0, int((result["speech_start"] - 0.03) * sr))]
    tail = x[int((result["speech_end"] + 0.03) * sr): int((result["speech_end"] + 0.15) * sr)]
    result["head_db"] = round(db(float(np.mean(head**2))) + gain_db, 2) if len(head) >= int(0.03 * sr) else None
    result["tail_db"] = round(db(float(np.mean(tail**2))) + gain_db, 2) if len(tail) >= int(0.03 * sr) else None

    # 破裂音（吹かれ）: 100Hz未満の短時間エネルギーが、その断片の低域中央値から急に跳ねる瞬間を数える。
    # 背景の定常的な低音は中央値へ吸収されるため、ここでは数えずノイズ床側で判定する。
    sos = butter(4, 100, btype="lowpass", fs=sr, output="sos")
    low = sosfilt(sos, x)
    low_db = np.array([db(p) for p in frame_power(low, hop)]) + gain_db
    low_rel = low_db - float(np.median(low_db))
    result["plosive_peak_rel_db"] = round(float(low_db.max() - speech_median), 2)
    result["plosive_burst_db"] = round(float(low_rel.max()), 2)
    result["plosive_frames"] = int(np.sum((low_rel > 15.0) & (low_db > speech_median - 10.0)))

    if previous is not None:
        # 音響指標だけ再計算する場合、重いWhisperと話者埋め込みは前回の測定を流用する。
        for key in ("_embedding", "_window_embeddings", "asr_text", "asr_avg_logprob", "asr_no_speech_prob", "kana_length", "kana_cer"):
            result[key] = previous.get(key)
        return result

    # 話者埋め込み（全体と2秒窓）。窓ごとの最小類似度で他人の声の混入を見る。
    result["_embedding"] = speaker_embedding(models, audio16).round(5).tolist()
    windows = []
    step = 16000
    for start in range(0, max(1, len(audio16) - 2 * 16000 + 1), step):
        chunk = audio16[start: start + 2 * 16000]
        if len(chunk) < 16000:
            continue
        chunk_mask = speech_mask[int(start * sr / 16000): int((start + len(chunk)) * sr / 16000)]
        if chunk_mask.mean() < 0.4:
            continue
        windows.append(speaker_embedding(models, chunk).round(5).tolist())
    result["_window_embeddings"] = windows

    # 文字起こし不一致: 候補文とWhisper再認識をカナへ揃えて比較する。
    segments, _ = models.whisper.transcribe(
        audio16, language="ja", beam_size=5, condition_on_previous_text=False,
        vad_filter=False, without_timestamps=True,
    )
    segments = list(segments)
    recognized = "".join(segment.text for segment in segments).strip()
    result["asr_text"] = recognized
    result["asr_avg_logprob"] = round(float(np.mean([s.avg_logprob for s in segments])), 3) if segments else None
    result["asr_no_speech_prob"] = round(float(np.max([s.no_speech_prob for s in segments])), 3) if segments else None
    reference, hypothesis = kana(text), kana(recognized)
    result["kana_length"] = len(reference)
    result["kana_cer"] = round(edit_distance(reference, hypothesis) / max(1, len(reference)), 3)
    return result


def command_measure(args: argparse.Namespace) -> int:
    dataset_root = Path(args.sbv2_root) / "Data" / args.dataset
    report = Path(args.report)
    report.parent.mkdir(parents=True, exist_ok=True)
    done: set[str] = set()
    if report.exists():
        for line in report.read_text(encoding="utf-8").splitlines():
            if line.strip():
                done.add(json.loads(line)["file"])
    candidates = read_candidates(dataset_root)
    if args.limit:
        candidates = candidates[: args.limit]
    previous_rows: dict[str, dict] = {}
    if args.reuse_report:
        for line in Path(args.reuse_report).read_text(encoding="utf-8").splitlines():
            if line.strip():
                previous = json.loads(line)
                previous_rows[previous["file"]] = previous
        missing = [c[0] for c in candidates if c[0] not in previous_rows]
        if missing:
            raise SystemExit(f"流用元の測定に無い断片があります: {missing[:3]}")
    models = load_models(args.device, vad_only=bool(previous_rows))
    with report.open("a", encoding="utf-8") as output:
        for index, (file_name, speaker, language, text) in enumerate(candidates, 1):
            if file_name in done:
                continue
            wav_path = dataset_root / "raw" / file_name
            row = {"file": file_name, "speaker": speaker, "language": language, "text": text}
            if not wav_path.exists():
                row["missing"] = True
            else:
                row.update(measure_clip(models, wav_path, text, previous_rows.get(file_name)))
            output.write(json.dumps(row, ensure_ascii=False) + "\n")
            output.flush()
            if index % 20 == 0 or index == len(candidates):
                print(f"測定 {index}/{len(candidates)}", flush=True)
    print(f"測定完了: {report}")
    return 0


# ---------------------------------------------------------------- 選別


def speaker_scores(rows: list[dict]) -> None:
    usable = [r for r in rows if r.get("_embedding")]
    matrix = np.array([r["_embedding"] for r in usable])
    centroid = matrix.mean(axis=0)
    # 外れ値に引っ張られないよう、上位70%で重心を2回作り直す。
    for _ in range(2):
        centroid /= np.linalg.norm(centroid)
        similarity = matrix @ centroid
        keep = similarity >= np.quantile(similarity, 0.3)
        centroid = matrix[keep].mean(axis=0)
    centroid /= np.linalg.norm(centroid)
    for row in usable:
        row["speaker_similarity"] = round(float(np.dot(row["_embedding"], centroid)), 4)
        windows = row.get("_window_embeddings") or []
        row["speaker_window_min"] = (
            round(float(min(np.dot(w, centroid) for w in windows)), 4) if windows else row["speaker_similarity"]
        )


def mark_unreadable_text(rows: list[dict], sbv2_root: Path) -> None:
    """SBV2の文章前処理と同じ読み変換で読めない文を印付けする。

    Whisperが存在しない漢字を付け足すと、カナ変換は黙って読み飛ばすためカナ誤り率では検出できず、
    学習前処理の preprocess_text.py で初めて停止する。選別の段階で同じ変換を通して除外する。
    """
    root = sbv2_root.resolve()
    previous_cwd = Path.cwd()
    sys.path.insert(0, str(root))
    os.chdir(root)  # 利用者辞書 dict_data/ を相対パスで読むため
    try:
        from style_bert_vits2.nlp import clean_text
        from style_bert_vits2.nlp.japanese import pyopenjtalk_worker
        from style_bert_vits2.nlp.japanese.user_dict import update_dict

        pyopenjtalk_worker.initialize_worker()
        update_dict()
        try:
            for row in rows:
                if row.get("missing"):
                    continue
                try:
                    clean_text(text=row["text"], language=row["language"], use_jp_extra=True, raise_yomi_error=True)
                    row["unreadable_text"] = False
                except Exception as error:  # 読み以外の変換失敗も学習前処理で停止するため、同じく除外理由として残す
                    row["unreadable_text"] = True
                    row["unreadable_detail"] = str(error)[:120]
        finally:
            pyopenjtalk_worker.terminate_worker()
    finally:
        os.chdir(previous_cwd)


@dataclass
class Thresholds:
    min_duration: float = 2.0
    max_duration: float = 12.0
    max_dc_offset: float = 0.005
    # 既定値は-23LUFS正規化後の値。BGM常時ありの配信（さくらみこ t79gKhjky7A）の通過を3%未満に抑えたまま、
    # 静かな単独雑談（さくらみこ UrOvtpGoW5s）から学習量を確保できる点に較正した。
    min_speech_ratio: float = 0.5
    max_noise_floor_db: float = -48.0
    min_snr_db: float = 22.0
    max_edge_db: float = -30.0
    max_plosive_peak_rel_db: float = 0.0
    max_plosive_frames: int = 3
    min_speaker_similarity: float = 0.55
    min_speaker_window: float = 0.40
    max_kana_cer: float = 0.25
    min_kana_length: int = 6


def rejection_reasons(row: dict, limits: Thresholds) -> list[str]:
    if row.get("missing"):
        return ["missing"]
    reasons = []
    if row["text"] in HALLUCINATIONS:
        reasons.append("asr-hallucination")
    if not limits.min_duration <= row["duration"] <= limits.max_duration:
        reasons.append(f"duration={row['duration']}")
    if row["clip_samples"] > 0:
        reasons.append(f"clip={row['clip_samples']}")
    if abs(row["dc_offset"]) > limits.max_dc_offset:
        reasons.append(f"dc={row['dc_offset']}")
    if row["speech_ratio"] < limits.min_speech_ratio:
        reasons.append(f"speech-ratio={row['speech_ratio']}")
    if row["noise_floor_db"] > limits.max_noise_floor_db:
        reasons.append(f"noise-floor={row['noise_floor_db']}")
    if row["snr_db"] < limits.min_snr_db:
        reasons.append(f"snr={row['snr_db']}")
    # 境界区間が断片内に残らない場合（発話が断片の端まで続く）は判定しない。
    if row.get("head_db") is not None and row["head_db"] > limits.max_edge_db:
        reasons.append(f"head-noise={row['head_db']}")
    if row.get("tail_db") is not None and row["tail_db"] > limits.max_edge_db:
        reasons.append(f"tail-noise={row['tail_db']}")
    if row["plosive_peak_rel_db"] > limits.max_plosive_peak_rel_db or row["plosive_frames"] > limits.max_plosive_frames:
        reasons.append(f"plosive={row['plosive_peak_rel_db']}/{row['plosive_frames']}")
    if row.get("speaker_similarity", 1.0) < limits.min_speaker_similarity:
        reasons.append(f"speaker={row['speaker_similarity']}")
    if row.get("speaker_window_min", 1.0) < limits.min_speaker_window:
        reasons.append(f"speaker-window={row['speaker_window_min']}")
    if row["kana_length"] < limits.min_kana_length:
        reasons.append(f"text-short={row['kana_length']}")
    if row["kana_cer"] > limits.max_kana_cer:
        reasons.append(f"transcript-mismatch={row['kana_cer']}")
    if row.get("unreadable_text"):
        reasons.append("unreadable-text")
    return reasons


def write_clean_clip(source: Path, destination: Path, row: dict) -> dict:
    """DC除去、発話境界までの切り詰め、-23LUFSかつ-1dBFS以下の単純ゲインだけを行う。"""
    import pyloudnorm

    x, sr = load_mono(source)
    x = x - x.mean()
    start = max(0, int((row["speech_start"] - 0.12) * sr))
    end = min(len(x), int((row["speech_end"] + 0.15) * sr))
    x = x[start:end]
    fade = min(int(0.01 * sr), len(x) // 4)
    if fade:
        ramp = np.linspace(0.0, 1.0, fade)
        x[:fade] *= ramp
        x[-fade:] *= ramp[::-1]
    loudness = pyloudnorm.Meter(sr).integrated_loudness(x)
    gain = 10 ** ((TARGET_LUFS - loudness) / 20)
    ceiling = 10 ** (PEAK_CEILING_DBFS / 20)
    peak = float(np.abs(x).max()) * gain
    if peak > ceiling:
        gain *= ceiling / peak
    y = x * gain
    destination.parent.mkdir(parents=True, exist_ok=True)
    sf.write(destination, y, sr, subtype="PCM_16")
    return {
        "output_seconds": round(len(y) / sr, 3),
        "output_peak_dbfs": round(20 * math.log10(max(float(np.abs(y).max()), 1e-9)), 2),
        "output_lufs": round(pyloudnorm.Meter(sr).integrated_loudness(y), 2),
    }


def command_select(args: argparse.Namespace) -> int:
    sbv2_root = Path(args.sbv2_root)
    output_root = sbv2_root / "Data" / args.output_dataset
    if len(args.dataset) != len(args.report):
        raise SystemExit("--dataset と --report は同じ数だけ、同じ順で指定してください")
    rows = []
    for dataset, report_path in zip(args.dataset, args.report):
        dataset_rows = [json.loads(line) for line in Path(report_path).read_text(encoding="utf-8").splitlines() if line.strip()]
        candidates = read_candidates(sbv2_root / "Data" / dataset)
        if len(dataset_rows) != len(candidates):
            raise SystemExit(f"測定が未完了です: {dataset} {len(dataset_rows)}/{len(candidates)}")
        for row in dataset_rows:
            row["dataset"] = dataset
        rows.extend(dataset_rows)
    speaker_scores(rows)
    mark_unreadable_text(rows, sbv2_root)
    limits = Thresholds(**{k: v for k, v in vars(args).items() if k in Thresholds.__dataclass_fields__ and v is not None})

    accepted, rejected = [], []
    for row in rows:
        reasons = rejection_reasons(row, limits)
        row["reasons"] = reasons
        (rejected if reasons else accepted).append(row)
    total_seconds = sum(r["duration"] for r in accepted)
    summary = {
        "source_dataset": args.dataset,
        "output_dataset": args.output_dataset,
        "thresholds": vars(limits),
        "candidates": len(rows),
        "accepted": len(accepted),
        "rejected": len(rejected),
        "accepted_minutes": round(total_seconds / 60, 1),
        "reason_counts": {},
    }
    for row in rejected:
        for reason in row["reasons"]:
            key = reason.split("=")[0]
            summary["reason_counts"][key] = summary["reason_counts"].get(key, 0) + 1
    for key in ("noise_floor_db", "snr_db", "speaker_similarity", "speaker_window_min", "kana_cer", "plosive_peak_rel_db"):
        values = np.array([r[key] for r in rows if r.get(key) is not None], dtype=np.float64)
        summary[f"distribution_{key}"] = {str(p): round(float(np.percentile(values, p)), 3) for p in (5, 25, 50, 75, 95)}

    report_dir = Path(args.work_dir) if args.work_dir else Path(args.report[0]).parent
    report_dir.mkdir(parents=True, exist_ok=True)
    lean = [{k: v for k, v in r.items() if not k.startswith("_")} for r in rows]
    (report_dir / "clip-decisions.jsonl").write_text(
        "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in lean), encoding="utf-8"
    )
    (report_dir / "selection-summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    if args.dry_run:
        return 0

    if output_root.exists():
        raise SystemExit(f"出力先が既に存在するため停止します（既存データを上書きしない）: {output_root}")
    if len(accepted) < args.min_clips:
        raise SystemExit(f"採用断片が少なすぎます: {len(accepted)} < {args.min_clips}")
    (output_root / "raw").mkdir(parents=True)
    lines, outputs = [], []
    for row in accepted:
        info = write_clean_clip(sbv2_root / "Data" / row["dataset"] / "raw" / row["file"], output_root / "raw" / row["file"], row)
        if info["output_peak_dbfs"] > PEAK_CEILING_DBFS + 0.05:
            raise SystemExit(f"正規化後のピークが上限を超えました: {row['file']} {info}")
        outputs.append({"file": row["file"], **info})
        lines.append(f"{row['file']}|{args.output_dataset}|{row['language']}|{row['text']}")
    (output_root / "esd.list").write_text("\n".join(lines) + "\n", encoding="utf-8")
    (output_root / "esd.rejected.tsv").write_text(
        "".join(f"{';'.join(r['reasons'])}\t{r['file']}|{r['text']}\n" for r in rejected), encoding="utf-8"
    )
    (output_root / "clean-clips.json").write_text(json.dumps(outputs, ensure_ascii=False, indent=1), encoding="utf-8")
    print(f"新データセットを作成しました: {output_root}（{len(accepted)}断片）")
    return 0


def command_quiet_sections(args: argparse.Namespace) -> int:
    """長い配信から、BGMや定常ノイズの少ない区間だけを区間ごとのWAVとして書き出す（slice.pyの入力用）。"""
    from scipy.signal import medfilt, welch

    source = Path(args.source)
    output_dir = Path(args.output_dir)
    if output_dir.exists() and any(output_dir.iterdir()):
        raise SystemExit(f"出力先が空ではないため停止します: {output_dir}")
    output_dir.mkdir(parents=True, exist_ok=True)
    info = sf.info(source)
    sr = info.samplerate
    window = int(args.window_seconds * sr)
    skip_head = int(args.skip_head_seconds * sr)
    skip_tail = int(args.skip_tail_seconds * sr)
    sections = []
    kept_seconds = 0.0
    for start in range(skip_head, info.frames - skip_tail - window + 1, window):
        x, _ = sf.read(source, start=start, frames=window, dtype="float64", always_2d=True)
        x = x.mean(axis=1)
        hop = int(0.05 * sr)
        levels = np.array([db(p) for p in frame_power(x, hop)])
        quiet_runs = current = 0
        for is_quiet in levels < -45.0:
            current = current + 1 if is_quiet else 0
            if current == 20:
                quiet_runs += 1
        low = x.reshape(-1)[: len(levels) * hop].reshape(len(levels), hop)[levels <= np.percentile(levels, 10)].reshape(-1)
        freqs, psd = welch(low, sr, nperseg=8192)
        band = (freqs > 60) & (freqs < 4000)
        level = 10 * np.log10(psd[band] + 1e-20)
        row = {
            "start": round(start / sr, 1),
            "p10_db": round(float(np.percentile(levels, 10)), 2),
            "quiet_runs": quiet_runs,
            "tonal_peaks": int(np.sum(level - medfilt(level, 101) > 12)),
            "clip_samples": int(np.sum(np.abs(x) >= 0.999)),
        }
        row["accepted"] = (
            row["p10_db"] <= args.max_p10_db
            and row["quiet_runs"] >= args.min_quiet_runs
            and row["tonal_peaks"] <= args.max_tonal_peaks
            and row["clip_samples"] == 0
        )
        if row["accepted"]:
            name = f"{args.prefix}-{int(start / sr):05d}.wav"
            sf.write(output_dir / name, x, sr, subtype="PCM_16")
            kept_seconds += window / sr
        sections.append(row)
    summary = {
        "source": str(source),
        "criteria": {k: getattr(args, k) for k in ("window_seconds", "skip_head_seconds", "skip_tail_seconds", "max_p10_db", "min_quiet_runs", "max_tonal_peaks")},
        "sections": len(sections),
        "accepted_sections": sum(r["accepted"] for r in sections),
        "accepted_minutes": round(kept_seconds / 60, 1),
    }
    (output_dir.parent / f"{args.prefix}-sections.json").write_text(
        json.dumps({**summary, "detail": sections}, ensure_ascii=False, indent=1), encoding="utf-8"
    )
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    quiet = sub.add_parser("quiet-sections", help="長い配信から静かな区間だけを書き出す")
    quiet.add_argument("--source", required=True)
    quiet.add_argument("--output-dir", required=True, help="slice.pyの--input_dirに渡す空のフォルダ")
    quiet.add_argument("--prefix", required=True)
    quiet.add_argument("--window-seconds", type=float, default=60.0)
    quiet.add_argument("--skip-head-seconds", type=float, default=300.0, help="オープニングBGMを避ける")
    quiet.add_argument("--skip-tail-seconds", type=float, default=300.0, help="エンディングBGMを避ける")
    quiet.add_argument("--max-p10-db", type=float, default=-52.0)
    quiet.add_argument("--min-quiet-runs", type=int, default=2, help="窓内で1秒以上-45dB未満になる回数")
    quiet.add_argument("--max-tonal-peaks", type=int, default=3)
    for name in ("measure", "select"):
        p = sub.add_parser(name)
        p.add_argument("--sbv2-root", required=True)
        many = "+" if name == "select" else None
        p.add_argument("--dataset", required=True, nargs=many)
        p.add_argument("--report", required=True, nargs=many)
        if name == "measure":
            p.add_argument("--device", default="cuda")
            p.add_argument("--limit", type=int, default=0, help="動作確認用に先頭N件だけ測定する")
            p.add_argument("--reuse-report", help="前回の測定からWhisperと話者埋め込みを流用し、音響指標だけ再計算する")
        else:
            p.add_argument("--output-dataset", required=True)
            p.add_argument("--dry-run", action="store_true", help="判定と集計だけを出力し、データセットを作らない")
            p.add_argument("--work-dir", help="判定一覧と集計の保存先。省略時は最初の--reportと同じフォルダ")
            p.add_argument("--min-clips", type=int, default=150)
            for field_name, field in Thresholds.__dataclass_fields__.items():
                p.add_argument(f"--{field_name.replace('_', '-')}", dest=field_name, type=type(field.default), default=None)
    args = parser.parse_args()
    commands = {"quiet-sections": command_quiet_sections, "measure": command_measure, "select": command_select}
    return commands[args.command](args)


if __name__ == "__main__":
    sys.exit(main())
