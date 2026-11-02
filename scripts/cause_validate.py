"""Independent receipt checks for resource-cause-v1, without executing workloads."""
import hashlib
import json
from pathlib import Path
import re

PROTOCOL = 'resource-cause-v1'
SHA = 'ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980'
REVISION = 'e8265778ef78ec4acd9da1375eedb0be37a123f5'
IO = dict(rbps=33554432, wbps=33554432, riops=2200, wiops=3600)
RAM = 4096 * 2**20


def need(ok, message):
    if not ok:
        raise ValueError(message)


def load(root, relative):
    path = Path(root) / relative
    need(not path.is_symlink() and path.stat().st_size <= 2 * 2**20, 'unsafe/oversized receipt: ' + relative)
    need(path.resolve().is_relative_to(Path(root).resolve()), 'receipt outside result root')
    return json.loads(path.read_text())


def counters(raw):
    result = {}
    for line in raw.splitlines():
        key, value = line.split()
        need(key not in result and value.isdecimal(), 'invalid/duplicate counter')
        result[key] = int(value)
    return result


def expected_schedule(stage):
    modes = ('idle', 'names', 'stat', 'scan')
    if stage == 'smoke':
        rows = [(f'smoke-{m}', m, 'full', False, 'smoke', None) for m in modes]
        rows += [('smoke-reserve', 'scan', 'reserve', False, 'collector', None)]
    else:
        need(stage == 'screen', 'unknown stage')
        orders = (('idle', 'names', 'scan', 'stat'), ('names', 'stat', 'idle', 'scan'),
                  ('stat', 'scan', 'names', 'idle'), ('scan', 'idle', 'stat', 'names'))
        rows = [(f'lat{i+1}-{m}', m, 'full', False, 'latin', i+1) for i, order in enumerate(orders) for m in order]
        rows += [(f'col{i}-{s}', 'scan', s, False, 'collector', i) for i in (1, 2, 3)
                 for s in (('reserve', 'full') if i == 2 else ('full', 'reserve'))]
    return rows + [('disk-pin', 'scan', 'full', True, 'disk', None)]


def manifest_ok(root, manifest):
    need(manifest['protocol'] == PROTOCOL and manifest['qualification'] is False, 'protocol/qualification')
    baseline = manifest['baseline']
    need(baseline['sha256'] == SHA and baseline['source_revision'] == REVISION, 'baseline identity')
    build = baseline['build']
    need(build['sha256'] == SHA and build['target'] == 'x86_64-unknown-linux-musl'
         and str(build['flags']['SOURCE_DATE_EPOCH']) == '1793464183', 'baseline build')
    expected = expected_schedule(manifest['stage'])
    observed = [(r['id'], r['mode'], r['sampler'], r['pin'], r['group'], r.get('round')) for r in manifest['cases']]
    need(observed == expected and manifest['order'] == [r[0] for r in expected], 'fixed schedule/order')
    fixture = manifest['fixture_receipt']
    files = 100000 if manifest['stage'] == 'smoke' else 1000000
    chunks = (files - 99 + 999) // 1000
    dirs = 3 + sum(2 + i % 6 for i in range(chunks))
    need(fixture['files'] == files and fixture['directories'] == dirs
         and fixture['expected_entries'] == files + dirs and fixture['expected_subtree_entries'] == 100,
         'fixture recipe/count')
    limits = manifest['limits']
    need((limits['memory_mib'], limits['cpus'], limits['workers'], limits['swap_mib']) == (4096, 2, 1, 0)
         and limits['io_limits'] == IO, 'manifest limits')
    environment_ok(manifest)
    need(re.fullmatch(r'[0-9a-f]{64}', manifest['setup']['seed_digest']), 'seed digest')
    source = manifest['source']
    need(source['revision'] == manifest['github']['GITHUB_SHA'] and re.fullmatch(r'[0-9a-f]{40}', source['revision']), 'source revision')
    required = {f'scripts/cause_{name}.py' for name in ('host', 'case', 'worker', 'probe', 'observe', 'global', 'validate', 'summary', 'outcomes', 'forecast', 'lifetime')}
    required |= {'scripts/cause_runner.sh', '.github/workflows/v2-balance.yml'}
    need(required <= source['sha256'].keys(), 'source capture missing')
    for relative, digest in source['sha256'].items():
        path = Path(root) / 'source' / relative
        need(path.resolve().is_relative_to((Path(root) / 'source').resolve()) and not path.is_symlink(), 'unsafe source path')
        need(hashlib.sha256(path.read_bytes()).hexdigest() == digest, 'source hash: ' + relative)


