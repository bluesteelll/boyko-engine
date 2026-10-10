MAKE-UP VARIANT: dropped protocol slots filled from raw/makeup/runs_makeup.jsonl where the make-up process is valid and clean: [{'slot': ('headline', 1, 0, 'HL-D-allpairs', 'tip', 2), 'makeup_found': True, 'makeup_valid_clean': True, 'makeup_mean_ms': 6.236716400000001, 'rb': 1.55, 'ra': 1.62}, {'slot': ('headline', 1, 0, 'HL-D-tree', 'tip', 1), 'makeup_found': True, 'makeup_valid_clean': True, 'makeup_mean_ms': 7.013496, 'rb': 2.36, 'ra': 1.55}, {'slot': ('g9', 1, 1, 'J-As', 'tip', 8), 'makeup_found': True, 'makeup_valid_clean': True, 'makeup_mean_ms': 4.2020138, 'rb': 1.62, 'ra': 4.79}]
timed processes 145 (re-runs 25), warm-ups 6, voided pass attempts 3 (45 processes discarded), invalid 0, contaminated before 0 / after 28 / build-proc-during 0, dropped slots 0

### Cells (ms/step; MEDIAN over K of the process [0,500) window mean; min-max; SE of the median = 1.2533*SD/sqrt(K))

