"""The unit lint for vase-gnome's systemd --user unit (wayfinder §3.6, S10).

    python3 -I tools/linux/lint-unit.py [TEMPLATE]          lint both renderings (default vase-gnome/systemd/...)
    python3 -I tools/linux/lint-unit.py --render shipped|test [EXEC]   print one rendering
    python3 -I tools/linux/lint-unit.py --self-test         check that the lint refuses what it should

The rules, from S10's measurements of a user unit on Ubuntu 26.04 with the user-namespace restriction:
- no namespace option at all: each is a silent no-op, a logged no-op or a unit that won't start, and the no-ops
  relabel the daemon `unprivileged_userns`, which would hide the vase AppArmor profile from doctor;
- no capability option: a user manager can't shrink the bounding set, so the unit won't start (218/CAPABILITIES);
- exactly the seccomp option set S10 measured together (p01), plus LimitCORE=1, nothing missing;
- SystemCallErrorNumber=EPERM in the shipped unit, absent in a test unit (the default kill shows a missing call);
- the lifecycle: PartOf=, After= and WantedBy=graphical-session.target, Restart=on-failure, no socket activation
  (it would make systemd the socket's peer and break the extension's peer check, F13).
"""

import os
import re
import sys

DEFAULT_TEMPLATE = 'vase-gnome/systemd/vase-gnome.service.in'
SHIPPED_EXEC = '/usr/local/libexec/vase/vase-gnome'

# Every option that needs a mount, user or other namespace, or CAP_SYS_ADMIN, in a user manager (systemd 259
# exec_needs_cap_sys_admin and its neighbours, as S10 read them), plus the logged no-ops.
NAMESPACE_OPTIONS = {
    'PrivateTmp', 'PrivateDevices', 'PrivateNetwork', 'PrivateIPC', 'PrivateMounts', 'PrivateUsers', 'PrivatePIDs',
    'ProtectSystem', 'ProtectHome', 'ProtectKernelTunables', 'ProtectKernelModules', 'ProtectKernelLogs',
    'ProtectControlGroups', 'ProtectClock', 'ProtectHostname', 'ProtectProc', 'ProcSubset', 'RestrictFileSystems',
    'ReadOnlyPaths', 'ReadWritePaths', 'InaccessiblePaths', 'ExecPaths', 'NoExecPaths', 'TemporaryFileSystem',
    'BindPaths', 'BindReadOnlyPaths', 'MountAPIVFS', 'MountFlags', 'MountImages', 'ExtensionImages',
    'ExtensionDirectories', 'RootDirectory', 'RootImage', 'RootEphemeral', 'NetworkNamespacePath',
    'IPCNamespacePath', 'DynamicUser', 'LogNamespace', 'IPAddressDeny', 'IPAddressAllow', 'JoinsNamespaceOf',
    'DelegateNamespaces',
}
CAPABILITY_OPTIONS = {'CapabilityBoundingSet', 'AmbientCapabilities', 'SecureBits', 'User', 'Group',
                      'SupplementaryGroups'}
REQUIRED_SERVICE = [
    ('NoNewPrivileges', 'yes'),
    ('RestrictAddressFamilies', 'AF_UNIX'),
    ('SystemCallFilter', '@system-service'),
    ('SystemCallFilter', '~@privileged @resources'),
    ('SystemCallArchitectures', 'native'),
    ('LockPersonality', 'yes'),
    ('MemoryDenyWriteExecute', 'yes'),
    ('RestrictNamespaces', 'yes'),
    ('RestrictRealtime', 'yes'),
    ('RestrictSUIDSGID', 'yes'),
    ('LimitCORE', '1'),
    ('Restart', 'on-failure'),
]
REQUIRED_UNIT = [('PartOf', 'graphical-session.target'), ('After', 'graphical-session.target')]
REQUIRED_INSTALL = [('WantedBy', 'graphical-session.target')]


def render(template, variant, exec_path=None):
    """The template with its two placeholders filled in."""
    if variant not in ('shipped', 'test'):
        raise ValueError(f'unknown variant {variant!r}')
    exec_path = exec_path or SHIPPED_EXEC
    syscall_error = 'SystemCallErrorNumber=EPERM' if variant == 'shipped' else ''
    return template.replace('{{EXEC}}', exec_path).replace('{{SYSCALL_ERROR}}', syscall_error)


def parse(text):
    """[(section, key, value, line number)], and the problems found reading it."""
    entries, problems, section = [], [], None
    for n, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line[0] in '#;':
            continue
        if re.fullmatch(r'\[[A-Za-z]+\]', line):
            section = line[1:-1]
            continue
        m = re.fullmatch(r'([A-Za-z][A-Za-z0-9]*)\s*=\s*(.*)', line)
        if not m or section is None:
            problems.append(f'line {n}: not a key=value line in a section: {raw!r}')
            continue
        entries.append((section, m.group(1), m.group(2).strip(), n))
    return entries, problems


