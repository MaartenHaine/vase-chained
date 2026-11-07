"""The metadata lint for the Linux Tiler's extension (docs/linux-tiler/devtest-security.md §4.4).

    python3 -I tools/linux/lint-metadata.py [EXTENSION_DIR]   lint one extension directory (default vase-gnome/extension)
    python3 -I tools/linux/lint-metadata.py --self-test       check that the lint refuses what it should

The rules:
- `uuid` is under the owner's domain;
- `shell-version` is exactly the supported majors: no future versions, which would load untested code;
- no `session-modes` key: the Tiler must not run on the lock screen (wayfinder §3.7, `unlock-dialog` rejected);
- a `settings-schema`, if any, is under org.gnome.shell.extensions;
- only known keys;
- the directory holds only reviewable text files (the review guidelines forbid binaries), and an extension.js.
"""

import json
import os
import re
import sys
import tempfile

OWNER_DOMAIN = 'maartenhaine.github.io'
SUPPORTED_SHELL_VERSIONS = ['50']
KNOWN_KEYS = {'uuid', 'name', 'description', 'shell-version', 'url', 'settings-schema', 'gettext-domain',
              'version-name', 'donations'}
REQUIRED_KEYS = {'uuid', 'name', 'description', 'shell-version'}
TEXT_SUFFIXES = {'.js', '.json', '.css', '.xml', '.md'}


def lint(directory):
    """Returns a list of problems; empty when the extension passes."""
    problems = []
    path = os.path.join(directory, 'metadata.json')
    try:
        with open(path, encoding='utf-8') as f:
            meta = json.load(f)
    except (OSError, ValueError) as e:
        return [f'{path}: {e}']
    if not isinstance(meta, dict):
        return [f'{path}: not a JSON object']

    for key in sorted(REQUIRED_KEYS - meta.keys()):
        problems.append(f'missing key {key!r}')
    for key in sorted(meta.keys() - KNOWN_KEYS):
        problems.append(f'unknown key {key!r}' + (': the Tiler must not run on the lock screen'
                                                  if key == 'session-modes' else ''))

    uuid = meta.get('uuid', '')
    if not isinstance(uuid, str) or not re.fullmatch(r'[A-Za-z0-9._-]+@' + re.escape(OWNER_DOMAIN), uuid):
        problems.append(f'uuid {uuid!r} is not NAME@{OWNER_DOMAIN}')
    if meta.get('shell-version') != SUPPORTED_SHELL_VERSIONS:
        problems.append(f'shell-version {meta.get("shell-version")!r} is not exactly {SUPPORTED_SHELL_VERSIONS!r}')
    schema = meta.get('settings-schema')
    if schema is not None and not (isinstance(schema, str) and schema.startswith('org.gnome.shell.extensions.')):
        problems.append(f'settings-schema {schema!r} is not under org.gnome.shell.extensions.')

    if not os.path.isfile(os.path.join(directory, 'extension.js')):
        problems.append('no extension.js')
    for root, _dirs, files in os.walk(directory):
        for name in files:
            file = os.path.join(root, name)
            rel = os.path.relpath(file, directory)
            if os.path.islink(file):
                problems.append(f'{rel}: a symlink')
                continue
            if os.path.splitext(name)[1] not in TEXT_SUFFIXES:
                problems.append(f'{rel}: not one of the reviewable file types {sorted(TEXT_SUFFIXES)}')
                continue
            with open(file, 'rb') as f:
                data = f.read()
            try:
                data.decode('utf-8')
            except UnicodeDecodeError:
                problems.append(f'{rel}: not UTF-8 text')
                continue
            if b'\0' in data:
                problems.append(f'{rel}: contains a NUL byte')
    return problems


def self_test():
    good = {'uuid': f'tiler@{OWNER_DOMAIN}', 'name': 'n', 'description': 'd', 'shell-version': ['50']}
    cases = [
        ('a good extension', good, {}, 0),
        ('a future shell version', dict(good, **{'shell-version': ['50', '51']}), {}, 1),
        ('a session-modes key', dict(good, **{'session-modes': ['user', 'unlock-dialog']}), {}, 1),
        ('a uuid under another domain', dict(good, uuid='tiler@example.com'), {}, 1),
        ('a schema outside org.gnome.shell.extensions', dict(good, **{'settings-schema': 'org.example'}), {}, 1),
        ('a missing name', {k: v for k, v in good.items() if k != 'name'}, {}, 1),
        ('a binary in the tree', good, {'helper.so': b'\x7fELF\0'}, 1),
    ]
    failed = 0
    for title, meta, extra, expected in cases:
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, 'metadata.json'), 'w', encoding='utf-8') as f:
                json.dump(meta, f)
            with open(os.path.join(d, 'extension.js'), 'w', encoding='utf-8') as f:
                f.write('export default class {}\n')
            for name, data in extra.items():
                with open(os.path.join(d, name), 'wb') as f:
                    f.write(data)
            got = len(lint(d))
        ok = got == expected
        failed += not ok
        print(f'lint-metadata self-test: {"ok" if ok else "FAIL"}: {title} ({got} problem(s), expected {expected})')
    return 1 if failed else 0


def main(argv):
    if argv[1:] == ['--self-test']:
        return self_test()
    directory = argv[1] if len(argv) > 1 else 'vase-gnome/extension'
    problems = lint(directory)
    for problem in problems:
        print(f'lint-metadata: {directory}: {problem}', file=sys.stderr)
    if not problems:
        print(f'lint-metadata: {directory}: ok')
    return 1 if problems else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