| row | W | binary | K | median | min | max | range | SE_med | range/med | SE/med | pose | TreeDiag (static_rebuilds/members/evictions; all other fields) | others busy % med/max | perf % med |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| HL-D-tree | 1 | tip | 6 | 7.0599 | 6.8913 | 7.2863 | 0.3950 | 0.0709 | 5.59 % | 1.00 % | 0x32d5e235342b4143 | 1/1/0; others sum 0 | 1.61/2.43 | 129.7 |
| HL-D-tree | 2 | tip | 6 | 4.6586 | 4.4690 | 5.5586 | 1.0896 | 0.2061 | 23.39 % | 4.42 % | 0x32d5e235342b4143 | 1/1/0; others sum 0 | 2.70/4.87 | 131.3 |
| HL-D-tree | 4 | tip | 6 | 3.3151 | 3.2973 | 3.3982 | 0.1010 | 0.0201 | 3.05 % | 0.61 % | 0x32d5e235342b4143 | 1/1/0; others sum 0 | 2.00/3.65 | 131.9 |
| HL-D-tree | 8 | tip | 6 | 2.7160 | 2.5201 | 2.9631 | 0.4429 | 0.0928 | 16.31 % | 3.42 % | 0x32d5e235342b4143 | 1/1/0; others sum 0 | 1.23/4.99 | 133.2 |
| HL-D-allpairs | 1 | tip | 6 | 8.7823 | 8.5276 | 9.1355 | 0.6079 | 0.1380 | 6.92 % | 1.57 % | 0x32d5e235342b4143 | 0/0/0; others sum 0 | 1.77/4.67 | 129.5 |
| HL-D-allpairs | 2 | tip | 6 | 6.3405 | 6.2367 | 6.5570 | 0.3203 | 0.0748 | 5.05 % | 1.18 % | 0x32d5e235342b4143 | 0/0/0; others sum 0 | 1.96/4.49 | 129.0 |
| HL-D-allpairs | 4 | tip | 6 | 4.9759 | 4.8674 | 5.5610 | 0.6936 | 0.1306 | 13.94 % | 2.62 % | 0x32d5e235342b4143 | 0/0/0; others sum 0 | 1.86/5.73 | 127.5 |
| HL-D-allpairs | 8 | tip | 6 | 4.2957 | 4.2130 | 4.5386 | 0.3256 | 0.0674 | 7.58 % | 1.57 % | 0x32d5e235342b4143 | 0/0/0; others sum 0 | 1.25/3.64 | 126.5 |
| HL-R-tree | 1 | tip | 6 | 9.7683 | 9.0820 | 10.2733 | 1.1913 | 0.2006 | 12.20 % | 2.05 % | 0xee2a67a98434919a | 1/1/0; others sum 0 | 2.81/4.29 | 129.4 |
| HL-R-tree | 8 | tip | 6 | 3.8031 | 3.6061 | 3.8108 | 0.2047 | 0.0450 | 5.38 % | 1.18 % | 0xee2a67a98434919a | 1/1/0; others sum 0 | 1.92/7.20 | 131.5 |
| HL-R-allpairs | 1 | tip | 6 | 11.2743 | 11.0407 | 11.3485 | 0.3078 | 0.0609 | 2.73 % | 0.54 % | 0xee2a67a98434919a | 0/0/0; others sum 0 | 2.12/3.45 | 129.4 |
| HL-R-allpairs | 8 | tip | 6 | 5.3968 | 5.1617 | 5.7924 | 0.6307 | 0.1121 | 11.69 % | 2.08 % | 0xee2a67a98434919a | 0/0/0; others sum 0 | 1.39/3.48 | 126.1 |
| J-As | 1 | parent | 6 | 8.8591 | 8.7088 | 9.1356 | 0.4268 | 0.0845 | 4.82 % | 0.95 % | 0x32d5e235342b4143 | 0/0/0; others sum 0 | 1.44/3.66 | 130.0 |
| J-As | 1 | tip | 6 | 8.6270 | 8.3888 | 8.9806 | 0.5918 | 0.1103 | 6.86 % | 1.28 % | 0x32d5e235342b4143 | 0/0/0; others sum 0 | 1.42/3.32 | 130.1 |
| J-As | 8 | parent | 6 | 4.7257 | 4.4552 | 4.8959 | 0.4406 | 0.0733 | 9.32 % | 1.55 % | 0x32d5e235342b4143 | 0/0/0; others sum 0 | 1.79/3.18 | 123.9 |
| J-As | 8 | tip | 6 | 4.4380 | 4.2020 | 4.6009 | 0.3989 | 0.0835 | 8.99 % | 1.88 % | 0x32d5e235342b4143 | 0/0/0; others sum 0 | 1.18/4.04 | 125.1 |
| J-As-a | 1 | parent | 6 | 8.8211 | 8.6291 | 8.9865 | 0.3574 | 0.0625 | 4.05 % | 0.71 % | 0x32d5e235342b4143 | 0/0/0; others sum 0 | 0.86/2.86 | 130.6 |
| J-As-a | 1 | tip | 6 | 8.4727 | 8.3959 | 8.6680 | 0.2721 | 0.0570 | 3.21 % | 0.67 % | 0x32d5e235342b4143 | 0/0/0; others sum 0 | 1.27/2.36 | 130.5 |
| J-As-a | 8 | parent | 6 | 4.4981 | 4.4246 | 5.2698 | 0.8453 | 0.1667 | 18.79 % | 3.71 % | 0x32d5e235342b4143 | 0/0/0; others sum 0 | 1.17/3.98 | 126.6 |
| J-As-a | 8 | tip | 6 | 4.3232 | 4.1786 | 4.4252 | 0.2466 | 0.0528 | 5.70 % | 1.22 % | 0x32d5e235342b4143 | 0/0/0; others sum 0 | 0.94/3.14 | 126.3 |

### (1) Headline, tip only - tree vs allpairs (ratio = tree / allpairs), and each against Jolt v5.6.0 (window 3 cells, not re-run)

Thresholds are the rule's right-hand side 2*sqrt(sA^2 + sB^2), s as a fraction of the median: SE reading and min-max (range) reading. Arithmetic only.

