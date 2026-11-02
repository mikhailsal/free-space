#!/usr/bin/env bash
# Dedicated GitHub VM only. A separate ext4 filesystem supplies enough inodes.
set -euo pipefail
if [[ ${GITHUB_ACTIONS:-} != true || ${RUNNER_ENVIRONMENT:-} != github-hosted ]]; then
  echo 'This benchmark requires an ephemeral GitHub-hosted runner.' >&2
  exit 1
fi
[[ ${CLEANUP_IO:?} == local ]] || exit 2
case ${CLEANUP_STAGE:?} in smoke|screen) ;; *) exit 2 ;; esac
[[ ${CLEANUP_STEP_DEADLINE_EPOCH:?} =~ ^[0-9]+$ ]] || exit 2
repo=$(pwd)
output=${CLEANUP_OUTPUT:-$repo/cleanup-results}
mkdir -p "$output/measurements" "$output/provenance"
volume=$(mktemp -d "$RUNNER_TEMP/cleanup-volume.XXXXXX")
mount_dir="$volume/mount"
loop_device=''
child_pid=''
space_watcher_pid=''
if [[ -n ${GITHUB_RUN_ID:-} ]]; then
  export FREESPACE_BENCHMARK_RUN_TOKEN="${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT:-1}-${GITHUB_JOB:-diagnostic}"
else
  read -r FREESPACE_BENCHMARK_RUN_TOKEN < /proc/sys/kernel/random/uuid
  export FREESPACE_BENCHMARK_RUN_TOKEN
fi
benchmark_label="freespace.benchmark=$FREESPACE_BENCHMARK_RUN_TOKEN"
finish_cleanup() {
  python3 -c 'import json,sys; json.dump({"success": int(sys.argv[2]) == 0, "exit_code": int(sys.argv[2])}, open(sys.argv[1], "w")); print()' \
    "$output/runner-cleanup.json" "$status" || status=1
  printf 'cleanup final_status=%s\n' "$status" >> "$output/cleanup.log"
}
cleanup() {
  local status=$?
  local containers container attempt containers_stopped=true storage_released=true
  trap - EXIT
  if [[ -n $space_watcher_pid ]]; then
    kill "$space_watcher_pid" 2>/dev/null || true
    wait "$space_watcher_pid" 2>/dev/null || true
  fi
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
  remaining=$((CLEANUP_STEP_DEADLINE_EPOCH - now - 245))
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
# Watch the backing and results filesystems during setup as well as measurement.
runner_pid=$$
(
  while true; do
    for path in "$volume" "$output"; do
      available=$(timeout --kill-after=2s 10s df -B1 --output=avail "$path" | tail -1 | tr -d ' ') || available=''
      if [[ ! $available =~ ^[0-9]+$ ]] || (( available < 2 * 1024 * 1024 * 1024 )); then
        printf 'outer reserve lost or unavailable: path=%s available=%s\n' "$path" "$available" >> "$output/cleanup.log"
        kill -TERM "$runner_pid" 2>/dev/null || true
        exit 1
      fi
    done
    sleep 1
  done
) &
space_watcher_pid=$!
df -h > "$output/host-disk.txt"
lsblk -o NAME,MAJ:MIN,SIZE,TYPE,ROTA,MODEL >> "$output/host-disk.txt"
# No host cache or device is changed outside this ephemeral runner.
# A sparse backing file permits real inner ext4 allocation without provisioning 500 GiB
# outside. Both filesystems are monitored independently and the fixture proves the
# unwritten-extent behavior before measurements are accepted.
available=$(df -B1 --output=avail "$volume" | tail -1)
volume_gib=500
required_gib=4
if (( available < required_gib * 1024 * 1024 * 1024 )); then
  echo "Need at least $required_gib GiB free on the runner for the dedicated filesystem." >&2
  exit 1
fi
bounded 30 truncate -s "${volume_gib}G" "$volume/disk.img"
export CLEANUP_OUTER_PATH="$volume" CLEANUP_BACKING_FILE="$volume/disk.img"
# Remember the selected device before attaching it, including on timeout.
loop_device=$(bounded 15 sudo losetup --find)
bounded 30 sudo losetup --direct-io=on "$loop_device" "$volume/disk.img"
bounded 300 sudo mkfs.ext4 -q -F -N 2000000 -m 0 -J size=64 -E nodiscard,lazy_itable_init=0,lazy_journal_init=0 "$loop_device"
mkdir "$mount_dir"
bounded 30 sudo mount "$loop_device" "$mount_dir"
sudo chown "$(id -u):$(id -g)" "$mount_dir"
findmnt -T "$mount_dir" -J > "$output/filesystem.json"
losetup -l -J > "$output/loop-devices.json"
df -i "$mount_dir" > "$output/inodes.txt"
image="cleanup-probe:${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT:-1}-${GITHUB_JOB}"
if [[ $CLEANUP_STAGE == smoke ]]; then
  bounded 600 docker build -f scripts/cleanup_probe.Dockerfile -t "$image" scripts/bench-v2-container
  bounded 300 docker save -o "$output/provenance/runtime-image.tar" "$image"
  bounded 300 gzip -1 "$output/provenance/runtime-image.tar"
else
  [[ -f ${CLEANUP_SMOKE_IMAGE:?} && -f ${CLEANUP_SMOKE_RUNTIME:?} ]] || exit 1
  bounded 300 docker load -i "$CLEANUP_SMOKE_IMAGE"
  smoke_image_id=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["image_id"])' "$CLEANUP_SMOKE_RUNTIME")
  bounded 30 docker tag "$smoke_image_id" "$image"