def environment_ok(manifest):
    evidence, limits = manifest['runner_evidence'], manifest['limits']
    calibration = evidence['disk-calibration.json']
    need(calibration['success'] is True and calibration['profile'] == 'local'
         and calibration['device'] == limits['io_device_number'] and calibration['requested_limits'] == IO, 'disk calibration')
    need([j['mode'] for j in calibration['jobs']] == ['write', 'read', 'randwrite', 'randread']
         and all(0 < j['observed'] <= j['limit'] * 1.25 for j in calibration['jobs']), 'disk calibration probes')
    systems = evidence['filesystem.json']['filesystems']
    need(len(systems) == 1 and systems[0]['source'] == limits['io_device']
         and systems[0]['fstype'] == 'ext4', 'scratch filesystem identity')
    loops = [row for row in evidence['loop-devices.json']['loopdevices'] if row['name'] == limits['io_device']]
    need(len(loops) == 1 and loops[0]['dio'] in (True, 1, '1')
         and not loops[0]['ro'], 'loop direct I/O disabled')
    prep = manifest['runner_preparation']
    need(prep['success'] is True and prep['floor_bytes'] == 42 * 2**30
         and prep['target_bytes'] == 44 * 2**30 and prep['duration_seconds'] >= 0, 'runner preparation')


def spec_ok(spec, row, manifest):
    need(spec['protocol'] == PROTOCOL and spec['id'] == row['id']
         and spec['mode'] == row['mode'] and spec['sampler'] == row['sampler'], 'case spec identity')
    need(re.fullmatch(r'[a-zA-Z0-9_-]{16,128}', spec['nonce']), 'nonce')
    need(spec['expected_entries'] == (0 if row['mode'] == 'idle' else manifest['fixture_receipt']['expected_entries']), 'spec count')
    need(spec['seed_sha256'] == manifest['setup']['seed_digest']
         and spec['seed_timestamp'] == manifest['setup']['seed_timestamp'], 'spec seed')
    need((spec['memory_mib'], spec['cpus'], spec['workers']) == (4096, 2, 1)
         and spec['io_limits'] == IO and spec['io_device_number'] == manifest['limits']['io_device_number'], 'spec limits')
    need(spec['backend'] == 'v1' and spec['operation'] == 'scan' and spec['scenario'] == 'unchanged'
         and spec['keep_previous'] is True and 0 < spec['timeout_seconds'] <= 1200, 'spec semantics/timeout')


def memory_ok(metrics):
    before, after = metrics['cgroup_before'], metrics['cgroup_after']
    need(before['start_ns'] <= before['end_ns'] <= metrics['cli_start_ns'] < metrics['cli_end_ns']
         <= after['start_ns'] <= after['end_ns'], 'frozen cgroup brackets')
    a, b = before['values'], after['values']
    for values in (a, b):
        need(int(values['memory.max']) == RAM and values['memory.swap.max'] == '0'
             and int(values['memory.swap.current']) == int(values['memory.swap.peak']) == 0, 'RAM/swap cap')
        quota, period = map(int, values['cpu.max'].split())
        need(period > 0 and quota == 2 * period, 'CPU quota')
        for key in ('memory.events', 'memory.events.local'):
            events = counters(values[key])
            need(all(events[n] == 0 for n in ('max', 'oom', 'oom_kill')), 'censored/OOM interval')
        for key in ('memory.pressure', 'io.pressure', 'io.max', 'io.stat', 'cpu.stat'):
            need(isinstance(values[key], str), 'required raw counter missing')
    stats = counters(b['memory.stat'])
    old = counters(a['memory.stat'])
    for key in ('anon', 'file', 'kernel', 'slab_reclaimable', 'slab_unreclaimable', 'pgscan', 'pgsteal'):
        need(key in stats and key in old, 'required memory.stat category: ' + key)
    need(all(stats[k] == old[k] for k in ('pgscan', 'pgsteal')), 'reclaim-censored comparison')
    peak = metrics['kernel_memory_peak_bytes']
    need(0 < peak == int(b['memory.peak']) < RAM and peak >= int(a['memory.peak']), 'frozen kernel peak')
    need(abs(metrics['elapsed_seconds'] - (metrics['cli_end_ns'] - metrics['cli_start_ns']) / 1e9) < 1e-6, 'CLI elapsed bracket')
    return stats


