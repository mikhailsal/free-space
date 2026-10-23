// Numbers and dates as the CLI prints them (core/src/format.rs).

const UNITS = ["K", "M", "G", "T", "P", "E"];

export function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  return v < 10 ? `${v.toFixed(1)} ${UNITS[i]}` : `${Math.round(v)} ${UNITS[i]}`;
}

/** "+9.8 G" / "−1.2 G" with a true minus; "—" for 0. */
export function delta(d: number): string {
  if (d === 0) return "—";
  return d > 0 ? `+${size(d)}` : `−${size(-d)}`;
}

export function signed(n: number): string {
  if (n === 0) return "—";
  return n > 0 ? `+${count(n)}` : `−${count(-n)}`;
}

/** 2 104 331 (narrow no-break spaces). */
export function count(n: number): string {
  return Math.round(n)
    .toString()
    .replace(/\B(?=(\d{3})+(?!\d))/g, " ");
}

/** Share of a folder: "59%", "<1%", "0%". */
export function pct(part: number, whole: number): string {
  if (whole <= 0 || part <= 0) return "0%";
  const p = (part / whole) * 100;
  return p >= 1 ? `${Math.round(p)}%` : "<1%";
}

function pad(n: number): string {
  return n < 10 ? `0${n}` : `${n}`;
}

export function date(ts: number): string {
  if (!ts) return "unknown";
  const d = new Date(ts * 1000);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

export function hm(ts: number): string {
  const d = new Date(ts * 1000);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function dateTime(ts: number): string {
  return ts ? `${date(ts)} ${hm(ts)}` : "unknown";
}

/** Modified column: "today", "3 d", "4 mo", "2 y". */
export function ago(ts: number, now: number): string {
  if (!ts) return "unknown";
  const days = Math.floor((now - ts) / 86400);
  if (days < 1) return "today";
  if (days < 45) return `${days} d`;
  if (days < 730) return `${Math.round(days / 30)} mo`;
  return `${Math.floor(days / 365)} y`;
}

/** Snapshot age: "40 s", "12 min", "7h", "3 d". */
export function age(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  if (s < 60) return `${s} s`;
  if (s < 3600) return `${Math.floor(s / 60)} min`;
  if (s < 86400 * 2) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)} d`;
}

/** Elapsed time of a running scan: "48 s", "1 m 12 s", "1 h 05 m". */
export function elapsed(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  if (s < 60) return `${s} s`;
  if (s < 3600) return `${Math.floor(s / 60)} m ${s % 60} s`;
  return `${Math.floor(s / 3600)} h ${pad(Math.floor((s % 3600) / 60))} m`;
}

/** "2.1 M" for entry counts in the scanning line. */
export function bigCount(n: number): string {
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)} M`;
  if (n >= 1e4) return `${Math.round(n / 1e3)} K`;
  return count(n);
}

/** Single-quote for a POSIX shell unless the word is plain. */
export function shellWord(s: string): string {
  if (/^[A-Za-z0-9_\/.,:=+@%-]+$/.test(s)) return s;
  return `'${s.replace(/'/g, `'\\''`)}'`;
}
