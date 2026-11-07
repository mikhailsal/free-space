# shellcheck shell=bash
# Sourced by rust-diff.sh and by the install comparator (rust-diff/install-kept.sh).
#
# Both read the answers of GNU's `timeout`: 124 for a call that ran out of time, the call's own status when it ended
# in time, an argument handed on byte for byte, and a TERM sent to timeout itself handed on to the call. Ubuntu 25.10
# and later, so the ubuntu-latest runner since 26.04, install uutils coreutils as /usr/bin/timeout. That one refuses an
# argument that is not UTF-8 (exit 125; the harness asks `ls` for a name that is not), and exits 124 when it is itself
# TERMed while the call ends with 0. GNU's stays installed under a gnu prefix: gnutimeout.
# shellcheck disable=SC2034 # read by the scripts that source this file
gnu_timeout_needed='GNU coreutils timeout is required (uutils coreutils answers some calls differently; Ubuntu 25.10 and later keep the GNU one as gnutimeout)'

gnu_tool() { # <tool>: prints the absolute path of GNU coreutils' <tool>, or fails
  local candidate path version
  for candidate in "$1" "gnu$1"; do
    path=$(command -v "$candidate") || continue
    version=$("$path" --version 2>/dev/null) || continue
    if [[ ${version%%$'\n'*} == *'GNU coreutils'* ]]; then
      printf '%s\n' "$path"
      return 0
    fi
  done
  return 1
}
