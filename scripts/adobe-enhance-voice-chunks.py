#!/usr/bin/env python3
"""選別済み学習データをAdobe Podcast Enhanceで補正するため、送信用チャンクへ連結し、補正版を元の断片へ切り戻す。

Style-Bert-VITS2の `.venv` のPythonで実行する。Adobe Podcast Enhance（Web版）への送受信はブラウザ操作で行い、
このスクリプトはファイルの準備と取り込みだけを担当する。

  # 1. 断片を1秒の無音をはさんで連結し、1本10MB未満のWAVとmanifest.jsonを作る
  python scripts/adobe-enhance-voice-chunks.py pack --source <Data/元> --out <送信用フォルダ>

  # 2. Adobe Podcast Enhanceで chunk-XX.wav を1本ずつ補正し、補正版を別フォルダへ chunk-XX.wav として置く
  #    （manifest.jsonも同じフォルダへ複製する）

  # 3. 補正版を検証して元の断片へ切り戻し、-23LUFS・ピーク-1dBFS以下へそろえた別名データセットを作る
  python scripts/adobe-enhance-voice-chunks.py unpack --source <Data/元> --enhanced <補正版フォルダ> --output <Data/新>

注意:
- FLACはAdobe側の受付一覧にあるが「未対応」で拒否されたためWAVで送る（2026-09-15確認）。
- 補正版は48kHzモノラルで返り、長さは元と一致する。unpackは全チャンクの長さを実データで照合し、1本でも違えば停止する。
- ChromeはデータがそろってもGUID名の`.tmp`のまま数分保存を確定しないことがある。`.tmp`を使う場合は、実データを
  最後まで読んで長さが元と一致したものだけを複製する（unpackの照合でも検出する）。
- 出力先が既にあれば停止する（既存データセットを上書きしない）。
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import sys
from pathlib import Path

import numpy as np
import soundfile as sf

GAP_SECONDS = 1.0
MAX_BYTES = 9_000_000
TARGET_LUFS = -23.0
PEAK_CEILING_DBFS = -1.0


def entries(source: Path):
    for line in (source / "esd.list").read_text(encoding="utf-8").splitlines():
        if line.strip():
            file_name, _speaker, language, text = line.split("|", 3)
            yield file_name, language, text


def pack(source: Path, out: Path) -> None:
    if out.exists():
        raise SystemExit(f"出力先が既にあるため停止します: {out}")
    out.mkdir(parents=True)
    manifest, chunk, cursor, index, sample_rate = [], [], 0, 1, None

    def flush() -> None:
        nonlocal chunk, cursor, index
        if not chunk:
            return
        path = out / f"chunk-{index:02d}.wav"
        sf.write(path, np.concatenate(chunk), sample_rate, subtype="PCM_16")
        if path.stat().st_size > MAX_BYTES:
            raise SystemExit(f"1本の上限を超えました: {path}")
        chunk, cursor, index = [], 0, index + 1

    for file_name, language, text in entries(source):
        x, sr = sf.read(source / "raw" / file_name, dtype="float32")
        sample_rate = sample_rate or sr
        if sr != sample_rate or x.ndim != 1:
            raise SystemExit(f"サンプルレートまたはチャンネル数が揃っていません: {file_name}")
        gap = np.zeros(int(GAP_SECONDS * sr), dtype=np.float32)
        # 16bit PCMは1サンプル2バイト。無音分の余裕を見て区切る。
        if cursor and (cursor + len(x) + len(gap) * 2) * 2 > MAX_BYTES:
            chunk.append(gap)
            flush()
        start = cursor + len(gap)
        chunk += [gap, x]
        cursor = start + len(x)
        manifest.append({"chunk": index, "file": file_name, "start_s": start / sr, "end_s": cursor / sr,
                         "language": language, "text": text})
    if chunk:
        chunk.append(np.zeros(int(GAP_SECONDS * sample_rate), dtype=np.float32))
    flush()
    (out / "manifest.json").write_text(
        json.dumps({"sample_rate": sample_rate, "source": source.name, "clips": manifest}, ensure_ascii=False, indent=1),
        encoding="utf-8")
    print(f"チャンク数 {index - 1} / 断片数 {len(manifest)} / 出力 {out}")


def load_pause_levels():
    spec = importlib.util.spec_from_file_location("cleaner", Path(__file__).resolve().parent / "clean-voice-training-pauses.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules["cleaner"] = module
    spec.loader.exec_module(module)
    return module.pause_levels


def unpack(source: Path, enhanced: Path, output: Path) -> None:
    import pyloudnorm

    if output.exists():
        raise SystemExit(f"出力先が既にあるため停止します: {output}")
    manifest = json.loads((enhanced / "manifest.json").read_text(encoding="utf-8"))
    originals = sorted({clip["chunk"] for clip in manifest["clips"]})
    # 先に全チャンクを検証し、途中で止まって半端なデータセットを残さない。
    chunks = {}
    for number in originals:
        path = enhanced / f"chunk-{number:02d}.wav"
        if not path.exists():
            raise SystemExit(f"補正版がありません: {path}")
        audio, sr = sf.read(path, dtype="float64", always_2d=True)
        # 送信チャンクの長さは「最後の断片の終端」か、末尾に無音1秒を足した長さ。
        # 補正版はAdobeで長さが変わらないため、この範囲外なら書き込み途中や取り違えとみなして止める。
        last_end = max(c["end_s"] for c in manifest["clips"] if c["chunk"] == number)
        extra = len(audio) / sr - last_end
        if not -0.01 < extra < GAP_SECONDS + 0.01:
            raise SystemExit(f"補正版の長さが送信チャンクと合いません（書き込み途中の可能性）: {path} {len(audio) / sr:.3f}s")
        chunks[number] = (audio.mean(axis=1), sr)

    pause_levels = load_pause_levels()
    lines, loudness_after, peaks, levels = [], [], [], []
    for clip in manifest["clips"]:
        x, sr = chunks[clip["chunk"]]
        segment = x[int(round(clip["start_s"] * sr)): int(round(clip["end_s"] * sr))]
        segment = segment - segment.mean()
        meter = pyloudnorm.Meter(sr)
        gain = 10 ** ((TARGET_LUFS - meter.integrated_loudness(segment)) / 20)
        peak = float(np.max(np.abs(segment * gain)))
        ceiling = 10 ** (PEAK_CEILING_DBFS / 20)
        if peak > ceiling:
            gain *= ceiling / peak
        y = segment * gain
        path = output / "raw" / clip["file"]
        path.parent.mkdir(parents=True, exist_ok=True)
        sf.write(path, y.astype(np.float32), sr, subtype="PCM_16")
        loudness_after.append(meter.integrated_loudness(y))
        peaks.append(20 * np.log10(max(float(np.max(np.abs(y))), 1e-9)))
        levels += pause_levels(y, sr)
        lines.append(f"{clip['file']}|{output.name}|{clip['language']}|{clip['text']}")
    (output / "esd.list").write_text("\n".join(lines) + "\n", encoding="utf-8")
    lv = np.array(levels) if levels else np.array([0.0])
    report = {
        "source": source.name, "output": output.name, "clips": len(lines),
        "lufs_median": round(float(np.median(loudness_after)), 1), "peak_max_dbfs": round(float(max(peaks)), 2),
        "pauses": len(levels), "pause_median_db": round(float(np.median(lv)), 1),
        "pauses_louder_than_-48db": round(float(np.mean(lv > -48)), 3),
    }
    (output / "adobe-enhance.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="mode", required=True)
    p = sub.add_parser("pack")
    p.add_argument("--source", required=True)
    p.add_argument("--out", required=True)
    u = sub.add_parser("unpack")
    u.add_argument("--source", required=True)
    u.add_argument("--enhanced", required=True)
    u.add_argument("--output", required=True)
    args = parser.parse_args()
    if args.mode == "pack":
        pack(Path(args.source).resolve(), Path(args.out).resolve())
    else:
        unpack(Path(args.source).resolve(), Path(args.enhanced).resolve(), Path(args.output).resolve())
    return 0


if __name__ == "__main__":
    sys.exit(main())
