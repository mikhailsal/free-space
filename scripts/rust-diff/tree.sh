# shellcheck shell=bash
# shellcheck disable=SC2154 # $work, $tree and the like are scripts/rust-diff.sh's
# The made-up trees of scripts/rust-diff.sh, sourced by it. Every function works
# on the folder in $tree, which the caller sets (a `local tree=...` reaches the
# functions it calls). The trees hold what a scan meets on a real disk: hard
# links (inside one folder, across folders and across the folder refresh
# rewalks), a sparse file, a symlink, a file and a folder whose names are not
# UTF-8, an empty folder, one nobody may read, and a wide, deep part for several
# walker threads.

blob() { # <path> <KiB> <mtime>: a file of that size, modified then, its blocks taken at once
  mkdir -p "$(dirname "$1")"
  # Blocks a write leaves for the kernel to place at writeback can land in more than four extents on a fragmented
  # disk, and ext4 then takes one block more for the extent tree: the same file reads 4 K larger after writeback than
  # before (base against itself differs, exit 3). Preallocated, it is one extent with its size fixed; the zeros are
  # then written in place (the same inode when the file grows; no truncate on the write).
  : >"$1"
  fallocate -l "$(($2 * 1024))" "$1" 2>/dev/null || true
  head -c "$(($2 * 1024))" /dev/zero 1<>"$1"
  touch -d "$3" "$1"
}

# Every folder its own mtime, fixed in 2024, from its path inside the tree, so
# that folders differ in mtime (`--sort mtime` then orders by date, not only by
# its tie-break) and keep their time while others come and go: a djb2 hash of
# the path's bytes below DIR_HASH_MOD, in steps of DIR_STEP seconds from
# DIR_EPOCH, so at most 359 days in (1000002 x 31 s), still 2024. One perl sets
# them all, after the files changed.
DIR_EPOCH=1704067200 # 2024-01-01 00:00 UTC
DIR_HASH_MOD=1000003
DIR_STEP=31
settle_dirs() {
  chmod 700 "$tree/locked" # find must enter it
  find "$tree" -type d -print0 |
    TREE=$tree EPOCH=$DIR_EPOCH MOD=$DIR_HASH_MOD STEP=$DIR_STEP perl -0ne '
      chomp;
      my $h = 5381;
      $h = ($h * 33 + ord) % $ENV{MOD} for split //, substr($_, length $ENV{TREE});
      my $t = $ENV{EPOCH} + $h * $ENV{STEP};
      utime $t, $t, $_ or die "$_: $!\n";'
  chmod 000 "$tree/locked"
}

# The wide part: RUST_DIFF_FAN folders in each folder, four levels deep (340
# folders at the default 4), one to three files in each, sizes and dates from
# the position, and every file of every 37th folder linked into the folder
# before it. Files are written by the shell's printf and dated one touch per
# date, so the part costs a few hundred processes, not thousands.
make_wide() {
  local fan=${RUST_DIFF_FAN:-4} level d n i k size f prev=
  local -a dirs=("$tree/wide") next dated0=() dated1=() dated2=() dated3=() dated4=()
  for ((level = 0; level < 4; level++)); do
    next=()
    for d in "${dirs[@]}"; do
      for ((i = 0; i < fan; i++)); do next+=("$d/d$i"); done
    done
    dirs=("${next[@]}")
    mkdir -p "${dirs[@]}"
    n=0
    for d in "${dirs[@]}"; do
      for ((k = 0; k <= n % 3; k++)); do
        f=$d/f$k.dat
        size=$((((n * 7 + k * 13) % 23 + 1) * 512))
        printf '%*s' "$size" '' >"$f"
        case $((n % 5)) in
          0) dated0+=("$f") ;; 1) dated1+=("$f") ;; 2) dated2+=("$f") ;; 3) dated3+=("$f") ;; *) dated4+=("$f") ;;
        esac
        if ((n % 37 == 0)) && [ -n "$prev" ]; then ln "$f" "$prev/link$level-$k.dat"; fi
      done
      prev=$d
      n=$((n + 1))
    done
  done
  [ "${#dated0[@]}" -eq 0 ] || touch -d "2022-02-02 02:00 UTC" "${dated0[@]}"
  [ "${#dated1[@]}" -eq 0 ] || touch -d "2023-03-03 03:00 UTC" "${dated1[@]}"
  [ "${#dated2[@]}" -eq 0 ] || touch -d "2024-04-04 04:00 UTC" "${dated2[@]}"
  [ "${#dated3[@]}" -eq 0 ] || touch -d "2024-08-08 08:00 UTC" "${dated3[@]}"
  [ "${#dated4[@]}" -eq 0 ] || touch -d "2024-10-10 10:00 UTC" "${dated4[@]}"
}