| scene | W | allpairs median | tree median | tree - allpairs ms | ratio tree/allpairs | abs(ratio-1) | thr SE | thr min-max | Jolt v5.6.0 ms | tree / Jolt | allpairs / Jolt |
|---|---|---|---|---|---|---|---|---|---|---|---|
| J (--cfg default) | 1 | 8.7823 | 7.0599 | -1.7225 | 0.8039 | 0.1961 | 0.0373 | 0.1780 | 9.828 | 0.7183 | 0.8936 |
| J (--cfg default) | 2 | 6.3405 | 4.6586 | -1.6819 | 0.7347 | 0.2653 | 0.0916 | 0.4786 | 5.770 | 0.8074 | 1.0989 |
| J (--cfg default) | 4 | 4.9759 | 3.3151 | -1.6608 | 0.6662 | 0.3338 | 0.0539 | 0.2854 | 3.581 | 0.9257 | 1.3895 |
| J (--cfg default) | 8 | 4.2957 | 2.7160 | -1.5797 | 0.6323 | 0.3677 | 0.0752 | 0.3597 | 2.569 | 1.0572 | 1.6721 |
| rest (--cfg default) | 1 | 11.2743 | 9.7683 | -1.5060 | 0.8664 | 0.1336 | 0.0425 | 0.2499 | - | - | - |
| rest (--cfg default) | 8 | 5.3968 | 3.8031 | -1.5937 | 0.7047 | 0.2953 | 0.0478 | 0.2573 | - | - | - |

### (2) G9-on-C3 - parent 0ca312bd vs tip cbd86a65 (ratio = tip / parent)

| row | W | parent median | tip median | tip - parent ms | ratio | abs(ratio-1) | thr SE | thr min-max | K p/t |
|---|---|---|---|---|---|---|---|---|---|
| J-As | 1 | 8.8591 | 8.6270 | -0.2320 | 0.9738 | 0.0262 | 0.0319 | 0.1677 | 6/6 |
| J-As | 8 | 4.7257 | 4.4380 | -0.2877 | 0.9391 | 0.0609 | 0.0487 | 0.2590 | 6/6 |
| J-As-a | 1 | 8.8211 | 8.4727 | -0.3484 | 0.9605 | 0.0395 | 0.0195 | 0.1034 | 6/6 |
| J-As-a | 8 | 4.4981 | 4.3232 | -0.1749 | 0.9611 | 0.0389 | 0.0780 | 0.3928 | 6/6 |

#### J-As-a W=1: per-stage spans (per process the median over steps [100, 500), then the median over K; ms)

