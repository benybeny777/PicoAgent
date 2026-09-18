#!/usr/bin/env python3
"""学習済み音声モデルの合成結果を検査し、クリップ・定常ノイズ・電子音・境界ノイズ・発音崩れ・声の混入を判定する。

Style-Bert-VITS2の `.venv` のPythonで実行する。測定の共通処理は `select-voice-training-clips.py` を読み込んで使う。

  # Style-Bert-VITS2のチェックポイントを直接合成して検査（学習途中の比較に使う）
  python scripts/check-synthesized-voice.py sbv2 \
      --sbv2-root temp/voice-models/Style-Bert-VITS2 --model-dir <model_assets/ID> --checkpoint <*.safetensors> \
      --reference temp/voice-models/clean-v2/<ID>/clip-metrics.jsonl --out-dir temp/voice-models/qa/<名前>

  # 起動中のAivisSpeech Engineで、PicoAgentと同じvolumeScale上限0.72を適用して合成・検査
  python scripts/check-synthesized-voice.py aivis --endpoint http://127.0.0.1:11436 --speaker <style ID> \
      --reference ... --out-dir ...

  # 既存WAVだけを検査
  python scripts/check-synthesized-voice.py files --wav a.wav b.wav --texts-file texts.txt --reference ... --out-dir ...

1件でも基準を外れた場合は終了コード1を返す。
`sbv2`モードはSBV2がピークを0dBへ正規化して書き出すため、クリップと音量は判定せず、
ノイズ床・SNR・電子音・境界・破裂音・発音・話者類似度だけで比較する。クリップと音量は`aivis`モードで判定する。
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import math
import os
import sys
import urllib.parse
import urllib.request
from pathlib import Path

import numpy as np
import soundfile as sf

AIVIS_MAX_VOLUME_SCALE = 0.72  # src-tauri/src/commands.rs の limit_aivis_volume と揃える
DEFAULT_TEXTS = [
    "おはようございます。今日も一緒に、楽しくお話ししていこうね。",
    "明日の東京の天気は晴れのち曇り、最高気温は二十八度の予報です。",
    "パソコンのファイルを整理して、デスクトップをすっきりさせました。",
    "えっ、本当に？それはすごいね、ぜひ詳しく聞かせてほしいな。",
    "午後三時にリマインダーを設定しました。忘れないように声をかけるね。",
    "ぴょんぴょん跳ねるカエルと、ぷかぷか浮かぶ風船を見に行こう。",
]
# 読点・疑問符・感嘆符・言いよどみの間に出るノイズを見るための追加文（--text-set pauses）
PAUSE_TEXTS = [
    "うーん、ちょっと待ってね。今、確認してみるから。",
    "ねえ、聞いて！昨日、すごく面白いことがあったんだよ。",
    "それで、どうしたの？そっか、大変だったね。",
    "午後三時に会議があります。資料は、もう準備できていますか？",
]

_spec = importlib.util.spec_from_file_location(
    "voice_clip_metrics", Path(__file__).with_name("select-voice-training-clips.py")
)
clips = importlib.util.module_from_spec(_spec)
# dataclassは定義元モジュールをsys.modulesから引くため、実行前に登録する。
sys.modules[_spec.name] = clips
_spec.loader.exec_module(clips)


class Limits:
    max_peak_dbfs = -0.5          # 0dB付近へ達したらクリップ扱い
    min_lufs = -28.0              # 音量上限0.72の下でも聞き取れる大きさ
    max_noise_floor_dbfs = -55.0  # 発話前後・語間の定常ノイズ（生の出力レベル）
    min_snr_db = 30.0
    max_edge_dbfs = -45.0         # 冒頭・末尾30msのレベル
    max_tonal_peaks = 6           # 無音区間の狭帯域ピーク（電子音・ハム）
    max_plosive_frames = 2
    max_kana_cer = 0.15
    min_speaker_similarity = 0.60


def reference_centroid(path: Path) -> np.ndarray:
    rows = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    decisions = path.with_name("clip-decisions.jsonl")
    accepted = None
    if decisions.exists():
        decision_rows = [
            json.loads(line)
            for line in decisions.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]
        accepted = {row["file"] for row in decision_rows if not row.get("reasons")}
        if not accepted:
            raise SystemExit(f"話者基準に使える採用断片がありません: {decisions}")
    vectors = [
        r["_embedding"]
        for r in rows
        if r.get("_embedding") and (accepted is None or r["file"] in accepted)
    ]
    if not vectors:
        raise SystemExit(f"話者基準の埋め込みがありません: {path}")
    matrix = np.asarray(vectors, dtype=np.float64)
    if matrix.ndim != 2 or not np.isfinite(matrix).all():
        raise SystemExit(f"話者基準の埋め込みが不正です: {path}")
    centroid = np.mean(matrix, axis=0)
    norm = float(np.linalg.norm(centroid))
    if not math.isfinite(norm) or norm <= 1e-12:
        raise SystemExit(f"話者基準の重心を計算できません: {path}")
    return centroid / norm


def inspect_wav(models, wav_path: Path, text: str, centroid: np.ndarray | None, judge_level: bool = True) -> dict:
    import torch
    from scipy.signal import butter, medfilt, resample_poly, sosfilt, welch

    x, sr = clips.load_mono(wav_path)
    result: dict = {"wav": str(wav_path), "text": text, "seconds": round(len(x) / sr, 3)}
    result["peak_dbfs"] = round(20 * math.log10(max(float(np.abs(x).max()), 1e-9)), 2)
    oversampled = resample_poly(x, 4, 1)
    result["true_peak_dbfs"] = round(20 * math.log10(max(float(np.abs(oversampled).max()), 1e-9)), 2)
    result["clip_samples"] = int(np.sum(np.abs(x) >= 0.999))
    result["dc_offset"] = round(float(x.mean()), 6)
    import pyloudnorm

    result["lufs"] = round(pyloudnorm.Meter(sr).integrated_loudness(x), 2)

    audio16 = clips.resample(x, sr, 16000).astype(np.float32)
    stamps = models.get_speech_timestamps(
        torch.from_numpy(audio16), models.vad, sampling_rate=16000, min_silence_duration_ms=120, speech_pad_ms=30
    )
    guard = np.zeros(len(x), dtype=bool)
    speech = np.zeros(len(x), dtype=bool)
    pad = int(0.05 * sr)
    for stamp in stamps:
        start, end = int(stamp["start"] * sr / 16000), int(stamp["end"] * sr / 16000)
        speech[start:end] = True
        guard[max(0, start - pad): end + pad] = True
    hop = int(0.02 * sr)
    speech_db = [clips.db(p) for p in clips.frame_power(x[speech], hop)] if speech.any() else [-120.0]
    result["speech_median_dbfs"] = round(float(np.median(speech_db)), 2)
    gap = x[~guard]
    result["gap_seconds"] = round(len(gap) / sr, 3)
    if len(gap) >= int(0.1 * sr):
        result["noise_floor_dbfs"] = round(clips.db(float(np.mean(gap**2))), 2)
        freqs, psd = welch(gap, sr, nperseg=min(4096, len(gap)))
        band = (freqs > 60) & (freqs < 12000)
        level = 10 * np.log10(psd[band] + 1e-20)
        result["tonal_peaks"] = int(np.sum((level - medfilt(level, 51) > 12) & (level > -110)))
    else:
        # 語間がほぼ無い短文は、最も静かなフレームで代用する。
        frames = [clips.db(p) for p in clips.frame_power(x, hop)]
        result["noise_floor_dbfs"] = round(float(np.percentile(frames, 5)), 2)
        result["tonal_peaks"] = 0
    result["snr_db"] = round(result["speech_median_dbfs"] - result["noise_floor_dbfs"], 2)
    edge = int(0.03 * sr)
    result["head_dbfs"] = round(clips.db(float(np.mean(x[:edge] ** 2))), 2)
    result["tail_dbfs"] = round(clips.db(float(np.mean(x[-edge:] ** 2))), 2)

    sos = butter(4, 100, btype="lowpass", fs=sr, output="sos")
    low_db = np.array([clips.db(p) for p in clips.frame_power(sosfilt(sos, x), hop)])
    low_rel = low_db - float(np.median(low_db))
    result["plosive_frames"] = int(np.sum((low_rel > 15.0) & (low_db > result["speech_median_dbfs"] - 10.0)))

    segments, _ = models.whisper.transcribe(
        audio16, language="ja", beam_size=5, condition_on_previous_text=False, vad_filter=False, without_timestamps=True
    )
    recognized = "".join(segment.text for segment in segments).strip()
    result["asr_text"] = recognized
    reference = clips.kana(text)
    result["kana_cer"] = round(clips.edit_distance(reference, clips.kana(recognized)) / max(1, len(reference)), 3)
    if centroid is not None:
        result["speaker_similarity"] = round(float(np.dot(clips.speaker_embedding(models, audio16), centroid)), 4)

    failures = []
    # SBV2のTTSModel.inferは書き出し前にピークを0dBへ正規化するため、sbv2モードではクリップと音量を判定しない。
    if judge_level and (result["clip_samples"] or result["true_peak_dbfs"] > Limits.max_peak_dbfs):
        failures.append("clip")
    if judge_level and result["lufs"] < Limits.min_lufs:
        failures.append("too-quiet")
    if result["noise_floor_dbfs"] > Limits.max_noise_floor_dbfs or result["snr_db"] < Limits.min_snr_db:
        failures.append("steady-noise")
    if result["tonal_peaks"] > Limits.max_tonal_peaks:
        failures.append("tonal-noise")
    if result["head_dbfs"] > Limits.max_edge_dbfs or result["tail_dbfs"] > Limits.max_edge_dbfs:
        failures.append("edge-noise")
    if result["plosive_frames"] > Limits.max_plosive_frames:
        failures.append("plosive")
    if result["kana_cer"] > Limits.max_kana_cer:
        failures.append("pronunciation")
    if centroid is not None and result["speaker_similarity"] < Limits.min_speaker_similarity:
        failures.append("voice-mismatch")
    result["failures"] = failures
    return result


def synthesize_sbv2(args: argparse.Namespace, texts: list[str], out_dir: Path) -> list[Path]:
    sbv2_root = Path(args.sbv2_root).resolve()
    sys.path.insert(0, str(sbv2_root))
    os.chdir(sbv2_root)
    from style_bert_vits2.tts_model import TTSModel

    model_dir = Path(args.model_dir).resolve()
    model = TTSModel(
        model_path=Path(args.checkpoint).resolve(),
        config_path=model_dir / "config.json",
        style_vec_path=model_dir / "style_vectors.npy",
        device=args.device,
    )
    paths = []
    for index, text in enumerate(texts, 1):
        sample_rate, audio = model.infer(text=text, language="JP")
        path = out_dir / f"{index:02d}.wav"
        sf.write(path, audio, sample_rate, subtype="PCM_16")
        paths.append(path)
    model.unload()
    return paths


def synthesize_aivis(args: argparse.Namespace, texts: list[str], out_dir: Path) -> list[Path]:
    paths = []
    for index, text in enumerate(texts, 1):
        query_url = f"{args.endpoint}/audio_query?" + urllib.parse.urlencode({"text": text, "speaker": args.speaker})
        with urllib.request.urlopen(urllib.request.Request(query_url, method="POST"), timeout=180) as response:
            query = json.loads(response.read())
        query["volumeScale"] = min(max(float(query.get("volumeScale", 1.0)), 0.0), AIVIS_MAX_VOLUME_SCALE)
        request = urllib.request.Request(
            f"{args.endpoint}/synthesis?" + urllib.parse.urlencode({"speaker": args.speaker}),
            data=json.dumps(query).encode("utf-8"),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(request, timeout=180) as response:
            path = out_dir / f"{index:02d}.wav"
            path.write_bytes(response.read())
        paths.append(path)
    return paths


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="mode", required=True)
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--out-dir", required=True)
    common.add_argument("--reference", help="選別スクリプトのclip-metrics.jsonl。話者類似度の基準にする")
    common.add_argument("--texts-file", help="1行1文。省略時は既定の6文")
    common.add_argument("--device", default="cuda")
    common.add_argument("--text-set", choices=["default", "pauses"], default="default",
                        help="pausesは既定6文に間を含む4文を加える")
    common.add_argument("--repeat", type=int, default=1,
                        help="合成は毎回ゆらぐため、文の並び全体をこの回数だけ繰り返して合成する")
    sbv2 = sub.add_parser("sbv2", parents=[common])
    sbv2.add_argument("--sbv2-root", required=True)
    sbv2.add_argument("--model-dir", required=True)
    sbv2.add_argument("--checkpoint", required=True)
    aivis = sub.add_parser("aivis", parents=[common])
    aivis.add_argument("--endpoint", default="http://127.0.0.1:11436")
    aivis.add_argument("--speaker", required=True)
    files = sub.add_parser("files", parents=[common])
    files.add_argument("--wav", nargs="+", required=True)
    args = parser.parse_args()

    out_dir = Path(args.out_dir).resolve()
    incoming = out_dir.with_name(f"{out_dir.name}.incoming")
    if out_dir.exists():
        raise SystemExit(f"出力先が既にあるため停止します（検査結果を上書きしない）: {out_dir}")
    if incoming.exists():
        raise SystemExit(f"前回の未完了出力があるため停止します: {incoming}")
    incoming.mkdir(parents=True)
    texts = DEFAULT_TEXTS + (PAUSE_TEXTS if args.text_set == "pauses" else [])
    if args.texts_file:
        texts = [t.strip() for t in Path(args.texts_file).read_text(encoding="utf-8").splitlines() if t.strip()]
    if args.repeat < 1:
        raise SystemExit("--repeat は1以上にしてください")
    texts = texts * args.repeat
    reference = Path(args.reference).resolve() if args.reference else None

    if args.mode == "sbv2":
        wavs = synthesize_sbv2(args, texts, incoming)
    elif args.mode == "aivis":
        wavs = synthesize_aivis(args, texts, incoming)
    else:
        wavs = [Path(p).resolve() for p in args.wav]
        if len(wavs) != len(texts):
            raise SystemExit(f"WAVと文の数が一致しません: {len(wavs)} != {len(texts)}")

    models = clips.load_models(args.device)
    centroid = reference_centroid(reference) if reference else None
    judge_level = args.mode != "sbv2"
    results = [inspect_wav(models, wav, text, centroid, judge_level) for wav, text in zip(wavs, texts)]
    if args.mode != "files":
        for result in results:
            result["wav"] = str(out_dir / Path(result["wav"]).name)
    summary = {
        "mode": args.mode,
        "level_judged": judge_level,
        "passed": all(not r["failures"] for r in results),
        "max_true_peak_dbfs": max(r["true_peak_dbfs"] for r in results),
        "mean_lufs": round(float(np.mean([r["lufs"] for r in results])), 2),
        "max_noise_floor_dbfs": max(r["noise_floor_dbfs"] for r in results),
        "min_snr_db": min(r["snr_db"] for r in results),
        "max_kana_cer": max(r["kana_cer"] for r in results),
        "min_speaker_similarity": min((r.get("speaker_similarity", 1.0) for r in results)),
        "failures": {Path(r["wav"]).name: r["failures"] for r in results if r["failures"]},
        "clips": results,
    }
    (incoming / "qa-report.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
    incoming.replace(out_dir)
    print(json.dumps({k: v for k, v in summary.items() if k != "clips"}, ensure_ascii=False, indent=2))
    return 0 if summary["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
