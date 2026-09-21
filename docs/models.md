# 管理導入する画像・動画モデル

PicoAgentはモデルを再配布せず、利用者が追加機能を選んだ後に固定リビジョンを公式配布元から直接取得し、サイズとSHA-256を検査する。WanとLTXは別項目であり、選ばなかった側を取得しない。

| 用途 | 固定モデル | 取得量 | 採用理由・条件 |
|---|---|---:|---|
| 画像編集・キャラ補完 | Qwen-Image-Edit-2511 BF16 | キャラ作成基盤約77GBの内数 | Apache-2.0。入力画像を保った局所編集に使い、純粋なテキスト画像生成とは分ける |
| 標準動画 | Wan 2.2 TI2V 5B FP16 | 約18.1GB | Apache-2.0。ComfyUI公式が8GB VRAM向けと案内し、テキスト／画像から動画を生成できる |
| 軽量LTX動画 | LTX-Video 0.9.8 2B Distilled FP8 | 約14.3GB | LTX-Video Open Weights License。低VRAM用途の旧世代候補。公式は13B distilledより品質低下と説明するが、8GBでの実動作・画質は未検証 |
| 文章から画像（任意導入） | Krea 2 Turbo NVFP4 | 約13.2GB | Krea 2 Community License。Windows x64・NVIDIA演算能力12.0以上・VRAM 8GB以上・RAM 32GB以上で評価する。編集はQwenを使う |
| 音声付き動画（任意導入） | LTX 2.5 22B Distilled NVFP4 | 約35.9GB | LTX 2.x Community License。Kreaと同じ評価条件。INT8 Gemma 4、Conv VAE、1段階8ステップ。公式DesktopやBF16本体の必要量とは別経路 |

2026-09-21に新モデルの公式配布情報を確認した。新しいNVFP4経路も8GB動作保証ではなく、速度・画質・長時間安定性の受入は別途必要である。Qwenは参照画像なしの生成も維持する（2026-09-13のRTX 5060 Laptop 8GB・RAM 34GBで512×512・20ステップは約9分）。旧LTXとWanの同条件画質比較も未実施。LTX 2.3は追加しない。

固定URL、リビジョン、SHA-256、サイズの実行正本は`character-maker/sidecar/media/setup_models.py`、同じ場所の`model_catalog.py`、`character-maker/installer/setup-runtime-models.py`に置く。ライセンスや配布内容は更新時に公式モデルカードを再確認する。

## 新しいNVFP4経路の導入条件と制限

設定の追加機能でモデルごとに提供元リンクを読み、本人が同意欄を選んだ場合だけ取得する。起動、ビルド、通常の生成要求では取得しない。利用条件の確認版は2026-09-21。Kreaは関連会社を含む年間売上100万米ドル未満、LTXは1,000万米ドル未満がコミュニティ版の商用利用の目安であり、該当額以上は提供元の別契約条件を確認する。金額だけで全条件を満たすわけではない。モデル・派生モデルの提供には元ライセンス、必要な告知、受領者への条件提示が必要であり、生成物の扱いとは区別する。生成物にも用途・商用利用・権利などの条件がある。生成前の入力と公開前の出力を人が確認し、モデルの安全策を無効化しない。生成物を無人で外部配信する用途はこの確認経路の対象外とする。

Hugging Face側の条件同意はPicoAgentの同意とは別である。認証が必要な場合は同意した本人のアカウントで`hf auth login`を実行する。標準保存先、`HF_HOME`、`HF_TOKEN_PATH`、`HF_TOKEN`を取得時だけ参照し、秘密値はログやコマンド引数に出さず、別ホストへの転送時にも渡さない。認証不足はモデルとランタイムの大容量取得前に停止する。

新経路は共通Python 3.12本体を再利用し、`bundled/media-runtime/venv`とComfyUI 0.37.0を分離する。既存ComfyUI 0.34.0・Qwen・Wan・旧LTXを上書きしない。専用環境はPyTorch 2.11 CUDA 13で、固定依存は`sidecar/requirements-media.lock`で管理する。現段階では既存管理PythonがあるWindows x64だけが導入対象であり、クリーン環境のPython初回導入、Linux、macOS、AMDへのNVFP4対応は未完了である。未対応環境向けに代替の大容量モデルを取得しない。

VRAM 16GB未満では`--lowvram --reserve-vram 1 --cache-none`とVAEのタイル復号を使う。Kreaの4Bテキスト処理はComfyのGPU自動管理に任せ、LTXの大きい12BだけCPUへ配置する。CPU退避はVRAMを減らす代わりに転送・CPU計算で遅くなる場合がある。LTXは軽いConv VAEを使い、公式の2段階高解像度化と重いDiffVAEは省くため、顔・文字・細部の画質が劣る可能性がある。補間による見かけの拡大は行わず、参照画像が生成寸法より小さい場合も拒否する。高品質構成まで検証済みとは扱わない。

2026-09-21のRTX 5060 Laptop 8GB・RAM約34GBでは、Krea 512×512・8ステップ・同じプロンプトとシードの単発生成が成功した。テキストCPU配置の初回は起動込み334秒、GPU自動管理の次回は47秒で、両方の猫画像を目視確認した。コールド起動・キャッシュ・他処理の条件が揃っていないため、倍率や一般的な速度保証には使わない。連続生成、他GPU、GUI経由の受入は別途必要である。

## GPUバックエンドとモデルの関係

