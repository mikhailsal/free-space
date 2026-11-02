from pathlib import Path
import sys
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).parents[1]))
from cause_global import background_sources


class BackgroundTests(unittest.TestCase):
    def test_raw_pressure_and_load_preserved_and_missing_not_zero(self):
        raw = 'some avg10=0.50 avg60=0.20 avg300=0.10 total=123\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n'
        def read(name, **unused):
            return dict(available=True, raw='1.0 2.0 3.0 1/200 123' if name == 'loadavg' else raw)
        with patch('cause_global._read', side_effect=read):
            result = background_sources(123)
        self.assertTrue(result['required_fields_available'])
        self.assertEqual(result['sources']['memory']['parsed']['some']['avg10'], .5)
        self.assertEqual(result['sources']['io']['raw'], raw)
        with patch('cause_global._read', return_value=dict(available=False, error='missing')):
            result = background_sources(123)
        self.assertFalse(result['required_fields_available'])
        self.assertNotIn('parsed', result['sources']['io'])

    def test_nonfinite_pressure_is_unavailable(self):
        with patch('cause_global._read', return_value=dict(available=True, raw='some avg10=nan avg60=0 avg300=0 total=0')):
            result = background_sources(123)
        self.assertFalse(result['required_fields_available'])
        self.assertIn('error', result['sources']['memory'])


if __name__ == '__main__':
    unittest.main()
