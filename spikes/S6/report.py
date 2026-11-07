"""Spike S6 (throwaway): turn the [s6] event lines of a run into the per-case summary in docs/linux-spike/S6.md.

    python3 -I spikes/S6/report.py RUN.LOG
"""

import json
import sys


def main(path):
    events = []
    lines = open(path, encoding='utf-8', errors='replace').read().split('\n')
    if '--- full log ---' in lines:
        lines = lines[lines.index('--- full log ---'):]
    for line in lines:
        if '[s6] ' not in line:
            continue
        try:
            events.append(json.loads(line.split('[s6] ', 1)[1]))
        except json.JSONDecodeError:
            pass

    cases, case = [], None
    for ev in events:
        if ev['ev'] == 'case':
            case = {'client': ev['client'], 'mode': ev['mode'], 'target': ev['target']['rect'], 'events': [],
                    'wid': None}
            cases.append(case)
        elif case is not None:
            if ev['ev'] == 'window-created' and case['wid'] is None and ev.get('windowType') == 0:
                case['wid'] = ev['id']
            case['events'].append(ev)

    print('| client | mode | order | set in configure | frame at first-frame | moves after first-frame | settled frame |')
    print('|---|---|---|---|---|---|---|')
    for c in cases:
        evs = [e for e in c['events'] if e.get('id') in (c['wid'], None) or e['ev'] in ('settled', 'error')]
        order = []
        for e in evs:
            name = {'window-created': 'created', 'configure': 'configure', 'first-frame': 'first-frame',
                    'shown': 'shown', 'late-connect': 'late-connect'}.get(e['ev'])
            if name and (not order or order[-1] != name):
                order.append(name)
        conf = next((e for e in evs if e['ev'] == 'configure'), None)
        ff = next((e for e in evs if e['ev'] == 'first-frame'), None)
        after = []
        seen_ff = False
        for e in evs:
            if e['ev'] == 'first-frame':
                seen_ff = True
            elif seen_ff and e['ev'] in ('position-changed', 'size-changed'):
                after.append(e['frame'])
        settled = next((e for e in evs if e['ev'] == 'settled'), {})
        err = next((e for e in evs if e['ev'] == 'error'), None)
        print('| {} | {} | {} | {} | {} | {} | {} |'.format(
            c['client'], c['mode'], ' → '.join(order),
            (conf.get('set') if conf else 'no configure') if not err else 'ERROR ' + err['error'],
            ff['frame'] if ff else '-', after if after else 'none', settled.get('frame')))


if __name__ == '__main__':
    main(sys.argv[1])
