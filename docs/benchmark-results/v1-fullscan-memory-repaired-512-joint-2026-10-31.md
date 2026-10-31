# 512 MiB full-scan joint comparison

Run 41249609974 completed successfully. Strict recomputation matched the raw summary; all six worker and observer semantic checks passed for current and previous snapshots of 1,004,499 entries. The baseline source was e8265778ef78ec4acd9da1375eedb0be37a123f5 and candidate source 0adc796a99fd33abec85ab250ad9b2529507b49a. Their actual binary SHA-256 values were ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980 and f0c25acd428e150a0ad5e1139f5ef7b14cf6404dacd1cd55e0d03cec166a6e74 respectively.

Three paired candidate-minus-baseline scan times were −15.936, −16.415 and −21.246 seconds (median −19.779%). RSS differences were +148, +376 and +192 KiB; disk peak differences were +28,672, +28,672 and −16,384 bytes. The positive candidate median RSS (+192 KiB) and disk (+28,672 bytes) do not meet the unchanged H2 no-growth rule. The earlier 512 MiB A/A maximum absolute differences were 3.062% time, 1,228 KiB RSS and 36,864 bytes disk, descriptive controls on another VM; they do not waive the rule.

All six workers had a 512 MiB cgroup cap and reached that peak, with swap disabled and no hierarchical OOM or OOM-kill events. Hierarchical max events ranged 2361–3665; PSI some totals ranged 649730–804825 microseconds. Endpoint memory breakdowns and complete pressure/events are retained in the JSON and raw evidence. Local memory events were not collected. Setup and observers used 4 GiB limits. Scan-only profile-v2 is not applicable. Lowering the cap does not establish a source-code RAM saving or a minimum RAM requirement.

Runner preparation succeeded without deletion: available bytes remained 91857043456, with no actions. This does not exercise the low-disk cleanup branch. Build and paired jobs took 144 and 847 runner seconds; tests reported 320 Rust passed, 0 failed, 1 ignored, and 111 Python passed.

The deterministic archive contains 126 original artifact files and supporting evidence, with a verified per-file index. Archive SHA-256: `20bf37ace036e9ef2e802b25f5d96a6e216dbd8c750f1cf4f4d31456bea0814b`.
