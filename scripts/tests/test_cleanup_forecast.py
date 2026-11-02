import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from cleanup_probe_forecast import forecast as calculate

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
    return {'stage':'smoke', 'source_sha':'source', 'success':True, 'elapsed_seconds':100,
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

    def test_bad_oracle_and_missing_cycle(self):
        manifest=smoke(); manifest['phases'][0]['success']=False
        with self.assertRaises(ValueError): forecast(manifest,{'success':True},'source',100)
        manifest=smoke(); manifest['phases'].pop()
        with self.assertRaises(ValueError): forecast(manifest,{'success':True},'source',100)

    def test_storage_and_setup_gates(self):
        self.assertFalse(forecast(smoke(),{'success':True},'source',100,available_bytes=2*GIB)['admitted'])
        with self.assertRaises(ValueError): forecast(smoke(),{'success':True},'source',100,space={})
        with self.assertRaises(ValueError): forecast(smoke(),{'success':True},'source',100,timing={})

    def test_over_cap_rejected(self):
        manifest=smoke(); manifest['elapsed_seconds']=1000
        self.assertFalse(forecast(manifest,{'success':True},'source',100)['admitted'])


if __name__=='__main__': unittest.main()