def sampler_ok(metrics, spec):
    sampler = metrics['sampling']
    need(sampler['coverage_valid'] is True and sampler['ticks'] > 0 and not sampler['errors']
         and sampler['capture_errors'] == 0 and sampler['interval_seconds'] == .02
         and sampler['reserve_max_gap_ns'] == 250000000 and sampler['max_tick_gap_ns'] <= 250000000, 'reserve coverage')
    need(sampler['coverage_start_ns'] <= metrics['cli_start_ns']
         and sampler['coverage_end_ns'] >= metrics['cli_end_ns'], 'sampler bounds')
    keys = ('sampled_disk_peak', 'sampled_file_peaks', 'sampled_memory_peak', 'filesystem_used_peak')
    if spec['sampler'] == 'reserve':
        need(sampler['availability'] == 'unavailable-reserve-only' and all(sampler[k] is None for k in keys), 'fabricated reserve peak')
    else:
        need(sampler['availability'] == 'sampled-lower-bound' and all(sampler[k] is not None for k in keys), 'full sampler incomplete')


def report_ok(metrics, spec, fixture):
    report, timing = metrics['stdout'], metrics['gnu_time']
    need(report['entries'] == spec['expected_entries'] and timing['exit_code'] == 0
         and timing['peak_rss_kib'] > 0 and timing['elapsed_seconds'] >= 0, 'CLI counts/GNU time')
    common = ['--backend', 'v1', 'scan', '/scratch/tree', '--workers', '1', '--json', '--low-memory']
    if spec['mode'] == 'scan':
        need(metrics['command'] == ['/binary', *common] and report['errors'] == 0
             and report['incomplete'] is False and report['workers'] == 1, 'scan command/status')
        return
    need(metrics['command'][-len(common):] == common and metrics['command'][-len(common)-1] == '/harness/cause_probe.py', 'probe command')
    need(report['protocol'] == PROTOCOL and report['mode'] == spec['mode'] and report['validated'] is True, 'probe identity')
    if spec['mode'] == 'idle':
        need(not report['traversal'] and report['path_kind_digest'] is None
             and all(report[k] == 0 for k in ('regular_files', 'directories', 'directory_opens', 'root_lstat_calls', 'entry_stat_calls')), 'idle traversal')
    else:
        need(report['regular_files'] == fixture['files'] and report['directories'] == fixture['directories']
             and report['directory_opens'] == fixture['directories'] and report['root_lstat_calls'] == 1
             and report['entry_stat_calls'] == (spec['expected_entries'] - 1 if spec['mode'] == 'stat' else 0)
             and report['traversal'] is True and re.fullmatch(r'[0-9a-f]{64}', report['path_kind_digest']), 'probe syscall/count invariants')


def global_ok(data):
    need(data['required_fields_available'] is True and data['scope'].startswith('GLOBAL'), 'global scope/availability')
    need(data['before']['monotonic_ns'] <= data['after']['monotonic_ns'], 'global brackets')
    for name in ('slabinfo', 'meminfo'):
        row = data['sources'][name]
        need(row['available'] is True and row['raw'] and not row['parsed']['parse_errors'], 'global source: ' + name)
    need(data['sources']['slabinfo']['parsed']['required_fields_available'], 'slab objects')
    need(data['background']['required_fields_available']
         and all(r['available'] and r['raw'] for r in data['background']['sources'].values()), 'host background capture')


