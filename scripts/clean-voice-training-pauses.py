#!/usr/bin/env python3
"""選別済み学習データの「発話の合間」だけを減衰させ、間が静かなデータセットを別名で作る。

Style-Bert-VITS2は学習データの合間に残る小さな環境音をそのまま覚え、合成時の間（読点・疑問符・言いよどみ）に
ノイズとして出す。clean-v2では合間がピーク基準で中央値-42dB前後あり、合成音の合間ノイズもほぼ同じ値になった。
このスクリプトはsilero-vadで発話区間を求め、区間外をなだらかに減衰させる。声の区間には一切手を加えない。

Style-Bert-VITS2の `.venv` のPythonで実行する。入力は select-voice-training-clips.py が作った Data/<source>。
出力先が既にあれば停止する（既存データセットを上書きしない）。

  python scripts/clean-voice-training-pauses.py --sbv2-root temp/voice-models/Style-Bert-VITS2 \
      --source sakura-miko-clean-v2 --output sakura-miko-clean-v3-gated
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import sys
from pathlib import Path

import numpy as np
import soundfile as sf

SCRIPT_DIR = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("clips", SCRIPT_DIR / "select-voice-training-clips.py")
clips = importlib.util.module_from_spec(_spec)
# dataclassの解決にsys.modulesへの登録が必要
sys.modules["clips"] = clips
_spec.loader.exec_module(clips)


def pause_levels(x: np.ndarray, sr: int) -> list[float]:
    """0.25秒以上続く合間の音量（ピーク基準dB、中央値）を返す。冒頭・末尾150msは除く。"""
    peak = float(np.max(np.abs(x))) + 1e-9
    hop = int(sr * 0.02)
    frames = 10 * np.log10(clips.frame_power(x, hop) / peak**2 + 1e-12)
    edge = int(0.15 / 0.02)
    inner = frames[edge : len(frames) - edge]
    levels, index = [], 0
    quiet = inner < -35
    while index < len(quiet):
        if not quiet[index]:
            index += 1
            continue
        end = index
        while end < len(quiet) and quiet[end]:
            end += 1
        if (end - index) * 0.02 >= 0.25:
            levels.append(float(np.median(inner[index:end])))
        index = end
    return levels


def gate(models, x: np.ndarray, sr: int, pad_ms: int, fade_ms: int, attenuation_db: float) -> tuple[np.ndarray, float]:
    import torch

    audio16 = clips.resample(x, sr, 16000).astype(np.float32)
    stamps = models.get_speech_timestamps(
        torch.from_numpy(audio16), models.vad, sampling_rate=16000,
        min_silence_duration_ms=120, speech_pad_ms=pad_ms,
    )
    keep = np.zeros(len(x), dtype=np.float64)
    for stamp in stamps:
        keep[int(stamp["start"] * sr / 16000) : int(stamp["end"] * sr / 16000)] = 1.0
    floor = 10 ** (-attenuation_db / 20)
    gain = floor + (1.0 - floor) * keep
    # 急な切り替えはクリックになるため、fade_msの移動平均で境界をなだらかにする。
    width = max(1, int(sr * fade_ms / 1000))
    gain = np.convolve(gain, np.ones(width) / width, mode="same")
    # 平滑化で声の区間の端が削れないよう、発話区間内は必ず1にする。
    gain = np.maximum(gain, keep)
    return x * gain, float(1.0 - keep.mean())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--sbv2-root", required=True)
    parser.add_argument("--source", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--pad-ms", type=int, default=60, help="発話区間の前後に残す余白")
    parser.add_argument("--fade-ms", type=int, default=20)
    parser.add_argument("--attenuation-db", type=float, default=40.0, help="合間を下げる量")
    args = parser.parse_args()

    data = Path(args.sbv2_root).resolve() / "Data"
    source, output = data / args.source, data / args.output
    if output.exists():
        raise SystemExit(f"出力先が既にあるため停止します（上書きしない）: {output}")
    entries = clips.read_candidates(source)
    models = clips.load_models("cpu", vad_only=True)

    before, after, gated_ratio, lines = [], [], [], []
    for file_name, _speaker, language, text in entries:
        src = source / "raw" / file_name
        audio, sr = sf.read(src, dtype="float64", always_2d=True)
        info = sf.info(src)
        x = audio.mean(axis=1)
        y, ratio = gate(models, x, sr, args.pad_ms, args.fade_ms, args.attenuation_db)
        dst = output / "raw" / file_name
        dst.parent.mkdir(parents=True, exist_ok=True)
        sf.write(dst, y.astype(np.float32), sr, subtype=info.subtype)
        before += pause_levels(x, sr)
        after += pause_levels(y, sr)
        gated_ratio.append(ratio)
        lines.append(f"{file_name}|{args.output}|{language}|{text}")
    (output / "esd.list").write_text("\n".join(lines) + "\n", encoding="utf-8")

    def summary(values: list[float]) -> dict:
        arr = np.array(values) if values else np.array([0.0])
        return {"count": len(values), "median_db": round(float(np.median(arr)), 1),
                "louder_than_-48db": round(float(np.mean(arr > -48)), 3)}

    report = {
        "source": args.source, "output": args.output, "clips": len(entries),
        "pad_ms": args.pad_ms, "fade_ms": args.fade_ms, "attenuation_db": args.attenuation_db,
        "gated_ratio_median": round(float(np.median(gated_ratio)), 3),
        "pauses_before": summary(before), "pauses_after": summary(after),
    }
    (output / "pause-cleaning.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
