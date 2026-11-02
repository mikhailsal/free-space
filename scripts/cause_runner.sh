#!/usr/bin/env bash
# Dedicated GitHub VM only. A separate ext4 filesystem supplies enough inodes.
set -euo pipefail
if [[ ${GITHUB_ACTIONS:-} != true || ${RUNNER_ENVIRONMENT:-} != github-hosted ]]; then
  echo 'This benchmark requires an ephemeral GitHub-hosted runner.' >&2
  exit 1
fi
[[ ${BENCH_IO:?} == local ]] || exit 2
case ${DIAGNOSTIC_STAGE:?} in smoke|screen) ;; *) exit 2 ;; esac
[[ ${CAUSE_STEP_DEADLINE_EPOCH:?} =~ ^[0-9]+$ ]] || exit 2
repo=$(pwd)
output=${BENCH_OUTPUT:-$repo/cause-results}
mkdir -p "$output/measurements" "$output/provenance"
volume=$(mktemp -d "$RUNNER_TEMP/representative-volume.XXXXXX")
mount_dir="$volume/mount"
loop_device=''
child_pid=''
if [[ -n ${GITHUB_RUN_ID:-} ]]; then
  export FREESPACE_BENCHMARK_RUN_TOKEN="${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT:-1}-${GITHUB_JOB:-diagnostic}"
else
  read -r FREESPACE_BENCHMARK_RUN_TOKEN < /proc/sys/kernel/random/uuid
  export FREESPACE_BENCHMARK_RUN_TOKEN
