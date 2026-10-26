#!/usr/bin/env python3
"""Whole-command v1/v2 disk-seed comparison under a fresh no-swap cgroup.
BINARY OUTPUT; STEPS='100000 1000000', MEMORY_MAX=128M, SOURCE_REVISION=<hash>.
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile
import time


def dump(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def group_path():
    line = next(v for v in Path('/proc/self/cgroup').read_text().splitlines() if v.startswith('0::'))
    return Path('/sys/fs/cgroup') / line[3:].lstrip('/')


def disk_bytes(folder):
    total = 0
    for path in folder.rglob('*'):
        try:
            total += path.stat().st_blocks * 512
        except FileNotFoundError:
            pass
    return total


def run(spec_file):
    spec = json.loads(spec_file.read_text())
    prefix = Path(spec['prefix'])
    group = group_path()
    peak_disk = 0
    peak_current = 0
    started = time.monotonic()
    time_format = '{"elapsed_seconds":%e,"peak_rss_kib":%M,"inputs":%I,"outputs":%O,"exit_code":%x}'
    cmd = ['/usr/bin/time', '-f', time_format, '-o', str(prefix.with_suffix('.time')), *spec['command']]
    with prefix.with_suffix('.stdout').open('w') as out, prefix.with_suffix('.stderr').open('w') as err:
        proc = subprocess.Popen(cmd, env=dict(os.environ, **spec['env']), stdout=out, stderr=err)
        while proc.poll() is None:
            peak_disk = max(peak_disk, disk_bytes(Path(spec['snapshot'])))
            peak_current = max(peak_current, int((group / 'memory.current').read_text()))
            time.sleep(0.02)
    usage = json.loads(prefix.with_suffix('.time').read_text().splitlines()[-1])
    cg = {name:(group/name).read_text().strip() for name in ['memory.max','memory.swap.max','memory.peak','memory.events','memory.pressure'] if (group/name).exists()}
    result = dict(spec, usage=usage, cgroup=cg, sampled_peak_cgroup_bytes=peak_current,
                  sampled_peak_disk_bytes=max(peak_disk,disk_bytes(Path(spec['snapshot']))),
                  harness_elapsed_seconds=time.monotonic()-started)
    if proc.returncode == 0:
        report=json.loads(prefix.with_suffix('.stdout').read_text())
        assert report['entries']==spec['expected_entries'],report
        assert report['errors']==0 and not report['incomplete'],report
        result['report']=report
    dump(prefix.with_suffix('.metrics.json'),result)
    if proc.returncode:
        raise SystemExit(proc.returncode)


def main():
    if len(sys.argv)==3 and sys.argv[1]=='--run':
        run(Path(sys.argv[2])); return
    binary=Path(sys.argv[1]).resolve()
    output=Path(sys.argv[2]).resolve(); output.mkdir(parents=True,exist_ok=False)
    steps=[int(v) for v in os.environ.get('STEPS','100000 1000000').split()]
    limit=os.environ.get('MEMORY_MAX','128M')
    backends=os.environ.get('BACKENDS','v1 v2').split()
    assert sorted(backends)==['v1','v2']
    dump(output/'manifest.json',dict(binary=str(binary),binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
         source_revision=os.environ.get('SOURCE_REVISION'),platform=platform.platform(),steps=steps,memory_max=limit,backend_order=backends,
         cache_policy='warm/uncontrolled; alternating backend order; helper and filesystem cache share cgroup',
         runner_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest()))
    (output/'config.toml').write_text('keep_previous = false\n')
    with tempfile.TemporaryDirectory(prefix='v2bm-') as scratch:
        tree=Path(scratch)/'r';tree.mkdir();previous=0
        for step in steps:
            for n in range(previous,step):
                parent=tree/f'd{n//1000:04}'
                parent.mkdir(exist_ok=True)
                (parent/f'f{n:07}').touch(exist_ok=False)
            previous=step
            entries=1+step+(step+999)//1000
            for backend in (backends if steps.index(step)%2==0 else list(reversed(backends))):
                snapshot=Path(scratch)/backend;snapshot.mkdir()
                prefix=output/f'{step}-{backend}'
                command=[str(binary),'--backend',backend,'scan',str(tree),'--workers','1','--low-memory','--json']
                spec=dict(command=command,prefix=str(prefix),snapshot=str(snapshot),expected_entries=entries,
                          env=dict(FREESPACE_CONFIG=str(output/'config.toml'),FREESPACE_SNAPSHOT_DIR=str(snapshot),LC_ALL='C'))
                spec_file=prefix.with_suffix('.run.json');dump(spec_file,spec)
                subprocess.run(['systemd-run','--user','--wait','--pipe','--quiet','--property=MemoryMax='+limit,
                                '--property=MemorySwapMax=0','--property=OOMPolicy=continue',sys.executable,
                                str(Path(__file__).resolve()),'--run',str(spec_file)],check=True)
                row=json.loads(prefix.with_suffix('.metrics.json').read_text())
                print(backend,step,row['usage'],row['sampled_peak_disk_bytes'],row['cgroup'],flush=True)
                shutil.rmtree(snapshot)

if __name__=='__main__': main()
