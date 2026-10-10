"""外部処理を実行せず、学習workerの保護境界を検証する。"""
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT/'scripts'))
SPEC=importlib.util.spec_from_file_location('learning_worker',ROOT/'scripts/character-learning-worker.py')
WORKER=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(WORKER)


class WorkerTests(unittest.TestCase):
    def setUp(self):
        (ROOT/'temp').mkdir(exist_ok=True)
        self.directory=tempfile.TemporaryDirectory(dir=ROOT/'temp')
        self.addCleanup(self.directory.cleanup)
        base=Path(self.directory.name)
        self.runtime=base/'runtime'
        python=self.runtime/'.venv/Scripts/python.exe'
        python.parent.mkdir(parents=True)
        python.write_bytes(b'fake')
        self.source=base/'original.wav'
        self.source.write_bytes(b'original')
        self.request=dict(jobDir=str(base/'job'),runtimeRoot=str(self.runtime),scriptsRoot=str(ROOT/'scripts'),source=dict(kind='file',value=str(self.source)),mode='both',characterName='試験',stopAtStep=1000)

    def test_認証付きや一般URLを拒否する(self):
        """ローカル・認証付き・一般URLを実行しない。"""
        for url in ('http://youtube.com/watch?v=eCaOrzfsvSs','https://127.0.0.1/a','https://youtube.com.evil/a','https://user@youtube.com/watch?v=eCaOrzfsvSs','https://youtube.com/playlist?list=abc'):
            with self.assertRaises(ValueError):
                WORKER.validate_url(url)
        self.assertEqual(WORKER.validate_url('https://youtu.be/eCaOrzfsvSs'),'https://www.youtube.com/watch?v=eCaOrzfsvSs')

    def test_動画URLの形式を正規化して余分なパスを拒否する(self):
        expected='https://www.youtube.com/watch?v=eCaOrzfsvSs'
        for host in ('youtube.com','www.youtube.com','m.youtube.com'):
            for path in ('/watch?v=eCaOrzfsvSs','/shorts/eCaOrzfsvSs','/live/eCaOrzfsvSs'):
                with self.subTest(host=host,path=path):
                    self.assertEqual(WORKER.validate_url('https://'+host+path),expected)
        for path in ('/shorts/eCaOrzfsvSs/extra','/live/eCaOrzfsvSs/','/shorts/eCaOrzfsvS','/live/eCaOrzfsvSsX','/channel/eCaOrzfsvSs','/shorts/','/lives/eCaOrzfsvSs'):
            with self.subTest(path=path),self.assertRaises(ValueError):
                WORKER.validate_url('https://www.youtube.com'+path)
        # /liveのURL形式を認めても、現ライブのmetadataは拒否する。
        self.assertIsNotNone(WORKER.youtube_limit({'is_live':True,'duration':60,'filesize':1000}))
        self.assertIsNone(WORKER.youtube_limit({'live_status':'was_live','duration':60,'filesize':1000}))

    def test_段階を原子的に保存して取得を無効にする(self):
        """stageは原子的に保存し、子処理の自動取得を無効にする。"""
        calls=[]
        def fake(args,**kwargs):
            calls.append(kwargs)
            return SimpleNamespace(returncode=0)
        worker=WORKER.Worker(self.request,fake)
        worker.run(['fake'],'prepare-test')
        status=json.loads((worker.job/'status.json').read_text(encoding='utf-8'))
        self.assertEqual(status['stage'],'prepare-test')
        self.assertEqual(calls[0]['env']['HF_HUB_OFFLINE'],'1')
        self.assertFalse((worker.job/'status.json.incoming').exists())

    def test_再開照合に使う本文は書き込み途中で中断しても旧内容を壊さない(self):
        target=Path(self.directory.name)/'transcript.txt'
        target.write_text('完成した文字起こし', encoding='utf-8')
        with patch.object(WORKER.os, 'replace', side_effect=OSError('停止')):
            with self.assertRaises(OSError):
                WORKER.atomic_text(target, '途中')
        self.assertEqual(target.read_text(encoding='utf-8'), '完成した文字起こし')
        self.assertFalse(target.with_suffix(target.suffix+'.incoming').exists())
        source=(ROOT/'scripts/character-learning-worker.py').read_text(encoding='utf-8')
        for name in ('transcript.txt', 'esd.list', 'selected-reference.jsonl'):
            self.assertNotIn(f"'{name}').write_text", source)

    def test_失敗後の工程を停止する(self):
        """失敗後に後続コマンドを実行しない。"""
        calls=[]
        worker=WORKER.Worker(self.request,lambda args,**kwargs: calls.append(args) or SimpleNamespace(returncode=2))
        with self.assertRaises(RuntimeError):
            worker.run(['first'],'first')
            worker.run(['second'],'second')
        self.assertEqual(len(calls),1)

    def test_既存入力と原音ハッシュを保護する(self):
        """既存入力を上書きせず原音ハッシュを保持する。"""
        worker=WORKER.Worker(self.request)
        (worker.job/'source').mkdir()
        before=WORKER.digest(self.source)
        with self.assertRaises(ValueError):
            worker.prepare()
        self.assertEqual(WORKER.digest(self.source),before)

    def test_未準備の学習を拒否する(self):
        """未準備の学習や既存モデルへの実行を開始しない。"""
        worker=WORKER.Worker(self.request,lambda *a,**k:self.fail('実行禁止'))
        with self.assertRaises(ValueError):
            worker.train()

    def test_原音改変で学習を停止する(self):
        """準備後の原音変更を検知し、前処理より前に停止する。"""
        worker=WORKER.Worker(self.request,lambda *a,**k:self.fail('実行禁止'))
        WORKER.atomic_json(worker.job/'prepared.json',{'dataset':worker.selected,'source_sha256':'変更前'})
        WORKER.atomic_json(worker.job/'source.json',{'snapshot':str(self.source)})
        with self.assertRaisesRegex(ValueError,'snapshot'):
            worker.train()

    def test_準備完了の再開はハッシュ一致を要求する(self):
        """準備済み成果物が一致すると再処理せず、改変時は停止する。"""
        worker=WORKER.Worker(self.request,lambda *a,**k:self.fail('再処理禁止'))
        WORKER.atomic_json(worker.job/'prepared.json',{'dataset':worker.selected,'request':self.request,'artifacts':{str(self.source):WORKER.digest(self.source)}})
        worker.prepare()
        self.source.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError,'SHA'):
            worker.prepare()

    def test_Aivis接続をループバックに限定する(self):
        """外部URLへQA接続しない。"""
        self.request.update(endpoint='http://example.com:11436',speaker=1)
        worker=WORKER.Worker(self.request,lambda *a,**k:self.fail('接続禁止'))
        with self.assertRaisesRegex(ValueError,'loopback'):
            worker.aivis_qa()

    def test_Aivis不合格と再試行で旧結果を保護する(self):
        """Aivis不合格レポートを残し、既定割当を拒否する。"""
        self.request.update(endpoint='http://127.0.0.1:11436',speaker=1)
        def fake(args,**kwargs):
            out=Path(args[args.index('--out-dir')+1])
            out.mkdir()
            WORKER.atomic_json(out/'qa-report.json',{'passed':False,'level_judged':True})
            return SimpleNamespace(returncode=1)
        worker=WORKER.Worker(self.request,fake)
        WORKER.atomic_json(worker.job/'result.json',{'quality':{'defaultAssignable':True},'artifactPaths':{}})
        worker.aivis_qa()
        result=json.loads((worker.job/'result.json').read_text(encoding='utf-8'))
        self.assertFalse(result['quality']['defaultAssignable'])
        self.assertFalse(result['quality']['aivisPassed'])
        self.assertTrue((worker.job/'qa-aivis/qa-report.json').exists())
        original=WORKER.digest(worker.job/'qa-aivis/qa-report.json')
        worker.aivis_qa()
        self.assertEqual(WORKER.digest(worker.job/'qa-aivis/qa-report.json'),original)
        self.assertTrue((worker.job/'qa-aivis-attempt-2/qa-report.json').exists())

    def test_原ファイル削除後も保存済み原音から再開する(self):
        worker=WORKER.Worker(self.request)
        snapshot=worker.job/'source/original.wav'
        snapshot.parent.mkdir()
        snapshot.write_bytes(self.source.read_bytes())
        WORKER.atomic_json(worker.job/'source.json',{'source':self.request['source'],'snapshot':str(snapshot),'sha256':WORKER.digest(snapshot)})
        WORKER.atomic_json(worker.job/'prepared.json',{'dataset':worker.selected,'request':self.request,'artifacts':{str(snapshot):WORKER.digest(snapshot)}})
        self.source.unlink()
        resumed=WORKER.Worker(self.request,lambda *a,**k:self.fail('再処理禁止'))
        resumed.prepare()
        snapshot.write_bytes(b'broken')
        with self.assertRaisesRegex(ValueError,'SHA'):
            WORKER.Worker(self.request)

    def test_実二プロセスで競合を拒否して終了時に解放する(self):
        """強制終了後もOSが排他を解放し、次プロセスが取得できる。"""
        script=ROOT/'scripts/character-learning-worker.py'
        code="import importlib.util,sys,time; from pathlib import Path; sys.path.insert(0,str(Path(sys.argv[1]).parent)); s=importlib.util.spec_from_file_location('w',sys.argv[1]); w=importlib.util.module_from_spec(s); s.loader.exec_module(w); lock=w.RuntimeLock(sys.argv[2]); lock.__enter__(); print('locked',flush=True); time.sleep(30)"
        child=subprocess.Popen([sys.executable,'-u','-c',code,str(script),str(self.runtime)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        try:
            import threading
            ready=threading.Event()
            output=[]
            def read_ready():
                output.append(child.stdout.readline())
                ready.set()
            reader=threading.Thread(target=read_ready,daemon=True)
            reader.start()
            self.assertTrue(ready.wait(10),'ロック取得の通知がありません')
            self.assertEqual(output,['locked\n'])
            with self.assertRaisesRegex(RuntimeError,'使用中'):
                with WORKER.RuntimeLock(self.runtime):
                    self.fail('競合が許可されました')
        finally:
            child.kill()
            child.communicate(timeout=10)
        with WORKER.RuntimeLock(self.runtime):
            pass

    def test_既定割当には両QAの合格が必要(self):
        self.request.update(endpoint='http://127.0.0.1:11436',speaker=1)
        def fake(args,**kwargs):
            out=Path(args[args.index('--out-dir')+1])
            out.mkdir()
            WORKER.atomic_json(out/'qa-report.json',{'passed':True,'level_judged':True})
            return SimpleNamespace(returncode=0)
        worker=WORKER.Worker(self.request,fake)
        for sbv2 in (False,True):
            WORKER.atomic_json(worker.job/'result.json',{'quality':{'sbv2Passed':sbv2},'artifactPaths':{}})
            worker.aivis_qa()
            quality=json.loads((worker.job/'result.json').read_text(encoding='utf-8'))['quality']
            self.assertTrue(quality['aivisPassed'])
            self.assertEqual(quality['defaultAssignable'],sbv2)

    def test_起動PIDとQA情報の変更だけで再開を妨げない(self):
        self.request['ownerProcessId']=100
        worker=WORKER.Worker(self.request)
        WORKER.atomic_json(worker.job/'prepare-owner.json',dict(self.request))
        WORKER.atomic_json(worker.job/'prepared.json',{'dataset':worker.selected,'request':dict(self.request),'artifacts':{str(self.source):WORKER.digest(self.source)}})
        changed=dict(self.request,ownerProcessId=200,endpoint='http://127.0.0.1:11436',speaker=123)
        resumed=WORKER.Worker(changed,lambda *a,**k:self.fail('再処理禁止'))
        resumed.prepare()
        saved=json.loads((worker.job/'prepare-owner.json').read_text(encoding='utf-8'))
        self.assertEqual(WORKER.provenance_request(saved),WORKER.provenance_request(changed))

    def test_素材と固定学習条件の変更は再開を拒否する(self):
        worker=WORKER.Worker(self.request)
        WORKER.atomic_json(worker.job/'prepared.json',{'dataset':worker.selected,'request':dict(self.request),'artifacts':{str(self.source):WORKER.digest(self.source)}})
        changes={'source':dict(kind='file',value=str(self.runtime/'.venv/Scripts/python.exe')),'mode':'persona','characterName':'別名','stopAtStep':2000,'scriptsRoot':str(self.runtime),'ffmpeg':'別のffmpeg.exe'}
        for key,value in changes.items():
            with self.subTest(key=key):
                modified=WORKER.Worker(dict(self.request,**{key:value}))
                with self.assertRaisesRegex(ValueError,'request'):
                    modified.prepare()

    def test_ライブと不明な容量を拒否する(self):
        for status in ('is_live','is_upcoming','post_live'):
            self.assertIsNotNone(WORKER.youtube_limit({'duration':1,'filesize':10,'live_status':status}))
        self.assertIsNotNone(WORKER.youtube_limit({'duration':1,'filesize':10,'is_live':True},incomplete=True))
        self.assertIsNotNone(WORKER.youtube_limit({'duration':1}))
        self.assertIsNone(WORKER.youtube_limit({'duration':60,'filesize':1000,'live_status':'not_live'}))

    def test_CUDAとCPUを選択して容量不足を拒否する(self):
        probe=lambda:dict(available=True,freeBytes=WORKER.ASR_MIN_FREE_BYTES)
        self.assertEqual(WORKER.choose_asr_device('cuda',probe)['computeType'],'float16')
        self.assertEqual(WORKER.choose_asr_device('cpu',lambda:self.fail('CPUでGPU確認禁止'))['computeType'],'int8')
        with self.assertRaisesRegex(RuntimeError,'1.00GB'):
            WORKER.choose_asr_device('cuda',lambda:dict(available=True,freeBytes=1_000_000_000))
        with self.assertRaisesRegex(RuntimeError,'利用不可'):
            WORKER.choose_asr_device('cuda',lambda:dict(available=False,reason='利用不可'))
        with self.assertRaises(ValueError):
            WORKER.choose_asr_device('auto',probe)

    def test_GPU容量不足の後も同じ来歴で再開し回復したGPUを利用する(self):
        self.request['asrDevice']='cuda'
        calls=[]
        row=SimpleNamespace(start=0,end=2,text=' 試験 ',words=[])
        def factory(*args,**kwargs):
            calls.append(kwargs)
            return SimpleNamespace(transcribe=lambda *a,**k:(iter([row]),None))
        worker=WORKER.Worker(self.request)
        with self.assertRaisesRegex(RuntimeError,'CUDA文字起こしを開始できません'):
            worker.transcribe('fake.wav',factory,lambda:dict(available=True,freeBytes=1_000_000_000))
        self.assertEqual(calls,[])
        resumed=WORKER.Worker(dict(self.request))
        result=resumed.transcribe('fake.wav',factory,lambda:dict(available=True,freeBytes=8_000_000_000))
        self.assertEqual(result[0]['text'],'試験')
        self.assertEqual(calls[0]['device'],'cuda')
        proof=json.loads((resumed.job/'asr.json').read_text(encoding='utf-8'))
        self.assertEqual(proof['state'],'completed')
        self.assertEqual(proof['device'],'cuda')

    def test_GPU推論失敗でCPUへ切り替えない(self):
        self.request['asrDevice']='cuda'
        worker=WORKER.Worker(self.request)
        calls=[]
        def factory(*args,**kwargs):
            calls.append(kwargs)
            raise RuntimeError('GPU推論エラー')
        with self.assertRaisesRegex(RuntimeError,'GPU推論エラー'):
            worker.transcribe('fake.wav',factory,lambda:dict(available=True,freeBytes=8_000_000_000))
        self.assertEqual(len(calls),1)
        self.assertEqual(calls[0]['device'],'cuda')
        self.assertTrue(calls[0]['local_files_only'])
        proof=json.loads((worker.job/'asr.json').read_text(encoding='utf-8'))
        self.assertEqual(proof['state'],'failed')
        self.assertEqual(proof['device'],'cuda')

    def test_文字起こし成功時に選択理由と証跡を保存する(self):
        worker=WORKER.Worker(self.request)
        row=SimpleNamespace(start=0,end=2,text=' 試験 ',words=[])
        factory=lambda *a,**k:SimpleNamespace(transcribe=lambda *a,**k:(iter([row]),None))
        result=worker.transcribe('fake.wav',factory,lambda:self.fail('CPUでGPU確認禁止'))
        self.assertEqual(result[0]['text'],'試験')
        proof=json.loads((worker.job/'asr.json').read_text(encoding='utf-8'))
        self.assertEqual(proof['state'],'completed')
        self.assertEqual(proof['device'],'cpu')
        self.assertTrue(proof['reason'])
        status=json.loads((worker.job/'status.json').read_text(encoding='utf-8'))
        self.assertEqual(status['asr']['requested'],'cpu')

    def test_旧来歴のCPU省略と明示CPUは再開互換を保つ(self):
        worker=WORKER.Worker(self.request)
        WORKER.atomic_json(worker.job/'prepared.json',{'dataset':worker.selected,'request':dict(self.request),'artifacts':{str(self.source):WORKER.digest(self.source)}})
        resumed=WORKER.Worker(dict(self.request,asrDevice='cpu'))
        resumed.prepare()
        with self.assertRaisesRegex(ValueError,'request'):
            WORKER.Worker(dict(self.request,asrDevice='cuda')).prepare()

    def test_YouTube入力は取得工程へ進む(self):
        self.request['source']=dict(kind='url',value='https://youtu.be/eCaOrzfsvSs')
        deno=self.runtime/'tools/deno/deno.exe'
        deno.parent.mkdir(parents=True)
        deno.write_bytes(b'fake-deno')
        observed=[]
        worker=WORKER.Worker(self.request,lambda *a,**k:SimpleNamespace(returncode=1))
        snapshot=worker.job/'source/original.webm'
        class Downloader:
            def __init__(self, options):
                observed.append(options)
            def __enter__(self):
                return self
            def __exit__(self,*args):
                pass
            def extract_info(self,url,download):
                observed.append((url,download))
                snapshot.write_bytes(b'fakeaudio')
                return dict(duration=60,filesize=9,live_status='not_live')
            def prepare_filename(self,info):
                return str(snapshot)
        with patch.dict(sys.modules,{'soundfile':SimpleNamespace(),'yt_dlp':SimpleNamespace(YoutubeDL=Downloader)}),patch.object(WORKER.shutil,'which',return_value='ffmpeg.exe'):
            with self.assertRaisesRegex(RuntimeError,'decode-48000'):
                worker.prepare()
        self.assertTrue(observed[0]['noplaylist'])
        self.assertEqual(observed[0]['js_runtimes'],{'deno':{'path':str(deno)}})
        self.assertEqual(observed[0]['remote_components'],[])
        self.assertEqual(observed[1],('https://www.youtube.com/watch?v=eCaOrzfsvSs',True))
        self.assertTrue((worker.job/'source.json').exists())

    def test_YouTube専用Deno欠損は取得前に停止する(self):
        self.request['source']=dict(kind='url',value='https://youtu.be/eCaOrzfsvSs')
        worker=WORKER.Worker(self.request,lambda *a,**k:self.fail('取得処理禁止'))
        with patch.dict(sys.modules,{'soundfile':SimpleNamespace(),'yt_dlp':SimpleNamespace(YoutubeDL=lambda *a,**k:self.fail('取得前停止'))}),patch.object(WORKER.shutil,'which',return_value='ffmpeg.exe'):
            with self.assertRaisesRegex(ValueError,'専用Deno'):
                worker.prepare()

    def test_ローカル音声にはDenoを要求しない(self):
        worker=WORKER.Worker(self.request,lambda *a,**k:SimpleNamespace(returncode=1))
        with patch.dict(sys.modules,{'soundfile':SimpleNamespace()}),patch.object(WORKER.shutil,'which',return_value='ffmpeg.exe'):
            with self.assertRaisesRegex(RuntimeError,'decode-48000'):
                worker.prepare()
        self.assertTrue((worker.job/'source.json').exists())


if __name__=='__main__':
    unittest.main()