def lint_rendering(text, variant):
    entries, problems = parse(text)
    if '{{' in text or '}}' in text:
        problems.append('a placeholder is left after rendering')
    sections = {s for s, _k, _v, _n in entries}
    for s in sorted(sections - {'Unit', 'Service', 'Install'}):
        problems.append(f'unexpected section [{s}] (no socket activation: F13)')
    for section, key, _value, n in entries:
        if key in NAMESPACE_OPTIONS:
            problems.append(f'line {n}: {key}= is a namespace option: in a user unit it confines nothing and '
                            'relabels the daemon unprivileged_userns (S10)')
        if key in CAPABILITY_OPTIONS:
            problems.append(f'line {n}: {key}= needs privileges a user manager lacks; the unit would not start (S10)')
        if key == 'Sockets' or key.startswith('Listen'):
            problems.append(f'line {n}: {key}= is socket activation; the daemon binds its own socket (F13)')
    have = [(s, k, v) for s, k, v, _n in entries]
    for section, required in (('Unit', REQUIRED_UNIT), ('Service', REQUIRED_SERVICE), ('Install', REQUIRED_INSTALL)):
        for key, value in required:
            if (section, key, value) not in have:
                problems.append(f'[{section}] lacks {key}={value}')
    errno = [v for s, k, v in have if s == 'Service' and k == 'SystemCallErrorNumber']
    if variant == 'shipped' and errno != ['EPERM']:
        problems.append(f'the shipped unit needs exactly SystemCallErrorNumber=EPERM, has {errno!r}')
    if variant == 'test' and errno:
        problems.append(f'a test unit keeps the default kill action, so it has no SystemCallErrorNumber=, has {errno!r}')
    execs = [v for s, k, v in have if s == 'Service' and k == 'ExecStart']
    if variant == 'shipped' and execs != [SHIPPED_EXEC]:
        problems.append(f'the shipped unit runs {SHIPPED_EXEC}, where the peer check expects it; has {execs!r}')
    return problems


def lint(template):
    problems = []
    for variant in ('shipped', 'test'):
        problems += [f'{variant}: {p}' for p in lint_rendering(render(template, variant, '/opt/vase-gnome'
                                                                      if variant == 'test' else None), variant)]
    return problems


def self_test():
    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..')
    with open(os.path.join(root, DEFAULT_TEMPLATE), encoding='utf-8') as f:
        good = f.read()
    service = '[Service]\n'
    cases = [
        ('the template as it is', good, 0),
        ('ProtectHome=read-only added', good.replace(service, service + 'ProtectHome=read-only\n'), 2),
        ('PrivateTmp=yes added', good.replace(service, service + 'PrivateTmp=yes\n'), 2),
        ('CapabilityBoundingSet= added', good.replace(service, service + 'CapabilityBoundingSet=\n'), 2),
        ('LimitCORE=1 dropped', good.replace('LimitCORE=1\n', ''), 2),
        ('MemoryDenyWriteExecute dropped', good.replace('MemoryDenyWriteExecute=yes\n', ''), 2),
        ('EPERM hard-coded in the template', good.replace('{{SYSCALL_ERROR}}', 'SystemCallErrorNumber=EPERM'), 1),
        ('PartOf dropped', good.replace('PartOf=graphical-session.target\n', ''), 2),
        ('a [Socket] section', good + '\n[Socket]\nListenStream=%t/vase/gnome.sock\n', 4),
    ]
    failed = 0
    for title, text, expected in cases:
        got = len(lint(text))
        ok = got == expected
        failed += not ok
        print(f'lint-unit self-test: {"ok" if ok else "FAIL"}: {title} ({got} problem(s), expected {expected})')
    return 1 if failed else 0


def main(argv):
    if argv[1:] == ['--self-test']:
        return self_test()
    if len(argv) >= 3 and argv[1] == '--render':
        with open(DEFAULT_TEMPLATE, encoding='utf-8') as f:
            sys.stdout.write(render(f.read(), argv[2], argv[3] if len(argv) > 3 else None))
        return 0
    path = argv[1] if len(argv) > 1 else DEFAULT_TEMPLATE
    with open(path, encoding='utf-8') as f:
        problems = lint(f.read())
    for p in problems:
        print(f'lint-unit: {path}: {p}', file=sys.stderr)
    if not problems:
        print(f'lint-unit: {path}: ok (shipped and test renderings)')
    return 1 if problems else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
