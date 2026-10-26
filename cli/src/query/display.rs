use freespace_core::format;
use freespace_core::queries::types::{DiffEntry, DiffStatus, EntryOut};
use freespace_core::scan::Kind;

use crate::output;
use crate::scan::count;

pub(super) fn diff_path_cell(e: &DiffEntry) -> String {
    let mut path = e.path.clone();
    if e.path_raw.is_some() {
        path.push_str("  [name is not UTF-8, see --json path_raw]");
    }
    path
}

pub(super) fn stamp(ts: i64) -> String {
    format!("{} {}", format::local_date(ts), format::local_hm(ts))
}

pub(super) fn items(e: &EntryOut) -> String {
    match e.kind {
        Kind::Dir | Kind::Mount => count(e.items),
        _ => String::new(),
    }
}

pub(super) fn items_delta(e: &DiffEntry) -> String {
    let d = e.items as i64 - e.items_prev as i64;
    match d.cmp(&0) {
        std::cmp::Ordering::Greater => format!("+{}", count(d as u64)),
        std::cmp::Ordering::Less => format!("−{}", count(d.unsigned_abs())),
        std::cmp::Ordering::Equal => String::new(),
    }
}

pub(super) fn status_badge(e: &DiffEntry) -> String {
    match e.status {
        DiffStatus::New => "new".into(),
        DiffStatus::Gone => "gone".into(),
        DiffStatus::Changed | DiffStatus::Same => String::new(),
    }
}

/// Name plus plain-text markers: "/" for folders, "@" for symlinks, and
/// bracketed notes for mounts, unreadable folders and hardlinks.
pub(super) fn name_cell(name: &str, e: &EntryOut) -> String {
    let mut s = name.to_owned();
    match e.kind {
        Kind::Dir => s.push('/'),
        Kind::Symlink => s.push('@'),
        _ => {}
    }
    s.push_str(&markers(e));
    s
}

pub(super) fn path_cell(e: &EntryOut) -> String {
    let mut s = e.path.clone();
    if e.kind == Kind::Dir && s != "/" {
        s.push('/');
    }
    s.push_str(&markers(e));
    s
}

fn markers(e: &EntryOut) -> String {
    let mut s = String::new();
    if let Some(m) = &e.mount {
        s.push_str(&format!("  [mount {}, not scanned here]", m.fs_type));
    }
    if e.denied {
        s.push_str("  [unreadable]");
    }
    if e.hardlink_alias {
        match &e.hardlink_owner {
            Some(o) => s.push_str(&format!("  [hardlink, counted at {o}]")),
            None => s.push_str("  [hardlink, counted elsewhere]"),
        }
    }
    if e.path_raw.is_some() {
        s.push_str("  [name is not UTF-8, see --json path_raw]");
    }
    s
}

pub(super) fn empty_note(e: &EntryOut) -> String {
    if e.denied {
        "Contents unavailable: this folder could not be read as root.\n".into()
    } else if let Some(m) = &e.mount {
        format!(
            "Not scanned here: separate filesystem ({}).\n",
            output::escape(&m.fs_type)
        )
    } else if e.kind != Kind::Dir {
        format!(
            "{} is a {}, not a folder.\n",
            output::escape(&e.path),
            kind_word(e.kind)
        )
    } else {
        "Nothing here in this snapshot.\n".into()
    }
}

fn kind_word(k: Kind) -> &'static str {
    match k {
        Kind::File => "file",
        Kind::Dir => "folder",
        Kind::Symlink => "symlink",
        Kind::Mount => "mount point",
        Kind::Other => "special file",
    }
}

pub(super) fn more_line(total: u64, shown: usize, flag: &str) -> String {
    format!(
        "… {} more (show them with {flag} {})\n",
        count(total.saturating_sub(shown as u64)),
        total.min(10_000)
    )
}
