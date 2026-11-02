import unittest
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from cleanup_probe_resources import GIB, capture, parse_meminfo, require


def memory(total, available):
    return f'MemTotal: {total // 1024} kB\nMemAvailable: {available // 1024} kB\n'


class ResourceTests(unittest.TestCase):
    def test_thresholds(self):
        self.assertTrue(capture(memory(10*GIB, 10*GIB), 2)['success'])
        for total, available, cpus in ((10*GIB-1024, 10*GIB, 2), (10*GIB, 10*GIB-1024, 2), (10*GIB, 10*GIB, 1)):
            receipt=capture(memory(total, available), cpus)
            self.assertFalse(receipt['success'])
            with self.assertRaises(ValueError): require(receipt)

    def test_malformed_and_missing(self):
        for text in ('', 'MemTotal: 1 MB\nMemAvailable: 1 kB',
                     'MemTotal: 1 kB\nMemTotal: 2 kB\nMemAvailable: 2 kB'):
            with self.assertRaises(ValueError): parse_meminfo(text)


if __name__ == '__main__': unittest.main()
