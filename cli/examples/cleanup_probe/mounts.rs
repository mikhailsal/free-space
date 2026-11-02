use crate::model::R;
use std::collections::HashSet;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

fn decode(raw: &[u8]) -> R<Vec<u8>> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\\' {
            if i + 3 >= raw.len() || !(1..=3).all(|j| (b'0'..=b'7').contains(&raw[i + j])) {
                return Err("invalid mountinfo escape".into());
            }
            let v = (raw[i + 1] - b'0') as u16 * 64
                + (raw[i + 2] - b'0') as u16 * 8
                + (raw[i + 3] - b'0') as u16;
            out.push(u8::try_from(v)?);
            i += 4;
        } else {
            out.push(raw[i]);
            i += 1;
        }
    }
    Ok(out)
}
pub fn nested(root: &Path) -> R<HashSet<PathBuf>> {
    let raw = std::fs::read("/proc/self/mountinfo")?;
    if raw.len() > 16 * 1024 * 1024 {
        return Err("mountinfo exceeds bound".into());
    }
    let mut paths = HashSet::new();
    for line in raw.split(|b| *b == b'\n').filter(|s| !s.is_empty()) {
        let field = line
            .split(|b| *b == b' ')
            .nth(4)
            .ok_or("invalid mountinfo")?;
        let path = PathBuf::from(OsString::from_vec(decode(field)?));
        if path != root && path.starts_with(root) {
            paths.insert(path);
        }
        if paths.len() > 100_000 {
            return Err("mountpoint limit exceeded".into());
        }
    }
    Ok(paths)
}