make_tree() {
  blob "$tree/projects/app/src/main.rs" 12 "2024-09-10 08:00 UTC"
  blob "$tree/projects/app/node_modules/pkg/index.js" 64 "2024-02-01 08:00 UTC"
  blob "$tree/projects/old-site/node_modules/lib/big.js" 180 "2023-05-05 08:00 UTC"
  blob "$tree/projects/old-site/index.html" 8 "2023-05-05 08:00 UTC"
  blob "$tree/media/iso/debian.iso" 400 "2024-01-20 08:00 UTC"
  ln "$tree/media/iso/debian.iso" "$tree/media/iso/debian-copy.iso"
  # a third link inside the folder refresh rewalks: its attribution crosses that boundary
  mkdir -p "$tree/logs"
  ln "$tree/media/iso/debian.iso" "$tree/logs/debian.link"
  truncate -s 50M "$tree/media/sparse.img"
  touch -d "2024-07-01 08:00 UTC" "$tree/media/sparse.img"
  blob "$tree/logs/app.log" 40 "2024-10-01 08:00 UTC"
  blob "$tree/logs/old.log" 16 "2022-12-31 08:00 UTC"
  blob "$tree/top.bin" 100 "2021-03-04 08:00 UTC"
  # past UTC midnight in UTC+5:30 only: a date bound must be local midnight to tell them apart
  blob "$tree/media/edge-new.bin" 4 "2024-09-30 20:00 UTC"
  blob "$tree/media/edge-old.bin" 4 "2023-03-03 20:00 UTC"
  # dated by age from the run's $epoch, days from any bound asked: 2w, 6mo, 400d and 1y split them
  blob "$tree/media/new/week.mkv" 200 "@$((${epoch:?} - 5 * 86400))"
  blob "$tree/media/new/season.mkv" 150 "@$((epoch - 100 * 86400))"
  blob "$tree/media/new/last-year.mkv" 120 "@$((epoch - 500 * 86400))"
  blob "$tree/odd/$(printf 'bad\377name.bin')" 4 "2024-04-04 08:00 UTC"
  blob "$tree/odd/$(printf 'dir\376')/inner.bin" 2 "2024-04-05 08:00 UTC"
  ln -s ../media "$tree/projects/media-link"
  touch -h -d "2024-06-06 06:00 UTC" "$tree/projects/media-link"
  mkdir -p "$tree/empty" "$tree/locked/inside"
  blob "$tree/locked/inside/secret.bin" 8 "2024-05-05 08:00 UTC"
  make_wide
  settle_dirs
}

change_tree() { # what the second scan sees: growth, a new file, a removal
  blob "$tree/media/iso/ubuntu.iso" 300 "2024-10-20 08:00 UTC"
  blob "$tree/logs/app.log" 90 "2024-10-21 08:00 UTC"
  blob "$tree/cache/blob.bin" 32 "2024-10-22 08:00 UTC"
  rm -r "$tree/projects/old-site"
  blob "$tree/wide/d1/d1/new.dat" 20 "2024-10-22 09:00 UTC"
  rm -rf "$tree/wide/d0/d0"
  settle_dirs
}

drift_tree() { # what refresh finds, after both scans: a folder changed, one gone, one now a file, one under a symlink
  blob "$tree/logs/app.log" 120 "2024-10-23 08:00 UTC"
  blob "$tree/logs/new.log" 24 "2024-10-23 09:00 UTC"
  rm "$tree/logs/old.log"
  rm -r "$tree/cache"
  rmdir "$tree/empty"
  blob "$tree/empty" 1 "2024-10-23 10:00 UTC"
  mv "$tree/projects/app" "$tree/projects/app.real"
  ln -s app.real "$tree/projects/app"
  touch -h -d "2024-10-23 11:00 UTC" "$tree/projects/app"
  settle_dirs
}

remove_tree() { # the folder nobody may read too
  [ ! -e "$tree" ] || { chmod -R u+rwx "$tree" && rm -rf "$tree"; }
}

# What the tree holds, for the check that no command changed it: names, types,
# modes, sizes, link counts and mtimes; not access times (reading a file sets
# them) or change times. The folder nobody may read is listed as it stands
# (find cannot enter it, and says so with exit 1).
tree_listing() {
  { find "$tree" -printf '%P %y %m %s %n %T@\n' 2>/dev/null || true; } | LC_ALL=C sort
}
