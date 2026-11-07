#!/usr/bin/python3 -I
# Spike S2 (throwaway): summarise one results dir (L3: shell.jsonl, gtk.jsonl, ghostty.jsonl, xev.log) or one L1
# output file, per scenario: what the prefix did, how long it took, and what the focused client received.
import json
import re
import sys
from pathlib import Path


def load_jsonl(text):
    out = []
    for line in text.splitlines():
        line = line.strip()
        if line.startswith('{'):
            try:
                out.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    return out


def sections(text):
    parts, cur = {}, None
    for line in text.splitlines():
        m = re.match(r'^=== (\S+)', line)
        if m:
            cur = m.group(1)
            parts[cur] = []
        elif cur:
            parts[cur].append(line)
    return {k: '\n'.join(v) for k, v in parts.items()}


def xev_events(text):
    evs = []
    for block in text.split('\n\n'):
        m = re.match(r'(KeyPress|KeyRelease|FocusIn|FocusOut|KeymapNotify) event', block.strip())
        if not m:
            continue
        t = re.search(r'time (\d+)', block)
        k = re.search(r'keysym 0x[0-9a-f]+, (\w+)\)', block)
        mode = re.search(r'mode (\w+), detail (\w+)', block)
        evs.append({'kind': m.group(1), 'xtime': int(t.group(1)) if t else None,
                    'key': k.group(1) if k else None, 'mode': mode.groups() if mode else None})
    return evs


WL_RE = re.compile(r'^\[\s*(\d+\.\d+)\].*?(wl_keyboard#\d+\.(enter|leave|key|modifiers)|'
                   r'zwp_text_input_v3#\d+\.(enter|leave|commit_string|preedit_string|enable|disable))\((.*)\)')


def wl_events(text):
    evs, offs = [], []
    for line in text.splitlines():
        m = WL_RE.match(line)
        if not m:
            continue
        ts = float(m.group(1))
        name = m.group(3) or m.group(4)
        args = m.group(5)
        e = {'dbg': ts, 'kind': ('ti.' if m.group(4) else 'kb.') + name, 'args': args}
        if name == 'key':
            a = [x.strip() for x in args.split(',')]
            e['time'] = int(a[1])
            e['key'] = int(a[2])
            e['state'] = int(a[3])
            offs.append(ts - e['time'])
        evs.append(e)
    off = sorted(offs)[len(offs) // 2] if offs else None
    for e in evs:
        e['t'] = (e['dbg'] - off) * 1000 if off is not None else None
    return [e for e in evs if e['t'] is not None]


def main(path):
    p = Path(path)
    if p.is_dir():
        shell = load_jsonl((p / 'shell.jsonl').read_text()) if (p / 'shell.jsonl').exists() else []
        gtk = load_jsonl((p / 'gtk.jsonl').read_text()) if (p / 'gtk.jsonl').exists() else []
        ghostty = load_jsonl((p / 'ghostty.jsonl').read_text()) if (p / 'ghostty.jsonl').exists() else []
        xev = xev_events((p / 'xev.log').read_text(errors='replace')) if (p / 'xev.log').exists() else []
        wl = wl_events((p / 'gtk-wayland-keyboard.log').read_text(errors='replace')) if (p / 'gtk-wayland-keyboard.log').exists() else []
    else:
        s = sections(p.read_text(errors='replace'))
        shell, gtk, ghostty, xev = load_jsonl(s.get('shell.jsonl', '')), load_jsonl(s.get('gtk.jsonl', '')), [], []
        wl = wl_events(s.get('gtk-wayland.log', ''))

    for r in shell:
        if r.get('ev') in ('note', 'error') and r.get('what') in ('start', 'ibus', 'done', 'setup') or \
                r.get('ev') == 'error':
            print('#', json.dumps(r)[:600])

    scen = []
    for r in shell:
        if r['ev'] == 'scenario-start':
            scen.append({'name': r['scenario'], 'target': r.get('target'), 't0': r['t'], 'recs': []})
        elif scen and r['ev'] == 'scenario-end':
            scen[-1]['t1'] = r['t']
        elif scen and 't1' not in scen[-1]:
            scen[-1]['recs'].append(r)

    for sc in scen:
        t0, t1 = sc['t0'], sc.get('t1', 1e30)
        print(f"\n== {sc['target']} / {sc['name']}")
        for r in sc['recs']:
            ev = r['ev']
            if ev == 'inject':
                continue
            if ev == 'armed':
                print(f"  armed mode={r['mode']} accelLag={r['accelLagUs']}us armLag={r['armLagUs']}us "
                      f"pushModal={r['pushModalUs']}us im={r.get('im')} altHeldAtArm={r['altHeldAtArm']} "
                      f"focus={r['keyFocusIsCatcher']} actionMode={r['actionMode']}")
            elif ev in ('grab-press', 'grab-release'):
                print(f"  {ev} sym={r['sym']:#x} uni={r['uni']!r} state={r['state']:#x} im={r['im']} "
                      f"lag={r['lagUs']}us revoked={r['revoked']}")
            elif ev == 'command':
                print(f"  command sym={r['sym']:#x} lag={r['lagUs']}us sinceArm={r['sinceArmUs']}us im={r['im']}")
            else:
                d = {k: v for k, v in r.items() if k not in ('t', 'scenario', 'target', 'ev')}
                print(f'  {ev} {json.dumps(d)[:300]}')
        inj = [r for r in sc['recs'] if r['ev'] == 'inject']
        if inj:
            print('  injected: ' + ' '.join(f"{r['code']}{'v' if r['down'] else '^'}@{(r['t'] - t0) / 1000:.1f}"
                                            for r in inj))
        g = [e for e in gtk if t0 <= e['t'] * 1e6 <= t1]
        if g:
            print('  gtk: ' + '; '.join(
                f"{e['kind']}{'=' + repr(e.get('text')) if 'text' in e else ''}"
                f"{' ' + str(e.get('name')) if e.get('name') else ''}"
                f"{' ' + str(e.get('value')) if 'value' in e else ''}@{(e['t'] * 1e6 - t0) / 1000:.1f}"
                for e in g))
        w = [e for e in wl if t0 <= e['t'] <= t1]
        if w:
            print('  wl: ' + '; '.join(
                f"{e['kind']}{' k%d%s' % (e['key'], 'v' if e['state'] else '^') if 'key' in e else ''}"
                f"@{(e['t'] - t0) / 1000:.1f}" for e in w))
        gh = [e for e in ghostty if t0 <= e['t'] * 1e6 <= t1]
        if gh:
            print('  ghostty: ' + '; '.join(f"{e['bytes']!r}@{(e['t'] * 1e6 - t0) / 1000:.1f}" for e in gh))
        x = [e for e in xev if e['xtime'] is None or t0 / 1000 <= e['xtime'] <= t1 / 1000]
        x = [e for e in x if e['xtime'] is not None]
        if x:
            print('  xev: ' + '; '.join(f"{e['kind']}{' ' + e['key'] if e['key'] else ''}"
                                        f"{' ' + '/'.join(e['mode']) if e['mode'] else ''}"
                                        f"@{e['xtime'] - t0 / 1000:.1f}" for e in x))


if __name__ == '__main__':
    main(sys.argv[1])
