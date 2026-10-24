# shellcheck shell=bash
# shellcheck disable=SC2034 # the lists are read by scripts/rust-diff.sh, which sources this file
# What scripts/rust-diff.sh asks each build. @ is the tree of the phase. A date
# bound is absolute; an age bound (`2w`) splits the files tree.sh dates by the
# run's $epoch, far from either side, so the answers do not move with the clock
# during a run.

# Every question, run once as written and once with --json: on the fixture
# snapshot, and again on the snapshot each build makes itself (the fresh phase).
QUERIES=(
  "status"
  "doctor"
  "ls @"
  "ls @ --sort apparent"
  "ls @ --sort mtime --limit 3"
  "ls @/projects/app"
  "ls @/odd"
  "ls @/wide/d2 --sort mtime"
  "ls @/nope"
  "ls @ --snapshot not-this-one"
  "top @"
  "top @ --min-size 100K --sort mtime"
  "top @ --older-than 2024-06-01"
  "top @ --older-than 1y --limit 5"
  "top @ --kind dir"
  "top @ --kind dir --older-than 2024-06-01 --limit 2"
  "top @/wide --kind dir --older-than 6mo --limit 6"
  "top @/media --older-than 400d --limit 3"
  "find @ --name *.iso"
  "find @ --iname NODE_MODULES --kind dir"
  "find @ --newer-than 2024-06-01 --min-size 10K --kind file"
  "find @ --name *.log --limit 1"
  "find @/wide --name link* --sort mtime --limit 40"
  "find @/media --older-than 2w"
  "find @ --newer-than 3y --kind file --limit 8"
  "diff @"
  "diff @ --depth 3 --kind any"
  "diff @ --kind file --limit 2"
)

# The date questions again with TZ at a half-hour offset: a YYYY-MM-DD bound is
# local midnight, so a build that takes it as UTC answers differently here.
TZ_QUERIES=(
  "ls @ --sort mtime --limit 4"
  "top @ --older-than 2024-06-01"
  "find @ --newer-than 2024-10-01 --kind file"
  "find @ --older-than 2023-03-04 --kind file --limit 4"
)
TZ_OTHER=Asia/Kolkata

# Asked on a terminal (under `script`), text only: what a person sees.
TTY_QUERIES=(
  ""
  "status"
  "ls @"
  "top @ --limit 3"
)

# Questions asked once, as written, on the fixture snapshot: a --json twin would
# mean nothing for texts, and the snapshot makes no difference to them (the
# bare run reads it, so it is here). The empty one is the bare run; --json is
# its summary. A usage error has a --json form of its own, listed too, and so
# does install-timer, which refuses to run without root.
ONCE=(
  ""
  "--json"
  "--version"
  "--help"
  "ls --no-such-flag"
  "ls --no-such-flag --json"
  "top @ --older-than 3x"
  "install-timer --no-start"
  "install-timer --no-start --json"
  "completions bash"
  "completions zsh"
  "completions fish"
  "schema"
  "skill print"
)
# `<command> --help` is asked of each of these (the list in `freespace --help`).
COMMANDS=(ls top find diff gui serve status doctor scan refresh install-timer skill schema completions)

# refresh, in this order, after drift_tree: a changed folder (text) that holds a
# third link of a file outside it, a folder gone from the disk (--json), a path
# the snapshot does not have and a file (both refused), and a folder of the wide
# part with 4 walker threads. Asked of a copy of the fixture snapshot, and of
# each build's own snapshot in the fresh phase. Then AFTER_REFRESH, text and
# --json. None of it needs root.
REFRESHES=(
  "refresh @/logs --workers 2"
  "refresh @/cache --workers 2 --json"
  "refresh @/nope --workers 2"
  "refresh @/top.bin --workers 2 --json"
  "refresh @/wide/d1 --workers 4"
)
AFTER_REFRESH=(
  "status"
  "ls @"
  "ls @/logs"
  "ls @/wide/d1 --sort apparent"
  "top @"
  "top @ --kind dir"
  "find @ --name *.log"
  "find @ --name debian*"
  "diff @"
)

# The config phase: a config file names two roots (in one tree), keeps no
# previous snapshot and sets the walker threads; `scan` without a root takes
# them, and refuses a path above both. Then a config with a key it does not
# know.
CONFIG_QUERIES=(
  "scan --json"
  "status"
  "ls @/projects"
  "diff @/media"
  "scan"
  "find @/media --name *.iso"
  "find @ --name *.iso"
  "top @/media --kind dir"
)
BAD_CONFIG_QUERIES=(
  "status"
  "scan"
)

# The start phase, text and --json: with no snapshot folder at all (the first
# run), with an empty one while a scan holds its lock (the first scan), and over
# a copy of the fixture snapshot while a scan holds its lock. The lock holds what
# a running scan writes into it (op, path, start, pid), at a fixed time.
EMPTY_QUERIES=(
  ""
  "status"
  "ls @"
  "doctor"
)
FIRST_QUERIES=(
  ""
  "status"
  "ls @"
)
BUSY_QUERIES=(
  ""
  "status"
  "ls @"
  "scan @ --workers 2"
  "refresh @/logs --workers 2"
)

# The exit status base must give, by phase and question (--json and text alike),
# and a few words its stderr must hold then; any other question must answer 0,
# and its --json form with JSON. A question base no longer answers as listed
# here (a renamed flag, a moved path, a refusal for another reason) would compare
# equal in base and head while it tests nothing, so the job stops as stale
# before head runs (exit 4).
EXPECTED=(
  "once|ls --no-such-flag|2|unexpected argument"
  "once|top @ --older-than 3x|2|bad date"
  "once|install-timer --no-start|1|needs root"
  "fixture|doctor|0 1|"
  "fixture|ls @/nope|2|is not in the snapshot"
  "fixture|ls @ --snapshot not-this-one|4|was replaced"
  "refresh|refresh @/nope --workers 2|2|is not in the snapshot"
  "refresh|refresh @/top.bin --workers 2|2|is not a directory"
  "empty|--json|8|no snapshot in"
  "empty|status|8|no snapshot in"
  "empty|ls @|8|no snapshot in"
  "empty|doctor|1|"
  "first|ls @|8|running since"
  "busy|scan @ --workers 2|6|another scan is running"
  "busy|refresh @/logs --workers 2|6|another scan is running"
  "fresh|doctor|0 1|"
  "fresh|ls @/nope|2|is not in the snapshot"
  "fresh|ls @ --snapshot not-this-one|4|was replaced"
  "config|diff @/media|3|no previous generation"
  "config|find @ --name *.iso|2|outside the scanned roots"
  "badconfig|status|2|keep_old"
  "badconfig|scan|2|keep_old"
)
