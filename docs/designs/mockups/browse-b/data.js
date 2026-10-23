// Mockup state: /home/tass, 97% disk, incomplete scan, .cache selected.
window.FS = {
  total: "206.8 G",
  rows: [
    // kind, name, allocated, share %, items, modified, unreadable-inside
    ["dir", ".cache", "121.4 G", 59, "2 104 331", "today", false],
    ["dir", "myprojects", "38.7 G", 19, "1 811 002", "today", true],
    ["dir", ".local", "14.2 G", 7, "402 110", "3 d", false],
    ["dir", "Downloads", "9.8 G", 5, "1 204", "2 d", false],
    ["dir", ".cargo", "6.1 G", 3, "98 230", "5 d", false],
    ["dir", "snap", "2.9 G", 1.4, "41 000", "1 mo", false],
    ["dir", ".rustup", "2.1 G", 1.0, "18 377", "3 mo", false],
    ["file", "big-dump.sql", "1.7 G", 0.8, "—", "9 mo", false],
    ["dir", ".npm", "1.1 G", 0.5, "64 210", "12 d", false],
    ["dir", ".vscode-server", "914 M", 0.44, "12 044", "2 d", false],
    ["dir", ".mozilla", "612 M", 0.3, "3 118", "today", false],
    ["dir", "Videos", "388 M", 0.19, "14", "8 mo", false],
  ],
  pct(p) { return p >= 1 ? Math.round(p) + "%" : "<1%"; },
  ge(r, s) { return r[6] && s !== "—" ? "≥ " + s : s; },
};
