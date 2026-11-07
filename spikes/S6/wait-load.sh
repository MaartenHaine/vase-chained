#!/bin/sh
# Wait while the host's 1-minute load average is over 40 (the shared-host rule), then return.
while awk '{ exit !($1 > 40) }' /proc/loadavg; do
	printf 'wait-load: load %s, waiting\n' "$(cut -d' ' -f1 /proc/loadavg)" >&2
	sleep 30
done
