import copy
import json
import tempfile
from unittest.mock import patch
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from cleanup_probe_forecast import forecast as calculate, main

GIB=1024**3

def forecast(manifest, cleanup, source_sha, build_seconds, **kwargs):
    timing=kwargs.get("timing",dict(setup_seconds=10,step_outer_available_bytes=10*GIB,
                   host_outer_available_bytes=10*GIB,step_results_available_bytes=10*GIB,host_results_available_bytes=10*GIB))
    cycle=dict(before_backing_allocated_bytes=100,peak_backing_allocated_bytes=120,after_backing_allocated_bytes=105)
    space=kwargs.get("space",dict(samples=100,initial_backing_allocated_bytes=100,peak_backing_allocated_bytes=120,
                 initial_outer_available_bytes=10*GIB,minimum_outer_available_bytes=10*GIB-20,
                 initial_results_available_bytes=10*GIB,minimum_results_available_bytes=10*GIB-20,
                 cycles=[dict(cycle) for _ in range(6)],trims=[{"exit_code":0} for _ in range(26)]))
    return calculate(manifest,cleanup,source_sha,build_seconds,timing,space,kwargs.get("available_bytes",10*GIB))


def smoke():
    return {'resource_context':{'success':True,'mem_total_bytes':16*GIB,'mem_available_bytes':12*GIB,'cpu_count':4},
            'protocol':'cleanup-threshold-roomy-v2', 'resource_policy':{'memory_mib':8192, 'swap_mib':0, 'cpus':2}, 'stage':'smoke', 'source_sha':'source', 'success':True, 'elapsed_seconds':100,
            'cases':[{'validated':True} for _ in range(30)],
            'phases':[{'name':f'{i}-oracle','success':True,'seconds':1} for i in range(6)] +
                     [{'name':f'{i}-cycle','success':True,'seconds':2} for i in range(6)]}


