# OI1516 — the M2 first-crossing certificate: price receipts (OPEN-ITEMS 16)

Lane `u/render-oi1516`, `oi1516/cut.md` D16-7 / C9. Host counts and static byte / op deltas only;
the timing rows (W16-1, W16-2) and the gpu-cap register row (W16-3) are the orchestrator's windows
and are appended below when they run. No timing verdict is made here.

## Host evaluation counts (the host M2 mirror, the shaders' order)

`compute::tests::m2_first_surface_tests::m2_eval_counts_receipt`: the 64×64 ortho frame, no mesh,
omega 1, the M1 skip on (the device M2 test's configuration). C5 is the base (no certificate), C7
the certificate.

| scene | hits | M2 calls | refine evals | accepts C5 → C7 | rejects | range evals | latch skips | fold evals C5 → C7 |
|---|---|---|---|---|---|---|---|---|
| crater_csg | 752 | 23 582 | 2 546 | 676 → 674 | 2 | 814 | 10 | 29 594 → 29 606 |
| box_csg | 676 | 28 396 | 32 460 | 676 → 676 | 0 | 676 | 0 | 34 560 → 34 560 |
| smooth_union | 728 | 22 272 | 2 888 | 660 → 660 | 0 | 868 | 0 | 28 348 → 28 348 |
| two_spheres | 812 | 24 055 | 2 948 | 796 → 764 | 32 | 1 172 | 92 | 26 543 → 26 667 |

- Range evaluations per certified-or-refused M2 hit: 1.20 (crater), 1.00 (box), 1.32 (smooth),
  1.47 (two spheres). The cut's "+1 range evaluation per accepted ray, typical" holds; the cap is 8.
- crater's 2 rejects are the two far-wall pixels (39,22) / (39,41) the item is about.
- box_csg's 32 460 refine evaluations on 676 accepts are the refines that never converge running
  the full 32 steps (unchanged by this lane: the budget stays 32).

## Static cost (the five re-DXC'd artifacts; VulkanSDK 1.4.350.0)

| artifact | bytes before → after |
|---|---|
| `sdf_gbuffer_composite.comp.spv` | 155 124 → 168 152 (+13 028) |
| `sdf_forward_march.comp.spv` | 144 424 → 157 200 (+12 776) |
| `sdf_forward_march_sdfonly.comp.spv` | 143 928 → 156 704 (+12 776) |
| `sdf_forward_march_viewt.comp.spv` | 144 704 → 157 480 (+12 776) |
| `sdf_forward_march_sdfonly_viewt.comp.spv` | 144 164 → 156 940 (+12 776) |

The cut estimated +3–6 KB per artifact; DXC inlines the three kind leaves at both segment ends, so
the certificate is ~700 instructions per marcher.

Op census, committed module vs the `-D M2_CERTIFY=0` twin (`tests/marcher_occupancy.rs` builds the
twin; the per-opcode deltas are the certificate's code):

- deferred marcher: 9 523 vs 8 824 instructions — `OpFMul` +88, `OpFSub` +62, `OpFAdd` +47,
  `OpFDiv` +15, `OpSelect` +16, `OpLoopMerge` +2; GLSL.std.450 `NMax` +24, `NMin` +9, `FClamp` +8,
  `FAbs` +6, `Sqrt` +6.
- forward march (`HAS_MESH`): 8 786 vs 8 104 — the same body (`OpFMul` +88, `OpFDiv` +15,
  `Sqrt` +6, `OpLoopMerge` +2).

Per range evaluation and edit, by source op count: two endpoint value + slope leaves (a sphere leaf
is 17 ops with one `sqrt` and one division), the convex bound (13 ops, one division) and two
combines, on one `load_edit` — about 2.5–3.5× the ops of one field evaluation per edit (`sdf`'s
`load_edit`, `edit_distance` and one `combine`). The I-cache cost sits outside the per-iteration
hot path: the body runs once per M2 hit, and the per-iteration change on the M2 path is one bool
test (`m2_trust`).

## Register twin (rule (xviii), gpu-cap; RTX 3060 Laptop, this lane)

`cargo test -p boyko_rhi_vulkan --test marcher_occupancy -- --ignored --test-threads=1 --nocapture`:
Register Count 64 committed vs 64 twin on the forward march's default row, its heaviest row
(`HAS_MESH + VIEWT`) and the deferred marcher; the driver reports no occupancy statistic. The row the
orchestrator re-takes in a window is W16-3.

## Windows (the orchestrator's; DIAGNOSTIC)

- W16-1 (base = the lane's C5 probe executable, head = the lane head): pending.
- W16-2 (the disarmed forward-marcher / deferred-marcher zone rows): pending.
