# Staged refresh isolated smoke

[Run 41244708164](https://github.com/mikhailsal/free-space/actions/runs/41244708164) completed one paired refresh of an unchanged 100-entry subtree in a 100,000-file fixture, using 4 GiB workers and musl builds. Strict recomputation matched the archived summary and passed. Both observers reported 100,449 current and previous entries, identical semantic digests, and unchanged logical content. Runner cleanup succeeded. This is one smoke pair, not a qualification or promotion result.

| Complete-command measure | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Wall time | 1.035050 s | 2.382853 s | +130.22% |
| Process peak RSS | 62,328 KiB | 37,716 KiB | -39.49% |
| Worker cgroup peak | 106,356,736 B | 110,542,848 B | +3.94% |
| Sampled application disk peak | 37,015,552 B | 54,747,136 B | +47.90% |

Both worker cgroups had a 4 GiB limit and recorded zero max, OOM and OOM-kill events and zero memory PSI. I/O PSI was observed. Worker cgroup peak includes the command and sampler, excluding the observer; sampled disk peaks are lower bounds. Candidate wall time and disk use increased substantially, and total cgroup peak increased despite lower process RSS. No further scale is authorized by this result alone.

GitHub ran 327 Rust tests successfully with one ignored and 113 Python tests successfully; the unchanged CLI history test and new public and differential history tests passed. Build and paired jobs took 278 and 82 seconds. The declared profile policy was `none` for both arms and seeds; profile-v2 was not run. The test-only cleanup isolation fix has an outside Opus review pending after a provider quota failure. No Rust tests or app scans were run locally.

Baseline binary SHA-256: `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`. Candidate binary SHA-256: `5ed741badb99a54d80fb43d7251ff6781aa85040b00c51dd99c09f64957accfd`. The companion JSON and archive retain provenance, resource receipts, SDK-space admission, and original raw files.
