"""Spike S4 (throwaway): turns the [s4] lines of a run log into tables. Usage: python3 -I summarize.py LOG..."""

import json
import re
import sys


def lines(path):
    with open(path, encoding='utf-8', errors='replace') as f:
        for line in f:
            m = re.search(r'\[s4\] (\{.*\})\s*$', line)
            if m:
                yield json.loads(m.group(1))


def rect(r):
    return f"{r['x']},{r['y']} {r['w']}x{r['h']}"


def main(paths):
    for path in paths:
        print(f'## {path}')
        for e in lines(path):
            what = e['what']
            if what == 'env':
                print(f"env: layout={e['layout']} load={e['loadavg']} features={e['experimentalFeatures']}")
            elif what in ('state-at-start', 'state-after-apply'):
                print(f'{what}: ' + '; '.join(
                    f"{','.join(l['connectors'])} at {l['x']},{l['y']} scale {l['scale']}{' primary' if l['primary'] else ''}"
                    for l in e['logical']))
                for m in e['monitors']:
                    for mode in m['modes']:
                        print(f"  {m['connector']} {mode['id']} supported scales {mode['supportedScales']}")
            elif what in ('monitors-at-start', 'monitors-after-apply'):
                print(f"{what}: primaryIndex {e['primaryIndex']}, stage {e['stage']['w']}x{e['stage']['h']}; " + '; '.join(
                    f"[{m['index']}] {m['x']},{m['y']} {m['width']}x{m['height']} scale {m['displayScale']} "
                    f"geometry_scale {m.get('geometryScale')} work area {rect(m['workArea'])}" for m in e['monitors']))
            elif what == 'apply':
                print(f"ApplyMonitorsConfig(scale {e['scale0']}) to monitors-changed: {e['ms']} ms")
            elif what == 'xwayland-root':
                print(f"X11 root: {e['root']}; stage {e['stage']['w']}x{e['stage']['h']}")
            elif what == 'mapped':
                print(f"\n### {e['client']} ({e['kind']}, wm_class {e['wmClass']}, x11={e['x11']}): window "
                      f"{e['createdMs']} ms, first frame {e['firstFrameMs']} ms after spawn; frame {rect(e['frame'])}, "
                      f"buffer {rect(e['buffer'])}, min {e['minSize']}, max {e['maxSize']}")
                print('| step | mode | scale | target | after move_frame | after move_resize_frame (sync) | final | '
                      'delta w,h | settled at ms | last change ms | size changes | showed older rect |')
                print('|---|---|---|---|---|---|---|---|---|---|---|---|')
            elif what == 'x11-hints':
                hints = ' '.join(h.strip() for h in e['hints'].split('\n')[1:] if h.strip())
                print(f'WM_NORMAL_HINTS: {hints}')
            elif what == 'step':
                d = e['delta']
                print(f"| {e['label']} | {e['mode']} | {e['scaleAfter']} | {rect(e['target'])} | {rect(e['afterMove'])} | "
                      f"{rect(e['afterCall'])} | {rect(e['final'])} | {d['w']},{d['h']}"
                      f"{'' if d['x'] == 0 and d['y'] == 0 else f' (pos {d[chr(120)]},{d[chr(121)]})'} | "
                      f"{e['settledAt']} | {e['settleMs']}{' TIMEOUT' if e['timedOut'] else ''} | {e['sizeChanges']} | "
                      f"{'JUMP BACK at ' + str([ev['t'] for ev in e['events'] if ev['t'] > e['tSecond'] and ev['x'] == e['first']['x'] and ev['y'] == e['first']['y']]) if e.get('jumpBack') else ('move_frame only' if e['visitedFirst'] else '')} |")
            elif what in ('client-error', 'apply-error'):
                print(f'{what}: {e}')
        print()




def stats(paths):
    """Settle time (ms to the final rect) per client and monitor scale, over move_frame + move_resize_frame steps that changed the size."""
    rows = {}
    for path in paths:
        for e in lines(path):
            if e['what'] == 'step' and e['sizeChanges'] > 0 and e['mode'] == 'mf+mrf':
                rows.setdefault((path.rsplit('/', 1)[-1].removesuffix('.log'), e['client'], e['scaleAfter']), []).append(e['settledAt'])
    print('| run | client | monitor scale | n | min | median | max (ms) |')
    print('|---|---|---|---|---|---|---|')
    for (run, client, scale), v in sorted(rows.items()):
        v.sort()
        print(f'| {run} | {client} | {scale} | {len(v)} | {v[0]} | {v[len(v) // 2]} | {v[-1]} |')


if __name__ == '__main__':
    if sys.argv[1:2] == ['--stats']:
        stats(sys.argv[2:])
    else:
        main(sys.argv[1:])