fi
bounded 30 docker image inspect "$image" > "$output/provenance/image.json"
version_limit=$(setup_timeout 60)
run_owned timeout --kill-after=5s "${version_limit}s" docker run --rm --label "$benchmark_label" --network none \
  --mount "type=bind,src=$repo/scripts,dst=/harness,readonly" \
  --entrypoint python3 "$image" /harness/cleanup_probe_versions.py > "$output/provenance/runtime-versions.json"
python3 - "$output/provenance" <<'PYRUNTIME'
import json,pathlib,sys
p=pathlib.Path(sys.argv[1])
r={'image_id':json.loads((p/'image.json').read_text())[0]['Id'],
   'versions':json.loads((p/'runtime-versions.json').read_text())}
(p/'runtime.json').write_text(json.dumps(r,indent=2,sort_keys=True)+'\n')
PYRUNTIME
if [[ $CLEANUP_STAGE == screen ]]; then cmp "$CLEANUP_SMOKE_RUNTIME" "$output/provenance/runtime.json"; fi
calibration_limit=$(setup_timeout 300)
run_owned timeout --kill-after=5s "${calibration_limit}s" python3 scripts/bench-disk-calibration.py --image "$image" --device "$loop_device" \
  --directory "$mount_dir" --output "$output" --profile local
mkdir "$mount_dir/jobs"
export RUNNER_TEMP="$mount_dir/jobs"
now=$(date +%s)
CLEANUP_HOST_START="$now" python3 - "$output/provenance/timing.json" <<'PYTIMING'
import json,os,sys
start=int(os.environ['CLEANUP_STEP_STARTED_EPOCH']); host=int(os.environ['CLEANUP_HOST_START'])
with open(sys.argv[1],'w') as out:
    fs=lambda path: os.statvfs(path).f_bavail * os.statvfs(path).f_frsize
    json.dump({'step_started_epoch':start,'host_started_epoch':host,'setup_seconds':host-start,
               'step_outer_available_bytes':int(os.environ['CLEANUP_STEP_OUTER_AVAILABLE']),
               'step_results_available_bytes':int(os.environ['CLEANUP_STEP_RESULTS_AVAILABLE']),
               'host_outer_available_bytes':fs(os.environ['CLEANUP_OUTER_PATH']),
               'host_results_available_bytes':fs(os.environ['CLEANUP_OUTPUT']),
               'job_started_epoch':int(os.environ['CLEANUP_JOB_STARTED_EPOCH']),
               'deadline_epoch':int(os.environ['CLEANUP_STEP_DEADLINE_EPOCH']),'upload_reserve_seconds':300},out,indent=2)
PYTIMING
budget=$((CLEANUP_STEP_DEADLINE_EPOCH - now - 240))
if (( budget < 181 )); then echo 'Insufficient time for workload and cleanup.' >&2; exit 1; fi
run_owned timeout --kill-after=5s "$((budget + 60))s" python3 scripts/cleanup_probe_host.py \
  --stage "$CLEANUP_STAGE" --image "$image" --io-device "$loop_device" \
  --output "$output/measurements" --budget-seconds "$budget"
