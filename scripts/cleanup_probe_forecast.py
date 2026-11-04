#!/usr/bin/env python3
"""Admit screening only using a successful same-source smoke projection."""
import argparse
import json
import math
import os
from pathlib import Path
import time

import cleanup_probe_plan as plan
from cleanup_probe_resources import capture as capture_resources, require as require_resources


def forecast(manifest, cleanup, source_sha, job_elapsed_seconds, timing, space, available_bytes, build_context_seconds=None):
    if not isinstance(job_elapsed_seconds, (int, float)) or not math.isfinite(job_elapsed_seconds) or job_elapsed_seconds < 0:
        raise ValueError('invalid current or projected elapsed job time')
    if manifest.get('protocol') != 'cleanup-threshold-roomy-v2' or manifest.get('resource_policy') != {'memory_mib':8192, 'swap_mib':0, 'cpus':2}:
        raise ValueError('smoke resource policy does not match roomy v2')
    require_resources(manifest.get('resource_context'))
    if manifest.get('stage') != 'smoke' or manifest.get('source_sha') != source_sha or manifest.get('success') is not True:
        raise ValueError('smoke manifest missing, invalid, or from different source')
    if cleanup.get('success') is not True:
        raise ValueError('smoke runner cleanup was not successful')
    want = plan.expected('smoke')
    cases = manifest['cases']
    if len(cases) != want['cases'] or any(c.get('validated') is not True for c in cases):
        raise ValueError('incomplete smoke writer cases')
    phases = manifest['phases']
    oracle = [p for p in phases if p['name'].endswith('-oracle')]
    cycles = [p for p in phases if p['name'].endswith('-cycle')]
    if len(oracle) != want['oracles'] or any(p.get('success') is not True for p in oracle):
        raise ValueError('incomplete or failed smoke semantic oracles')
    if len(cycles) != want['cycles'] or any(p.get('success') is not True or not isinstance(p.get('seconds'), (int,float))
                               or not math.isfinite(p['seconds']) or p['seconds'] < 0 for p in cycles):
        raise ValueError('incomplete repeatable smoke cycles')
    cycle_seconds = sum(p['seconds'] for p in cycles)
    projected_host = 10 * manifest['elapsed_seconds'] + 20 * cycle_seconds
    conservative = projected_host * 1.3
    setup = timing.get('setup_seconds')
    if not isinstance(setup, (int,float)) or not math.isfinite(setup) or setup < 0:
        raise ValueError('missing observed pre-host setup time')
    required_space = ('initial_backing_allocated_bytes','peak_backing_allocated_bytes',
                      'initial_outer_available_bytes','minimum_outer_available_bytes',
                      'initial_results_available_bytes','minimum_results_available_bytes')
    if (not isinstance(space,dict) or space.get('error') or space.get('samples',0) < 2
            or any(type(space.get(k)) is not int or space[k] < 0 for k in required_space)):
        raise ValueError('missing or invalid smoke outer-space evidence')
    cycles_space=space.get('cycles',[])
    trims=space.get('trims',[])
    if (len(cycles_space)!=want['cycles'] or len(trims)<want['trims'] or any(t.get('exit_code') != 0 for t in trims)
            or any(any(type(c.get(k)) is not int or c[k]<0 for k in
                       ('before_backing_allocated_bytes','peak_backing_allocated_bytes','after_backing_allocated_bytes'))
                   for c in cycles_space)):
        raise ValueError('incomplete outer-space cycle or trim evidence')
    if type(available_bytes) is not int or available_bytes < 0:
        raise ValueError('missing available outer space')
    growth=max(0,space['peak_backing_allocated_bytes']-space['initial_backing_allocated_bytes'],
               space['initial_outer_available_bytes']-space['minimum_outer_available_bytes'],
               space['initial_results_available_bytes']-space['minimum_results_available_bytes'])
    residual=sum(max(0,c['after_backing_allocated_bytes']-c['before_backing_allocated_bytes']) for c in cycles_space)
    keys=('step_outer_available_bytes','host_outer_available_bytes','step_results_available_bytes','host_results_available_bytes')
    if any(type(timing.get(k)) is not int or timing[k]<0 for k in keys):
        raise ValueError('missing setup storage evidence')
    setup_space=max(0,timing[keys[0]]-timing[keys[1]],timing[keys[2]]-timing[keys[3]])
    projected_space=math.ceil((10*growth+20*residual)*1.3)+setup_space
    step_time_ok = setup + conservative + 180 + 240 < 12600
    job_time_ok = job_elapsed_seconds + setup + conservative + 180 + 240 + 300 < 14400
    time_ok = step_time_ok and job_time_ok
    space_ok = available_bytes - projected_space >= 2 * 1024**3
    return dict(protocol='cleanup-threshold-forecast-roomy-v2', source_sha=source_sha, admitted=time_ok and space_ok,
                smoke_host_seconds=manifest['elapsed_seconds'], smoke_cycle_seconds=cycle_seconds,
                projected_host_seconds=projected_host, projected_host_with_30pct=conservative,
                observed_setup_seconds=setup, job_elapsed_seconds=job_elapsed_seconds,
                observed_fixed_build_seconds=build_context_seconds,
                host_margin_seconds=180, cleanup_reserve_seconds=240, upload_reserve_seconds=300,
                measured_step_cap_seconds=12600, whole_job_cap_seconds=14400,
                outer_available_bytes=available_bytes, smoke_outer_peak_growth_bytes=growth,
                smoke_cycle_residual_growth_bytes=residual, setup_space_bytes=setup_space,
                projected_outer_growth_bytes=projected_space, outer_reserve_bytes=2*1024**3,
                time_admitted=time_ok, step_time_admitted=step_time_ok, job_time_admitted=job_time_ok, space_admitted=space_ok,
                method='10x host plus 20x full copy/writer/oracle/cleanup cycles, 30% margin; setup and build fixed; outer peak 10x plus residual cycle growth 20x with 30% margin',
                limitation='Planning projection, not a measured screen time or physical upper bound.')