| zone | parent median | parent min-max | parent SE | tip median | tip min-max | tip SE | tip - parent | ratio | 2*hypot(SE) |
|---|---|---|---|---|---|---|---|---|---|
| phys_solve_build_ns | 0.3209 | 0.3129-0.3740 | 0.0140 | 0.3298 | 0.2936-0.4617 | 0.0340 | +0.0089 | 1.0278 | 0.0735 |
| phys_gravity_ns | 0.0075 | 0.0072-0.0076 | 0.0001 | 0.0076 | 0.0074-0.0082 | 0.0002 | +0.0001 | 1.0174 | 0.0004 |
| phys_warm_apply_ns | 0.5285 | 0.5265-0.5297 | 0.0006 | 0.2960 | 0.2949-0.2978 | 0.0006 | -0.2325 | 0.5601 | 0.0017 |
| phys_integrate_ns | 0.0654 | 0.0652-0.0731 | 0.0016 | 0.0654 | 0.0653-0.0656 | 0.0001 | +0.0001 | 1.0008 | 0.0033 |
| phys_pass_biased_ns | 0.8464 | 0.8157-0.9489 | 0.0246 | 0.8201 | 0.8173-0.8457 | 0.0056 | -0.0262 | 0.9690 | 0.0505 |
| phys_pass_relax_ns | 1.6720 | 1.6116-1.8756 | 0.0489 | 1.6218 | 1.6157-1.6716 | 0.0110 | -0.0502 | 0.9700 | 0.1002 |
| phys_color_wide_ns | 2.5071 | 2.4155-2.8110 | 0.0731 | 2.4306 | 2.4213-2.5057 | 0.0165 | -0.0766 | 0.9695 | 0.1499 |
| phys_color_narrow_ns | 0.0093 | 0.0091-0.0105 | 0.0003 | 0.0092 | 0.0090-0.0093 | 0.0001 | -0.0001 | 0.9881 | 0.0005 |
| phys_restitution_ns | 0.0032 | 0.0030-0.0034 | 0.0001 | 0.0033 | 0.0032-0.0033 | 0.0000 | +0.0001 | 1.0347 | 0.0002 |
| phys_store_ns | 0.0250 | 0.0220-0.0304 | 0.0016 | 0.0274 | 0.0209-0.0455 | 0.0049 | +0.0025 | 1.0981 | 0.0103 |
| phys_write_back_ns | 0.0026 | 0.0026-0.0027 | 0.0000 | 0.0026 | 0.0025-0.0027 | 0.0000 | -0.0000 | 0.9819 | 0.0001 |
| phys_np_dispatch_ns | 0.0000 | 0.0000-0.0000 | 0.0000 | 0.0000 | 0.0000-0.0000 | 0.0000 | +0.0000 | - | 0.0000 |
| phys_np_compact_ns | 0.0000 | 0.0000-0.0000 | 0.0000 | 0.0000 | 0.0000-0.0000 | 0.0000 | +0.0000 | - | 0.0000 |
| phys_np_axis_commit_ns | 0.0000 | 0.0000-0.0000 | 0.0000 | 0.0000 | 0.0000-0.0000 | 0.0000 | +0.0000 | - | 0.0000 |
| sys_physics_integrate_ns | 0.0000 | 0.0000-0.0001 | 0.0000 | 0.0001 | 0.0000-0.0001 | 0.0000 | +0.0000 | 1.5000 | 0.0000 |
| sys_physics_gather_ns | 0.0242 | 0.0234-0.0244 | 0.0002 | 0.0241 | 0.0234-0.0339 | 0.0021 | -0.0001 | 0.9964 | 0.0042 |
| sys_select_broadphase_ns | 0.0001 | 0.0000-0.0001 | 0.0000 | 0.0001 | 0.0001-0.0001 | 0.0000 | +0.0000 | 1.0000 | 0.0000 |
| sys_physics_broadphase_ns | 1.9256 | 1.9242-1.9345 | 0.0019 | 1.9255 | 1.9243-1.9278 | 0.0006 | -0.0001 | 0.9999 | 0.0041 |
| sys_physics_narrowphase_ns | 3.0162 | 2.9840-3.0888 | 0.0201 | 3.0110 | 2.9829-3.0745 | 0.0175 | -0.0053 | 0.9983 | 0.0533 |
| sys_physics_build_graph_ns | 0.1141 | 0.1130-0.1156 | 0.0005 | 0.1159 | 0.1135-0.1181 | 0.0010 | +0.0018 | 1.0160 | 0.0022 |
| sys_physics_solve_colored_ns | 3.5344 | 3.3785-3.7690 | 0.0693 | 3.2055 | 3.1475-3.3349 | 0.0366 | -0.3290 | 0.9069 | 0.1566 |
| sys_physics_apply_ns | 0.0129 | 0.0111-0.0133 | 0.0004 | 0.0112 | 0.0109-0.0134 | 0.0006 | -0.0017 | 0.8670 | 0.0015 |
| u_ns | 0.0011 | 0.0010-0.0012 | 0.0000 | 0.0012 | 0.0011-0.0013 | 0.0000 | +0.0000 | 1.0336 | 0.0001 |
| g_ns | 0.0397 | 0.0384-0.0425 | 0.0008 | 0.0388 | 0.0375-0.0412 | 0.0007 | -0.0009 | 0.9776 | 0.0021 |
| r_ns | 0.0026 | 0.0025-0.0029 | 0.0001 | 0.0026 | 0.0025-0.0028 | 0.0000 | -0.0000 | 0.9839 | 0.0002 |
| sys_sum_ns | 8.7110 | 8.4713-8.8400 | 0.0719 | 8.3021 | 8.2528-8.5221 | 0.0593 | -0.4090 | 0.9531 | 0.1864 |
| wall_ns | 8.7553 | 8.5135-8.8783 | 0.0721 | 8.3403 | 8.2949-8.5708 | 0.0610 | -0.4151 | 0.9526 | 0.1888 |

