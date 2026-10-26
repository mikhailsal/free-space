#!/bin/bash
set -euo pipefail
binary=$1 scratch=$2
scratch_owner=$(stat -c %u "$scratch")
scratch_group=$(stat -c %g "$scratch")
trap 'chown -hR "$scratch_owner:$scratch_group" "$scratch"' EXIT
# Before any application runs: break mount propagation, hide every host path
# the installer can mutate, isolate /proc, and provide a fake systemctl.
mount --make-rprivate /
for path in /etc /usr/local /run /var/lib /root; do
  mount -t tmpfs -o mode=0755 tmpfs "$path"
done
for path in /etc /usr/local /run /var/lib /root; do
  findmnt -n -o FSTYPE --target "$path" | /usr/bin/grep -qx tmpfs
done
[ "$$" = 1 ] || { echo 'private PID namespace required' >&2; exit 1; }
mkdir -p /usr/local/bin /run/systemd/system /etc/systemd/system /root/.claude
printf 'root:x:0:0:root:/root:/bin/bash\n' > /etc/passwd
printf 'root:x:0:\nfreespace:x:4242:root\n' > /etc/group
mkdir -p "$scratch/tools" "$scratch/root/sub" "$scratch/units"
printf test > "$scratch/root/sub/a"
mkdir -p "$scratch/install-link-target"
chmod 700 "$scratch/install-link-target"
printf keep-install-target > "$scratch/install-link-target/marker"
ln -s "$scratch/install-link-target" "$scratch/install-link"
export FREESPACE_CONFIG="$scratch/config.toml"
export FREESPACE_UNIT_DIR="$scratch/units"
export HOME=/root
unset SUDO_USER FREESPACE_SNAPSHOT_DIR
export INSTALL_TEST_LOG="$scratch/tools.log" INSTALL_TEST_ROOT="$scratch"
cat > "$scratch/tools/systemctl" <<'TOOL'
#!/bin/bash
printf 'systemctl %s\n' "$*" >> "$INSTALL_TEST_LOG"
case "$1" in
  is-active|is-failed) exit 1 ;;
  show) echo 'Tue 2026-10-27 02:00:00 UTC' ;;
  start|restart)
    /usr/local/bin/freespace scan --json > "$INSTALL_TEST_ROOT/first-scan.json"
    # Installer's poll must observe a running scan without a five-second wait.
    flock -x "$INSTALL_TEST_ROOT/snap/v2/.lock" -c 'sleep 2' >/dev/null 2>&1 </dev/null &
    ;;
esac
TOOL
cat > "$scratch/tools/setfacl" <<'TOOL'
#!/bin/bash
printf 'setfacl %s\n' "$*" >> "$INSTALL_TEST_LOG"
exit "${INSTALL_TEST_NO_ACL:-0}"
TOOL
cat > "$scratch/tools/systemd-analyze" <<'TOOL'
#!/bin/bash
exit 0
TOOL
chmod 755 "$scratch/tools/"*
export PATH="$scratch/tools:/usr/bin:/bin"
[ "$(command -v systemctl)" = "$scratch/tools/systemctl" ]
# Reject a symlinked v2 base before installing the binary, writing config, or
# changing ownership/mode on its target.
install_target_stat=$(stat -c %u:%g:%a "$scratch/install-link-target")
if "$binary" --backend v2 install-timer --no-start --roots "$scratch/root" --snapshot-dir "$scratch/install-link" --user root --json > "$scratch/install-link.json" 2>&1; then
  echo 'v2 install accepted a symlinked snapshot base' >&2; exit 1
fi
[ "$(stat -c %u:%g:%a "$scratch/install-link-target")" = "$install_target_stat" ]
[ "$(cat "$scratch/install-link-target/marker")" = keep-install-target ]
[ ! -e "$scratch/config.toml" ] && [ ! -e /usr/local/bin/freespace ]
# Explicit selection must be persisted so the unit's ordinary `scan` uses v2.
"$binary" --backend v2 install-timer --no-start --roots "$scratch/root" --snapshot-dir "$scratch/snap" --user root --json > "$scratch/install.json"
python3 - "$scratch" <<'PY'
import json, pathlib, sys, stat
p=pathlib.Path(sys.argv[1]); r=json.loads((p/'install.json').read_text())
assert not r['scan_started']
assert r['snapshot_dir']==str(p/'snap/v2')
assert 'backend = "v2"' in (p/'config.toml').read_text()
assert 'ExecStart=/usr/local/bin/freespace scan' in (p/'units/freespace.service').read_text()
assert str(p/'config.toml') in (p/'units/freespace.service').read_text()
for d in ['snap','snap/v2']:
    s=(p/d).stat(); assert s.st_uid==0 and s.st_gid==4242 and stat.S_IMODE(s.st_mode)==0o750
