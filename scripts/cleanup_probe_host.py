#!/usr/bin/env python3
"""Run fixed threshold-probe comparisons on a disposable hosted VM only."""
import argparse
import hashlib
import gzip
import importlib.util
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import sys
import time
import uuid

from cleanup_probe_fixture import build, guard

spec = importlib.util.spec_from_file_location('legacy_bench', Path(__file__).with_name('bench-v2-container.py'))
legacy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(legacy)


def command(arm, operation, target):
    if arm == 'probe':
        if operation == 'scan':
            return ['/binary', 'scan', '--root', '/scratch/tree', '--snapshot', '/scratch/snapshot', '--ppm', '1000']
        return ['/binary', 'refresh', '--root', '/scratch/tree', '--snapshot', '/scratch/snapshot', '--target', target]
    return ['/binary', '--backend', arm, operation, target, '--workers', '1', '--json'] + (['--low-memory'] if operation == 'scan' else [])


def run_container(args, arm, operation, target, tree, case, out, ident, seconds):
    binary = Path('binaries/cleanup_probe' if arm == 'probe' else 'binaries/freespace').resolve(strict=True)
    dev = os.stat(args.io_device).st_rdev
    config = dict(id=ident, command=command(arm, operation, target), timeout_seconds=seconds,
                  io_device_number=f'{os.major(dev)}:{os.minor(dev)}',
                  io_limits=dict(rbps=33554432, wbps=33554432, riops=2200, wiops=3600))
    (out / (ident + '.json')).write_text(json.dumps(config, indent=2)+'\n')
    name = 'cleanup-probe-' + uuid.uuid4().hex
    label = 'freespace.benchmark=' + os.environ['FREESPACE_BENCHMARK_RUN_TOKEN']
    cmd = ['docker','run','--name',name,'--label',label,'--network','none','--cpus','2',
           '--memory','4096m','--memory-swap','4096m','--cgroupns','private','--pids-limit','128',
           '--user',f'{os.getuid()}:{os.getgid()}','--entrypoint','python3']
    for src,dst,ro in ((case,'/scratch',False),(tree,'/scratch/tree',True),(out,'/results',False),
                       (Path('scripts').resolve(),'/harness',True),(binary,'/binary',True),
                       (Path(os.environ['CLEANUP_OUTER_PATH']),'/outer',True)):
        cmd += ['--mount',f'type=bind,src={src},dst={dst}' + (',readonly' if ro else '')]
    for flag,key in (('read-bps','rbps'),('write-bps','wbps'),('read-iops','riops'),('write-iops','wiops')):
        cmd += ['--device-'+flag,f'{args.io_device}:{config["io_limits"][key]}']
    cmd += [args.image,'/harness/cleanup_probe_worker.py','/results/'+ident+'.json']
    try:
        with (out/(ident+'.docker.log')).open('w') as log:
            result = subprocess.run(cmd, stdout=log, stderr=subprocess.STDOUT, timeout=seconds+90)
        state = json.loads(subprocess.check_output(['docker','inspect',name],text=True,timeout=15))[0]['State']
        (out/ident/'docker-state.json').write_text(json.dumps(state,indent=2)+'\n')
        if result.returncode or state.get('OOMKilled'):
            raise RuntimeError('writer failed '+ident)
        return json.loads((out/ident/'metrics.json').read_text())
    finally:
        subprocess.run(['docker','rm','-f',name],check=True,timeout=30,stdout=subprocess.DEVNULL)


def oracle(args, cases, out, ident, seconds):
    name = 'cleanup-oracle-' + uuid.uuid4().hex
    label = 'freespace.benchmark=' + os.environ['FREESPACE_BENCHMARK_RUN_TOKEN']
    cmd = ['docker','run','--name',name,'--label',label,'--network','none','--cpus','2',
           '--memory','4096m','--memory-swap','4096m','--cgroupns','private',
           '--user',f'{os.getuid()}:{os.getgid()}','--entrypoint','python3',
           '--mount',f'type=bind,src={cases["v1"] / "snapshot"},dst=/baseline,readonly',
           '--mount',f'type=bind,src={cases["probe"] / "snapshot"},dst=/coarse,readonly',
           '--mount',f'type=bind,src={cases["v2"] / "snapshot"},dst=/v2,readonly',
           '--mount',f'type=bind,src={Path("scripts").resolve()},dst=/harness,readonly',
           args.image,'/harness/cleanup_oracle.py','/baseline/snapshot.db','/coarse/coarse.db','/v2/v2/generations.db']
    try:
        with (out/(ident+'.json')).open('w') as stdout, (out/(ident+'.log')).open('w') as stderr:
            result=subprocess.run(cmd,stdout=stdout,stderr=stderr,timeout=seconds)
        if result.returncode or json.loads((out/(ident+'.json')).read_text()).get('success') is not True:
            raise ValueError('oracle rejected '+ident)
    finally:
        subprocess.run(['docker','rm','-f',name],check=True,timeout=30,stdout=subprocess.DEVNULL)


