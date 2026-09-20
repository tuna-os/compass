import json
import os
import pathlib
import shutil
import socket
import struct
import subprocess
import sys
sys.path.insert(0, '/src/scripts/bench')
import session

out = pathlib.Path('/results')
shutil.copytree('/src/crates/compass-testkit/corpus/desktop-entries/real', out / 'corpus/applications')
cpp_env, rust_env = session.profile('cpp'), session.profile('rust')
cpp_env['QT_QPA_PLATFORM'] = 'offscreen'
config = pathlib.Path(rust_env['XDG_CONFIG_HOME']) / 'vicinae'
config.mkdir()
(config / 'vicinae.json').write_text(json.dumps({'launcher': {'max_results': 10000}}))
cpp_socket = pathlib.Path(cpp_env['XDG_RUNTIME_DIR']) / 'vicinae/vicinae.sock'
rust = '/port/vicinae'

def query(text):
    body = json.dumps({'jsonrpc': '2.0', 'id': 1, 'method': 'Ipc/rootQuery',
        'params': {'q': text, 'params': {'limit': 0, 'providerId': 'applications', 'includeDisabled': False}}}).encode()
    with socket.socket(socket.AF_UNIX) as stream:
        stream.settimeout(5)
        stream.connect(str(cpp_socket))
        stream.sendall(struct.pack('=I', len(body)) + body)
        def read(size):
            result = b''
            while len(result) < size:
                part = stream.recv(size - len(result))
                if not part:
                    raise RuntimeError('truncated frame')
                result += part
            return result
        size = struct.unpack('=I', read(4))[0]
        assert size < 16 * 1024 * 1024
        response = json.loads(read(size))
        assert 'error' not in response, response
        return response['result']

with session.processes() as (children, start):
    start('upstream', ['/usr/bin/vicinae', 'server', '--no-extension-runtime'], cpp_env)
    session.wait_ready(lambda: session.rpc(cpp_socket, 'Ipc/ping'), children)
    start('rust', [rust, 'serve', '--no-hotkey'], rust_env)
    session.wait_ready(lambda: subprocess.run([rust, 'ping'], env=rust_env, capture_output=True).returncode == 0, children)
    results = []
    for text in ['', 'chrom', 'terminal', 'calculator', 'code', 'é', 'zzzznonexistent']:
        cpp = query(text)
        other = json.loads(subprocess.check_output([rust, 'query', '--json', text], env=rust_env))
        results.append({'query': text, 'cpp': cpp, 'rust': other})
        print(text, 'cpp', len(cpp), 'rust', len(other), flush=True)
    (out / 'queries.json').write_text(json.dumps(results, indent=2))
