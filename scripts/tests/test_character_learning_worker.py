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

ROOT=Path(__file__).resolve().parents[2]
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
        code="import importlib.util,sys,time; s=importlib.util.spec_from_file_location('w',sys.argv[1]); w=importlib.util.module_from_spec(s); s.loader.exec_module(w); lock=w.RuntimeLock(sys.argv[2]); lock.__enter__(); print('locked',flush=True); time.sleep(30)"
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


if __name__=='__main__':
    unittest.main()