既存のQwen・Wan・LTX 0.9.8はNVIDIA（CUDA）、AMD（ROCm）、Apple Silicon（MPS）で同じモデルを共有し、GPU別に二重取得しない。新しいNVFP4構成はこの共有対象ではなく、Windows x64のNVIDIA Blackwell以降に限定する。判定と導入は`src-tauri/src/generation_backend.rs`が行う。

| バックエンド | 対象 | PyTorch | 依存ロック |
|---|---|---|---|
| CUDA | Windows x64・CUDA対応NVIDIA GPU | 2.11.0+cu128 | `sidecar/requirements-comfy.lock` |
| ROCm | Windows x64・AMD公式のWindows向けROCm対応GPU（Radeonソフトウェア26.2.2以降） | 2.9.1+rocm7.2.1 | `sidecar/requirements-comfy-rocm.lock` |
| MPS（Metal） | Apple Silicon Mac（Intel Macは対象外） | 2.11.0 | `sidecar/requirements-comfy-mps.lock` |

共通依存は`sidecar/requirements-comfy-common.in`に置き、CUDA／ROCm／MPS固有の`.in`がそれぞれのPyTorchだけを足す。ROCm版wheelはAMD公式配布（`repo.radeon.com`）のURLを直接固定し、PyPIのCPU版torchで上書きされないようにする。macOS arm64はPyPIの標準wheelがMetal（MPS）を含むため、専用インデックスも専用ビルドも使わない。

### Apple Silicon Macのメモリ判定

**専用VRAMの判定を流用しない。** ユニファイドメモリはCPUとGPUで同じ実体を使い、OSとアプリの作業領域を含む総量なので、16GBの専用VRAMと16GBのユニファイドメモリは同じ意味を持たない。Macではモデルごとに必要メモリを判定し、足りなければ取得を始めない。

| モデル | 最大の重み | 必要なユニファイドメモリ（暫定） |
|---|---:|---:|
| Qwen画像編集・キャラクター作成（Grounding DINO、SAM 2.1を含む） | 約41GB | 約49GB |
| Wan 2.2動画生成 | 約10.0GB | 約18.0GB |
| LTX軽量動画生成 | 約9.8GB | 約17.8GB |

必要量は「そのworkflowで読み込む最大の重み＋8GB（OSと作業領域）」で見積もっている。**実機計測が済むまでの暫定値**であり、計測後にこの表と`src-tauri/src/generation_backend.rs`の`GENERATION_MODELS`を実測へ置き換える。

Qwen、Grounding DINO、SAM 2.1、Wan 2.2、LTXはMPSでの実機確認が済んでいない。確認が済むまで設定画面と生成時に「実機未確認」と明示し、`対応`とは表示しない。Wanの映像破損が再現した場合は動画生成だけを`非対応`にする（`MpsSupport::Unsupported`）。LTXのFP8重みとWanのFP8テキストエンコーダーはMPSでの可否が未確認で、精度を変える場合は上の記録手順に従う。

AMD実機で未確認の事項（Qwen Image Edit、Grounding DINO、SAM 2.1、Wan 2.2、LTXの実動作、LTXのFP8可否、Wan 2.2の既知問題、ROCm wheelのSHA-256）は[未完了タスク](TASKS.md)に置く。**AMDで精度やモデルを変える場合は、対応GPU、必要VRAMとRAM、取得量、速度と画質への影響、公式URL、固定リビジョン、SHA-256をここへ記録してから切り替える。NVIDIAの既存精度（LTXのFP8を含む）は変更しない。**

## ComfyUIの起動メモリ設定

ComfyUIを使う画像・動画・キャラクター生成は、専用VRAM量で起動方法を変える（判定は十進の16GB）。

| GPUのメモリ | 起動 |
|---|---|
| 専用VRAM 16GB以上 | 通常起動 |
| 専用VRAM 16GB未満 | `--lowvram` |
| APUなど共有メモリ | `--lowvram` |
| 専用VRAM量を取得できない | `--lowvram`（安全側） |
| Apple Siliconのユニファイドメモリ | 通常起動（`--lowvram`は使わない） |

通常起動でGPUメモリ不足になった場合だけ、ComfyUIと子孫プロセスを停止し、途中出力を回収したうえで、同じ入力・シード・モデル・workflow・出力先のまま`--lowvram`で**1回だけ**やり直す。既に`--lowvram`で動いていた場合、GPUメモリ不足以外の失敗では再試行しない。

**MPSでは`--lowvram`を使わず、メモリ不足でも再試行しない。** MPSで実機検証済みの起動引数だけを使うという方針のためで、代わりに画像・動画のサイズかフレーム数を小さくするよう案内する。

参照先:

- [ComfyUI公式 Qwen Image Edit](https://docs.comfy.org/tutorials/image/qwen/qwen-image-edit)
- [ComfyUI公式 Wan 2.2](https://docs.comfy.org/tutorials/video/wan/wan2_2)
- [Wan 2.2公式リポジトリ](https://github.com/Wan-Video/Wan2.2)
- [LTX-Video公式モデル](https://huggingface.co/Lightricks/LTX-Video)
- [LTX 2.5公式モデル](https://huggingface.co/Lightricks/LTX-2.5)
- [Krea 2 Turbo公式モデル・利用条件](https://huggingface.co/krea/Krea-2-Turbo)
- [ComfyUI公式Krea 2量子化モデル](https://huggingface.co/Comfy-Org/Krea-2)
- [LTX Desktop公式の実行条件](https://github.com/Lightricks/LTX-Desktop)
- [LTX 2.5モデル本体のシステム要件](https://docs.ltx.io/open-source-model/getting-started/system-requirements)
- [LTX 2.xライセンス](https://github.com/Lightricks/LTX-2/blob/main/LICENSE.md)