def preserve_evidence(cases, out, profile):
    """Bound optional post-timing evidence; never treat compression as a result."""
    target = out / 'snapshot-evidence'
    target.mkdir(exist_ok=True)
    result = {}
    for arm, name in (('v1', 'snapshot.db'), ('probe', 'coarse.db')):
        source = cases[arm] / 'snapshot' / name
        final = target / f'{profile}-{arm}.db.gz'
        temp = final.with_suffix('.tmp')
        digest = hashlib.sha256()
        try:
            with source.open('rb') as inp, temp.open('wb') as raw:
                with gzip.GzipFile(fileobj=raw, mode='wb', mtime=0) as compressed:
                    while True:
                        data = inp.read(1024 * 1024)
                        if not data: break
                        digest.update(data)
                        compressed.write(data)
                        fs = os.statvfs(out)
                        if raw.tell() > 128 * 1024**2 or fs.f_bavail * fs.f_frsize < 2 * 1024**3:
                            raise ValueError('evidence size or free-space bound')
            if temp.stat().st_size > 128 * 1024**2:
                raise ValueError('evidence size bound')
            temp.replace(final)
            result[arm] = dict(path=str(final.relative_to(out)), source_bytes=source.stat().st_size,
                               source_sha256=digest.hexdigest(), compressed_bytes=final.stat().st_size)
        except (OSError, ValueError) as error:
            temp.unlink(missing_ok=True)
            result[arm] = dict(omitted=True, reason=str(error))
    (target / f'{profile}-receipt.json').write_text(json.dumps(result,indent=2)+'\n')


