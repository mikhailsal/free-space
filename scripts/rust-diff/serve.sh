# shellcheck shell=bash
# shellcheck disable=SC2154 # $work, $tree and the helpers are scripts/rust-diff.sh's
# The serve phase of scripts/rust-diff.sh, sourced by it: each build serves the
# fixture snapshot on a free port (`serve --port 0`), answers these requests
# with its token, and stops on TERM. Compared: the HTTP status, the headers
# (all but Date and Content-Length, which follows the times in the body), the
# body normalized like --json, the addresses the server listens on, what serve
# prints, its exit status, and the snapshot folder afterwards. The page itself
# is asked for its status and headers only: its bytes are the web bundle, which
# pixel-diff compares drawn. curl reads no ~/.curlrc and takes no proxy from the
# environment. rust-diff/gui.sh asks `gui` with the same helpers.

# label|endpoint|parameters (k=v, space-separated; @ is the tree)|HTTP status base must give
SERVE_QUESTIONS=(
  "status|/api/status||200"
  "ls|/api/ls|path=@|200"
  "ls by apparent size|/api/ls|path=@ sort=apparent limit=3|200"
  "ls of a snapshot that is gone|/api/ls|path=@ snapshot=not-this-one|409"
  "top|/api/top|path=@ min_size=100K sort=mtime|200"
  "top folders|/api/top|path=@ kind=dir older_than=2024-06-01|200"
  "find|/api/find|path=@ name=*.iso|200"
  "diff|/api/diff|path=@ depth=2 kind=any|200"
  "a path not in the snapshot|/api/ls|path=@/nope|404"
  "a bad limit|/api/ls|path=@ limit=x|400"
  "an unknown endpoint|/api/nope||404"
)

# How long a server may take to print its address (ticks of 0.1 s; a loaded runner can take seconds, and the wait ends
# as soon as the address shows or the server dies), and a request to answer (s).
SERVE_WAIT_TICKS=600
CURL_SECONDS=30

served=0 http_phase=serve
http() { # <label> <want> <body: json, page or none> <curl args...>: one request: its status, headers and body
  local label=$1 want=$2 body=$3 code
  shift 3
  served=$((served + 1))
  : >"$work/body" # a request curl cannot make leaves no body and no headers, not the last request's
  : >"$work/headers"
  code=$(curl -q --noproxy '*' -sS --max-time "$CURL_SECONDS" -o "$work/body" -D "$work/headers" -w '%{http_code}' "$@" 2>"$work/curl.err") ||
    code="curl failed: $(head -c 300 "$work/curl.err")"
  floor "$http_phase" "$label" "$code" "$want"
  {
    echo "== $http_phase: $label"
    echo "http $code"
    echo "-- headers"
    tr -d '\r' <"$work/headers" | sed -n '2,$p' | sed -E '/^$/d; s/^([^:]+):/\L\1:/' |
      grep -vE '^(date|content-length):' | LC_ALL=C sort || true
    echo "-- body"
    case $body in
      page) echo "(the web bundle: pixel-diff compares it)" ;;
      none) echo "(none: a HEAD)" ;;
      *) normalize_json "$work/body" false ;;
    esac
  } | mask
}

wait_for() { # <var> <ERE> <file> <pid>: into var, the first match in the file, once it shows or the process ends
  # (the file may hold half a line: a pattern must end where the text it wants ends). It waits in this shell, not in a
  # $(...): bash runs a trap once the foreground command ends, and a TERM must not wait for the whole wait.
  local i got=''
  for ((i = 0; i < SERVE_WAIT_TICKS; i++)); do
    got=$(grep -m1 -oE "$2" "$3" 2>/dev/null || true)
    if [ -n "$got" ] || ! kill -0 "$4" 2>/dev/null; then break; fi
    sleep 0.1
  done
  [ -n "$got" ] || got=$(grep -m1 -oE "$2" "$3" 2>/dev/null || true)
  printf -v "$1" '%s' "$got"
}

