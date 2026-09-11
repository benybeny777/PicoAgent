"""ComfyUI通信の入出力と、起動失敗・待機期限をGPUなしで検証する。"""
import io
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import Mock, patch
import urllib.error

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import comfy_client


class ComfyClientTests(unittest.TestCase):
    def test_prompt_posts_json_and_decodes_response(self):
        response = io.BytesIO(b'{"prompt_id":"fixture"}')
        with patch.object(comfy_client.urllib.request, 'urlopen', return_value=response) as opened:
            self.assertEqual(comfy_client.request_json('http://127.0.0.1:8188/prompt', {'prompt': '閉眼'}),
                             {'prompt_id': 'fixture'})
        request = opened.call_args.args[0]
        self.assertEqual(request.get_method(), 'POST')
        self.assertEqual(json.loads(request.data), {'prompt': '閉眼'})
        self.assertEqual(opened.call_args.kwargs['timeout'], 30)

    def test_dead_server_is_not_reported_ready(self):
        process = Mock(returncode=7)
        process.poll.return_value = 7
        with patch.object(comfy_client, 'request_json') as request:
            with self.assertRaisesRegex(RuntimeError, '7'):
                comfy_client.wait_for_server('http://127.0.0.1:8188', process, 10)
            request.assert_not_called()

    def test_unavailable_server_expires_without_real_sleep(self):
        process = Mock()
        process.poll.return_value = None
        with patch.object(comfy_client.time, 'monotonic', side_effect=[0, 0, 11]), \
             patch.object(comfy_client.time, 'sleep') as sleep, \
             patch.object(comfy_client, 'request_json', side_effect=urllib.error.URLError('fixture')):
            with self.assertRaises(TimeoutError):
                comfy_client.wait_for_server('http://127.0.0.1:8188', process, 10)
            sleep.assert_called_once_with(1)
