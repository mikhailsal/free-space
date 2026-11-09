#!/bin/bash
# A manual `sudo freespace scan` gives the user behind sudo read access to what it
# published when no timer was installed, and only then. Runs as root in private mount and
# PID namespaces (see ../common/namespace.rs); setfacl and usermod are fakes that log.
set -euo pipefail
binary=$1 scratch=$2
scratch_owner=$(stat -c %u "$scratch")
scratch_group=$(stat -c %g "$scratch")
trap 'chown -hR "$scratch_owner:$scratch_group" "$scratch"' EXIT
# Before any application runs: break mount propagation, hide every host path
# the scan or the access step can mutate, and isolate /proc.
mount --make-rprivate /
for path in /etc /usr/local /run /var/lib /root; do
  mount -t tmpfs -o mode=0755 tmpfs "$path"
done
for path in /etc /usr/local /run /var/lib /root; do
  findmnt -n -o FSTYPE --target "$path" | /usr/bin/grep -qx tmpfs
done
[ "$$" = 1 ] || { echo 'private PID namespace required' >&2; exit 1; }
printf 'root:x:0:0:root:/root:/bin/bash\nalice:x:1000:1000::/home/alice:/bin/bash\nbob:x:1001:1001::/home/bob:/bin/bash\n' > /etc/passwd
printf 'root:x:0:\nfreespace:x:4242:\nalice:x:1000:\nbob:x:1001:\n' > /etc/group
mkdir -p "$scratch/tools" "$scratch/root/sub" "$scratch/units"
printf test > "$scratch/root/sub/a"
export FREESPACE_CONFIG="$scratch/none.toml" FREESPACE_UNIT_DIR="$scratch/units"
export FREESPACE_SNAPSHOT_DIR="$scratch/snap" HOME=/root SCAN_TEST_LOG="$scratch/tools.log"
unset SUDO_USER
cat > "$scratch/tools/setfacl" <<'TOOL'
#!/bin/bash
printf 'setfacl %s\n' "$*" >> "$SCAN_TEST_LOG"
exit "${SCAN_TEST_NO_ACL:-0}"
TOOL
cat > "$scratch/tools/usermod" <<'TOOL'
#!/bin/bash
# usermod -aG GROUP USER: append USER to the member list of GROUP, like the real tool
printf 'usermod %s\n' "$*" >> "$SCAN_TEST_LOG"
[ -z "${SCAN_TEST_NO_USERMOD:-}" ] || { echo 'usermod: cannot lock /etc/group' >&2; exit 10; }
# Plain bash: /etc is a tmpfs here, so /usr/bin/awk (a link into /etc/alternatives) is dangling.
group=$2 user=$3
while IFS= read -r line; do
  if [[ $line == "$group:"* ]]; then
    members=${line##*:}
    line="${line%:*}:${members:+$members,}$user"
  fi
  printf '%s\n' "$line"
done < /etc/group > /etc/group.new
mv /etc/group.new /etc/group
TOOL
chmod 755 "$scratch/tools/"*
export PATH="$scratch/tools:/usr/bin:/bin"
snap=$scratch/snap

# scan NAME [freespace options]: a scan of the scratch tree, its stdout and stderr kept in
# NAME.json and NAME.err. It must succeed whatever the access step did.
scan() {
  local name=$1
  shift
  "$binary" "$@" scan "$scratch/root" --json > "$scratch/$name.json" 2> "$scratch/$name.err"
  python3 -c 'import json,sys; json.load(open(sys.argv[1]))["generation"]' "$scratch/$name.json"
}
# called LINE: a fake tool was called exactly so. logged LINE and said NAME TEXT (scan NAME's
# stderr has TEXT) insist on it and show what there was instead.
called() { grep -qxF -- "$1" "$SCAN_TEST_LOG"; }
logged() { called "$1" || { echo "not called: $1" >&2; cat "$SCAN_TEST_LOG" >&2; exit 1; }; }
said() { grep -qF -- "$2" "$scratch/$1.err" || { echo "scan $1 did not say: $2" >&2; cat "$scratch/$1.err" >&2; exit 1; }; }
# silent NAME: scan NAME neither called a tool nor said anything about access
silent() {
  if [ -s "$SCAN_TEST_LOG" ] || grep -qE 'can read|added .* to group|cannot read|read access' "$scratch/$1.err"; then
    echo "scan $1 touched access but should not have:" >&2; cat "$SCAN_TEST_LOG" "$scratch/$1.err" >&2; exit 1
  fi
}

# No timer, run under sudo by alice: she gets an ACL on the directory and its files, and
# the default ACL that later scans' new files inherit.
: > "$SCAN_TEST_LOG"
SUDO_USER=alice scan acl
logged "setfacl -R -m u:alice:rX $snap"
logged "setfacl -d -m u:alice:rX $snap"
said acl 'alice can read the snapshot'
[ "$(stat -c %u:%g:%a "$snap")" = 0:4242:750 ]
[ "$(stat -c %u:%g:%a "$snap/snapshot.db")" = 0:4242:640 ]

# Cron and the timer run as root without $SUDO_USER; a stale $SUDO_USER names nobody.
: > "$SCAN_TEST_LOG"
scan cron
silent cron
SUDO_USER=nobody-such-user scan stale
silent stale

# A timer unit means install-timer ran and made its own choice: leave it alone.
printf '[Timer]\n' > "$scratch/units/freespace.timer"
SUDO_USER=alice scan timer
silent timer
rm "$scratch/units/freespace.timer"

# Without ACL support the user joins the freespace group and has to log in again; once a
# member, a later scan only says so.
SCAN_TEST_NO_ACL=1 SUDO_USER=alice scan group
logged 'usermod -aG freespace alice'
said group 'added alice to group freespace'
said group 'newgrp freespace'
[[ " $(id -nG alice) " == *' freespace '* ]]
: > "$SCAN_TEST_LOG"
SCAN_TEST_NO_ACL=1 SUDO_USER=alice scan member
said member 'through group freespace'
if called 'usermod -aG freespace alice'; then echo 'usermod ran for a member' >&2; exit 1; fi

# Failing to grant is a warning, not a failed scan: the snapshot is published and root reads it.
SCAN_TEST_NO_ACL=1 SCAN_TEST_NO_USERMOD=1 SUDO_USER=bob scan failed
said failed 'could not add bob to group freespace'
said failed 'bob cannot read the snapshot'

# The coarse and v2 backends keep their snapshots one level down: traversal on the base, the
# recursive and default ACLs on their own directory.
for backend in coarse v2; do
  : > "$SCAN_TEST_LOG"
  SUDO_USER=alice scan "$backend" --backend "$backend"
  logged "setfacl -m u:alice:rX $snap"
  logged "setfacl -R -m u:alice:rX $snap/$backend"
  logged "setfacl -d -m u:alice:rX $snap/$backend"
done
printf 'scan access passed\n'
