# V1 refresh allocation investigation

Read-only source/profile analysis, 2026-10-31. No additional scan, refresh,
build or GH job was run for this note. This does not replace the approved W5
staged-refresh experiment, change its order or allocate runner time.

## Observed symptom and attribution limit

The authorized local calibration refreshed a two-entry subtree in a
7,274,483-entry snapshot. It took66.61s and reached1,866,100KiB process VmHWM
(1,822.36MiB). Load took16.155s; the6.636s finalize timer includes grafting and
`finalize_with`, not just aggregate folding. The1Hz samples first show that HWM
between17.3 and18.3 seconds, inside this combined phase. The result reported two
inaccessible temporary subtrees and is not a clean completeness pass.

The installed binary's exact source correspondence remains unknown. Matching
report phases and current source are investigative evidence, not build provenance.
Sparse process samples cannot assign the peak to one allocation. See the
[refresh receipt](../benchmark-results/local-v1-refresh-calibration-2026-10-31.json).

## Source facts

| Allocation | Loader/graft behavior | Consequence |
| --- | --- | --- |
| `Tree.nodes: Vec<Node>` | `refresh/load.rs::load_nodes` resizes an initially empty vector toN. `graft` marks old descendants REMOVED; `replace_subtree` appends each new descendant except the reused root. | For a two-entry rescan, exactly one node is appended to theN stored nodes. Old removed slots remain allocated. Capacity is at leastN, but its actual value was not measured. |
| `Tree.names: Vec<u8>` | Loader calls `push_name` for each name without recording capacity. Graft appends all `sub.names`, including the reused root's stored name. | Old name bytes remain. The names vector can also grow if spare capacity is insufficient; the trace does not reveal that capacity. |
| Layout vectors | Loader constructs parent/child-count/block-start/fill/raw-id vectors. Only `raw_of` is returned with the loaded tree. | Several O(N) u32 buffers coexist during loading; some are gone before graft. `raw_of` remains through finalization. |
| Finalization vectors | `build_preorder` takesN from `nodes.len()` and allocates order/rank/path hashes; hardlink structures and path stacks also coexist. | Removed slots still contribute to allocation bounds even when omitted from final output. Their allocation can overlap graft buffers. |
| Refresh-only link/mapping data | `loaded.raw_of`, `loaded.links`, policy and snapshot are dropped after finalization and before SQLite writing. | Moving these drops could reduce overlap, but source inspection alone cannot establish25% savings. |

Source references: `core/src/refresh/load.rs::load_nodes`,
`core/src/refresh.rs::{graft,replace_subtree,refresh_locked}` and
`core/src/scan/tree.rs::{push_name,build_preorder}`. The CLI selects mimalloc;
assuming plain musl/system-malloc behavior would be unsupported.

## Hypothesis, not a confirmed root cause

If the loaded node vector has capacityN, appending one new descendant requires
growth. Depending on the actual Rust/allocator behavior, a larger allocation and
copy could overlap the oldN-element buffer. Name-vector growth is a second
possible contributor. Current source neither records exact capacities nor proves
a doubling factor, copying, in-place growth, retained allocator pages or which
allocation establishes process HWM.

A reserve made after loading can itself create the overlap. Preallocating during
loading or reserving only the exact additional capacity are candidate diagnostic
counterfactuals, not established fixes. Reserving too much may retain more memory
into finalization. Neither a capacity counter nor allocated-byte arithmetic
establishes physical RSS or the minimum successful cgroup budget.

## Bounded future falsification probe

Under a separately admitted future W5 diagnostic allocation, record diagnostic-only
phase samples before graft, after name append, after node append, after preorder
construction and after finalization. Capture node/name lengths and capacities,
append counts, actual capacity changes and a higher-frequency RSS/HWM trace.
Keep diagnostic instrumentation out of ordinary per-entry production tracing.

Compare the unchanged baseline with one isolated capacity-reservation
counterfactual on identical seeds/workloads. The hypothesis is weakened if the
capacity never changes, the allocator extends in place without a peak, or the
peak remains in later finalization. An allocator-capable diagnostic build could
provide stronger evidence of simultaneous live allocations, if available.

Only measured semantic parity plus the existing four-metric gates can justify a
later optimization. No RAM saving, latency gain or release suitability is claimed
by this note, and no such workload is authorized by D18.