class ForecastTests(unittest.TestCase):
    def test_cycle_includes_copy_and_cleanup_cost(self):
        manifest=smoke()
        first=forecast(manifest,{'success':True},'source',100)
        self.assertEqual(first['projected_host_seconds'],1240)
        manifest['phases'][-1]['seconds']+=3
        second=forecast(manifest,{'success':True},'source',100)
        self.assertEqual(second['projected_host_seconds']-first['projected_host_seconds'],60)

    def test_wrong_source_and_failed_cleanup(self):
        with self.assertRaises(ValueError): forecast(smoke(),{'success':True},'other',100)
        with self.assertRaises(ValueError): forecast(smoke(),{'success':False},'source',100)

    def test_incomplete_and_failed_cases(self):
        manifest=smoke(); manifest['cases'].pop()
        with self.assertRaises(ValueError): forecast(manifest,{'success':True},'source',100)
        manifest=smoke(); manifest['cases'][0]['validated']=False
        with self.assertRaises(ValueError): forecast(manifest,{'success':True},'source',100)

    def test_original_projection_and_new_cap_boundaries(self):
        manifest=smoke()
        manifest['elapsed_seconds']=422.412317624
        for phase in manifest['phases']:
            if phase['name'].endswith('-cycle'): phase['seconds']=167.510380595/6
        timing=dict(setup_seconds=65,step_outer_available_bytes=10*GIB,host_outer_available_bytes=10*GIB,
                    step_results_available_bytes=10*GIB,host_results_available_bytes=10*GIB)
        result=forecast(manifest,{'success':True},'source',600,timing=timing)
        self.assertTrue(result['admitted'])
        self.assertTrue(result['step_time_admitted'])
        self.assertTrue(result['job_time_admitted'])
        self.assertGreater(result['projected_host_with_30pct'],7200)
        timing['setup_seconds']=12600-result['projected_host_with_30pct']-420+0.01
        result=forecast(manifest,{'success':True},'source',600,timing=timing)
        self.assertFalse(result['step_time_admitted'])
        self.assertTrue(result['job_time_admitted'])
        timing['setup_seconds']=65
        elapsed=14400-65-result['projected_host_with_30pct']-720+0.01
        result=forecast(manifest,{'success':True},'source',elapsed,timing=timing)
        self.assertTrue(result['step_time_admitted'])
        self.assertFalse(result['job_time_admitted'])

    def test_smoke_host_headroom_required(self):
        for field, value in (('mem_total_bytes',10*GIB-1),('mem_available_bytes',10*GIB-1),('cpu_count',1)):
            manifest=smoke(); manifest['resource_context'][field]=value
            with self.assertRaises(ValueError): forecast(manifest,{'success':True},'source',100)

    def test_old_resource_policy_rejected(self):
        manifest=smoke(); manifest['resource_policy']['memory_mib']=4096
        with self.assertRaises(ValueError): forecast(manifest,{'success':True},'source',100)
        manifest=smoke(); manifest.pop('protocol')
        with self.assertRaises(ValueError): forecast(manifest,{'success':True},'source',100)

    def test_bad_oracle_and_missing_cycle(self):
        manifest=smoke(); manifest['phases'][0]['success']=False
        with self.assertRaises(ValueError): forecast(manifest,{'success':True},'source',100)
        manifest=smoke(); manifest['phases'].pop()
        with self.assertRaises(ValueError): forecast(manifest,{'success':True},'source',100)

    def test_storage_and_setup_gates(self):
        self.assertFalse(forecast(smoke(),{'success':True},'source',100,available_bytes=2*GIB)['admitted'])
        with self.assertRaises(ValueError): forecast(smoke(),{'success':True},'source',100,space={})
        with self.assertRaises(ValueError): forecast(smoke(),{'success':True},'source',100,timing={})

    def test_preflight_uses_explicit_assumptions_only(self):
        timing=dict(setup_seconds=10,step_outer_available_bytes=10*GIB,host_outer_available_bytes=10*GIB,
                    step_results_available_bytes=10*GIB,host_results_available_bytes=10*GIB)
        cycle=dict(before_backing_allocated_bytes=100,peak_backing_allocated_bytes=120,after_backing_allocated_bytes=105)
        space=dict(samples=100,initial_backing_allocated_bytes=100,peak_backing_allocated_bytes=120,
                   initial_outer_available_bytes=10*GIB,minimum_outer_available_bytes=10*GIB-20,
                   initial_results_available_bytes=10*GIB,minimum_results_available_bytes=10*GIB-20,
                   cycles=[dict(cycle) for _ in range(6)],trims=[{'exit_code':0} for _ in range(26)])
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp); (root/'measurements').mkdir(); (root/'provenance').mkdir()
            for name,value in (('measurements/manifest.json',smoke()),('runner-cleanup.json',{'success':True}),
                               ('measurements/space-monitor.json',space),('provenance/timing.json',timing)):
                (root/name).write_text(json.dumps(value))
            args=[str(root/'measurements/manifest.json'),str(root/'forecast.json'),'--preflight',
                  '--source-sha','source','--job-elapsed-seconds','150','--build-seconds','100',
                  '--available-bytes',str(10*GIB)]
            with patch('cleanup_probe_forecast.os.statvfs',side_effect=AssertionError('local disk used')):
                main(args)
            result=json.loads((root/'forecast.json').read_text())
            self.assertEqual(result['admission_mode'],'preflight')
            self.assertEqual(result['job_elapsed_seconds'],150)
            self.assertEqual(result['observed_fixed_build_seconds'],100)

    def test_over_cap_rejected(self):
        manifest=smoke(); manifest['elapsed_seconds']=1000
        self.assertFalse(forecast(manifest,{'success':True},'source',100)['admitted'])
        self.assertFalse(forecast(smoke(),{'success':True},'source',15000)['admitted'])


if __name__=='__main__': unittest.main()
