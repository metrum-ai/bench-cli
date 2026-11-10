# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Session reset and first-request cold-counter controls; no live GPU evidence."""
import copy
import csv
import json
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import kv_r5_session_cold as cold
import kv_r5_vllm_runner as runner
import trace_proxy
from test_kv_r5_vllm import phase


def page(completed, query, hit=0, engine='0'):
    labels = f'engine="{engine}",model_name="fixture"'
    return (''.join(f'# TYPE {name} counter\n{name}{{{labels}}} {value}\n'
                   for name, value in ((cold.QUERY, query), (cold.HIT, hit)))
            + f'# TYPE {cold.SUCCESS} counter\n'
            + ''.join(f'{cold.SUCCESS}{{{labels},finished_reason="{reason}"}} {completed if reason == "stop" else 0}\n'
                      for reason in ('stop', 'length', 'abort', 'error', 'repetition'))
            + ''.join(f'# TYPE {name} gauge\n{name}{{{labels}}} 0\n' for name in (cold.RUNNING, cold.WAITING)))


class SessionColdControls(unittest.TestCase):
    def test_dev_exception_does_not_enable_events_for_sessions(self):
        for case in runner.CASES:
            argv = runner.server({'case': case, 'server_flags': ['--enable-prefix-caching']})
            self.assertEqual('VLLM_SERVER_DEV_MODE=1' in argv, case in ('BC-KV-002', 'BC-KV-008', 'BC-KV-009'))
            self.assertEqual('--kv-events-config' in argv, case in ('BC-KV-002', 'BC-KV-009'))

    def test_cold_requires_zero_hits_one_completion_and_stable_series(self):
        cold.compare(page(1, 10), page(2, 20), 'fixture', 1)
        for after in (page(2, 20, 1), page(3, 30), page(1, 20), page(2, 9), page(2, 20, engine='1')):
            with self.subTest(after=after), self.assertRaises(ValueError):
                cold.compare(page(1, 10), after, 'fixture', 1)

    def test_http_200_with_unsuccessful_reset_cannot_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)/'reset.json'
            for reply in ({'status': 200, 'body': '{"success":false}'},
                          {'status': 200, 'body': '{"success":1}'},
                          {'status': 503, 'body': '{"success":true}'}):
                path.write_text(json.dumps({'method': 'POST', 'path': '/reset_prefix_cache', **reply}))
                with self.assertRaises(ValueError):
                    cold.reset_ack(path)
            path.write_text(json.dumps({'method': 'POST', 'path': '/reset_prefix_cache', 'status': 200, 'body': '{"success":true}'}))
            cold.reset_ack(path)

    def fixture(self, root, name, prior):
        p = phase(root, name, count=13)
        requests = p['plan']['expected_requests']
        with (p['directory']/'requests.csv').open() as stream:
            rows = list(csv.DictReader(stream))
        for i, (request, row) in enumerate(zip(requests, rows)):
            request.update(warmup=i == 0, session_id='warmup' if i == 0 else 'session-'+str((i-1)//3),
                           turn=0 if i == 0 else (i-1)%3)
            row.update(warmup=str(request['warmup']).lower(), session_id=request['session_id'], turn=request['turn'])
        with (p['directory']/'requests.csv').open('w') as stream:
            writer = csv.DictWriter(stream, fieldnames=list(rows[0])); writer.writeheader(); writer.writerows(rows)
        p['plan']['reset_prefix_cache'] = True
        (p['directory']/'reset.json').write_text(json.dumps({'method': 'POST', 'path': '/reset_prefix_cache', 'status': 200, 'body': '{"success":true}'}))
        p['before'], p['after'] = page(prior, prior*10), page(prior+13, (prior+13)*10)
        for side, text in [('before', page(prior+1, (prior+1)*10)), ('after', page(prior+2, (prior+2)*10))]:
            (p['directory']/f'first-measured-{side}-000.prom').write_text(text)
        (p['directory']/'first-measured-cold.json').write_text(json.dumps({
            'schema_version': 'kv-session-cold.v1', 'request': 2, 'completed_before': prior+1,
            'before': 'first-measured-before-000.prom', 'after': 'first-measured-after-000.prom'}))
        return p

    def test_each_same_engine_stage_checks_its_first_measured_delta(self):
        with tempfile.TemporaryDirectory() as tmp:
            stages = [self.fixture(Path(tmp), 'stage-a', 0), self.fixture(Path(tmp), 'stage-b', 13)]
            result = runner.evaluate({'case': 'BC-KV-008', 'served_model': 'fixture'}, stages)
            for name in ('stage-a', 'stage-b'):
                self.assertTrue(result['phases'][name]['first_measured_counter_window'])
            # Wire usage still says cached_tokens=0. An independent hit rejects it.
            path = stages[1]['directory']/'first-measured-after-000.prom'
            path.write_text(page(15, 150, hit=1))
            stages[1]['after'] = page(26, 260, hit=1)
            with self.assertRaisesRegex(ValueError, 'not prefix-cold'):
                runner.evaluate({'case': 'BC-KV-008', 'served_model': 'fixture'}, stages)

    def test_missing_window_or_wrong_first_request_cannot_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = self.fixture(Path(tmp), 'stage', 0)
            path = p['directory']/'first-measured-cold.json'
            receipt = json.loads(path.read_text()); receipt['request'] = 3
            path.write_text(json.dumps(receipt))
            with self.assertRaisesRegex(ValueError, 'differs from frozen'):
                cold.verify(p, 'fixture', 0)
            path.unlink()
            with self.assertRaises(OSError):
                cold.verify(p, 'fixture', 0)

    def test_frozen_proxy_boundary_accounts_for_previous_stages(self):
        data = {'case': 'BC-KV-008', 'served_model': 'fixture', 'phases': [
            {'name': 'a', 'expected_requests': [{'warmup': True}, {'warmup': False}]},
            {'name': 'b', 'expected_requests': [{'warmup': True}, {'warmup': True}, {'warmup': False}]}]}
        self.assertEqual(runner.proxy_args(data, data['phases'][1]), [
            '--session-first-measured', '3', '--session-completed-before', '4', '--session-model', 'fixture'])
        self.assertEqual(runner.proxy_args({**data, 'case': 'BC-KV-001'}, data['phases'][0]), [])

    def test_real_http_recorder_retains_selected_boundary_pages(self):
        state = {'completed': 0}
        class Engine(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass
            def do_GET(self):
                raw = page(state['completed'], state['completed']*10).encode()
                self.send_response(200); self.send_header('Content-Length', str(len(raw))); self.end_headers(); self.wfile.write(raw)
            def do_POST(self):
                self.rfile.read(int(self.headers['Content-Length']))
                state['completed'] += 1
                raw = b'data: [DONE]\n\n'
                self.send_response(200); self.send_header('Content-Length', str(len(raw))); self.end_headers(); self.wfile.write(raw)
        engine = ThreadingHTTPServer(('127.0.0.1', 0), Engine)
        threading.Thread(target=engine.serve_forever, daemon=True).start()
        with tempfile.TemporaryDirectory() as tmp:
            recorder = cold.Recorder(Path(tmp)/'trace.jsonl', engine.server_port, 'fixture', 1, 0)
            proxy = ThreadingHTTPServer(('127.0.0.1', 0), cold.handler(trace_proxy.make_handler(engine.server_port, recorder), recorder))
            threading.Thread(target=proxy.serve_forever, daemon=True).start()
            try:
                conn = cold.http.client.HTTPConnection('127.0.0.1', proxy.server_port, timeout=5)
                conn.request('POST', '/v1/chat/completions', '{}', {'Content-Type': 'application/json'})
                response = conn.getresponse(); self.assertEqual(response.read(), b'data: [DONE]\n\n'); conn.close()
                receipt = json.loads((Path(tmp)/'first-measured-cold.json').read_text())
                self.assertEqual(receipt['request'], 1)
                self.assertTrue((Path(tmp)/receipt['before']).is_file())
                self.assertTrue((Path(tmp)/receipt['after']).is_file())
            finally:
                proxy.shutdown(); proxy.server_close(); recorder.stream.close()
                engine.shutdown(); engine.server_close()


if __name__ == '__main__':
    unittest.main()
