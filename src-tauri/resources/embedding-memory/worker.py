"""長期メモリ検索専用。標準入力と標準出力のJSON行だけで本体と通信する。"""
import contextlib
import hashlib
import json
import os
import sys
from pathlib import Path

REVISION = "914f7f89142e33e77833254d9c9b90c3cef7303b"
DIMENSIONS = 768


def fingerprint(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def save_cache(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    stage = path.with_suffix(".incoming")
    try:
        with stage.open("w", encoding="utf-8") as stream:
            json.dump(value, stream, ensure_ascii=False, allow_nan=False)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(stage, path)
    finally:
        stage.unlink(missing_ok=True)


class Search:
    def __init__(self, root, requested, cache):
        import torch
        from sentence_transformers import SentenceTransformer

        self.torch = torch
        if requested not in ("gpu", "cpu"):
            raise ValueError("記憶検索の実行先はgpuまたはcpuを指定してください")
        installed = json.loads((root / "install.json").read_text(encoding="utf-8"))
        if requested == "gpu" and installed.get("backend") == "cuda" and not torch.cuda.is_available():
            raise RuntimeError("導入済みCUDA環境でGPUを利用できません。ドライバーを確認するかCPUを選んでください")
        if requested == "gpu" and installed.get("backend") == "mps" and not torch.backends.mps.is_available():
            raise RuntimeError("導入済みMPS環境でGPUを利用できません。CPUを選んでください")
        if requested == "gpu" and torch.cuda.is_available():
            self.device = "cuda"
        elif requested == "gpu" and torch.backends.mps.is_available():
            self.device = "mps"
        else:
            # フォールバック許可: GPU非対応構成は取得前にも判定し、CPU版を標準利用する。
            self.device = "cpu"
        torch.set_num_threads(min(10, torch.get_num_threads()))
        self.model = SentenceTransformer(
            str(root / "model"), device=self.device, local_files_only=True,
            config_kwargs={"vision_config": None, "audio_config": None},
            model_kwargs={"torch_dtype": torch.bfloat16 if self.device == "cuda" else torch.float32},
        )
        self.cache_path = cache
        self.cached = self.load_cache()

    def load_cache(self):
        if not self.cache_path.exists():
            return {}
        try:
            data = json.loads(self.cache_path.read_text(encoding="utf-8"))
            if not isinstance(data, dict):
                raise ValueError("記憶索引の形式が不正です")
            if data.get("revision") != REVISION:
                return {}
            entries = data["vectors"]
            if not isinstance(entries, dict):
                raise ValueError("記憶ベクトルの形式が不正です")
            for key, vector in entries.items():
                self.validate(vector)
                if len(key) != 64:
                    raise ValueError("記憶ベクトルのハッシュが不正です")
            return entries
        except (ValueError, KeyError, TypeError):
            # フォールバック許可: 再生成可能な索引だけを退避し、記憶原本には触れない。
            saved = self.cache_path.with_suffix(".corrupt")
            os.replace(self.cache_path, saved)
            print("記憶検索の索引を退避し、原本から再計算します", file=sys.stderr, flush=True)
            return {}

    def validate(self, vector):
        value = self.torch.tensor(vector, dtype=self.torch.float32)
        if value.shape != (DIMENSIONS,) or not self.torch.isfinite(value).all():
            raise ValueError("記憶ベクトルの次元または数値が不正です")
        if abs(float(value.norm()) - 1.0) > 0.02:
            raise ValueError("記憶ベクトルが正規化されていません")

    def encode(self, texts, prompt):
        values = self.model.encode(texts, prompt_name=prompt, normalize_embeddings=True,
                                   convert_to_tensor=True, show_progress_bar=False)
        if not self.torch.isfinite(values).all():
            raise ValueError("記憶検索に非有限値が含まれます")
        return values.float()

    def search(self, body):
        docs = body["documents"]
        query = body["query"]
        if not isinstance(docs, list) or len(docs) > 200 or not all(isinstance(x, str) and len(x) <= 1000 for x in docs):
            raise ValueError("記憶の件数または本文が不正です")
        if not isinstance(query, str) or len(query) > 32768:
            raise ValueError("記憶検索の質問が長すぎます")
        if not docs:
            return {"indices": [], "scores": [], "device": self.device}
        hashes = [fingerprint(text) for text in docs]
        missing = dict((key, text) for key, text in zip(hashes, docs) if key not in self.cached)
        if missing:
            rows = self.encode(list(missing.values()), "Document").cpu().tolist()
            self.cached.update(zip(missing, rows))
        current = {key: self.cached[key] for key in hashes}
        if missing or set(current) != set(self.cached) or not self.cache_path.exists():
            save_cache(self.cache_path, {"revision": REVISION, "vectors": current})
            self.cache_path.with_suffix(".corrupt").unlink(missing_ok=True)
        self.cached = current
        matrix = self.torch.tensor([current[key] for key in hashes], device=self.device)
        query_vector = self.encode(query, "SearchQuery")
        scores = matrix @ query_vector
        indices = self.torch.argsort(scores, descending=True, stable=True).cpu().tolist()
        return {"indices": indices, "scores": scores.cpu().tolist(), "device": self.device}


def send(value):
    print(json.dumps(value, ensure_ascii=False, allow_nan=False), flush=True)


def main():
    root, device, cache = Path(sys.argv[1]), sys.argv[2], Path(sys.argv[3])
    os.environ["HF_HUB_OFFLINE"] = "1"
    os.environ["TRANSFORMERS_OFFLINE"] = "1"
    os.environ["HF_HOME"] = str(root / "cache")
    if device == "cpu":
        os.environ["CUDA_VISIBLE_DEVICES"] = "-1"
    try:
        # ライブラリの進捗は診断ログへ送り、JSON行の通信を壊さない。
        with contextlib.redirect_stdout(sys.stderr):
            search = Search(root, device, cache)
        send({"ready": True, "device": search.device})
    except Exception as error:
        send({"error": f"記憶検索モデルを起動できません: {error}"})
        return 1
    for line in sys.stdin:
        try:
            body = json.loads(line)
            with contextlib.redirect_stdout(sys.stderr):
                result = search.search(body)
            send(result)
        except Exception as error:
            send({"error": f"記憶検索に失敗しました: {error}"})
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
