"""既存Windows SBV2環境でキャラ学習候補を作る。導入・既存上書きは行わない。"""
import argparse
import hashlib
import json
import ipaddress
import math
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path
from urllib.parse import parse_qs, urlparse
from voice_training_common import atomic_move

MAX_BYTES = 500_000_000
MAX_SECONDS = 7200
ASR_MIN_FREE_BYTES = 6_000_000_000


def probe_asr_cuda():
    import torch
    import ctranslate2
    if not torch.cuda.is_available() or ctranslate2.get_cuda_device_count()<1:
        return {'available':False,'reason':'CUDA対応GPUまたは必要なCUDAランタイムを利用できません'}
    if 'float16' not in ctranslate2.get_supported_compute_types('cuda',device_index=0):
        return {'available':False,'reason':'このGPUのCTranslate2はfloat16に対応していません'}
    free,total=torch.cuda.mem_get_info(0)
    return {'available':True,'freeBytes':int(free),'totalBytes':int(total)}


def choose_asr_device(requested, probe=probe_asr_cuda):
    if requested not in ('cuda','cpu'):
        raise ValueError('文字起こし実行先はcuda/cpuを指定してください')
    if requested=='cpu':
        return {'requested':requested,'device':'cpu','computeType':'int8','reason':'CPUが指定されています（旧request省略時もCPU）'}
    try:
        capability=probe()
    except Exception as error:
        capability={'available':False,'reason':f'CUDA能力を確認できません: {error}'}
    ready=capability.get('available') is True and capability.get('freeBytes',0)>=ASR_MIN_FREE_BYTES
    if ready:
        return {'requested':requested,'device':'cuda','computeType':'float16','deviceIndex':0,'reason':'CUDA float16対応を確認し、空きGPUメモリが6.0GB以上あります','capability':capability}
    reason=capability.get('reason') or f'空きGPUメモリ{capability.get("freeBytes",0)/1_000_000_000:.2f}GBが保守的な必要容量6.0GBを下回っています'
    raise RuntimeError('CUDA文字起こしを開始できません。'+reason)


def provenance_request(request):
    """再起動PID・QA接続情報だけを素材来歴から除外する。"""
    stable={key:value for key,value in request.items() if key not in ('ownerProcessId','endpoint','speaker')}
    stable.setdefault('stopAtStep',1000)
    stable.setdefault('asrDevice','cpu')
    return stable


def youtube_limit(info, *, incomplete=False):
    if info.get('is_live') or info.get('live_status') in ('is_live','is_upcoming','post_live'):
        return 'ライブ・予約配信・確定前のライブ音声には対応しません'
    duration=info.get('duration')
    size=info.get('filesize') or info.get('filesize_approx')
    if not incomplete and (duration is None or duration>MAX_SECONDS or size is None or size>MAX_BYTES):
        return '長さ・容量が不明、または2時間/500MB上限超過です'
    return None


class RuntimeLock:
    """OSがプロセス終了時にも解放する共有ランタイムの排他。"""
    def __init__(self, root):
        self.path=Path(root)/'.character-learning-operation.lock'
        self.handle=None

    def __enter__(self):
        self.handle=self.path.open('a+b')
        try:
            if os.name=='nt':
                import msvcrt
                self.handle.seek(0)
                if not self.handle.read(1):
                    self.handle.write(b'0')
                    self.handle.flush()
                self.handle.seek(0)
                msvcrt.locking(self.handle.fileno(),msvcrt.LK_NBLCK,1)
            else:
                import fcntl
                fcntl.flock(self.handle.fileno(),fcntl.LOCK_EX|fcntl.LOCK_NB)
        except OSError as error:
            self.handle.close()
            self.handle=None
            raise RuntimeError('共有ランタイムは他の学習・検査で使用中です') from error
        return self

    def __exit__(self, *args):
        if self.handle:
            try:
                if os.name=='nt':
                    import msvcrt
                    self.handle.seek(0)
                    msvcrt.locking(self.handle.fileno(),msvcrt.LK_UNLCK,1)
                else:
                    import fcntl
                    fcntl.flock(self.handle.fileno(),fcntl.LOCK_UN)
            finally:
                self.handle.close()
                self.handle=None


