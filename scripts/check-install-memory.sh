#!/usr/bin/env bash
# Correctness only: all installer writes occur inside disposable Docker containers.
set -Eeuo pipefail
export LC_ALL=C

fail() { printf 'install-memory: %s\n' "$*" >&2; exit 1; }

probe() {
  python3 - "$1" "$2" <<'PY'
import json, os, pathlib, sys
limit = int(sys.argv[2]) * 1024 * 1024
cg = pathlib.Path('/sys/fs/cgroup')
assert (cg / 'memory.max').read_text().strip() == str(limit)
assert (cg / 'memory.swap.max').read_text().strip() == '0'
current = int((cg / 'memory.current').read_text())
mem = dict(line.split(':', 1) for line in pathlib.Path('/proc/meminfo').read_text().splitlines())
host_available = int(mem['MemAvailable'].split()[0]) * 1024
stat = os.statvfs('/fixture')
assert 0 < stat.f_files <= 4096, stat.f_files
remaining = max(0, limit - current)
result = dict(memory_max=limit, memory_current=current, memory_remaining=remaining,
              host_available=host_available, available=min(host_available, remaining),
              memory_swap_max=0, inodes=stat.f_files, free_inodes=stat.f_ffree,
              entries=stat.f_files-stat.f_ffree)
pathlib.Path(sys.argv[1]).write_text(json.dumps(result, indent=2) + '\n')
PY
}

check_report() {
  python3 - "$1" "$2" "$3" <<'PY'
import json, pathlib, sys, tomllib
report = json.loads(pathlib.Path(sys.argv[1]).read_text())
expected = sys.argv[2] == 'true'
assert type(report['low_memory']) is bool, report
assert report['low_memory'] is expected, report
assert report['low_memory_reason'].startswith(sys.argv[3] + ':'), report
assert report['scan_started'] is False, report
config = tomllib.loads(pathlib.Path('/etc/freespace/config.toml').read_text())
assert type(config['low_memory']) is bool and config['low_memory'] is expected, config
assert not list(pathlib.Path('/snapshots').rglob('*.db')), 'no-start created a database'
print(json.dumps(report, sort_keys=True))
PY
}

run_install() {
  local label=$1 expected=$2 reason=$3
  shift 3
  probe "/out/$backend-$label-before.json" "$memory_mib"
  /binary/freespace --backend "$backend" install-timer --no-start \
    --roots /fixture --snapshot-dir /snapshots --user nobody --json "$@" \
    >"/out/$backend-$label.json" 2>"/out/$backend-$label.stderr"
  probe "/out/$backend-$label-after.json" "$memory_mib"
  check_report "/out/$backend-$label.json" "$expected" "$reason"
}

container_checks() {
  [[ ${INSTALL_MEMORY_DISPOSABLE:-} = 1 && $EUID = 0 ]] || fail 'container-only mode'
  local memory_mib=$1 expected=$2 backend=$3
  mkdir -p /run/systemd/system /etc/systemd/system /snapshots
  touch /fixture/file
  mkdir /fixture/empty
  run_install initial "$expected" auto
  run_install preserved-auto "$expected" preserved
  # Explicit false must remain a saved false even though 128 MiB auto chooses true.
  run_install explicit-false false explicit --low-memory false
  run_install preserved-false false preserved
  # A bare flag means true; saved true must also survive a 4 GiB rerun.
  run_install explicit-true true explicit --low-memory true
  run_install preserved-true true preserved
  run_install bare-true true explicit --low-memory
  run_install recalculate "$expected" auto --low-memory auto
  run_install preserved-recalculated "$expected" preserved
  python3 - "/out/$backend-initial-before.json" "$backend" "$expected" <<'PY'
import json, pathlib, sys
probe = json.loads(pathlib.Path(sys.argv[1]).read_text())
coefficient = 1024 if sys.argv[2] == 'v2' else 384
estimate = 64*1024*1024 + probe['entries']*coefficient
budget = probe['available']//4
assert (estimate > budget) is (sys.argv[3] == 'true'), (probe, estimate, budget)
assert probe['available'] <= probe['memory_max'] - probe['memory_current']
print(json.dumps(dict(reference_estimate=estimate, quarter_available=budget,
                      backend=sys.argv[2], correctness_only=True)))
PY
  cp /etc/freespace/config.toml "/out/$backend-final-config.toml"
  cp /etc/systemd/system/freespace.service "/out/$backend.service"
  # The stub allows timer setup but fails if any service start slips past no-start.
  ! grep -E '(^| )(start|restart|try-restart)( |$)' /out/systemctl.log
}

host_checks() {
  [[ $# = 2 ]] || fail 'usage: check-install-memory.sh BINARY OUTPUT'
  local binary script output image context memory_mib expected backend
  binary=$(realpath -e "$1")
  script=$(realpath -e "$0")
  output=$(realpath -m "$2")
  mkdir -p "$output"
  context=$(mktemp -d)
  trap 'rm -rf -- "$context"' RETURN
  image="freespace-install-memory:${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-1}"
  cat >"$context/Dockerfile" <<'DOCKER'
FROM ubuntu:24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends bash python3 passwd ca-certificates && rm -rf /var/lib/apt/lists/*
RUN mkdir -p /usr/local/bin /run/systemd/system /etc/systemd/system /binary && \
    printf '#!/bin/sh\nprintf "%%s\\n" "$*" >> /out/systemctl.log\ncase "$1" in start|restart|try-restart) exit 91;; esac\nexit 0\n' > /usr/local/bin/systemctl && \
    printf '#!/bin/sh\nexit 0\n' > /usr/local/bin/systemd-analyze && \
    printf '#!/bin/sh\nexit 0\n' > /usr/local/bin/setfacl && \
    chmod 755 /usr/local/bin/systemctl /usr/local/bin/systemd-analyze /usr/local/bin/setfacl
DOCKER
  docker build --tag "$image" "$context"
  for memory_mib in 128 4096; do
    expected=false
    [[ $memory_mib != 128 ]] || expected=true
    for backend in v1 v2; do
      mkdir -p "$output/$memory_mib-$backend"
      docker run --rm --network none --cgroupns private --pids-limit 64 \
        --memory "${memory_mib}m" --memory-swap "${memory_mib}m" \
        --tmpfs /fixture:rw,nosuid,nodev,size=4m,nr_inodes=4096 \
        --mount "type=bind,src=$binary,dst=/binary/freespace,readonly" \
        --mount "type=bind,src=$script,dst=/check.sh,readonly" \
        --mount "type=bind,src=$output/$memory_mib-$backend,dst=/out" \
        --env INSTALL_MEMORY_DISPOSABLE=1 "$image" \
        timeout 120 bash /check.sh --container "$memory_mib" "$expected" "$backend"
    done
  done
}

if [[ ${1:-} = --container ]]; then
  [[ $# = 4 ]] || fail 'expected container limit, result and backend'
  container_checks "$2" "$3" "$4"
else
  host_checks "$@"
fi
