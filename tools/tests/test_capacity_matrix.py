from __future__ import annotations

import unittest

from tools.capacity_matrix import parallel_map


class MatrixCapacityUtilitiesTests(unittest.TestCase):
    def test_parallel_map_returns_every_result_without_order_assumption(self) -> None:
        results = parallel_map(lambda value: value * value, range(20), workers=4)
        self.assertEqual(sorted(results), [value * value for value in range(20)])


if __name__ == "__main__":
    unittest.main()