def docker_ok(host, spec):
    live, terminal = host['docker_inspect_before_ack'], host['docker_state']
    state, config = live['State'], live['HostConfig']
    need(state['Running'] is True and state['Pid'] > 0 and terminal['Running'] is False
         and terminal['ExitCode'] == 0 and not terminal['OOMKilled'], 'Docker state')
    need(config['Memory'] == config['MemorySwap'] == RAM and config['NanoCpus'] == 2000000000
         and config['PidsLimit'] == 128 and config['NetworkMode'] == 'none'
         and config['CgroupnsMode'] == 'private', 'Docker caps/network')
    mounts = {m['Destination']: m for m in live['Mounts']}
    need(all(mounts[p]['RW'] is False for p in ('/scratch/tree', '/binary', '/harness'))
         and all(mounts[p]['RW'] is True for p in ('/scratch', '/results')), 'Docker mount access')
    for flag, key in (('ReadBps', 'rbps'), ('WriteBps', 'wbps'), ('ReadIOps', 'riops'), ('WriteIOps', 'wiops')):
        rows = config['BlkioDevice' + flag]
        need(len(rows) == 1 and rows[0]['Path'] == spec['io_device'] and rows[0]['Rate'] == IO[key], 'Docker I/O limit')
    cg = host['cgroup_after']
    need(cg['process_identity_stable'] is True and cg['pid'] == state['Pid']
         and all(r['available'] for r in cg['sources'].values()), 'host cgroup identity/availability')


def cold_and_io_ok(root, ident, metrics, spec):
    cache = load(root, ident + '/cache-flush.json')
    need(cache['success'] is True and cache['runner_environment'] == 'github-hosted'
         and cache['sync_command'] == 'os.sync()' and cache['flush_input'] == '3\n'
         and cache['flush_command'] == ['sudo', '-n', 'tee', '/proc/sys/vm/drop_caches']
         and cache['finished_unix_seconds'] >= cache['started_unix_seconds'], 'cold cache request failed')
    for capture in ('cgroup_before', 'cgroup_after'):
        rows = {}
        for line in metrics[capture]['values']['io.max'].splitlines():
            device, *fields = line.split()
            need(device not in rows, 'duplicate io.max device')
            rows[device] = dict(field.split('=', 1) for field in fields)
        values = rows.get(spec['io_device_number'], {})
        need(all(values.get(k) == str(v) for k, v in IO.items()), 'effective I/O cgroup limit')


def inventory_ok(data):
    need(data['complete'] is True and data['root_path_matches'] is True and not data['errors']
         and not data['excluded'], 'disk inventory coverage')
    for ident, row in data['entries'].items():
        obs = row['observation']
        need(ident == f'{obs["device"]}:{obs["inode"]}' and not row['raced'], 'disk inode identity/race')
    need(sum(r['observation']['allocated_bytes'] for r in data['entries'].values()) == data['allocated_bytes'], 'unique allocated sum')
    need(sum(r['observation']['logical_bytes'] for r in data['entries'].values()) == data['logical_bytes'], 'unique logical sum')


