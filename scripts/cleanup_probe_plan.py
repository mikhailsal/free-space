"""Frozen plan of the bucketed-probe experiment, shared by the host harness, the forecast and the contract tests.

Two arms run on identical fixtures: v1 (the baseline backend) and the bucketed coarse probe. The scenarios,
their order and the work each arm must report are fixed here before any measurement and must not be
tuned after results are seen. v2 is not run; its numbers from the earlier screen are only quoted as a reference.
"""
import cleanup_buckets as cb

ARMS = ('v1', 'probe')
PROFILES = ('files', 'directories')
SEED_SCANS = 2
STAGES = {'smoke': dict(count=100_000, pairs=1), 'screen': dict(count=1_000_000, pairs=3)}
ROOT = '/scratch/tree'
SCENARIOS = (('full', 'scan', ROOT),
             ('retained', 'refresh', ROOT + '/large'),
             ('omitted', 'refresh', ROOT + '/small'),
             ('wide', 'refresh', ROOT + '/bulk/g000050'))
LARGE_ENTRIES = 74    # 'large': 73 descendants plus itself
SMALL_ENTRIES = 100   # 'small': 99 empty files plus itself
CHUNK_ENTRIES = 1001  # one bulk chunk directory plus its 1000 children
WIDE_TARGET = b'g000050'
ORDERS = (('v1', 'probe'), ('probe', 'v1'))


def order(scenario_index, pair):
    """Rotate which arm runs first so neither arm always follows the other."""
    return ORDERS[(scenario_index + pair) % len(ORDERS)]


def expected(stage):
    """Number of writer cases, oracle and cycle phases and outer-space trims a successful run must show."""
    pairs = STAGES[stage]['pairs']
    cycles = len(PROFILES) * len(SCENARIOS) * pairs
    runs = len(SCENARIOS) * pairs * len(ARMS)
    return dict(cases=len(PROFILES) * (len(ARMS) * SEED_SCANS + runs), oracles=cycles, cycles=cycles,
                trims=len(PROFILES) * (1 + len(ARMS) + runs))


def chunk_names(count):
    """Names of the bulk chunk directories the fixture creates for `count` bulk entries."""
    return [f'g{i:06}'.encode() for i in range(count // 1000)]


def subtree_sizes(fixture):
    """Entry counts of the 'large' and 'small' subtrees, derived from the fixture receipt."""
    counts = fixture['expected_counts']
    large = fixture['controls']['group']['files'] + 10
    # The receipt counts 5 other control files, the raw-name file and the group children;
    # adding the small directory cancels that raw file.
    small = counts['files'] - fixture['bulk_files'] - fixture['controls']['group']['files'] - 5
    if (large, small) != (LARGE_ENTRIES, SMALL_ENTRIES):
        raise ValueError('fixture subtree contract differs')
    return large, small


def wide_range(count):
    """Slot range and entries of the bucket that holds the wide target among the equal-sized chunk directories."""
    return cb.range_of(chunk_names(count), CHUNK_ENTRIES, WIDE_TARGET)


def v1_scope(scenario, fixture):
    large, small = subtree_sizes(fixture)
    return {'full': dict(entries=fixture['expected_counts']['entries']),
            'retained': dict(subtree_entries=large),
            'omitted': dict(subtree_entries=small),
            'wide': dict(subtree_entries=CHUNK_ENTRIES)}[scenario]


def probe_scope(scenario, fixture):
    """Work the bucketed probe must report: each scan pass visits the scan root plus every admitted subtree."""
    large, small = subtree_sizes(fixture)
    if scenario == 'full':
        return dict(visited=2 * fixture['expected_counts']['entries'], boundary=ROOT, scope='full', fallback_full_root=False, range=None)
    if scenario == 'retained':
        return dict(visited=2 * large, boundary=ROOT + '/large', scope='retained', fallback_full_root=False, range=None)
    if scenario == 'omitted':
        # The root retains 'large' and 'bulk', so its one bucket spans every slot and holds only 'small'.
        window = dict(lo=0, hi=cb.SLOTS, old_entries=small, new_entries=small)
        return dict(visited=2 * (1 + small), boundary=ROOT, scope='range', fallback_full_root=False, range=window)
    lo, hi, entries = wide_range(fixture['scale'])
    window = dict(lo=lo, hi=hi, old_entries=entries, new_entries=entries)
    return dict(visited=2 * (1 + entries), boundary=ROOT + '/bulk', scope='range', fallback_full_root=False, range=window)


def scope(arm, scenario, fixture):
    return (probe_scope if arm == 'probe' else v1_scope)(scenario, fixture)
