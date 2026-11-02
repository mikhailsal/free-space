import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from cleanup_probe_forecast import forecast


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

    def test_over_cap_rejected(self):
        manifest=smoke(); manifest['elapsed_seconds']=1000
        self.assertFalse(forecast(manifest,{'success':True},'source',100)['admitted'])


if __name__=='__main__': unittest.main()
