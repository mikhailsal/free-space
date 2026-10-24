# shellcheck shell=bash
# shellcheck disable=SC2154 # $work, $tree and the helpers are scripts/rust-diff.sh's
# The serve phase of scripts/rust-diff.sh, sourced by it: each build serves the
# fixture snapshot on a free port (`serve --port 0`), answers these requests
# with its token, and stops on TERM. Compared: the HTTP status, the headers
# (all but Date and Content-Length, which follows the times in the body), the
# body normalized like --json, what serve prints, its exit status, and the
# snapshot folder afterwards. The page itself is asked for its status and
# headers only: its bytes are the web bundle, which pixel-diff compares drawn.
# curl reads no ~/.curlrc and takes no proxy from the environment.

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

served=0
http() { # <label> <want> <body: json or skip> <curl args...>: one request: its status, headers and body
  local label=$1 want=$2 body=$3 code
  shift 3
  served=$((served + 1))
  code=$(curl -q --noproxy '*' -sS --max-time 30 -o "$work/body" -D "$work/headers" -w '%{http_code}' "$@" 2>"$work/curl.err") ||
    code="curl failed: $(head -c 300 "$work/curl.err")"
  floor serve "$label" "$code" "$want"
  {
    echo "== serve: $label"
    echo "http $code"
    echo "-- headers"
    tr -d '\r' <"$work/headers" | sed -n '2,$p' | sed -E '/^$/d; s/^([^:]+):/\L\1:/' |
      grep -vE '^(date|content-length):' | LC_ALL=C sort || true
    echo "-- body"
    if [ "$body" = skip ]; then echo "(the web bundle: pixel-diff compares it)"; else normalize_json "$work/body" false; fi
  } | mask
}

serve_answers() { # <bin>: serve the fixture snapshot, ask SERVE_QUESTIONS and the refusals, stop
  local bin=$1 argv pid i url='' addr token q label path params want p code=0
  local -a args ps
  run_prefix
  rm -f "$work/serve.err"
  served=0
  (exec "${argv[@]}" "$bin" serve --port 0 >"$work/serve.out" 2>"$work/serve.err") &
  pid=$! serving=$!
  for ((i = 0; i < 100; i++)); do
    url=$(grep -m1 -oE 'http://127\.0\.0\.1:[0-9]+/#t=[0-9a-f]+' "$work/serve.err" 2>/dev/null || true)
    if [ -n "$url" ] || ! kill -0 "$pid" 2>/dev/null; then break; fi
    sleep 0.1
  done
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
    http "no token" 403 json "$addr/api/status"
    http "a wrong token" 403 json -H "x-freespace-token: 0$token" "$addr/api/status"
    http "a foreign origin" 403 json -H "x-freespace-token: $token" -H "Origin: http://evil.example" "$addr/api/status"
    http "another host name" 403 json -H "x-freespace-token: $token" -H "Host: evil.example" "$addr/api/status"
    http "a POST" 405 json -X POST -H "x-freespace-token: $token" "$addr/api/status"
    http "the page" 200 skip "$addr/"
  fi
  kill -TERM "$pid" 2>/dev/null || true
  wait "$pid" || code=$?
  # shellcheck disable=SC2034 # the EXIT trap of scripts/rust-diff.sh reads it
  serving=
  {
    echo "== serve stopped by TERM"
    echo "exit $code"
    echo "-- stdout"
    text_of "$work/serve.out"
    echo "-- stderr"
    text_of "$work/serve.err"
  } | mask
}