counters (median over K of the per-process median): manifolds parent 4519.0 / tip 4519.0; pairs parent 9559.0 / tip 9559.0; colors parent 11.0 / tip 11.0; wide_colors parent 9.0 / tip 9.0; waves parent 108.0 / tip 108.0; phys_np_points parent 17054.0 / tip 17054.0; phys_slots_wide parent 17013.5 / tip 17013.5; phys_slots_narrow parent 36.0 / tip 36.0
waves_total parent [54660] / tip [54660]; drops_total parent [0] / tip [0]; solve_on_dispatcher_steps parent [0] / tip [0]

warm_apply at W=1, reference numbers side by side (arithmetic only): window 4b C2 0.519 ms; this window parent 0.5285 ms, tip 0.2960 ms; tip - parent -0.2325 ms; tip - 0.519 -0.2230 ms; design bar -0.19 ms; 2*hypot(SE) 0.0017 ms

#### J-As-a W=8: per-stage spans (per process the median over steps [100, 500), then the median over K; ms)

| zone | parent median | parent min-max | parent SE | tip median | tip min-max | tip SE | tip - parent | ratio | 2*hypot(SE) |
|---|---|---|---|---|---|---|---|---|---|
| phys_solve_build_ns | 0.3242 | 0.3124-0.3711 | 0.0115 | 0.3359 | 0.3129-0.3697 | 0.0103 | +0.0117 | 1.0361 | 0.0309 |
| phys_gravity_ns | 0.0127 | 0.0123-0.0131 | 0.0002 | 0.0125 | 0.0123-0.0131 | 0.0001 | -0.0002 | 0.9860 | 0.0005 |
| phys_warm_apply_ns | 0.5391 | 0.5342-0.5459 | 0.0026 | 0.3043 | 0.2995-0.3131 | 0.0026 | -0.2348 | 0.5645 | 0.0075 |
| phys_integrate_ns | 0.0845 | 0.0796-0.0901 | 0.0024 | 0.0803 | 0.0798-0.0825 | 0.0005 | -0.0042 | 0.9503 | 0.0050 |
| phys_pass_biased_ns | 0.2311 | 0.2285-0.2461 | 0.0034 | 0.2325 | 0.2285-0.2417 | 0.0025 | +0.0013 | 1.0057 | 0.0085 |
| phys_pass_relax_ns | 0.4285 | 0.4225-0.4568 | 0.0067 | 0.4288 | 0.4194-0.4487 | 0.0053 | +0.0003 | 1.0007 | 0.0170 |
| phys_color_wide_ns | 0.6443 | 0.6368-0.6855 | 0.0095 | 0.6457 | 0.6333-0.6742 | 0.0076 | +0.0014 | 1.0021 | 0.0244 |
| phys_color_narrow_ns | 0.0109 | 0.0103-0.0114 | 0.0002 | 0.0108 | 0.0106-0.0110 | 0.0001 | -0.0001 | 0.9929 | 0.0005 |
| phys_restitution_ns | 0.0031 | 0.0030-0.0034 | 0.0001 | 0.0033 | 0.0032-0.0035 | 0.0000 | +0.0002 | 1.0764 | 0.0002 |
| phys_store_ns | 0.0266 | 0.0253-0.0335 | 0.0015 | 0.0308 | 0.0258-0.0370 | 0.0019 | +0.0042 | 1.1599 | 0.0050 |
| phys_write_back_ns | 0.0027 | 0.0027-0.0027 | 0.0000 | 0.0027 | 0.0026-0.0028 | 0.0000 | -0.0000 | 0.9963 | 0.0001 |
| phys_np_dispatch_ns | 0.5089 | 0.5033-0.5252 | 0.0049 | 0.5111 | 0.5021-0.5327 | 0.0063 | +0.0022 | 1.0044 | 0.0159 |
| phys_np_compact_ns | 0.0200 | 0.0189-0.0234 | 0.0008 | 0.0202 | 0.0194-0.0215 | 0.0004 | +0.0002 | 1.0118 | 0.0018 |
| phys_np_axis_commit_ns | 0.0036 | 0.0036-0.0037 | 0.0000 | 0.0036 | 0.0036-0.0037 | 0.0000 | +0.0000 | 1.0034 | 0.0001 |
| sys_physics_integrate_ns | 0.0001 | 0.0000-0.0001 | 0.0000 | 0.0000 | 0.0000-0.0001 | 0.0000 | -0.0000 | 0.9000 | 0.0000 |
| sys_physics_gather_ns | 0.0254 | 0.0248-0.0269 | 0.0004 | 0.0255 | 0.0252-0.0263 | 0.0002 | +0.0001 | 1.0023 | 0.0009 |
| sys_select_broadphase_ns | 0.0001 | 0.0001-0.0001 | 0.0000 | 0.0001 | 0.0001-0.0001 | 0.0000 | +0.0000 | 1.0833 | 0.0000 |
| sys_physics_broadphase_ns | 1.9593 | 1.9463-1.9828 | 0.0090 | 1.9537 | 1.9469-2.0261 | 0.0160 | -0.0056 | 0.9972 | 0.0367 |
| sys_physics_narrowphase_ns | 0.5374 | 0.5312-0.5622 | 0.0066 | 0.5400 | 0.5299-0.5629 | 0.0067 | +0.0026 | 1.0049 | 0.0187 |
| sys_physics_build_graph_ns | 0.1146 | 0.1129-0.1178 | 0.0010 | 0.1163 | 0.1125-0.1201 | 0.0014 | +0.0017 | 1.0146 | 0.0034 |
| sys_physics_solve_colored_ns | 1.6548 | 1.6277-1.7763 | 0.0289 | 1.4502 | 1.3903-1.5073 | 0.0222 | -0.2046 | 0.8764 | 0.0729 |
| sys_physics_apply_ns | 0.0123 | 0.0112-0.0135 | 0.0006 | 0.0121 | 0.0110-0.0138 | 0.0007 | -0.0002 | 0.9870 | 0.0018 |
| u_ns | 0.0011 | 0.0010-0.0014 | 0.0001 | 0.0011 | 0.0011-0.0012 | 0.0000 | +0.0001 | 1.0508 | 0.0001 |
| g_ns | 0.0338 | 0.0331-0.0365 | 0.0007 | 0.0338 | 0.0330-0.0352 | 0.0005 | +0.0000 | 1.0005 | 0.0017 |
| r_ns | 0.0038 | 0.0037-0.0039 | 0.0000 | 0.0039 | 0.0038-0.0041 | 0.0001 | +0.0001 | 1.0370 | 0.0001 |
| sys_sum_ns | 4.3237 | 4.2801-4.5872 | 0.0625 | 4.1310 | 4.0265-4.3137 | 0.0528 | -0.1927 | 0.9554 | 0.1637 |
| wall_ns | 4.3570 | 4.3126-4.6237 | 0.0633 | 4.1653 | 4.0601-4.3495 | 0.0535 | -0.1917 | 0.9560 | 0.1658 |

counters (median over K of the per-process median): manifolds parent 4519.0 / tip 4519.0; pairs parent 9559.0 / tip 9559.0; colors parent 11.0 / tip 11.0; wide_colors parent 9.0 / tip 9.0; waves parent 108.0 / tip 108.0; phys_np_points parent 17054.0 / tip 17054.0; phys_slots_wide parent 17013.5 / tip 17013.5; phys_slots_narrow parent 36.0 / tip 36.0
waves_total parent [54660] / tip [54660]; drops_total parent [0] / tip [0]; solve_on_dispatcher_steps parent [0] / tip [0]

### Load receipts

- 145 timed processes (non-voided); 5-s receipt before: median 2.70 %, max 4.97 %, > 5 %: 0; after: median 2.95 %, max 13.20 %, > 5 %: 28
- during-process witness others_busy_pct: median 1.73 %, max 10.73 %, > 5 %: 7 (of which in the chosen set: 2)
- build processes busy in receipts: none; build/lane processes present at receipts: none; during a process: none; total wait for quiet before processes 1452 s; affinity masks read back ['0xffff']