log=(p/'tools.log').read_text()
assert f'setfacl -m u:root:rX {p}/snap\n' in log
assert f'setfacl -R -m u:root:rX {p}/snap/v2\n' in log
assert f'setfacl -d -m u:root:rX {p}/snap/v2\n' in log
assert 'systemctl start' not in log
PY
# A manual scan before first publication must be found through the v2 lock.
flock -x "$scratch/snap/v2/.lock" -c 'sleep 2' &
holder=$!
sleep .2
"$binary" install-timer --user root --json > "$scratch/running.json"
python3 - "$scratch" <<'PYTEST'
import pathlib,json,sys
p=pathlib.Path(sys.argv[1]); r=json.loads((p/'running.json').read_text())
assert not r['scan_started'] and 'already running' in r['scan_reason']
assert 'systemctl start' not in (p/'tools.log').read_text()
PYTEST
wait "$holder"
# No store starts the service against persisted v2 configuration.
"$binary" install-timer --user root --json > "$scratch/started.json"
# Let the fake first-scan lock drain before running cleanup/lock assertions.
sleep 3
python3 - "$scratch" <<'PY'
import json,pathlib,sys
p=pathlib.Path(sys.argv[1]); assert json.loads((p/'started.json').read_text())['scan_started']
assert json.loads((p/'first-scan.json').read_text())['generation']==1
assert (p/'snap/v2/generations.db').exists()
assert not (p/'snap/snapshot.db').exists()
PY
"$binary" install-timer --user root --json > "$scratch/healthy.json"
"$binary" status --json > "$scratch/status.json"
python3 - "$scratch" <<'PY'
import json,pathlib,sys
p=pathlib.Path(sys.argv[1]); assert not json.loads((p/'healthy.json').read_text())['scan_started']
assert json.loads((p/'status.json').read_text())['generation']==1
assert (p/'tools.log').read_text().count('systemctl start --no-block freespace.service')==1
PY
# Existing v2 files are re-secured without changing generation history.
chmod 666 "$scratch/snap/v2/generations.db" "$scratch/snap/v2/generations.db.readers"
"$binary" install-timer --user root --json > "$scratch/resecured.json"
[ "$(stat -c %a "$scratch/snap/v2/generations.db")" = 640 ]
[ "$(stat -c %g "$scratch/snap/v2/generations.db.readers")" = 4242 ]
# Missing ACL support falls back to membership in the prepared freespace group.
INSTALL_TEST_NO_ACL=1 "$binary" install-timer --no-start --user root --json > "$scratch/group.json"
python3 - "$scratch" <<'PYTEST'
import pathlib,json,sys
p=pathlib.Path(sys.argv[1]); r=json.loads((p/'group.json').read_text())
assert r['access']=='group' and not r['relogin_needed']
PYTEST
# Shared reader leases must also prevent removal of pinned WAL transactions.
flock -s "$scratch/snap/v2/generations.db.readers" -c 'sleep 2' &
holder=$!
sleep .2
if "$binary" install-timer --uninstall --user root --json > "$scratch/readers.json"; then
  echo 'uninstall ignored active readers' >&2; exit 1
fi
grep -q 'v2 readers are active' "$scratch/readers.json"
[ -f "$scratch/config.toml" ] && [ -f "$scratch/snap/v2/generations.db" ]
wait "$holder"
# An independent manual writer lock must prevent uninstall without deleting data.
flock -x "$scratch/snap/v2/.lock" -c 'sleep 4' &
holder=$!
sleep .2
if "$binary" install-timer --uninstall --user root --json > "$scratch/locked.json"; then
  echo 'uninstall ignored writer lock' >&2; exit 1