listening() { # <port>: the addresses a socket listens on at that port (IPv4 dotted, IPv6 as /proc writes it)
  local hex file _ local_addr state a
  printf -v hex '%04X' "$1"
  echo "== $http_phase listens on"
  for file in tcp tcp6; do
    [ -r "/proc/net/$file" ] || continue
    while read -r _ local_addr _ state _; do
      if [ "$state" != 0A ] || [ "${local_addr#*:}" != "$hex" ]; then continue; fi
      a=${local_addr%:*}
      if [ "$file" = tcp ]; then
        printf 'tcp %d.%d.%d.%d\n' "0x${a:6:2}" "0x${a:4:2}" "0x${a:2:2}" "0x${a:0:2}"
      else
        echo "tcp6 $a"
      fi
    done <"/proc/net/$file"
  done | LC_ALL=C sort
}

refusals() { # <addr> <token>: what the server must refuse, and what it must not
  local addr=$1 token=$2 port=${1##*:} last=${2: -1} other
  if [ "$last" = 0 ]; then other=1; else other=0; fi
  http "no token" 403 json "$addr/api/status"
  http "a longer token" 403 json -H "x-freespace-token: 0$token" "$addr/api/status"
  http "a token one character short" 403 json -H "x-freespace-token: ${token%?}" "$addr/api/status"
  http "a token with its last character changed" 403 json -H "x-freespace-token: ${token%?}$other" "$addr/api/status"
  http "a foreign origin" 403 json -H "x-freespace-token: $token" -H "Origin: http://evil.example" "$addr/api/status"
  http "a cross-site fetch" 403 json -H "x-freespace-token: $token" -H "Sec-Fetch-Site: cross-site" "$addr/api/status"
  http "a same-site fetch" 403 json -H "x-freespace-token: $token" -H "Sec-Fetch-Site: same-site" "$addr/api/status"
  http "another host name" 403 json -H "x-freespace-token: $token" -H "Host: evil.example" "$addr/api/status"
  http "another host name for the page" 403 json -H "Host: evil.example" "$addr/"
  http "a POST" 405 json -X POST -H "x-freespace-token: $token" "$addr/api/status"
  http "the host name localhost" 200 json -H "x-freespace-token: $token" -H "Host: localhost:$port" "$addr/api/status"
  http "its own origin" 200 json -H "x-freespace-token: $token" -H "Origin: http://127.0.0.1:$port" "$addr/api/status"
  http "the origin localhost" 200 json -H "x-freespace-token: $token" -H "Origin: http://localhost:$port" "$addr/api/status"
  http "a HEAD" 200 none --head -H "x-freespace-token: $token" "$addr/api/status"
  http "the page" 200 page "$addr/"
}

stopped() { # <pid> <pid to TERM> <what> <stdout> [<stderr>]: TERM the one, wait for the other; its status and what it printed
  local code=0
  kill -TERM "$2" 2>/dev/null || true
  wait "$1" || code=$?
  # shellcheck disable=SC2034 # the EXIT trap of scripts/rust-diff.sh reads it
  serving=
  {
    echo "== $3 stopped by TERM"
    echo "exit $code"
    echo "-- stdout"
    text_of "$4"
    if [ -n "${5-}" ]; then
      echo "-- stderr"
      text_of "$5"
    fi
  } | mask
}

serve_answers() { # <bin>: serve the fixture snapshot, ask SERVE_QUESTIONS and the refusals, stop
  local bin=$1 argv pid url addr token q label path params want p
  local -a args ps
  run_prefix
  rm -f "$work/serve.err"
  served=0 http_phase=serve
  (exec "${argv[@]}" "$bin" serve --port 0 >"$work/serve.out" 2>"$work/serve.err") &
  # shellcheck disable=SC2034 # the EXIT trap of scripts/rust-diff.sh reads it
  pid=$! serving=$!
  # the whole address: a line half written must not pass for a shorter port or token
  wait_for url 'http://127\.0\.0\.1:[0-9]+/#t=[0-9a-f]{64}' "$work/serve.err" "$pid"
  if [ -z "$url" ]; then
    floor serve start "no address" "an address"
    echo "== serve did not print its address"
  else
    addr=${url%%/#*} token=${url##*#t=}
    for q in "${SERVE_QUESTIONS[@]}"; do
      IFS='|' read -r label path params want <<<"$q"
      args=()
      read -ra ps <<<"$params"
      for p in "${ps[@]}"; do args+=(--data-urlencode "${p/=@/=$tree}"); done
      http "$label" "$want" json -G -H "x-freespace-token: $token" "${args[@]}" "$addr$path"
    done
    refusals "$addr" "$token"
    listening "${addr##*:}" | mask
  fi
  stopped "$pid" "$pid" serve "$work/serve.out" "$work/serve.err"
}