def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest',type=Path)
    parser.add_argument('output',type=Path)
    parser.add_argument('--preflight',action='store_true',help='use explicitly supplied pre-dispatch runner assumptions')
    parser.add_argument('--source-sha')
    parser.add_argument('--job-elapsed-seconds',type=float,help='conservative expected job time before measurement')
    parser.add_argument('--build-seconds',type=float,help='observed or conservative build time for context')
    parser.add_argument('--available-bytes',type=int,help='conservative expected free outer/results bytes at measurement start')
    args=parser.parse_args(argv)
    source=args.manifest; root=source.parent.parent
    if args.preflight:
        if any(v is None for v in (args.source_sha,args.job_elapsed_seconds,args.build_seconds,args.available_bytes)):
            parser.error('--preflight requires --source-sha, --job-elapsed-seconds, --build-seconds and --available-bytes')
        if args.build_seconds < 0 or not math.isfinite(args.build_seconds) or args.job_elapsed_seconds < args.build_seconds:
            parser.error('job elapsed must be finite and at least the nonnegative build time')
        sha=args.source_sha; elapsed=args.job_elapsed_seconds; build=args.build_seconds; available=args.available_bytes
    else:
        current_resources = capture_resources()
        require_resources(current_resources)
        if any(v is not None for v in (args.source_sha,args.job_elapsed_seconds,args.build_seconds,args.available_bytes)):
            parser.error('explicit assumptions require --preflight')
        start=int(Path('cleanup-results/provenance/build-start.epoch').read_text())
        end=int(Path('cleanup-results/provenance/build-end.epoch').read_text())
        build=end-start
        elapsed=time.time()-int(os.environ['CLEANUP_JOB_STARTED_EPOCH'])
        sha=os.environ['GITHUB_SHA']
        paths=(os.environ['RUNNER_TEMP'],'cleanup-results')
        available=min(os.statvfs(p).f_bavail * os.statvfs(p).f_frsize for p in paths)
    result=forecast(json.loads(source.read_text()),json.loads((root/'runner-cleanup.json').read_text()),
                    sha,elapsed,json.loads((root/'provenance/timing.json').read_text()),
                    json.loads((source.parent/'space-monitor.json').read_text()),available,build)
    result['admission_mode']='preflight' if args.preflight else 'in-job'
    if not args.preflight: result['current_host_resources']=current_resources
    args.output.write_text(json.dumps(result,indent=2)+'\n')
    if not result['admitted']: raise SystemExit('screen forecast exceeds budget')


if __name__=='__main__': main()
