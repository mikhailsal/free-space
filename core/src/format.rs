//! Human formatting shared by CLI text output and messages.

/// Binary units with one decimal under 10: "512 B", "1.5 K", "14 G".
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["K", "M", "G", "T", "P", "E"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64 / 1024.0;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if v < 10.0 {
        format!("{v:.1} {}", UNITS[i])
    } else {
        format!("{} {}", v.round() as u64, UNITS[i])
    }
}

/// Signed size: "+9.0 G", "−120 M".
pub fn delta(d: i64) -> String {
    if d >= 0 {
        format!("+{}", size(d as u64))
    } else {
        format!("−{}", size(d.unsigned_abs()))
    }
}

pub fn age(seconds: i64) -> String {
    let s = seconds.max(0);
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else if s < 86400 * 2 {
        format!("{} h", s / 3600)
    } else {
        format!("{} d", s / 86400)
    }
}

fn local_tm(ts: i64) -> libc::tm {
    let t: libc::time_t = ts as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&t, &mut tm);
    }
    tm
}

/// "12:03" in local time.
pub fn local_hm(ts: i64) -> String {
    let tm = local_tm(ts);
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

/// "2026-10-23" in local time; "unknown" for 0.
pub fn local_date(ts: i64) -> String {
    if ts == 0 {
        return "unknown".into();
    }
    let tm = local_tm(ts);
    format!(
        "{:04}-{:02}-{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday
    )
}

/// Local midnight of YYYY-MM-DD as a UNIX timestamp.
pub fn local_midnight(y: i32, m: u32, d: u32) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = y - 1900;
    tm.tm_mon = m as i32 - 1;
    tm.tm_mday = d as i32;
    tm.tm_isdst = -1;
    let t = unsafe { libc::mktime(&mut tm) };
    if t == -1 || tm.tm_mday != d as i32 {
        return None;
    }
    Some(t as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(1536), "1.5 K");
        assert_eq!(size(14 << 30), "14 G");
        assert_eq!(delta(-(120 << 20)), "−120 M");
    }

    #[test]
    fn midnight_roundtrip() {
        let t = local_midnight(2023, 1, 1).unwrap();
        assert_eq!(local_date(t), "2023-01-01");
        assert_eq!(local_hm(t), "00:00");
        assert!(local_midnight(2023, 2, 30).is_none());
    }
}
