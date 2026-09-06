#!/usr/bin/env python3
"""Read-only UDS archive using paginated get_messages. No launch/prompt commands.
Use AFTER the required bare agent_cmd get_messages report, before container cleanup.
Writes responses verbatim as JSON, not terminal output. Source protocol: 4-byte BE.
"""
import argparse
import json
from pathlib import Path
import socket
import struct
import uuid


def query(path, command):
    command = dict(command, id=str(uuid.uuid4()))
    raw = json.dumps(command).encode()
    with socket.socket(socket.AF_UNIX) as sock:
        sock.settimeout(30)
        sock.connect(str(path))
        sock.sendall(struct.pack('>I', len(raw)) + raw)
        stream = sock.makefile('rb')
        def exact(n):
            data = stream.read(n)
            if len(data) != n:
                raise RuntimeError('Incomplete socket frame')
            return data
        while True:  # correlated response read, NOT agent status polling
            first = exact(1)
            if first == b'{':
                data = json.loads(first + stream.readline())
            else:
                size = struct.unpack('>I', first + exact(3))[0]
                if size > 16 * 1024 * 1024:
                    raise RuntimeError('Unexpected oversized frame')
                data = json.loads(exact(size))
            if data.get('type') == 'response' and data.get('id') == command['id']:
                return data


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--socket', required=True, type=Path)
    p.add_argument('--output', required=True, type=Path)
    p.add_argument('--page-size', type=int, default=20)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=False)
    def save(name, data):
        (a.output / name).write_text(json.dumps(data, indent=2) + '\n')
    for name in ('get_state', 'get_session_stats'):
        save(name + '.json', query(a.socket, {'type':name}))
    cursor = None
    seen = set()
    messages = {}
    page_index = 0
    while True:  # finite backwards history pagination, never a waiting loop
        command = {'type':'get_messages', 'count':a.page_size}
        if cursor is not None:
            command['before'] = cursor
        response = query(a.socket, command)
        save(f'page-{page_index:04d}.json', response)
        if not response.get('success'):
            raise RuntimeError('History request failed; raw response saved')
        data = response['data']
        for message in data.get('messages', []):
            messages[message['id']] = message
        page_index += 1
        if not data.get('hasMoreBefore'):
            break
        cursor = data.get('before')
        if not cursor or cursor in seen:
            raise RuntimeError('Missing/repeated history cursor')
        seen.add(cursor)
    ordered = sorted(messages.values(), key=lambda m:m.get('ordinal',0))
    save('messages.json', ordered)
    warnings = []
    for message in ordered:
        if message.get('collapsed') or message.get('truncated'):
            warnings.append({'message_id':message['id'],'reason':'collapsed/truncated; original content may need supplementary retrieval'})
    save('archive-summary.json', {'pages':page_index,'messages':len(ordered),'warnings':warnings,
        'limitations':['Committed history, not live token events or every workflow broadcast.',
                        'Inspect raw page/tool fields for truncation. Missing evidence stays explicitly missing.']})
    print(f'Archived {page_index} pages / {len(ordered)} messages to {a.output}')


if __name__ == '__main__':
    main()
