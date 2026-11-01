# Cgroup trace same-binary overhead screen

Run 41269627684 on `d64f94369a97a35a644010e19c98de5992c06185` passed independent strict ordinary and diagnostic recomputation, each equal to the saved summary. All six observers validated 1,004,499 current and previous entries; cleanup succeeded. Both downloaded musl binaries independently hashed to `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`, with baseline epoch 1793464183. Three scan pairs compared baseline tracing off with candidate tracing on under 4 GiB worker limits and no swap.

The frozen evaluator independently returned **pass on all eight preregistered overhead checks**. Signed on-minus-off pairs were:

| Pair | Time change | Process RSS | Total worker-cgroup peak | Sampled app disk |
| --- | ---: | ---: | ---: | ---: |
| 1 | +0.043933% | +216 KiB | +1,810,432 B | 0 B |
| 2 | −0.137656% | −116 KiB | +2,015,232 B | 0 B |
| 3 | +0.441107% | +92 KiB | +1,994,752 B | 0 B |

The corresponding median / worst bars were: absolute signed-median time ≤1% and worst absolute pair ≤2%; absolute median RSS ≤256 KiB and worst ≤512 KiB; positive median total cgroup ≤4 MiB and worst ≤8 MiB; disk median exactly zero and worst absolute pair ≤8,192 bytes. Exact fractional time values and individual check results are in the receipt. The bars are prospective diagnostic controls, not confidence intervals, a product gain, an H2 waiver or promotion.

Each candidate trace had 15 samples and zero missed ticks, errors or missing values. Pairs 1 and 2 each recorded one late tick: maximum lateness 20.579836 ms and 20.239417 ms against the 20 ms tolerance; pair 3 had none. The frozen validator accepted the complete traces and their recorded lateness. Five-second points are non-atomic and do not decompose the peak or attribute phases; sampler activity is cocharged to the worker and output may share storage. Ordinary endpoint collection does not capture local memory events.

Runner disk preparation required no deletions. GitHub tests: 307 Rust passed, one ignored, and 151 Python passed. Build and paired jobs used 155 and 810 seconds.

The deterministic archive preserves 131 original artifact files and supporting source, receipts, logs, both independent evaluations and validation; all 152 indexed payloads were verified. Archive SHA-256: `352f6f406024d7f5e11afb73a9525df6122bbf7e25ddce44c5b9b3d159d666a7`.