def disk_ok(data):
    for key in ('held', 'unlinked', 'final'):
        inventory_ok(data[key])
    need(data['aliases_dropped']['available'] is True and not data['aliases_dropped']['errors']
         and not data['aliases_dropped']['remaining_aliases'], 'inspection aliases retained')
    need(not data['final']['pins'], 'pins retained after release')
    for name in ('stage', 'previous'):
        pin = data['unlinked']['pins'][name]
        need(pin['naturally_unlinked'] is True and pin['observation']['nlink'] == 0
             and pin['app_path_retired'] is True, 'pin lifetime intervention')
        release = data['releases'][name]
        expected = pin['observation']['allocated_bytes']
        need(release['expected_inode_bytes'] == expected and release['requires_unlinked'] is True
             and release['release']['released'] == pin['observation'], 'release identity/allocation')
        need(len(release['samples']) == len(release['comparisons']) > 0, 'release observations absent')
        for sample, compare in zip(release['samples'], release['comparisons']):
            delta = sample['free_bytes'] - release['before']['free_bytes']
            need(compare['free_delta_bytes'] == delta and compare['residual_bytes'] == delta - expected
                 and compare['quantization_bytes'] == sample['fragment_size'], 'release arithmetic')
    need(data['releases']['stage']['release']['after']['monotonic_ns']
         < data['releases']['previous']['before']['at']['monotonic_ns'], 'release order')
    need(set(data['databases']) == {'stage', 'previous', 'published_current', 'published_previous'}, 'database coverage')
    identities = {}
    for name, db in data['databases'].items():
        need(db['available'] is True and not db['errors'] and db['query_connection_closed'] is True
             and db['immutable'] is True and db['read_only'] is True and db['source_identity_stable'] is True,
             'dbstat inspection unavailable')
        source = db['source_identity']
        ident = f'{source["device"]}:{source["inode"]}'
        identities[name] = ident
        need(ident in data['held']['entries'], 'database not in held inventory')
        if name in ('stage', 'previous'):
            pin = data['held']['pins'][name]['observation']
            need((pin['device'], pin['inode']) == (source['device'], source['inode']), 'database pin identity')
        else:
            filename = 'snapshot.db' if name == 'published_current' else 'snapshot.prev.db'
            need(any(a['kind'] == 'named' and a['path'] == filename
                     for a in data['held']['entries'][ident]['aliases']), 'published database identity')
        pragmas = db['pragmas']
        need(db['sqlite_version'] and db['header']['signature_valid'] and db['header']['available']
             and db['header']['write_version'] == db['header']['read_version'] == 1, 'immutable inspection WAL/header')
        need(pragmas['auto_vacuum'] == 0 and source['logical_bytes'] < 2**30,
             'page equality scope requires no pointer-map/lock-byte pages; new scope needs new accounting')
        need(pragmas['page_size'] * pragmas['page_count'] == db['source_identity']['logical_bytes'], 'database page/file accounting')
        need(db['dbstat_columns'] == ['name', 'pagetype', 'pages', 'bytes', 'payload', 'unused'], 'dbstat schema')
        need(sum(r[2] for r in db['dbstat']) + pragmas['freelist_count'] == pragmas['page_count'], 'database page categories')
        need(sum(r[3] for r in db['dbstat']) == pragmas['page_size'] * sum(r[2] for r in db['dbstat']), 'dbstat byte categories')
    need(len(set(identities.values())) == 4, 'expected four distinct database inodes')


def lifetime_ok(data, host):
    need(data['available'] is True and data['host_pid'] == host['cgroup_after']['pid']
         and data['cgroup_path'] == host['cgroup_after']['cgroup_path'], 'lifetime cgroup identity')
    counts = data['counts']
    need(counts['polls'] > 0 and counts['complete_polls'] > 0 and counts['errors'] - counts['races'] - counts['limits'] == 0
         and counts['limits'] == 0 and counts['transition_records_dropped'] == 0, 'lifetime capture errors/limits')
    need(data['maxima']['named_snapshot'] is not None and data['maxima']['combined'] is not None, 'lifetime inventory unavailable')
    for inventory in data['maxima'].values():
        if inventory is None:
            continue
        need(inventory['available'] is True and inventory['before']['monotonic_ns'] <= inventory['after']['monotonic_ns'], 'lifetime brackets')
        need(sum(r['observation']['allocated_bytes'] for r in inventory['entries'].values())
             == inventory['allocated_bytes'], 'lifetime unique allocation arithmetic')
    need(any(row['path'] == 'named:snapshot.stage.tmp' and row['current'].get('state') == 'absent'
             for row in data['transitions']), 'stage retirement not observed')