def atomic_text(path, text):
    # 停止（taskkill /F）やクラッシュが書き込み中に当たっても、途中までの内容を正本として残さない。
    # 再開時は既存ファイルとの一致で完了工程を再利用するため、欠けたファイルは永久に再開を止める。
    temporary = path.with_suffix(path.suffix + '.incoming')
    temporary.write_text(text, encoding='utf-8')
    atomic_move(temporary, path, cleanup_file=True)


def atomic_json(path, value):
    atomic_text(path, json.dumps(value, ensure_ascii=False, indent=2))


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()


def validate_url(value):
    parsed = urlparse(value)
    if parsed.scheme != 'https' or parsed.username or parsed.password or parsed.port not in (None, 443):
        raise ValueError('認証情報なしのHTTPS YouTube URLを指定してください')
    if parsed.hostname == 'youtu.be' and re.fullmatch(r'/[A-Za-z0-9_-]{11}', parsed.path):
        return 'https://www.youtube.com/watch?v=' + parsed.path[1:]
    if parsed.hostname in ('youtube.com', 'www.youtube.com', 'm.youtube.com') and parsed.path == '/watch':
        video = parse_qs(parsed.query).get('v', [])
        if len(video) == 1 and re.fullmatch(r'[A-Za-z0-9_-]{11}', video[0]):
            return 'https://www.youtube.com/watch?v=' + video[0]
    if parsed.hostname in ('youtube.com','www.youtube.com','m.youtube.com'):
        match=re.fullmatch(r'/(?:shorts|live)/([A-Za-z0-9_-]{11})',parsed.path)
        if match:
            return 'https://www.youtube.com/watch?v=' + match.group(1)
    raise ValueError('単一YouTube動画だけに対応します。一般URL・playlistは利用できません')


