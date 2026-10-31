#!/usr/bin/env bash
# Dedicated GitHub VM only. A separate ext4 filesystem supplies enough inodes.
set -euo pipefail
if [[ ${GITHUB_ACTIONS:-} != true || ${RUNNER_ENVIRONMENT:-} != github-hosted ]]; then
  echo 'This benchmark requires an ephemeral GitHub-hosted runner.' >&2
  exit 1
fi
case ${BENCH_IO:?} in native|limited|local) ;; *) exit 2 ;; esac
case ${BENCH_PROTOCOL:-v2} in v2|v1-pairs) ;; *) exit 2 ;; esac
repo=$(pwd)
output=${BENCH_OUTPUT:-$repo/representative-results}
mkdir -p "$(dirname "$output")"
mkdir "$output"
volume=$(mktemp -d "$RUNNER_TEMP/representative-volume.XXXXXX")
mount_dir="$volume/mount"
loop_device=''
child_pid=''
if [[ -n ${GITHUB_RUN_ID:-} ]]; then
  export FREESPACE_BENCHMARK_RUN_TOKEN="$GITHUB_RUN_ID"
else
  read -r FREESPACE_BENCHMARK_RUN_TOKEN < /proc/sys/kernel/random/uuid
  export FREESPACE_BENCHMARK_RUN_TOKEN
fi
benchmark_label="freespace.benchmark=$FREESPACE_BENCHMARK_RUN_TOKEN"
finish_cleanup() {
  if [[ ${BENCH_PROTOCOL:-v2} == v1-pairs && -d $output/measurements ]]; then
    python3 -c 'import json,sys; json.dump({"success": int(sys.argv[2]) == 0, "exit_code": int(sys.argv[2])}, open(sys.argv[1], "w")); print()' \
      "$output/measurements/runner-cleanup.json" "$status" || status=1
    if ! python3 scripts/bench_v1_summary.py "$output/measurements" > "$output/final-summary.md"; then
      if (( status == 0 )); then status=1; fi
      python3 -c 'import json,sys; json.dump({"success": False, "exit_code": int(sys.argv[2])}, open(sys.argv[1], "w")); print()' \
        "$output/measurements/runner-cleanup.json" "$status" || true
    fi
    cat "$output/final-summary.md" >> "$GITHUB_STEP_SUMMARY" || { if (( status == 0 )); then status=1; fi; }
    if (( status != 0 )); then
      python3 -c 'import json,sys; json.dump({"success": False, "exit_code": int(sys.argv[2])}, open(sys.argv[1], "w")); print()' \
        "$output/measurements/runner-cleanup.json" "$status" || true
    fi
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
    timeout --kill-after=5s 15s sudo losetup -d "$loop_device" >> "$output/cleanup.log" 2>&1 || { status=1; storage_released=false; }
  fi
  if [[ $storage_released == true ]]; then
    rm -f -- "$volume/disk.img" || status=1
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
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
df -h > "$output/host-disk.txt"
lsblk -o NAME,MAJ:MIN,SIZE,TYPE,ROTA,MODEL >> "$output/host-disk.txt"
# No host cache or device is changed outside this ephemeral runner.
# Reserve actual backing blocks; inner statvfs must not promise absent storage.
available=$(df -B1 --output=avail "$volume" | tail -1)
volume_gib=30
required_gib=32
if [[ ${BENCH_PROTOCOL:-v2} == v1-pairs ]]; then volume_gib=40; required_gib=42; fi
if (( available < required_gib * 1024 * 1024 * 1024 )); then
  echo "Need at least $required_gib GiB free on the runner for the dedicated filesystem." >&2
  exit 1
fi
fallocate -l "${volume_gib}G" "$volume/disk.img"
loop_device=$(sudo losetup --find --show --direct-io=on "$volume/disk.img")
sudo mkfs.ext4 -q -F -N 8500000 -m 0 -E nodiscard,lazy_itable_init=0,lazy_journal_init=0 "$loop_device"
mkdir "$mount_dir"
sudo mount "$loop_device" "$mount_dir"
sudo chown "$(id -u):$(id -g)" "$mount_dir"
findmnt -T "$mount_dir" -J > "$output/filesystem.json"
losetup -l -J > "$output/loop-devices.json"
df -i "$mount_dir" > "$output/inodes.txt"
image="representative-calibration:${GITHUB_RUN_ID}-${GITHUB_JOB}-${BENCH_IO}"
docker build -t "$image" scripts/bench-v2-container
chmod +x binaries/candidate
extra=()
if [[ $BENCH_IO == limited ]]; then
  extra=(--io-device "$loop_device" --read-bps 33554432 --write-bps 16777216 --read-iops 1000 --write-iops 500)
elif [[ $BENCH_IO == local ]]; then
  # Sensitivity profile around observed local bursts, not local-device equivalence.
  extra=(--io-device "$loop_device" --read-bps 33554432 --write-bps 33554432 --read-iops 2200 --write-iops 3600)
fi
run_owned python3 scripts/bench-disk-calibration.py --image "$image" --device "$loop_device" \
  --directory "$mount_dir" --output "$output" --profile "$BENCH_IO"
mkdir "$mount_dir/jobs"
export RUNNER_TEMP="$mount_dir/jobs"
if [[ ${BENCH_PROTOCOL:-v2} == v1-pairs ]]; then
  chmod +x binaries/baseline
  measurement_status=0
  run_owned python3 scripts/bench-v2-container.py --v1-pairs --baseline binaries/baseline \
    --candidate binaries/candidate --output "$output/measurements" --files "${BENCH_FILES:?}" \
    --pairs "${BENCH_PAIRS:-1}" --operation "${BENCH_OPERATION:-scan}" \
    --scenario "${BENCH_SCENARIO:-unchanged}" --memory "${BENCH_MEMORY:-4096}" \
    --timeout-seconds "${BENCH_COMMAND_TIMEOUT:-1200}" "${extra[@]}" || measurement_status=$?
  if [[ -f $output/measurements/summary.md ]]; then
    cat "$output/measurements/summary.md" >> "$GITHUB_STEP_SUMMARY" || measurement_status=1
  fi
  (( measurement_status == 0 )) || exit "$measurement_status"
else
  run_owned python3 scripts/bench-v2-container.py --candidate binaries/candidate --current-backends \
    --output "$output/measurements" --steps "${BENCH_FILES:?}" --memory 4096 \
    --workers 1 --cpus 2 --scans "${BENCH_SCANS:?}" --repeats "${BENCH_REPEATS:?}" \
    --refreshes 3 --scan-profile disk --keep-previous true --timeout-seconds 7200 "${extra[@]}"
  python3 scripts/summarize-v2-container.py "$output/measurements" | tee "$output/summary.md" >> "$GITHUB_STEP_SUMMARY"
fi
