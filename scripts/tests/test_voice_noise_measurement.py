"""既存SBV2環境のCPUだけで、無音区間の測定を回帰検査する。"""
import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np
import soundfile as sf
from scipy.signal import medfilt, welch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
SPEC = importlib.util.spec_from_file_location("voice_qa", ROOT / "scripts/check-synthesized-voice.py")
QA = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(QA)


def tonal_count(psd, frequencies):
    band = (frequencies > 60) & (frequencies < 12000)
    level = 10 * np.log10(psd[band] + 1e-20)
    return int(np.sum((level - medfilt(level, 51) > 12) & (level > -110)))


class VoiceNoiseMeasurementTests(unittest.TestCase):
    def test_separate_psd_avoids_artificial_boundary(self):
        """位相の異なる離れた無音候補を連結せず、区間別PSDで測る。"""
        sr = 16000
        audio = np.zeros(sr)
        speech = np.ones(sr, dtype=bool)
        speech[:4000] = False
        speech[8000:12000] = False
        audio[:4000] = .01 * np.sin(2 * np.pi * 1000 * np.arange(4000) / sr)
        audio[8000:12000] = .01 * np.sin(2 * np.pi * 1000 * np.arange(4000) / sr + .9)
        (ROOT / "temp").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="voice-noise-test-", dir=ROOT / "temp") as temporary:
            wav = Path(temporary) / "two-gaps.wav"
            sf.write(wav, audio, sr, subtype="FLOAT")
            audio, actual_sr = sf.read(wav)
            self.assertEqual(actual_sr, sr)
        result = QA.measure_gaps(audio, sr, speech, speech)
        self.assertEqual(result["noise_measured_regions"], [
            {"start_seconds": 0.0, "end_seconds": .25},
            {"start_seconds": .5, "end_seconds": .75},
        ])
        self.assertTrue(result["noise_measurement_valid"])
        self.assertAlmostEqual(result["noise_floor_dbfs"], -43.01, places=2)
        frequencies, first = welch(audio[:4000], sr, nperseg=4000)
        _, second = welch(audio[8000:12000], sr, nperseg=4000)
        expected = tonal_count((first + second) / 2, frequencies)
        self.assertEqual(result["tonal_peaks"], expected)
        old_frequencies, joined = welch(audio[~speech], sr, nperseg=4096)
        self.assertNotEqual(tonal_count(joined, old_frequencies), expected)

    def test_short_gaps_are_unmeasurable(self):
        """短い区間の合算や発声の最小音量を無音として代用しない。"""
        sr = 16000
        audio = .1 * np.sin(2 * np.pi * 300 * np.arange(sr) / sr)
        speech = np.ones(sr, dtype=bool)
        speech[:800] = False
        speech[4000:4800] = False
        result = QA.measure_gaps(audio, sr, speech, speech)
        self.assertFalse(result["noise_measurement_valid"])
        self.assertEqual(result["gap_seconds"], .1)
        self.assertEqual(result["noise_measured_regions"], [])
        self.assertIsNone(result["noise_floor_dbfs"])
        self.assertIsNone(result["tonal_peaks"])
        self.assertIn("noise_measurement_error", result)

    def test_silence_is_measurable(self):
        """連続した完全無音は測定可能で、電子音を検出しない。"""
        sr = 16000
        speech = np.zeros(sr, dtype=bool)
        result = QA.measure_gaps(np.zeros(sr), sr, speech, speech)
        self.assertTrue(result["noise_measurement_valid"])
        self.assertEqual(result["noise_floor_dbfs"], -120)
        self.assertEqual(result["tonal_peaks"], 0)


if __name__ == "__main__":
    unittest.main()
