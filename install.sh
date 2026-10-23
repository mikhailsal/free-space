#!/bin/sh
# freespace installer (T36). Downloads the release binary for this machine,
# checks its sha256, prints what it will do, then with sudo installs it to
# /usr/local/bin, writes shell completions and runs `freespace install-timer`
# (nightly scan, read access for you, first scan now). The Claude Code skill
# goes to your ~/.claude/skills/freespace.
#
#   curl -fsSL https://raw.githubusercontent.com/mikhailsal/free-space/master/install.sh | sh
#   curl -fsSL …/install.sh | sh -s -- --version 0.1.0    # that release; also how to roll back
#   curl -fsSL …/install.sh | sh -s -- -- --roots / /home  # after "--": install-timer options
#
# Rerunning upgrades in place; the snapshot is kept unless its schema changed.
# FREESPACE_RELEASES stands for https://github.com/mikhailsal/free-space/releases
# (a URL or a local directory with the same layout), for mirrors and tests.
set -eu

REPO=mikhailsal/free-space
BIN=/usr/local/bin/freespace
INSTALL_LINE="curl -fsSL https://raw.githubusercontent.com/$REPO/master/install.sh | sh"
START=$(date +%s)

say() { printf '%s\n' "$*" >&2; }
die() {
    say "freespace install: $*"
    exit 1
}

usage() {
    cat >&2 <<USAGE
usage: install.sh [--version X] [--no-timer] [--no-skill] [-- install-timer options]
  --version X   install release X (default: the latest); also rolls back
  --no-timer    binary, completions and skill only; no nightly scan
  --no-skill    do not install the Claude Code skill
USAGE
}

version=""
timer=1
skill=1
while [ "$#" -gt 0 ]; do
    case $1 in
        --version)
            [ "$#" -ge 2 ] || die "--version needs a value, e.g. --version 0.1.0"
            version=${2#v}
            shift 2
            ;;
        --version=*)
            version=${1#--version=}
            version=${version#v}
            shift
            ;;
        --no-timer) timer=0; shift ;;
        --no-skill) skill=0; shift ;;
        -h | --help) usage; exit 0 ;;
        --) shift; break ;;
        *) usage; die "unknown option $1" ;;
    esac
done
# whatever is left goes to install-timer

[ "$(uname -s)" = Linux ] || die "freespace runs on Linux with systemd; this is $(uname -s)"
case $(uname -m) in
    x86_64 | amd64) target=x86_64-unknown-linux-musl ;;
    aarch64 | arm64) target=aarch64-unknown-linux-musl ;;
    *) die "no release build for $(uname -m); build from source: https://github.com/$REPO#from-source" ;;
esac

if [ "$(id -u)" -eq 0 ]; then
    SUDO=""
    user=${SUDO_USER:-}
    [ "$user" = root ] && user=""
else
    SUDO=sudo
    user=$(id -un)
fi
need="curl tar sha256sum mktemp"
[ -n "$SUDO" ] && need="$need sudo"
for t in $need; do
    command -v "$t" > /dev/null 2>&1 || die "$t is missing; install it and rerun"
done

releases=${FREESPACE_RELEASES:-https://github.com/$REPO/releases}
if [ -n "$version" ]; then
    base="$releases/download/v$version"
else
    base="$releases/latest/download"
fi
archive="freespace-$target.tar.gz"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

fetch() {
    case $1 in
        /*) cp "$1" "$2" ;;
        *) curl -fsSL --proto '=https,file' --retry 3 -o "$2" "$1" ;;
    esac
}
fetch "$base/$archive" "$tmp/$archive" ||
    die "cannot download $base/$archive${version:+ (is $version a release?)}"
fetch "$base/$archive.sha256" "$tmp/sum" || die "cannot download $base/$archive.sha256"
want=$(cut -d ' ' -f 1 < "$tmp/sum")
have=$(sha256sum "$tmp/$archive" | cut -d ' ' -f 1)
[ "$want" = "$have" ] || die "sha256 mismatch for $archive: expected $want, got $have"
tar -xzf "$tmp/$archive" -C "$tmp"
new=$(find "$tmp" -type f -name freespace | head -n 1)
[ -n "$new" ] || die "no freespace binary in $archive"
chmod 0755 "$new"
ver=$("$new" --version | cut -d ' ' -f 2) || die "the downloaded binary does not run on this machine"

old=""
if [ -x "$BIN" ]; then
    old=$("$BIN" --version 2> /dev/null | cut -d ' ' -f 2 || true)
fi
home=""
if [ -n "$user" ]; then
    home=$(getent passwd "$user" | cut -d : -f 6)
fi

say "freespace $ver ($target) will:"
say "  install the binary   $BIN (root, 0755)${old:+, replacing $old}"
say "  add completions      bash, zsh, fish under /usr/local/share"
if [ "$timer" = 1 ]; then
    say "  add a nightly scan   freespace.timer, first scan now (freespace install-timer)"
    [ -n "$user" ] && say "  give $user read access to the snapshots (ACL; group freespace without ACL support)"
fi
if [ "$skill" = 1 ] && [ -n "$home" ]; then
    say "  install the skill    $home/.claude/skills/freespace (remove: freespace skill uninstall)"
fi

if [ -n "$SUDO" ] && ! sudo -n true 2> /dev/null; then
    # sudo prompts on the controlling terminal; without one it can only fail
    if ! (: < /dev/tty) 2> /dev/null; then
        say "sudo needs a password and no terminal is attached."
        say "fix: ask a human to run in their terminal: $INSTALL_LINE"
        exit 1
    fi
    say "sudo may ask for your password."
fi

$SUDO install -o root -g root -m 0755 "$new" "$BIN"
for sh in bash zsh fish; do
    case $sh in
        bash) dest=/usr/local/share/bash-completion/completions/freespace ;;
        zsh) dest=/usr/local/share/zsh/site-functions/_freespace ;;
        fish) dest=/usr/local/share/fish/vendor_completions.d/freespace.fish ;;
    esac
    "$BIN" completions "$sh" > "$tmp/completion"
    $SUDO install -D -o root -g root -m 0644 "$tmp/completion" "$dest"
done
if [ "$timer" = 1 ]; then
    $SUDO "$BIN" install-timer "$@"
fi
if [ "$skill" = 1 ] && [ -n "$home" ]; then
    # as the user, so the files are theirs; under sudo the binary chowns them
    "$BIN" skill install > /dev/null
fi

"$BIN" --version > /dev/null
smoke=""
"$BIN" status --json > /dev/null 2>&1 || smoke="status"
if ! "$BIN" doctor --json > /dev/null 2>&1; then
    smoke="${smoke:+$smoke, }doctor"
fi
took=$(($(date +%s) - START))
line="freespace $ver installed in $took s"
if [ -n "$old" ] && [ "$old" != "$ver" ]; then
    line="$line (was $old)"
fi
say "$line · Open: freespace gui"
if [ -n "$smoke" ]; then
    say "note: $smoke did not pass yet (normal while the first scan runs); check: freespace doctor"
fi
