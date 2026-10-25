# shellcheck shell=bash
# shellcheck disable=SC2154 # $work, $tree and the helpers are scripts/rust-diff.sh's and rust-diff/serve.sh's
# The gui phase of scripts/rust-diff.sh, sourced by it after rust-diff/serve.sh.
# gui needs a terminal, so each build runs it under script(1) on a free port,
# three times: over ssh (it prints the tunnel line and asks no browser), and
# here twice, with a stub xdg-open first on PATH that records what it was given
# and opens (exit 0) or fails (exit 1). Here gui writes a redirect file that
# holds its token into XDG_RUNTIME_DIR (a folder of the run's) and hands it to
# xdg-open. That folder is listed (name, type, mode) and the file read; then
# one request with the token, which consumes the file, and the folder is listed
# again. TERM stops gui. Compared, masked: what it printed, the redirect file,
# what xdg-open got, the request's answer, the addresses it listens on, and its
# exit status. The three open no folder, @/media, and the folder whose name is
# not UTF-8 (each a fragment of its own form).

gui_run() { # <bin> <label> <ssh, opens or fails> [<folder>]
  local bin=$1 label=$2 browser=$3 argv cmd pid child seen addr token file
  local -a call=(env) # run_prefix has dropped the host's display already
  rm -rf "$work/runtime" "$work/browser.log" "$work/gui.out"
  mkdir -m 700 "$work/runtime"
  case $browser in
    ssh) call+=("SSH_CONNECTION=192.0.2.1 50000 192.0.2.2 22") ;;
    opens) call+=(-u SSH_CONNECTION XDG_OPEN_EXIT=0) ;;
    *) call+=(-u SSH_CONNECTION XDG_OPEN_EXIT=1) ;;
  esac
  call+=("XDG_RUNTIME_DIR=$work/runtime" "XDG_OPEN_LOG=$work/browser.log" "PATH=$work/browser:$PATH" "$bin" gui --port 0)
  [ -z "${4-}" ] || call+=("${4/#@/$tree}")
  run_prefix
  printf -v cmd '%q ' "${argv[@]}" "${call[@]}"
  # bash runs the command: %q may quote a byte that is not UTF-8 as $'...'
  (SHELL=$BASH exec script -qfec "exec $cmd" /dev/null </dev/null >"$work/gui.out" 2>&1) &
  # shellcheck disable=SC2034 # the EXIT trap of scripts/rust-diff.sh reads it
  pid=$! serving=$!
  case $browser in
    ssh) wait_for seen '[#?&]t=[0-9a-f]{64}' "$work/gui.out" "$pid" ;;
    opens) wait_for seen 'freespace-gui-[0-9]+\.html' "$work/browser.log" "$pid" ;;
    *) wait_for seen 'no browser opened' "$work/gui.out" "$pid" ;;
  esac
  # script(1) may relay the address a moment after xdg-open logs, and part of a line at a time: up to the / after the
  # port, or a port cut short would pass
  wait_for addr 'http://127\.0\.0\.1:[0-9]+/' "$work/gui.out" "$pid"
  addr=${addr%/}
  file=$(head -n 1 "$work/browser.log" 2>/dev/null || true)
  # the redirect file holds the address twice (refresh and link): the first token only
  token=$(cat "$work/gui.out" "$file" 2>/dev/null | grep -oE '[#?&]t=[0-9a-f]{64}' | head -n 1 | cut -c 4- || true)
  # shellcheck disable=SC2034 # rust-diff/serve.sh's http() and listening() read it
  http_phase="gui $label"
  if [ -z "$seen" ] || [ -z "$addr" ] || [ -z "$token" ]; then
    floor gui "$label" "no address" "an address"
    echo "== gui $label did not start"
  else
    echo "== gui $label"
    if [ "$browser" != ssh ]; then
      files "$work/runtime"
      { echo "-- xdg-open got"; text_of "$work/browser.log"; echo "-- the redirect file"; text_of "$file"; } | mask
    fi
    http status 200 json -H "x-freespace-token: $token" "$addr/api/status"
    [ "$browser" = ssh ] || files "$work/runtime"
    listening "${addr##*:}" | mask
  fi
  # TERM goes to gui (the child of script), so its own exit status is the one script gives back
  child=$(cat "/proc/$pid/task/$pid/children" 2>/dev/null || true)
  stopped "$pid" "${child%% *}" "gui $label" "$work/gui.out"
  # shellcheck disable=SC2034 # as above
  http_phase=serve
}

gui_answers() { # <bin>: gui over ssh, with a browser that opens, and with one that fails
  mkdir -p "$work/browser"
  # shellcheck disable=SC2016 # the stub's own variables
  printf '%s\n' '#!/bin/sh' 'printf "%s\n" "$@" >>"$XDG_OPEN_LOG"' 'exit "${XDG_OPEN_EXIT:-0}"' >"$work/browser/xdg-open"
  chmod 755 "$work/browser/xdg-open"
  gui_run "$1" "over ssh" ssh
  gui_run "$1" "here, a browser opens" opens @/media
  gui_run "$1" "here, no browser" fails $'@/odd/dir\376'
  rm -rf "$work/runtime" "$work/browser.log" "$work/gui.out"
}