fi
[ -f "$scratch/config.toml" ] && [ -f "$scratch/snap/v2/generations.db" ]
wait "$holder"
# A symlinked configured base must abort uninstall before locks, systemd, or
# manifest cleanup can reach the target.
mv "$scratch/snap" "$scratch/snap-real"
mkdir -p "$scratch/uninstall-link-target/v2"
chmod 700 "$scratch/uninstall-link-target"
printf keep-uninstall-target > "$scratch/uninstall-link-target/v2/generations.db"
ln -s "$scratch/uninstall-link-target" "$scratch/snap"
uninstall_target_stat=$(stat -c %u:%g:%a "$scratch/uninstall-link-target")
systemctl_calls_before=$(grep -c '^systemctl ' "$scratch/tools.log" || true)
if "$binary" install-timer --uninstall --user root --json > "$scratch/uninstall-link.json" 2>&1; then
  echo 'v2 uninstall accepted a symlinked snapshot base' >&2; exit 1
fi
[ "$(stat -c %u:%g:%a "$scratch/uninstall-link-target")" = "$uninstall_target_stat" ]
[ "$(cat "$scratch/uninstall-link-target/v2/generations.db")" = keep-uninstall-target ]
[ -f "$scratch/config.toml" ] && [ -f "$scratch/units/freespace.service" ]
[ "$(grep -c '^systemctl ' "$scratch/tools.log" || true)" -eq "$systemctl_calls_before" ]
rm "$scratch/snap"
mv "$scratch/snap-real" "$scratch/snap"
# Cleanup removes only the v2 manifest, preserving v1 and unknown files.
printf preserve-v1 > "$scratch/snap/snapshot.db"
printf preserve-base > "$scratch/snap/unrelated.txt"
printf preserve-v2 > "$scratch/snap/v2/unrelated.txt"
printf preserve-crash > "$scratch/snap/v2/crash-unrelated.log"
printf known-crash > "$scratch/snap/v2/crash-123.log"
printf known-stage > "$scratch/snap/v2/scan-0123456789abcdef.stage.tmp"
printf unrelated-stage > "$scratch/snap/v2/scan-0123456789ABCDEf.stage.tmp"
printf preserve-target > "$scratch/outside"
ln -s "$scratch/outside" "$scratch/snap/v2/generations.db-journal"
"$binary" install-timer --uninstall --user root --json > "$scratch/uninstall.json"
python3 - "$scratch" <<'PY'
import pathlib,json,sys
p=pathlib.Path(sys.argv[1]); v=p/'snap/v2'
assert (p/'outside').read_text()=='preserve-target'
assert (p/'snap/snapshot.db').read_text()=='preserve-v1'
assert (p/'snap/unrelated.txt').read_text()=='preserve-base'
assert (v/'unrelated.txt').read_text()=='preserve-v2'
assert (v/'crash-unrelated.log').read_text()=='preserve-crash'
assert (v/'scan-0123456789ABCDEf.stage.tmp').read_text()=='unrelated-stage'
for name in ['generations.db','generations.db-wal','generations.db-shm','generations.db-journal','generations.db.readers','state.json','.lock','crash-123.log','scan-0123456789abcdef.stage.tmp']:
    assert not (v/name).exists(), name
assert not (p/'config.toml').exists()
assert not (p/'units/freespace.service').exists()
assert not pathlib.Path('/usr/local/bin/freespace').exists()
r=json.loads((p/'uninstall.json').read_text()); assert str(v)+'/' in r['kept']
PY
# Exercise the unchanged v1 default lifecycle in the same isolated environment.
"$binary" install-timer --no-start --roots "$scratch/root" --snapshot-dir "$scratch/v1" --user root --json > "$scratch/v1-install.json"
if grep -q 'backend = "v2"' "$scratch/config.toml"; then exit 1; fi
[ -d "$scratch/v1" ] && [ ! -d "$scratch/v1/v2" ]
printf own > "$scratch/v1/snapshot.db"
printf unrelated > "$scratch/v1/unrelated"
"$binary" install-timer --uninstall --user root --json > "$scratch/v1-uninstall.json"
[ ! -f "$scratch/v1/snapshot.db" ] && [ -f "$scratch/v1/unrelated" ]
printf 'isolated v2 lifecycle passed\n'