fi
benchmark_label="freespace.benchmark=$FREESPACE_BENCHMARK_RUN_TOKEN"
finish_cleanup() {
  python3 -c 'import json,sys; json.dump({"success": int(sys.argv[2]) == 0, "exit_code": int(sys.argv[2])}, open(sys.argv[1], "w")); print()' \
    "$output/measurements/runner-cleanup.json" "$status" || status=1
  if ! python3 scripts/cause_summary.py "$output/measurements" --require-cleanup; then
    (( status != 0 )) || status=1
  fi
  printf 'cleanup final_status=%s\n' "$status" >> "$output/cleanup.log"
}
cleanup() {
  local status=$?
  local containers container attempt containers_stopped=true storage_released=true
  trap - EXIT
  printf 'cleanup label=%s initial_status=%s\n' "$benchmark_label" "$status" >> "$output/cleanup.log"
  # Stop the producer before listing containers, so it cannot start another arm.
  if [[ -n $child_pid ]]; then
    kill -TERM -- "-$child_pid" 2>/dev/null || kill -TERM "$child_pid" 2>/dev/null || true
    for ((attempt = 0; attempt < 20; attempt++)); do
      kill -0 "$child_pid" 2>/dev/null || break
      sleep 0.25
    done
    if kill -0 "$child_pid" 2>/dev/null; then
      echo 'Producer did not stop after TERM; sending KILL.' >> "$output/cleanup.log"
      kill -KILL -- "-$child_pid" 2>/dev/null || kill -KILL "$child_pid" 2>/dev/null || true
      for ((attempt = 0; attempt < 20; attempt++)); do
        kill -0 "$child_pid" 2>/dev/null || break
        sleep 0.25
      done
    fi
    if kill -0 "$child_pid" 2>/dev/null; then
      echo 'Producer is still alive; preserving mounted volume.' >> "$output/cleanup.log"
      containers_stopped=false
      status=1
    else
      # Only reap a process already known to have exited; never block shutdown.
      wait "$child_pid" 2>/dev/null || true
    fi
  fi
  if containers=$(timeout --kill-after=5s 30s docker ps -aq --filter "label=$benchmark_label" 2>> "$output/cleanup.log"); then
    while IFS= read -r container; do
      [[ -n $container ]] || continue
      printf 'removing container=%s\n' "$container" >> "$output/cleanup.log"
      if ! timeout --kill-after=5s 60s docker rm -f "$container" >> "$output/cleanup.log" 2>&1; then
        printf 'Failed to remove container=%s\n' "$container" >> "$output/cleanup.log"
        containers_stopped=false
        status=1
      fi
    done <<< "$containers"
  else
    containers_stopped=false
    status=1
    echo 'Failed to list run-owned containers.' >> "$output/cleanup.log"
  fi
  if [[ $containers_stopped != true ]]; then
    echo 'Container cleanup failed; preserving mounted volume.' >> "$output/cleanup.log"
    finish_cleanup
    exit "$status"
  fi
  if mountpoint -q "$mount_dir"; then
    timeout --kill-after=5s 30s sudo umount "$mount_dir" >> "$output/cleanup.log" 2>&1 || { status=1; storage_released=false; }
  fi
  if mountpoint -q "$mount_dir"; then
    storage_released=false
    status=1
  fi
  if [[ -n $loop_device && $storage_released == true ]]; then
    local backing
    if ! backing=$(timeout --kill-after=5s 15s sudo losetup -n -O BACK-FILE --raw "$loop_device" 2>> "$output/cleanup.log"); then
      status=1; storage_released=false
      echo 'Cannot verify loop backing file; preserving it.' >> "$output/cleanup.log"
    elif [[ $backing != "$volume/disk.img" ]]; then
      status=1; storage_released=false
      echo 'Loop backing file is not owned by this run; preserving it.' >> "$output/cleanup.log"
    else
      timeout --kill-after=5s 15s sudo losetup -d "$loop_device" >> "$output/cleanup.log" 2>&1 || { status=1; storage_released=false; }
    fi
  fi
  if [[ $storage_released == true ]]; then
    if rm -f -- "$volume/disk.img"; then
      timeout --kill-after=2s 15s sync -f -- "$volume" >> "$output/cleanup.log" 2>&1 || { status=1; storage_released=false; }
    else
      status=1
      storage_released=false
    fi
    if [[ -d $mount_dir ]]; then rmdir -- "$mount_dir" 2>/dev/null || true; fi
    rmdir -- "$volume" 2>/dev/null || true
  else
    echo 'Unmount or loop detach failed; preserving backing file.' >> "$output/cleanup.log"
  fi
  finish_cleanup
  exit "$status"
}
run_owned() {
  local status=0
  # Keep Python and its Docker CLI children in one stoppable process group.
  setsid "$@" &
  child_pid=$!
  wait "$child_pid" || status=$?
  child_pid=''
  return "$status"
}
# Cap setup commands by remaining step time, leaving cleanup time.
setup_timeout() {
  local cap=$1 now remaining
  now=$(date +%s)
  remaining=$((CAUSE_STEP_DEADLINE_EPOCH - now - 245))
  if (( remaining < 1 )); then
    echo 'Step deadline leaves no time for setup and cleanup.' >&2
    return 1
  fi
  (( cap <= remaining )) || cap=$remaining
  printf '%s' "$cap"
}
bounded() {
  local seconds
  seconds=$(setup_timeout "$1") || return 1
  shift
  timeout --kill-after=5s "${seconds}s" "$@"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
df -h > "$output/host-disk.txt"
lsblk -o NAME,MAJ:MIN,SIZE,TYPE,ROTA,MODEL >> "$output/host-disk.txt"
# No host cache or device is changed outside this ephemeral runner.
# Reserve actual backing blocks; inner statvfs must not promise absent storage.
available=$(df -B1 --output=avail "$volume" | tail -1)
volume_gib=40
required_gib=42
if (( available < required_gib * 1024 * 1024 * 1024 )); then
  echo "Need at least $required_gib GiB free on the runner for the dedicated filesystem." >&2
  exit 1
fi
bounded 300 fallocate -l "${volume_gib}G" "$volume/disk.img"
# Remember the selected device before attaching it, including on timeout.
loop_device=$(bounded 15 sudo losetup --find)
bounded 30 sudo losetup --direct-io=on "$loop_device" "$volume/disk.img"
bounded 300 sudo mkfs.ext4 -q -F -N 8500000 -m 0 -E nodiscard,lazy_itable_init=0,lazy_journal_init=0 "$loop_device"
mkdir "$mount_dir"
bounded 30 sudo mount "$loop_device" "$mount_dir"
sudo chown "$(id -u):$(id -g)" "$mount_dir"
findmnt -T "$mount_dir" -J > "$output/filesystem.json"
losetup -l -J > "$output/loop-devices.json"
df -i "$mount_dir" > "$output/inodes.txt"
image="resource-cause-calibration:${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT:-1}-${GITHUB_JOB}"
bounded 600 docker build -t "$image" scripts/bench-v2-container
bounded 30 docker image inspect "$image" > "$output/provenance/calibration-image.json"
bounded 15 python3 -c 'import json,platform,shutil,sqlite3,subprocess; p=shutil.which("fio"); print(json.dumps({"python":platform.python_version(),"sqlite":sqlite3.sqlite_version,"fio":subprocess.check_output([p,"--version"],text=True,timeout=5).strip() if p else None},indent=2))' > "$output/provenance/host-versions.json"
version_limit=$(setup_timeout 60)
run_owned timeout --kill-after=5s "${version_limit}s" docker run --rm --label "$benchmark_label" --network none \
  --entrypoint python3 "$image" -c 'import json,platform,sqlite3,subprocess; print(json.dumps({"python":platform.python_version(),"sqlite":sqlite3.sqlite_version,"fio":subprocess.check_output(["fio","--version"],text=True).strip()},indent=2))' > "$output/provenance/container-versions.json"
chmod +x binaries/baseline
calibration_limit=$(setup_timeout 300)
run_owned timeout --kill-after=5s "${calibration_limit}s" python3 scripts/bench-disk-calibration.py --image "$image" --device "$loop_device" \
  --directory "$mount_dir" --output "$output" --profile "$BENCH_IO"
mkdir "$mount_dir/jobs"
export RUNNER_TEMP="$mount_dir/jobs"
now=$(date +%s)
host_budget=$((CAUSE_STEP_DEADLINE_EPOCH - now - 240))
(( host_budget <= 2100 )) || host_budget=2100
HOST_BUDGET="$host_budget" HOST_BUDGET_EPOCH="$now" python3 - "$output/deadline.json" <<'PY'
import json, os, pathlib, sys
path = pathlib.Path(sys.argv[1])
receipt = json.loads(path.read_text())
receipt.update(host_budget_seconds=int(os.environ['HOST_BUDGET']),
               host_budget_computed_epoch=int(os.environ['HOST_BUDGET_EPOCH']),
               host_outer_timeout_seconds=int(os.environ['HOST_BUDGET']) + 60)
path.write_text(json.dumps(receipt, indent=2) + '\n')
PY
if (( host_budget < 181 )); then
  echo 'Insufficient time for the diagnostic and cleanup.' >&2
  exit 1
fi
run_owned timeout --kill-after=5s "$((host_budget + 60))s" python3 scripts/cause_host.py --stage "$DIAGNOSTIC_STAGE" --binary binaries/baseline \
  --budget-seconds "$host_budget" --output "$output/measurements" --io-device "$loop_device" --read-bps 33554432 \
  --write-bps 33554432 --read-iops 2200 --write-iops 3600