class Worker:
    def __init__(self, request, runner=subprocess.run):
        self.request = request
        if request.get('asrDevice','cpu') not in ('cuda','cpu'):
            raise ValueError('文字起こし実行先が不正です')
        self.dll_handles=[]
        if request.get('mode') not in ('persona', 'voice', 'both'):
            raise ValueError('modeが不正です')
        if not isinstance(request.get('characterName'), str) or not request['characterName'].strip():
            raise ValueError('キャラ名が空です')
        self.job = Path(request['jobDir']).resolve()
        self.root = Path(request['runtimeRoot']).resolve()
        self.scripts = Path(request['scriptsRoot']).resolve()
        self.python = self.root / '.venv/Scripts/python.exe'
        if not self.python.is_file():
            raise ValueError('既存SBV2 Pythonがありません')
        self.source = request['source']
        if self.source.get('kind') == 'url':
            self.url = validate_url(self.source['value'])
        elif self.source.get('kind') == 'file':
            source = Path(self.source['value']).resolve()
            source_proof=self.job/'source.json'
            saved=None
            if source_proof.is_file():
                proof=json.loads(source_proof.read_text(encoding='utf-8'))
                candidate=Path(proof['snapshot']).resolve()
                if proof.get('source')==self.source and candidate.parent==(self.job/'source').resolve() and candidate.is_file() and candidate.stat().st_size<=MAX_BYTES and digest(candidate)==proof.get('sha256'):
                    saved=candidate
                else:
                    raise ValueError('保存済み原音snapshotのSHA・来歴が一致しません')
            if saved is None and (not source.is_file() or source.stat().st_size > MAX_BYTES):
                raise ValueError('音声ファイルがないか500MBを超えています')
            self.original = source
        else:
            raise ValueError('source.kindが不正です')
        self.dataset = 'character-learning-' + hashlib.sha256(str(self.job).encode()).hexdigest()[:20]
        self.selected = self.dataset + '-selected'
        self.runner = runner
        self.job.mkdir(parents=True, exist_ok=True)
        self.env = dict(os.environ, HF_HUB_OFFLINE='1', TRANSFORMERS_OFFLINE='1', HF_DATASETS_OFFLINE='1')
        runtime_env = self.root/'runtime-env.json'
        if runtime_env.is_file():
            environment=json.loads(runtime_env.read_text(encoding='utf-8')).get('environment',{})
            for key,value in environment.items():
                if key in ('HF_HOME','HF_HUB_CACHE','TORCH_HOME','PYTHONPATH','HF_DATASETS_CACHE','DENO_DIR') and isinstance(value,str):
                    self.env[key]=value
                    os.environ[key]=value
        for key in ('HF_HUB_OFFLINE', 'TRANSFORMERS_OFFLINE', 'HF_DATASETS_OFFLINE'):
            os.environ[key] = '1'

    def stage(self, name):
        path=self.job/'status.json'
        status=json.loads(path.read_text(encoding='utf-8')) if path.exists() else {}
        status.update(stage=name,state='running',dataset=self.dataset)
        atomic_json(path,status)

    def transcribe(self, audio, factory=None, probe=probe_asr_cuda):
        # Windows DLL探索ハンドルは推論・generator消費が終わるまで所有する。
        try:
            if os.name=='nt' and hasattr(os,'add_dll_directory'):
                directory=self.root/'.venv/Lib/site-packages/torch/lib'
                if directory.is_dir():
                    self.dll_handles.append(os.add_dll_directory(str(directory)))
            return self._transcribe(audio,factory,probe)
        finally:
            for handle in self.dll_handles:
                handle.close()
            self.dll_handles.clear()

    def _transcribe(self, audio, factory, probe):
        self.stage('transcribe')
        try:
            selection=choose_asr_device(self.request.get('asrDevice','cpu'),probe)
        except Exception as error:
            selection={'requested':self.request.get('asrDevice','cpu'),'device':self.request.get('asrDevice','cpu'),'reason':str(error),'state':'failed'}
            atomic_json(self.job/'asr.json',selection)
            status=json.loads((self.job/'status.json').read_text(encoding='utf-8'))
            status['asr']=selection
            atomic_json(self.job/'status.json',status)
            raise
        atomic_json(self.job/'asr.json',dict(selection,state='running'))
        status=json.loads((self.job/'status.json').read_text(encoding='utf-8'))
        status['asr']=selection
        atomic_json(self.job/'status.json',status)
        model=None
        try:
            if factory is None:
                from faster_whisper import WhisperModel
                factory=WhisperModel
            model=factory('large-v3',device=selection['device'],compute_type=selection['computeType'],local_files_only=True)
            segments,_=model.transcribe(str(audio),language='ja',condition_on_previous_text=False,vad_filter=False,word_timestamps=True)
            rows=[{'start':float(s.start),'end':float(s.end),'text':s.text.strip(),'words':[{'start':float(w.start),'end':float(w.end),'text':w.word} for w in (s.words or [])]} for s in segments]
            atomic_json(self.job/'asr.json',dict(selection,state='completed'))
            return rows
        except Exception as error:
            atomic_json(self.job/'asr.json',dict(selection,state='failed',error=str(error)))
            # GPU推論失敗はCPUへ降格せず、原因を保持する。
            raise RuntimeError(f'{selection["device"]}文字起こしに失敗しました: {error}') from error
        finally:
            del model

    def run(self, args, stage, allow_failure=False):
        self.stage(stage)
        with (self.job / (stage + '.log')).open('ab') as log:
            result = self.runner([str(v) for v in args], cwd=self.root, env=self.env, stdout=log, stderr=log, check=False)
        if result.returncode and not allow_failure:
            raise RuntimeError(f'{stage}に失敗しました（終了コード{result.returncode}）。{stage}.logを確認してください')
        return result.returncode

    def script(self, name, args, stage, **kwargs):
        path = self.scripts / name
        if not path.is_file():
            raise ValueError(f'必要なスクリプトがありません: {name}')
        return self.run([self.python, path, *args], stage, **kwargs)

    def prepare(self):
        completed = self.job/'prepared.json'
        if completed.is_file():
            proof=json.loads(completed.read_text(encoding='utf-8'))
            if proof.get('dataset') != self.selected or provenance_request(proof.get('request',{})) != provenance_request(self.request):
                raise ValueError('準備済みrequestが一致しません')
            for name, expected in proof['artifacts'].items():
                if digest(Path(name)) != expected:
                    raise ValueError('準備済み成果物SHAが一致しません')
            status={'stage':'prepared','state':'completed'}
            if (self.job/'asr.json').exists():
                status['asr']=json.loads((self.job/'asr.json').read_text(encoding='utf-8'))
            atomic_json(self.job/'status.json',status)
            return
        target = self.job / 'source'
        owner=self.job/'prepare-owner.json'
        if owner.exists():
            if provenance_request(json.loads(owner.read_text(encoding='utf-8'))) != provenance_request(self.request):
                raise ValueError('再開requestが一致しません')
        else:
            if target.exists() or (self.job/'transcript.txt').exists():
                raise ValueError('所有証跡のない既存入力は上書きしません')
            atomic_json(owner,provenance_request(self.request))
        import soundfile as sf
        ffmpeg = self.request.get('ffmpeg')
        if ffmpeg:
            if not Path(ffmpeg).is_file():
                raise ValueError('指定ffmpeg実体がありません')
            ffmpeg=str(Path(ffmpeg).resolve())
        else:
            # フォールバック許可: 親が実体指定を省略した場合だけ既存PATHを探索する。
            ffmpeg = shutil.which('ffmpeg')
        if not ffmpeg:
            raise ValueError('ffmpegが見つかりません。管理ランタイムを指定してください')
        target.mkdir(exist_ok=True)
        source_proof=self.job/'source.json'
        if source_proof.exists():
            proof=json.loads(source_proof.read_text(encoding='utf-8'))
            snapshot=Path(proof['snapshot'])
            if proof['source']!=self.source or snapshot.parent!=target or digest(snapshot)!=proof['sha256']:
                raise ValueError('原音再開SHAが一致しません')
        elif self.source['kind'] == 'file':
            before = digest(self.original)
            snapshot = target / ('original' + self.original.suffix)
            if snapshot.exists():
                if digest(snapshot)!=before:
                    raise ValueError('中断snapshotが原音と一致しません')
            else:
                temporary=snapshot.with_suffix(snapshot.suffix+'.incoming')
                shutil.copyfile(self.original, temporary)
                temporary.replace(snapshot)
            if digest(snapshot) != before or digest(self.original) != before:
                raise ValueError('コピー中に原音が変わりました')
        else:
            deno=self.root/'tools/deno/deno.exe'
            if not deno.is_file():
                raise ValueError('YouTube取得用の専用Denoがありません。専用Denoを含む現行のキャラ学習環境を導入・選択してください。旧環境の自動更新は未対応です')
            import yt_dlp
            self.stage('download')
            with yt_dlp.YoutubeDL({'noplaylist': True, 'format': 'bestaudio', 'outtmpl': str(target/'original.%(ext)s'), 'max_filesize': MAX_BYTES, 'match_filter': youtube_limit, 'overwrites': False,'js_runtimes':{'deno':{'path':str(deno)}},'remote_components':[]}) as downloader:
                info = downloader.extract_info(self.url, download=True)
                if not info or info.get('entries'):
                    raise ValueError('単一音声の取得に失敗しました')
                if youtube_limit(info):
                    raise ValueError(youtube_limit(info))
                snapshot = Path(downloader.prepare_filename(info))
            if not snapshot.is_file() or snapshot.stat().st_size > MAX_BYTES:
                raise ValueError('取得音声がありません、または500MB超過です')
        atomic_json(self.job/'source.json', {'sha256': digest(snapshot), 'snapshot': str(snapshot), 'source': self.source})
        for rate in (48000, 16000):
            decoded=target/f'audio-{rate}.wav'
            proof_path=self.job/f'decode-{rate}.json'
            if decoded.exists() and proof_path.exists():
                if digest(decoded)!=json.loads(proof_path.read_text(encoding='utf-8'))['sha256']:
                    raise ValueError('変換音声の再開SHAが一致しません')
                continue
            temporary=target/f'audio-{rate}.incoming.wav'
            if temporary.exists():
                temporary.unlink()
            self.run([ffmpeg, '-nostdin', '-v', 'error', '-n', '-i', snapshot, '-t', str(MAX_SECONDS + 1), '-vn', '-ac', '1', '-ar', str(rate), '-c:a', 'pcm_s16le', temporary], f'decode-{rate}')
            if decoded.exists():
                if digest(decoded)!=digest(temporary):
                    raise ValueError('中断変換音声が再変換結果と一致しません')
                temporary.unlink()
            else:
                temporary.replace(decoded)
            atomic_json(proof_path,{'sha256':digest(decoded)})
        full = target/'audio-48000.wav'
        info = sf.info(full)
        if not 0 < info.duration <= MAX_SECONDS:
            raise ValueError('音声が空、または2時間を超えています')
        # 親がこのworkerを既存SBV2 Pythonで起動する。キャッシュ取得は行わない。
        if (self.job/'segments.json').exists():
            rows=json.loads((self.job/'segments.json').read_text(encoding='utf-8'))
        else:
            rows=self.transcribe(target/'audio-16000.wav')
        if not rows or not any(r['text'] for r in rows):
            raise ValueError('文字起こし結果が空です')
        if any(not math.isfinite(r['start']) or not math.isfinite(r['end']) or not 0<=r['start']<r['end']<=info.duration for r in rows):
            raise ValueError('文字起こし区間時刻が不正です')
        if any('|' in r['text'] or '\n' in r['text'] or '\r' in r['text'] for r in rows):
            raise ValueError('文字起こしにデータセット区切り文字が含まれています。編集レビューが必要です')
        if not (self.job/'segments.json').exists():
            atomic_json(self.job/'segments.json', rows)
        transcript='\n'.join(r['text'] for r in rows)
        if (self.job/'transcript.txt').exists():
            if (self.job/'transcript.txt').read_text(encoding='utf-8')!=transcript:
                raise ValueError('既存文字起こしが区間と一致しません')
        else:
            atomic_text(self.job/'transcript.txt', transcript)
        if self.request['mode'] != 'persona':
            chunks=[]
            for row in rows:
                words=row.get('words',[])
                if 2<=row['end']-row['start']<=12 or not words:
                    chunks.append(row)
                    continue
                current=[]
                for word in words:
                    if not math.isfinite(word['start']) or not math.isfinite(word['end']) or not row['start']<=word['start']<word['end']<=row['end']:
                        raise ValueError('語時刻が不正です')
                    if current and word['end']-current[0]['start']>12:
                        chunks.append({'start':current[0]['start'],'end':current[-1]['end'],'text':''.join(w['text'] for w in current).strip()})
                        current=[]
                    current.append(word)
                if current:
                    chunks.append({'start':current[0]['start'],'end':current[-1]['end'],'text':''.join(w['text'] for w in current).strip()})
            dataset = self.root/'Data'/self.dataset
            dataset_proof=self.job/'dataset.json'
            if dataset.exists() and not dataset_proof.exists():
                raise ValueError('所有証跡のない既存データセットは上書きできません')
            if not dataset_proof.exists():
                atomic_json(dataset_proof,{'dataset':str(dataset),'source_sha256':digest(full)})
            elif json.loads(dataset_proof.read_text(encoding='utf-8'))!={'dataset':str(dataset),'source_sha256':digest(full)}:
                raise ValueError('データセット所有証跡が一致しません')
            (dataset/'raw').mkdir(parents=True,exist_ok=True)
            lines=[]
            with sf.SoundFile(full) as audio:
                for index, row in enumerate(chunks):
                    if not 2 <= row['end']-row['start'] <= 12 or not row['text']:
                        continue
                    a,b=round(row['start']*48000),round(row['end']*48000)
                    if not 0 <= a < b <= len(audio):
                        raise ValueError('文字起こし区間が音声範囲外です')
                    audio.seek(a)
                    name=f'clip-{index:05}.wav'
                    samples=audio.read(b-a)
                    destination=dataset/'raw'/name
                    if destination.exists():
                        import numpy as np
                        old,old_sr=sf.read(destination)
                        # PCM16の同一量子化を比較し、既存断片を上書きしない。
                        expected=np.clip(np.floor(samples*32768),-32768,32767).astype(np.int16)
                        if old_sr!=48000 or len(old)!=len(samples) or not np.array_equal(np.round(old*32768).astype(np.int16),expected):
                            raise ValueError('既存rawが再切出し結果と一致しません')
                    else:
                        temporary=destination.with_suffix('.incoming.wav')
                        sf.write(temporary, samples, 48000, subtype='PCM_16')
                        atomic_move(temporary, destination)
                    lines.append(f'{name}|{self.dataset}|JP|{row["text"]}')
            if not lines:
                raise ValueError('2〜12秒の学習候補区間がありません')
            text='\n'.join(lines)+'\n'
            if (dataset/'esd.list').exists():
                if (dataset/'esd.list').read_text(encoding='utf-8')!=text:
                    raise ValueError('既存esd.listが一致しません')
            else:
                atomic_text(dataset/'esd.list', text)
            report=self.job/'clip-metrics.jsonl'
            self.script('select-voice-training-clips.py', ['measure','--sbv2-root',self.root,'--dataset',self.dataset,'--report',report,'--device','cuda'], 'measure')
            selected=self.root/'Data'/self.selected
            selection_owner=self.job/'selection-owner.json'
            if not selection_owner.exists():
                if selected.exists():
                    raise ValueError('所有証跡のない選別データセットが既存です')
                atomic_json(selection_owner,{'dataset':str(selected)})
            if not selected.exists():
                incoming=selected.with_name(selected.name+'.incoming')
                if incoming.exists():
                    # 自分が記録した選別先の未確定出力だけ回収して再選別する。
                    if incoming.is_symlink() or incoming.resolve().parent!=(self.root/'Data').resolve():
                        raise ValueError('未確定出力が管理Data直下ではありません')
                    shutil.rmtree(incoming)
                self.script('select-voice-training-clips.py', ['select','--sbv2-root',self.root,'--dataset',self.dataset,'--report',report,'--output-dataset',self.selected,'--work-dir',self.job/'selection'], 'select')
            accepted = {line.split('|', 1)[0] for line in (self.root/'Data'/self.selected/'esd.list').read_text(encoding='utf-8').splitlines() if line.strip()}
            reference = [line for line in report.read_text(encoding='utf-8').splitlines() if json.loads(line)['file'] in accepted]
            if len(reference) != len(accepted):
                raise ValueError('選別済み話者基準の件数が一致しません')
            atomic_text(self.job/'selected-reference.jsonl', '\n'.join(reference)+'\n')
        artifacts=[snapshot,self.job/'source.json',self.job/'transcript.txt',self.job/'segments.json']
        if (self.job/'asr.json').exists():
            artifacts.append(self.job/'asr.json')
        if self.request['mode']!='persona':
            artifacts.extend((self.root/'Data'/self.selected).rglob('*.wav'))
            artifacts.extend([self.root/'Data'/self.selected/'esd.list',self.job/'selected-reference.jsonl'])
        atomic_json(self.job/'prepared.json', {'dataset':self.selected,'mode':self.request['mode'],'source_sha256':digest(snapshot),'request':provenance_request(self.request),'artifacts':{str(p):digest(p) for p in artifacts}})
        status=json.loads((self.job/'status.json').read_text(encoding='utf-8'))
        status.update(stage='prepared',state='completed')
        atomic_json(self.job/'status.json',status)

    def train(self):
        if self.request['mode']=='persona' or not (self.job/'prepared.json').is_file():
            raise ValueError('音声準備が完了していません')
        prepared=json.loads((self.job/'prepared.json').read_text(encoding='utf-8'))
        if prepared['dataset']!=self.selected:
            raise ValueError('準備済みデータセットが一致しません')
        if 'request' in prepared and provenance_request(prepared['request'])!=provenance_request(self.request):
            raise ValueError('準備済み素材・学習条件が一致しません')
        source = json.loads((self.job/'source.json').read_text(encoding='utf-8'))
        if digest(Path(source['snapshot'])) != prepared['source_sha256']:
            raise ValueError('準備後に原音snapshotが変更されています')
        stop=self.request.get('stopAtStep',1000)
        if isinstance(stop,bool) or not isinstance(stop,int) or stop<1000 or stop%1000:
            raise ValueError('停止stepは1000以上の1000単位です')
        tool=self.scripts/'train-voice-model.ps1'
        if not tool.is_file():
            raise ValueError('正規学習スクリプトがありません')
        pwsh=shutil.which('pwsh')
        if not pwsh:
            raise ValueError('PowerShell 7がありません')
        assets=self.root/'model_assets'/self.selected
        prefix=[pwsh,'-NoProfile','-File',tool,'-SbvRoot',self.root,'-Dataset',self.selected]
        marker=self.job/'preprocess-completed.json'
        if marker.exists():
            config=self.root/'Data'/self.selected/'config.json'
            if not config.is_file() or json.loads(config.read_text(encoding='utf-8')).get('model_name')!=self.selected:
                raise ValueError('前処理完了configが一致しません')
        if not marker.exists():
            ownership=self.job/'preprocess-owner.json'
            if assets.exists() and not ownership.exists():
                raise ValueError('所有証跡のない既存学習候補は上書きしません')
            if not ownership.exists():
                atomic_json(ownership,{'dataset':self.selected})
            self.run([*prefix,'-Stage','preprocess'],'preprocess')
            config=self.root/'Data'/self.selected/'config.json'
            atomic_json(marker,{'config':str(config),'sha256':digest(config)})
        saved=[int(m.group(1)) for p in assets.glob('*.safetensors') if (m:=re.search(r'_s(\d+)\.safetensors$',p.name))]
        if not saved or max(saved)<stop:
            self.run([*prefix,'-Stage','train','-StopAtStep',str(stop)],'train')
        checkpoints=[]
        for file in assets.glob('*.safetensors'):
            match=re.search(r'_s(\d+)\.safetensors$',file.name)
            if match:
                checkpoints.append((int(match[1]),file))
        if not checkpoints:
            raise ValueError('チェックポイントが保存されませんでした')
        checkpoint=max(checkpoints)[1]
        checkpoint_sha=digest(checkpoint)
        qa=self.job/('qa-'+checkpoint_sha[:12])
        code=0 if (qa/'qa-report.json').exists() else self.script('check-synthesized-voice.py',['sbv2','--sbv2-root',self.root,'--model-dir',assets,'--checkpoint',checkpoint,'--reference',self.job/'selected-reference.jsonl','--out-dir',qa,'--text-set','pauses','--repeat','3'], 'qa',allow_failure=True)
        report=json.loads((qa/'qa-report.json').read_text(encoding='utf-8'))
        passed=code==0 and report.get('passed') is True
        export=self.job/('export-'+checkpoint_sha[:12])
        if not export.exists():
            incoming=export.with_name(export.name+'.incoming')
            if incoming.exists():
                if incoming.is_symlink() or incoming.resolve().parent!=self.job:
                    raise ValueError('export未確定出力がjob直下ではありません')
                shutil.rmtree(incoming)
            self.run([*prefix,'-Stage','export','-Checkpoint',checkpoint,'-OutputDir',export],'export')
        files=list(export.glob('*.aivmx'))
        if len(files)!=1:
            raise ValueError('AIVMX出力が一意ではありません')
        if digest(export/checkpoint.name)!=checkpoint_sha:
            raise ValueError('export元チェックポイントが一致しません')
        import aivmlib
        with files[0].open('rb') as handle:
            metadata=aivmlib.read_aivmx_metadata(handle)
        import numpy as np
        import io
        config=json.loads((export/'config.json').read_text(encoding='utf-8'))
        if metadata.hyper_parameters.model_name!=config['model_name'] or metadata.hyper_parameters.data.spk2id!=config['data']['spk2id'] or metadata.hyper_parameters.data.style2id!=config['data']['style2id']:
            raise ValueError('exportのモデル名・話者・styleがconfigと一致しません')
        if not np.array_equal(np.load(io.BytesIO(metadata.style_vectors)),np.load(export/'style_vectors.npy')):
            raise ValueError('export style_vectorsが一致しません')
        export_proof=export/'worker-proof.json'
        identity={'checkpointSHA256':checkpoint_sha,'aivmxSHA256':digest(files[0]),'modelUUID':str(metadata.manifest.uuid),'speakerUUID':str(metadata.manifest.speakers[0].uuid)}
        if export_proof.exists():
            if json.loads(export_proof.read_text(encoding='utf-8'))!=identity:
                raise ValueError('exportの保存済みUUID/SHAが一致しません')
        else:
            atomic_json(export_proof,identity)
        atomic_json(self.job/'result.json', {'artifactPaths':{'aivmx':str(files[0]),'checkpoint':str(checkpoint),'qa':str(qa/'qa-report.json'),'transcript':str(self.job/'transcript.txt')},'modelUUID':str(metadata.manifest.uuid),'speakerUUID':str(metadata.manifest.speakers[0].uuid),'quality':{'passed':passed,'sbv2Passed':passed,'aivisPassed':False,'defaultAssignable':False,'failures':report.get('failures',{})}})
        atomic_json(self.job/'status.json', {'stage':'candidate','state':'completed','qualityPassed':passed})

    def aivis_qa(self):
        endpoint=self.request.get('endpoint','')
        parsed=urlparse(endpoint)
        try:
            loopback=ipaddress.ip_address(parsed.hostname).is_loopback
        except ValueError:
            loopback=parsed.hostname=='localhost'
        if parsed.scheme not in ('http','https') or not loopback or parsed.username or parsed.password or parsed.path not in ('','/') or parsed.query or parsed.fragment:
            raise ValueError('Aivis QA接続先はloopback URLだけに限定します')
        speaker=self.request.get('speaker')
        if isinstance(speaker,bool) or not isinstance(speaker,int):
            raise ValueError('Aivis global style IDを指定してください')
        result_path=self.job/'result.json'
        result=json.loads(result_path.read_text(encoding='utf-8'))
        result['quality'].update(aivisPassed=False,defaultAssignable=False)
        atomic_json(result_path,result)
        reference=self.job/'selected-reference.jsonl'
        out=self.job/'qa-aivis'
        if out.exists() or out.with_name(out.name+'.incoming').exists():
            # 再試行は前回reportを残し、新しい試行先へ出力する。
            attempt=2
            while (self.job/f'qa-aivis-attempt-{attempt}').exists() or (self.job/f'qa-aivis-attempt-{attempt}.incoming').exists():
                attempt+=1
            out=self.job/f'qa-aivis-attempt-{attempt}'
        code=self.script('check-synthesized-voice.py',['aivis','--endpoint',endpoint,'--speaker',str(speaker),'--text-set','pauses','--repeat','3','--reference',reference,'--out-dir',out],'qa-aivis',allow_failure=True)
        report=json.loads((out/'qa-report.json').read_text(encoding='utf-8'))
        passed=code==0 and report.get('passed') is True and report.get('level_judged') is True
        result['quality'].update(aivisPassed=passed,defaultAssignable=passed and result['quality'].get('sbv2Passed') is True)
        result['artifactPaths']['aivisQA']=str(out/'qa-report.json')
        atomic_json(result_path,result)
        atomic_json(self.job/'status.json',{'stage':'qa-aivis','state':'completed','qualityPassed':passed})


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command',choices=('prepare','train','aivis-qa'))
    parser.add_argument('--request',type=Path,required=True)
    args=parser.parse_args()
    worker=None
    try:
        worker=Worker(json.loads(args.request.read_text(encoding='utf-8')))
        if Path(sys.executable).resolve() != worker.python.resolve():
            raise ValueError('request.runtimeRootの既存SBV2 Pythonでworkerを起動してください')
        with RuntimeLock(worker.root):
            getattr(worker,args.command.replace('-','_'))()
        return 0
    except Exception as error:
        if worker:
            status_path=worker.job/'status.json'
            status=json.loads(status_path.read_text(encoding='utf-8')) if status_path.exists() else {}
            status.update(state='failed',error=str(error))
            atomic_json(status_path,status)
        print(str(error),file=sys.stderr)
        return 1


if __name__=='__main__':
    sys.exit(main())
