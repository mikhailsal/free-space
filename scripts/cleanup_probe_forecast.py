#!/usr/bin/env python3
"""Admit screening only using a successful same-source smoke projection."""
import json
import math
import os
from pathlib import Path
import sys


def forecast(manifest, cleanup, source_sha, build_seconds):
    if manifest.get('stage') != 'smoke' or manifest.get('source_sha') != source_sha or manifest.get('success') is not True:
        raise ValueError('smoke manifest missing, invalid, or from different source')
    if cleanup.get('success') is not True:
        raise ValueError('smoke runner cleanup was not successful')
    cases = manifest['cases']
    if len(cases) != 30 or any(c.get('validated') is not True for c in cases):
        raise ValueError('incomplete smoke writer cases')
    phases = manifest['phases']
    oracle = [p for p in phases if p['name'].endswith('-oracle')]
    cycles = [p for p in phases if p['name'].endswith('-cycle')]
    if len(oracle) != 6 or any(p.get('success') is not True for p in oracle):
        raise ValueError('incomplete or failed smoke semantic oracles')
    if len(cycles) != 6 or any(p.get('success') is not True or not isinstance(p.get('seconds'), (int,float))
                               or not math.isfinite(p['seconds']) or p['seconds'] < 0 for p in cycles):
        raise ValueError('incomplete repeatable smoke cycles')
    cycle_seconds = sum(p['seconds'] for p in cycles)
    projected_host = 10 * manifest['elapsed_seconds'] + 20 * cycle_seconds
    conservative = projected_host * 1.3
    accepted = conservative + 240 < 5100 and build_seconds + conservative + 240 < 7200
    return dict(protocol='cleanup-threshold-forecast-v1', source_sha=source_sha, admitted=accepted,
                smoke_host_seconds=manifest['elapsed_seconds'], smoke_cycle_seconds=cycle_seconds,
                projected_host_seconds=projected_host, projected_host_with_30pct=conservative,
                observed_fixed_build_seconds=build_seconds, cleanup_reserve_seconds=240,
                measured_step_cap_seconds=5100, whole_job_cap_seconds=7200,
                method='10x host plus 20x full copy/writer/oracle/cleanup cycles for two additional pairs, then 30% margin; observed build held fixed',
                limitation='Planning projection, not a measured screen time or upper bound.')


def main():
    source=Path(sys.argv[1]); target=Path(sys.argv[2]); root=source.parent.parent
    start=int(Path('cleanup-results/provenance/build-start.epoch').read_text())
    end=int(Path('cleanup-results/provenance/build-end.epoch').read_text())
    result=forecast(json.loads(source.read_text()),json.loads((root/'runner-cleanup.json').read_text()),os.environ['GITHUB_SHA'],end-start)
    target.write_text(json.dumps(result,indent=2)+'\n')
    if not result['admitted']: raise SystemExit('screen forecast exceeds budget')


if __name__=='__main__': main()
