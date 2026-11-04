"""Independent Python reference for the bucketed coarse snapshot.

The Rust probe (cli/examples/cleanup_probe/buckets.rs) and this module must agree on the name
hash and the greedy tiling; the oracle uses it to verify a snapshot against the full v1 baseline and
the host harness uses it to predict how much work a bucket refresh must report.
"""

SLOT_BITS = 12
SLOTS = 1 << SLOT_BITS
BUCKET_ENTRIES = 4096
FORMAT = 2
_MASK = (1 << 64) - 1


def fnv1a(name):
    """FNV-1a, 64 bit, over the raw bytes of a name (the published algorithm)."""
    h = 0xcbf29ce484222325
    for byte in name:
        h ^= byte
        h = (h * 0x100000001b3) & _MASK
    return h


def slot_of(name):
    """FNV-1a, then a Fibonacci mix; the top SLOT_BITS bits are the slot."""
    return ((fnv1a(name) * 0x9e3779b97f4a7c15) & _MASK) >> (64 - SLOT_BITS)


class Stats:
    """Aggregate of folded entries: bytes, entry counts, errors, hardlinks and newest file mtime."""
    __slots__ = ('allocated', 'apparent', 'files', 'dirs', 'others', 'errors', 'links', 'newest')

    def __init__(self):
        self.allocated = self.apparent = self.files = self.dirs = self.others = self.errors = self.links = 0
        self.newest = None

    @property
    def entries(self):
        return self.files + self.dirs + self.others

    def add(self, other):
        for field in ('allocated', 'apparent', 'files', 'dirs', 'others', 'errors', 'links'):
            setattr(self, field, getattr(self, field) + getattr(other, field))
        if other.newest is not None:
            self.newest = other.newest if self.newest is None else max(self.newest, other.newest)

    def add_totals(self, totals, newest):
        """Add one folded child given as [allocated, apparent, files, dirs, others, errors]."""
        self.allocated += totals[0]
        self.apparent += totals[1]
        self.files += totals[2]
        self.dirs += totals[3]
        self.others += totals[4]
        self.errors += totals[5]
        if newest is not None:
            self.newest = newest if self.newest is None else max(self.newest, newest)


def tile(slots, cap=BUCKET_ENTRIES, lo=0, hi=SLOTS):
    """Greedy tiling of [lo, hi): close a bucket before the slot that would push it over `cap`.

    `slots` maps slot -> Stats. A single slot may exceed `cap`; there is always at least one bucket.
    Returns a list of (lo, hi, Stats).
    """
    if not 0 <= lo < hi <= SLOTS:
        raise ValueError('invalid slot range')
    out, start, acc = [], lo, Stats()
    for slot in sorted(s for s in slots if lo <= s < hi):
        stats = slots[slot]
        if acc.entries > 0 and acc.entries + stats.entries > cap:
            out.append((start, slot, acc))
            start, acc = slot, Stats()
        acc.add(stats)
    out.append((start, hi, acc))
    return out


def validate(buckets):
    """Buckets of one directory must be contiguous and cover every slot exactly once."""
    nxt = 0
    for lo, hi in buckets:
        if lo != nxt or hi <= lo or hi > SLOTS:
            raise ValueError('bucket tiling is not contiguous')
        nxt = hi
    if nxt != SLOTS:
        raise ValueError('bucket tiling does not cover all slots')


def range_of(names, entries_each, target):
    """Slot range and entry count of the bucket that holds `target` among equal-sized children."""
    slots = {}
    for name in names:
        stats = slots.setdefault(slot_of(name), Stats())
        stats.files += entries_each
    wanted = slot_of(target)
    for lo, hi, stats in tile(slots):
        if lo <= wanted < hi:
            return lo, hi, stats.entries
    raise ValueError('target slot is not covered')