def run(args):
    guard()
    start=time.monotonic()
    deadline=start+args.budget_seconds
    out=args.output.resolve()
    out.mkdir(parents=True,exist_ok=True)
    manifest={'stage':args.stage, 'source_sha':os.environ.get('GITHUB_SHA'), 'success':False,
              'cases':[], 'phases':[], 'limitation':'Unchanged common fixture; performance compares full scan, retained refresh, and omitted-ancestor refresh.'}
    (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    count=100000 if args.stage=='smoke' else 1000000
    pairs=1 if args.stage=='smoke' else 3
    scratch=Path(os.environ['RUNNER_TEMP']).resolve()
    def phase(name, fn):
        t=time.monotonic()
        success=False
        try:
            result=fn()
            success=True
            return result
        finally:
            manifest['phases'].append(dict(name=name, seconds=time.monotonic()-t, success=success))
            (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    def remaining():
        seconds=int(deadline-time.monotonic()-180)
        if seconds < 10: raise TimeoutError('host budget exhausted')
        return min(seconds,1800)
    def execute(arm,op,target,tree,case,ident):
        legacy.flush_caches(out,ident)
        row=run_container(args,arm,op,target,tree,case,out,ident,remaining())
        if arm == 'probe' and op == 'refresh':
            expected_fallback = target.endswith('/small')
            if row['report'].get('fallback_full_root') is not expected_fallback:
                raise ValueError('unexpected refresh boundary: '+ident)
        manifest['cases'].append(dict(id=ident,arm=arm,operation=op,target=target,elapsed_seconds=row['elapsed_seconds'],validated=row['validated'],memory_comparison_inconclusive=row['memory_comparison_inconclusive'],report=row['report']))
        return row
    try:
        for profile in ('files','directories'):
            tree=scratch/('fixture-'+profile)
            fixture=phase('fixture-'+profile,lambda: build(tree,profile,count))
            backing = os.stat(os.environ['CLEANUP_BACKING_FILE'])
            outer = os.statvfs(os.environ['CLEANUP_OUTER_PATH'])
            result_fs = os.statvfs(out)
            fixture['backing_file'] = dict(logical_bytes=backing.st_size, allocated_bytes=backing.st_blocks * 512,
                                           outer_available_bytes=outer.f_bavail * outer.f_frsize,
                                           results_available_bytes=result_fs.f_bavail * result_fs.f_frsize)
            (out/('fixture-'+profile+'.json')).write_text(json.dumps(fixture,indent=2)+'\n')
            if min(fixture['backing_file']['outer_available_bytes'], fixture['backing_file']['results_available_bytes']) < 2 * 1024**3:
                raise ValueError('outer free-space reserve failed before measurement')
            seeds={}
            for arm in ('v1','v2','probe'):
                case=scratch/('seed-'+profile+'-'+arm)
                (case/'snapshot'/'tmp').mkdir(parents=True)
                (case/'tree').mkdir()
                (case/'config.toml').write_text('keep_previous = true\n')
                for n in (1,2):
                    phase('seed-'+profile+'-'+arm+'-'+str(n),lambda: execute(arm,'scan','/scratch/tree',tree,case,'seed-'+profile+'-'+arm+'-'+str(n)))
                    time.sleep(1.05)
                if arm == 'probe':
                    required = ('coarse.db', 'coarse.prev.db')
                elif arm == 'v1':
                    required = ('snapshot.db', 'snapshot.prev.db')
                else:
                    required = ('v2/generations.db',)
                if any(not (case/'snapshot'/name).is_file() for name in required):
                    raise ValueError('missing seeded snapshot: '+arm)
                seeds[arm]=case
            for scenario_index,(scenario,op,target) in enumerate((('full','scan','/scratch/tree'),('retained','refresh','/scratch/tree/large'),('omitted','refresh','/scratch/tree/small'))):
                for pair in range(pairs):
                    orders=(('v1','v2','probe'),('v2','probe','v1'),('probe','v1','v2'))
                    arms=orders[(scenario_index+pair)%len(orders)]
                    cycle_started=time.monotonic()
                    evidence_seconds=0.0
                    cases={}
                    for arm in arms:
                        case=scratch/('case-'+arm)
                        shutil.copytree(seeds[arm],case)
                        cases[arm]=case
                        ident=f'{profile}-{scenario}-{pair+1}-{arm}'
                        phase(ident,lambda:execute(arm,op,target,tree,case,ident))
                    ident=f'{profile}-{scenario}-{pair+1}-oracle'
                    phase(ident,lambda: oracle(args,cases,out,ident,remaining()))
                    if scenario == 'full' and pair == 0:
                        evidence_started=time.monotonic()
                        phase(profile+'-evidence',lambda: preserve_evidence(cases,out,profile))
                        evidence_seconds=time.monotonic()-evidence_started
                    for case in cases.values(): shutil.rmtree(case)
                    manifest['phases'].append(dict(name=f'{profile}-{scenario}-{pair+1}-cycle',
                                                   seconds=time.monotonic()-cycle_started-evidence_seconds, success=True))
            for case in seeds.values(): shutil.rmtree(case)
            shutil.rmtree(tree)
        comparisons={}
        for profile in ('files','directories'):
            for scenario in ('full','retained','omitted'):
                key=profile+'/'+scenario
                comparisons[key]={}
                for arm in ('v1','v2','probe'):
                    rows=[json.loads((out/c['id']/'metrics.json').read_text()) for c in manifest['cases']
                          if c['id'].startswith(profile+'-'+scenario+'-') and c['arm']==arm]
                    if len(rows)!=pairs: raise ValueError('incomplete comparison '+key+'/'+arm)
                    comparisons[key][arm]=dict(
                        count=len(rows), elapsed_seconds_median=statistics.median(r['elapsed_seconds'] for r in rows),
                        peak_rss_kib_median=statistics.median(r['time']['peak_rss_kib'] for r in rows),
                        cgroup_memory_peak_bytes_median=statistics.median(int(r['cgroup_after']['memory.peak']) for r in rows),
                        output_peak_estimate_bytes_median=statistics.median(r['output_peak_estimate_bytes'] for r in rows),
                        memory_comparison_inconclusive=any(r['memory_comparison_inconclusive'] for r in rows))
        manifest['comparisons']=comparisons
        manifest['success']=True
    finally:
        manifest['elapsed_seconds']=time.monotonic()-start
        (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    return 0


def main():
    p=argparse.ArgumentParser()
    p.add_argument('--stage',required=True,choices=('smoke','screen'))
    p.add_argument('--image',required=True)
    p.add_argument('--io-device',required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--budget-seconds',type=int,required=True)
    return run(p.parse_args())


if __name__=='__main__':
    sys.exit(main())
