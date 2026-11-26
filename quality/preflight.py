"""Bounded, non-executing repository inspection at an exact Git commit."""
import configparser
import hashlib
import json
import pathlib
import os
import re
import shutil
import subprocess
import sys
import urllib.request

MAX_FILES = 32
MAX_BYTES = 56000


def git(*args):
    return subprocess.check_output(['git', *args], timeout=15)


def scan(base, issue):
    if not re.fullmatch(r'(?:[0-9a-f]{40}|[0-9a-f]{64})', base):
        raise ValueError('exact base commit required')
    actual = git('rev-parse', '--verify', base + '^{commit}').decode('ascii').strip()
    if actual != base:
        raise ValueError('exact base commit required')
    # NUL-delimited tree records preserve literal paths and their exact object IDs.
    entries = {}
    undecodable_paths = False
    for record in git('ls-tree', '-r', '-z', base).split(b'\0'):
        if not record:
            continue
        metadata, raw_path = record.split(b'\t', 1)
        mode, _, oid = metadata.decode('ascii').split()
        try:
            path = raw_path.decode('utf-8')
        except UnicodeDecodeError:
            undecodable_paths = True
            continue
        entries[path] = (mode, oid)
    paths = list(entries)
    manifests = {'pyproject.toml', 'package.json', 'Cargo.toml', 'go.mod', 'build.gradle',
                 'build.gradle.kts', 'settings.gradle', 'settings.gradle.kts', 'constants.gradle',
                 'gradle.properties', 'Makefile', 'tox.ini', 'pytest.ini', '.gitmodules',
                 'AGENTS.md', 'CONTRIBUTING.md', 'README.md', 'requirements.txt'}
    names = set(re.findall(r'[A-Za-z0-9_.-]+\.(?:java|kt|py|rs|go|js|ts|tsx|c|cpp|h)', issue))
    wanted = [p for p in paths if pathlib.PurePosixPath(p).name in names]
    wanted += sorted((p for p in paths if pathlib.PurePosixPath(p).name in manifests), key=lambda p: (p.count('/'), p))
    wanted += [p for p in paths if p.startswith('.github/workflows/') and p.endswith(('.yml', '.yaml'))][:4]
    files = []
    inventory_incomplete = undecodable_paths
    used = 0
    for path in dict.fromkeys(wanted):
        if len(files) >= MAX_FILES or used >= MAX_BYTES:
            break
        # Reject symlinks/gitlinks; inspect tracked blobs, never local files.
        mode, oid = entries[path]
        if mode not in ('100644', '100755'):
            continue
        size = int(git('cat-file', '-s', oid))
        if size > 200000:
            inventory_incomplete = True
            continue
        raw = git('cat-file', 'blob', oid)
        limit = min(20000 if pathlib.PurePosixPath(path).name in names else 4000, MAX_BYTES-used)
        text = raw.decode('utf-8', 'replace')
        location = re.search(re.escape(pathlib.PurePosixPath(path).name)+r':(\d+)', issue)
        if location and len(raw)>limit:
            lines = text.splitlines(keepends=True)
            start = max(0, int(location[1])-65)
            text = ''.join(lines[start:])
        # The encoded text is valid UTF-8; ignore only an incomplete final codepoint.
        text = text.encode()[:limit].decode('utf-8', 'ignore')
        emitted = text.encode('utf-8')
        truncated = emitted != raw
        inventory_incomplete |= truncated
        files.append({'path': path, 'content': text, 'sha256': hashlib.sha256(raw).hexdigest(), 'truncated': truncated})
        used += len(emitted)
    unresolved = []
    modules = next((f for f in files if f['path'] == '.gitmodules'), None)
    parser = configparser.ConfigParser(interpolation=None)
    if modules is not None:
        try:
            if modules['truncated']:
                raise configparser.Error('truncated .gitmodules')
            parser.read_string(modules['content'])
        except configparser.Error:
            unresolved.append({'reason': 'unreadable or truncated .gitmodules; dependency constraints unresolved'})
            parser = configparser.ConfigParser(interpolation=None)
    accounted = set()
    # Read constants at the EXACT gitlink commit, not dependency default branch.
    # Credential-free GitHub raw HTTPS only, bounded files and timeouts.
    for index, section in enumerate(parser.sections()):
        path = parser.get(section, 'path', fallback='')
        url = parser.get(section, 'url', fallback='')
        match = re.fullmatch(r'https://github\.com/([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+?)(?:\.git)?/?', url)
        entry = entries.get(path)
        if not entry or entry[0] != '160000':
            continue
        revision = entry[1]
        accounted.add(path)
        if not match:
            unresolved.append({'path': path, 'revision': revision, 'reason': 'unsupported pinned dependency URL'})
            continue
        if index >= 2:
            inventory_incomplete = True
            unresolved.append({'path': path, 'revision': revision, 'reason': 'pinned dependency constants exceed inspection count budget'})
            continue
        remote = match[1]
        if len(files) >= MAX_FILES or used >= MAX_BYTES:
            inventory_incomplete = True
            unresolved.append({'path': path, 'revision': revision, 'reason': 'pinned dependency constants exceed inspection budget'})
            continue
        limit = min(8000, MAX_BYTES - used)
        reason = 'pinned dependency constants unavailable'
        found = False
        for name in ['constants.gradle']:
            try:
                req = urllib.request.Request(f'https://raw.githubusercontent.com/{remote}/{revision}/{name}')
                with urllib.request.urlopen(req, timeout=5) as res:
                    raw = res.read(limit + 1)
                if len(raw) > limit:
                    inventory_incomplete = True
                    reason = 'pinned dependency constants exceed inspection budget'
                    continue
                files.append({'path': f'{path}/{name}', 'content': raw.decode(), 'dependency_revision': revision,
                              'sha256': hashlib.sha256(raw).hexdigest(), 'truncated': False})
                used += len(raw)
                found = True
            except Exception:
                pass
        if not found:
            unresolved.append({'path': path, 'revision': revision, 'reason': reason})
    # The tree is authoritative even if .gitmodules was missing or unreadable.
    # Bound additional diagnostics without hiding the number of omitted gitlinks.
    missing = 0
    for path, (mode, revision) in entries.items():
        if mode == '160000' and path not in accounted:
            missing += 1
            if missing <= MAX_FILES:
                unresolved.append({'path': path, 'revision': revision, 'reason': 'pinned dependency configuration unavailable'})
    if missing > MAX_FILES:
        unresolved.append({'reason': 'additional unconfigured gitlinks omitted from diagnostics',
                           'omitted_count': missing - MAX_FILES})
    inventory_incomplete |= bool(unresolved)
    sdk = []
    desugaring = []
    for f in files:
        for value in re.findall(r'minSdk(?:Version)?\s*(?:[=:]\s*|\(\s*)?(\d+)', f['content']):
            sdk.append({'value': int(value), 'path': f['path'], 'revision': f.get('dependency_revision', base)})
        if re.search(r'coreLibraryDesugaring(?:Enabled)?\s*(?:[=(]\s*)?true|isCoreLibraryDesugaringEnabled\s*=\s*true', f['content']):
            desugaring.append(f['path'])
    tools = {}
    for tool in ['python3', 'java', 'javac', 'node', 'npm', 'cargo', 'go', 'gradle']:
        executable = shutil.which(tool)
        if executable:
            try:
                out = subprocess.run([executable, '-version' if tool in ('java', 'javac') else '--version'], capture_output=True, timeout=3, text=True)
                tools[tool] = (out.stdout+out.stderr).strip()[:180]
            except Exception:
                tools[tool] = 'present; version probe unavailable'
        else:
            tools[tool] = 'missing'
    tools['android_sdk'] = 'present' if any((pathlib.Path(os.environ.get(k, '/nonexistent'))/'platforms').is_dir() for k in ['ANDROID_HOME', 'ANDROID_SDK_ROOT']) else 'missing'
    return {'base': base, 'files': files, 'tools': tools, 'min_sdk': sdk, 'library_desugaring_evidence': desugaring,
            'unresolved_dependencies': unresolved, 'inventory_truncated': inventory_incomplete or len(wanted) > MAX_FILES or used >= MAX_BYTES,
            'test_paths': [p for p in paths if re.search(r'(^|/)(tests?|__tests__)(/|\.)|test_', p)][:60]}


if __name__ == '__main__':
    print(json.dumps(scan(sys.argv[1], sys.argv[2])))
