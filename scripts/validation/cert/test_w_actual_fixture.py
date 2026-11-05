# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Optional real candidate/loopback wrapper wiring, synthetic and not GPU evidence."""
import datetime as dt
import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import socket
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

import manifest_tool
import w_finalize
import w_runner


WRAPPER = r'''#!/usr/bin/env python3
import argparse,json,subprocess,sys
p=argparse.ArgumentParser()
for name in ('candidate','url','prompt','cli-log','wrapper-log','max-tokens','count','model','extra-body-json'):
    p.add_argument('--'+name,required=True)
p.add_argument('--streaming',action='store_true'); p.add_argument('--strict-reasoning-truncation',action='store_true'); p.add_argument('--ci-safety',action='store_true')
a=p.parse_args()
cmd=[a.candidate,'--url',a.url,'--api-key','dummy','--scenario','W','--num-requests',a.count,'--concurrency','2',
     '--prompts',a.prompt,'--data-log',a.cli_log,'--model',a.model,'--max-tokens',a.max_tokens,'--mode','chat',
     '--warmup-requests','0','--streaming','--extra-body-json',a.extra_body_json]
if a.strict_reasoning_truncation: cmd+=['--strict-reasoning-truncation']
if a.ci_safety: cmd+=['--ci-safety']
r=subprocess.run(cmd,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
rows=[json.loads(x) for x in open(a.cli_log) if x.strip()]
req=[x for x in rows if '.request.' in x.get('schema_version','') and x.get('phase')=='measure']
s=next(x for x in rows if '.summary.' in x.get('schema_version',''))
fields=['seq','ttft_s','ttft_visible_answer_s','ttft_source','outcome','answer_completed','usage_missing']
out=[]
for row in req:
    item={k:row.get(k) for k in fields}
    if item['outcome']=='reasoning_truncated': item['warning']='truncated and unanswered'
    out.append(item)
summary={k:s[k] for k in ['attempted','answered_requests','successes','errors','usage_missing_count']}
summary['reasoning_truncated']=s['ttft_coverage']['reasoning_truncated']
with open(a.wrapper_log,'w') as f: json.dump({'cli_argv':cmd,'cli_exit_code':r.returncode,'requests':out,'summary':summary},f)
sys.exit(r.returncode)
'''


class Fixture(BaseHTTPRequestHandler):
    def log_message(self, *_args): pass
    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        low = request.get('max_tokens', 0) == 128
        self.send_response(200); self.send_header('Content-Type', 'text/event-stream'); self.end_headers()
        chunks = [{'choices': [{'index': 0, 'delta': {'role': 'assistant'}}]},
                  {'choices': [{'index': 0, 'delta': {'reasoning_content': 'Thinking.'}}]}]
        if not low: chunks.append({'choices': [{'index': 0, 'delta': {'content': 'Answer.'}}]})
        chunks += [{'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'length' if low else 'stop'}]},
                   {'choices': [], 'usage': {'prompt_tokens': 5, 'completion_tokens': 3, 'total_tokens': 8}}]
        for chunk in chunks:
            self.wfile.write(('data: ' + json.dumps(chunk) + '\n\n').encode()); self.wfile.flush()
        self.wfile.write(b'data: [DONE]\n\n'); self.wfile.flush()


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0)); return sock.getsockname()[1]


@unittest.skipUnless(os.getenv('CERT_TEST_LLM_BIN'), 'actual candidate fixture binary not supplied')
class WActualFixture(unittest.TestCase):
    def test_collect_with_actual_cli_and_synthetic_wrapper_server(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); ev = root / 'evidence'; ev.mkdir(); (root / 'results').mkdir()
            (ev / 'gpu.csv').write_text('NVIDIA H100, fixture-driver\n'); (ev / 'binary-hashes.txt').write_text('fixture')
            (ev / 'models.json').write_text(json.dumps({'data': [{'id': 'Qwen/Qwen3-8B'}]}))
            binary = Path(os.environ['CERT_TEST_LLM_BIN']).resolve(); prompt = root / 'prompt.jsonl'; prompt.write_text('{"prompt":"hello"}\n')
            wrapper = root / 'wrapper'; wrapper.write_text(WRAPPER); wrapper.chmod(0o755)
            manifest = root / 'manifest.json'; manifest.write_text('{}')
            server = ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
            thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
            config = {'image': 'fixture', 'model': 'Qwen/Qwen3-8B', 'revision': 'r', 'port': server.server_port, 'proxy_port': free_port(),
                      'owner': 'fixture', 'evidence_link': 'fixture', 'wrapper': {'path': str(wrapper), 'sha256': manifest_tool.sha256_file(wrapper), 'version': 'fixture'},
                      'prompt_sets': {'default': {'path': str(prompt), 'sha256': manifest_tool.sha256_file(prompt)}},
                      'wrapper_args': ['--candidate','{candidate}','--url','{url}','--prompt','{prompt}','--cli-log','{cli_log}',
                                       '--wrapper-log','{wrapper_log}','--max-tokens','{max_tokens}','--count','{count}','--model','{model}',
                                       '--streaming','--extra-body-json','{"chat_template_kwargs":{"enable_thinking":true}}'],
                      'strict_args': ['--strict-reasoning-truncation'], 'safety_args': ['--ci-safety']}
            argv = ['test','--out',str(root),'--candidate',str(binary),'--baseline',str(binary),'--manifest',str(manifest),'--start-utc','2026-01-01T00:00:00Z']
            try:
                with patch('sys.argv',argv), patch.dict(os.environ, {'CUDA_VISIBLE_DEVICES':'0'}), \
                     patch.object(w_runner.manifest_tool,'load_manifest',return_value=({'candidate_sha':'a'*40},config)):
                    code = w_runner.main()
                    details = '\n'.join(path.read_text(errors='replace') for path in ev.glob('w-*-stderr.txt'))
                    self.assertEqual(code, 0, details)
                self.assertTrue((ev / 'w-collection.json').is_file())
                self.assertFalse((root / 'results/BC-INTEG-001.json').exists())
                with self.assertRaises(OSError): w_finalize.finalize(root, root / 'missing-signoff')
                (ev / 'cleanup-verified.json').write_text('{"cleanup":"verified"}')
                (ev / 'server.log').write_text('synthetic server'); (ev / 'server-stop.txt').write_text('synthetic stop')
                fake = {'team': 'Metrum Bench platform', 'approved': True, 'candidate_sha': 'a' * 40,
                        'signer': 'synthetic fixture only', 'signed_utc': dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z'),
                        'evidence_link': 'synthetic fixture', 'wrapper_sha256': manifest_tool.sha256_file(wrapper),
                        'collection_sha256': manifest_tool.sha256_file(ev / 'w-collection.json'),
                        'cleanup_sha256': manifest_tool.sha256_file(ev / 'cleanup-verified.json'),
                        'server_log_sha256': manifest_tool.sha256_file(ev / 'server.log')}
                signoff = root / 'fixture-signoff'; signoff.write_text(json.dumps(fake))
                w_finalize.finalize(root, signoff)
                self.assertEqual(json.loads((root / 'results/BC-INTEG-001.json').read_text())['result'], 'PASS')
            finally:
                server.shutdown(); server.server_close(); thread.join()


if __name__ == '__main__': unittest.main()
