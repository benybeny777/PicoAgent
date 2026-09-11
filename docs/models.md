# 管理導入する画像・動画モデル

PicoAgentはモデルを再配布せず、利用者が追加機能を選んだ後に固定リビジョンを公式配布元から直接取得し、サイズとSHA-256を検査する。WanとLTXは別項目であり、選ばなかった側を取得しない。

| 用途 | 固定モデル | 取得量 | 採用理由・条件 |
|---|---|---:|---|
| 画像編集・キャラ補完 | Qwen-Image-Edit-2511 BF16 | キャラ作成基盤約77GBの内数 | Apache-2.0。入力画像を保った局所編集に使い、純粋なテキスト画像生成とは分ける |
| 標準動画 | Wan 2.2 TI2V 5B FP16 | 約18.1GB | Apache-2.0。ComfyUI公式が8GB VRAM向けと案内し、テキスト／画像から動画を生成できる |
| 軽量LTX動画 | LTX-Video 0.9.8 2B Distilled FP8 | 約14.3GB | LTX-Video Open Weights License。低VRAM用途の旧世代候補。公式は13B distilledより品質低下と説明するが、8GBでの実動作・画質は未検証 |
| 高VRAM候補（未対応） | LTX 2.5 22B | 取得しない | 最新世代。公式Desktopのローカル条件は16GB以上、モデル本体のシステム要件は32GB以上。経路ごとの条件が異なり、8GB環境では既定取得しない |

2026-09-11に公式一次資料を確認した。LTX 0.9.8は8GB環境で評価する候補であり、8GB動作保証ではない。Wanとの同条件画質比較も未実施。現在の管理ツールは入力必須のQwen画像編集だけで、純粋なtext-to-imageとLTX 2.3／最新2.5は未対応である。旧モデルの導入でこれらを代替完了と扱わない。LTX 2.5の低メモリ経路まで不可能と断定せず、対応GPU・固定ワークフロー・ライセンス条件を確認できる環境で別途評価する。

固定URL、リビジョン、SHA-256、サイズの実行正本は`character-maker/sidecar/media/setup_models.py`と`character-maker/installer/setup-runtime-models.py`に置く。ライセンスや配布内容は更新時に公式モデルカードを再確認する。

参照先:

- [ComfyUI公式 Qwen Image Edit](https://docs.comfy.org/tutorials/image/qwen/qwen-image-edit)
- [ComfyUI公式 Wan 2.2](https://docs.comfy.org/tutorials/video/wan/wan2_2)
- [Wan 2.2公式リポジトリ](https://github.com/Wan-Video/Wan2.2)
- [LTX-Video公式モデル](https://huggingface.co/Lightricks/LTX-Video)
- [LTX 2.5公式モデル](https://huggingface.co/Lightricks/LTX-2.5)
- [LTX Desktop公式の実行条件](https://github.com/Lightricks/LTX-Desktop)
- [LTX 2.5モデル本体のシステム要件](https://docs.ltx.io/open-source-model/getting-started/system-requirements)
- [LTX 2.xライセンス](https://github.com/Lightricks/LTX-2/blob/main/LICENSE.md)
