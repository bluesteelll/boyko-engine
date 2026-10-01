# Window 8b - J-Son-T + DM1 (results-analyst, 2026-09-29)

J-SON-T: sleeping ON on the tree broadphase is claimed faster, STRONG, at W1 and W8 in both post-awake windows.
The settled pile [274,500) costs 0.1254 ms/step at W1 and 0.1337 ms at W8, against 14.6145 / 3.8594 ms with sleeping
OFF (-99.14 % / -96.54 %). The product window [100,500) is -55.82 % at W1 and -55.24 % at W8. The awake cost [0,100) is
still NOT CLAIMED: +0.17 % at W1, where 1.88 % was the smallest effect the rule could have claimed, and +0.65 % at W8,
where that floor is 12.93 %. C1b is supported by its pre-registered bar. One caveat on the letter: in pass 0 the OFF
disarmed cells have K = 2, because two slots were dropped. Window 8's code guard (K >= 3) would refuse that pass. The
armed twins have K = 3 in every pass and are CLAIMED STRONG under either reading.
DM1: C6 stays KEPT under ruling 9's rule. VB_EARLY_CULL B-A is -6.94 us (-0.88 %, B faster) idle and +2.42 us
(+0.31 %, inside a 23.01 us band) with the 100-row edit. Window 8's +22.3 us / +2.8 % did not reproduce. Every gated zone
reads B <= A.

Sources: RULINGS-2026-09-27-W8.md rulings 1, 8, 9; RULINGS-2026-09-26.md item 6; win8/analysis.md sections 7-8;
rows8b.json rows JSonT/JSonT-a/JSoffT/JSoffT-a and dm-*; dm1/cut.md sections 5 and 11.5 (the DM1 gate text).
Method as window 8: the cell is the median over K of the per-process means; i = IQR (inclusive quartiles), s =
1.2533*SD/sqrt(K), r = min-max, each relative to the cell median. A comparison clears a bar iff
|B/A-1| > 2*hypot(A.x, B.x). **Ruling 1:** CLAIMED iff i AND s clear in every pass (0 voided passes) and pooled, with
one sign throughout; STRONG iff r also clears everywhere. Slot rule: the original if valid and clean, else its re-run
if valid and clean, else the slot is dropped. Clean = receipts <= 5 % and witness <= 2 %. Placement receipts are
recorded and were not used. Everything is recomputed from raw/ (run.csv, stdout SUMMARY, artifact.toml). Nothing is
taken from the driver's statistics. The per-process means equal the driver's `cols` in 340 of 340 cases.

## Verdict table

"smallest claimable" = the largest i or s bar over the pooled cell and the three passes, i.e. the smallest effect
that ruling 1 could have claimed. The flags are printed as i/s/r. For DM1 the pre-registered rule is the ABBA band
gate, and the i/s/r column there is post hoc.

| claim id | pre-registered bar | cells + n | numbers (ms/step): OFF, ON = median (IQR, SE, min-max); ON/OFF | i/s/r pooled; p0 p1 p2 (smallest claimable) | verdict |
|---|---|---|---|---|---|
| JST-AW-W1 [0,100) | ruling 8: "no measurable awake cost" -> ON not claimed slower (ruling 1 i AND s, every pass + pooled) | JSoffT n=8 (2/3/3), JSonT n=9 (3/3/3) | OFF 15.8843 (IQR 0.0423, SE 0.0190, 15.8007-15.9408); ON 15.9107 (IQR 0.1245, SE 0.0353, 15.7714-16.0471); ON/OFF 1.0017 (+0.17 %, +0.0264 ms) | n/n/n; n/n/n n/n/n n/n/n (1.88 %) | **NOT CLAIMED (no resolved awake cost)** |
| JST-AW-W1-a [0,100) | ruling 8: "no measurable awake cost" -> ON not claimed slower (ruling 1 i AND s, every pass + pooled) | JSoffT-a n=9 (3/3/3), JSonT-a n=9 (3/3/3) | OFF 15.9196 (IQR 0.0980, SE 0.0259, 15.8153-15.9895); ON 15.9363 (IQR 0.0939, SE 0.0401, 15.8338-16.1450); ON/OFF 1.0011 (+0.11 %, +0.0167 ms) | n/n/n; n/n/n n/n/n n/n/n (1.78 %) | **NOT CLAIMED (no resolved awake cost)** |
| JST-AW-W8 [0,100) | ruling 8: "no measurable awake cost" -> ON not claimed slower (ruling 1 i AND s, every pass + pooled) | JSoffT n=8 (2/3/3), JSonT n=9 (3/3/3) | OFF 4.0971 (IQR 0.1746, SE 0.0459, 3.8807-4.1347); ON 4.1235 (IQR 0.2005, SE 0.0558, 3.8187-4.1790); ON/OFF 1.0065 (+0.65 %, +0.0265 ms) | n/n/n; n/n/n n/n/n n/n/n (12.93 %) | **NOT CLAIMED (no resolved awake cost)** |
| JST-AW-W8-a [0,100) | ruling 8: "no measurable awake cost" -> ON not claimed slower (ruling 1 i AND s, every pass + pooled) | JSoffT-a n=9 (3/3/3), JSonT-a n=9 (3/3/3) | OFF 4.2259 (IQR 0.2844, SE 0.0633, 3.9162-4.3020); ON 4.2392 (IQR 0.0970, SE 0.0538, 3.9497-4.2935); ON/OFF 1.0032 (+0.32 %, +0.0134 ms) | n/n/n; n/n/n n/n/n n/n/n (14.22 %) | **NOT CLAIMED (no resolved awake cost)** |
| JST-G-W1 [100,500) | ruling 8 product row: ON claimed faster (ruling 1; window 8 AllPairs -49 % W1 / -36 % W8) | JSoffT n=8 (2/3/3), JSonT n=9 (3/3/3) | OFF 14.7501 (IQR 0.1053, SE 0.0670, 14.6881-15.0911); ON 6.5164 (IQR 0.0186, SE 0.0061, 6.4901-6.5343); ON/OFF 0.4418 (-55.82 %, -8.2338 ms) | Y/Y/Y; Y/Y/Y Y/Y/Y Y/Y/Y (2.81 %) | **CLAIMED STRONG (ON faster); strict K>=3 guard: NOT CLAIMED (p0 OFF K=2)** |
| JST-G-W1-a [100,500) | ruling 8 product row: ON claimed faster (ruling 1; window 8 AllPairs -49 % W1 / -36 % W8) | JSoffT-a n=9 (3/3/3), JSonT-a n=9 (3/3/3) | OFF 14.7674 (IQR 0.0638, SE 0.0508, 14.7382-15.0432); ON 6.5397 (IQR 0.0297, SE 0.0190, 6.5208-6.6667); ON/OFF 0.4429 (-55.71 %, -8.2276 ms) | Y/Y/Y; Y/Y/Y Y/Y/Y Y/Y/Y (3.04 %) | **CLAIMED STRONG (ON faster)** |
| JST-G-W8 [100,500) | ruling 8 product row: ON claimed faster (ruling 1; window 8 AllPairs -49 % W1 / -36 % W8) | JSoffT n=8 (2/3/3), JSonT n=9 (3/3/3) | OFF 3.8260 (IQR 0.1102, SE 0.0305, 3.7364-3.9122); ON 1.7127 (IQR 0.1158, SE 0.0255, 1.6680-1.8061); ON/OFF 0.4476 (-55.24 %, -2.1134 ms) | Y/Y/Y; Y/Y/Y Y/Y/Y Y/Y/Y (14.70 %) | **CLAIMED STRONG (ON faster); strict K>=3 guard: NOT CLAIMED (p0 OFF K=2)** |
| JST-G-W8-a [100,500) | ruling 8 product row: ON claimed faster (ruling 1; window 8 AllPairs -49 % W1 / -36 % W8) | JSoffT-a n=9 (3/3/3), JSonT-a n=9 (3/3/3) | OFF 3.9420 (IQR 0.1089, SE 0.0326, 3.8701-4.1038); ON 1.7813 (IQR 0.0952, SE 0.0231, 1.7151-1.8543); ON/OFF 0.4519 (-54.81 %, -2.1607 ms) | Y/Y/Y; Y/Y/Y Y/Y/Y Y/Y/Y (12.03 %) | **CLAIMED STRONG (ON faster)** |
| JST-F-W1 [274,500) | ruling 8 product row, frozen pile after step 274: ON claimed faster (ruling 1) | JSoffT n=8 (2/3/3), JSonT n=9 (3/3/3) | OFF 14.6145 (IQR 0.1344, SE 0.0743, 14.5637-14.9805); ON 0.1254 (IQR 0.0066, SE 0.0017, 0.1223-0.1349); ON/OFF 0.0086 (-99.14 %, -14.4891 ms) | Y/Y/Y; Y/Y/Y Y/Y/Y Y/Y/Y (10.62 %) | **CLAIMED STRONG (ON faster); strict K>=3 guard: NOT CLAIMED (p0 OFF K=2)** |
| JST-F-W1-a [274,500) | ruling 8 product row, frozen pile after step 274: ON claimed faster (ruling 1) | JSoffT-a n=9 (3/3/3), JSonT-a n=9 (3/3/3) | OFF 14.6551 (IQR 0.1092, SE 0.0576, 14.6263-14.9638); ON 0.1277 (IQR 0.0048, SE 0.0014, 0.1237-0.1334); ON/OFF 0.0087 (-99.13 %, -14.5275 ms) | Y/Y/Y; Y/Y/Y Y/Y/Y Y/Y/Y (7.59 %) | **CLAIMED STRONG (ON faster)** |
| JST-F-W8 [274,500) | ruling 8 product row, frozen pile after step 274: ON claimed faster (ruling 1) | JSoffT n=8 (2/3/3), JSonT n=9 (3/3/3) | OFF 3.8594 (IQR 0.1155, SE 0.0449, 3.6371-3.9422); ON 0.1337 (IQR 0.0073, SE 0.0020, 0.1246-0.1395); ON/OFF 0.0346 (-96.54 %, -3.7257 ms) | Y/Y/Y; Y/Y/Y Y/Y/Y Y/Y/Y (12.48 %) | **CLAIMED STRONG (ON faster); strict K>=3 guard: NOT CLAIMED (p0 OFF K=2)** |
| JST-F-W8-a [274,500) | ruling 8 product row, frozen pile after step 274: ON claimed faster (ruling 1) | JSoffT-a n=9 (3/3/3), JSonT-a n=9 (3/3/3) | OFF 3.9985 (IQR 0.2022, SE 0.0547, 3.7288-4.0900); ON 0.1334 (IQR 0.0105, SE 0.0022, 0.1242-0.1371); ON/OFF 0.0334 (-96.66 %, -3.8651 ms) | Y/Y/Y; Y/Y/Y Y/Y/Y Y/Y/Y (18.71 %) | **CLAIMED STRONG (ON faster)** |
| DM1-VB_SHADE-vb-idle | ruling 9 / dm1 cut 11.5: paired median of B-A <= 0 or inside band max(abs(A1-A2), abs(B1-B2)) -> B<=A; above band = red for C6 (VB_EARLY_CULL above band -> drop C6) | per-frame median over 220 frames; A n=3, B n=4, 3 complete pairs | A 328.7 (IQR 35.6, SE 29.0, 261.1-332.3) us; B 308.2 (IQR 2.4, SE 1.1, 305.7-309.2) us; d = median(B-A) -23.04 us (-7.50 %); band 71.17 us | gate: R1 B<=A (d<=0); R2 [B<=A (inside band) / B<=A (d<=0)]; R3 B<=A (d<=0); all originals R1 B<=A (inside band) (d +43.52 / band 72.70); post hoc i/s/r n/n/n | **B <= A** |
| DM1-EC-vb-idle | as above | per-frame median over 220 frames; A n=3, B n=4, 3 complete pairs | A 783.0 (IQR 7.6, SE 6.1, 781.8-797.0) us; B 777.8 (IQR 2.9, SE 1.4, 774.8-780.1) us; d = median(B-A) -6.94 us (-0.88 %); band 15.22 us | gate: R1 B<=A (d<=0); R2 [B<=A (d<=0) / B<=A (d<=0)]; R3 B<=A (d<=0); all originals R1 B<=A (d<=0) (d -16.83 / band 18.21); post hoc i/s/r n/n/n | **B <= A** |
| DM1-VB_RUN-vb-idle | as above | per-frame median over 220 frames; A n=3, B n=4, 3 complete pairs | A 852.0 (IQR 8.7, SE 6.9, 849.9-867.3) us; B 847.1 (IQR 4.0, SE 1.6, 843.8-848.9) us; d = median(B-A) -6.14 us (-0.72 %); band 17.41 us | gate: R1 B<=A (d<=0); R2 [B<=A (d<=0) / B<=A (d<=0)]; R3 B<=A (d<=0); all originals R1 B<=A (d<=0) (d -16.90 / band 19.97); post hoc i/s/r n/n/n | **B <= A** |
| DM1-VB_PRODUCE_NET-vb-idle | as above | per-frame median over 220 frames; A n=3, B n=4, 3 complete pairs | A 329.7 (IQR 36.1, SE 29.4, 261.1-333.3) us; B 309.2 (IQR 2.7, SE 1.4, 305.7-310.3) us; d = median(B-A) -24.06 us (-7.81 %); band 72.19 us | gate: R1 B<=A (d<=0); R2 [B<=A (inside band) / B<=A (d<=0)]; R3 B<=A (d<=0); all originals R1 B<=A (inside band) (d +43.78 / band 73.22); post hoc i/s/r n/n/n | **B <= A** |
| DM1-GBUF_DEFERRED_RESOLVE-deferred-idle | as above | per-frame median over 220 frames; A n=4, B n=4, 4 complete pairs | A 594.4 (IQR 1.4, SE 0.8, 592.4-594.9) us; B 507.1 (IQR 126.6, SE 46.2, 440.3-570.4) us; d = median(B-A) -87.81 us (-14.78 %); band 130.05 us | gate: R1 B<=A (d<=0); R2 [B<=A (d<=0) / B<=A (d<=0)]; R3 B<=A (d<=0); all originals R1 B<=A (d<=0) (d -150.02 / band 130.05); post hoc i/s/r n/n/n | **B <= A** |
| DM1-VB_SHADE-vb-edit100 | as above | per-frame median over 220 frames; A n=4, B n=3, 3 complete pairs | A 300.5 (IQR 67.3, SE 25.5, 259.1-334.8) us; B 262.1 (IQR 29.2, SE 22.3, 249.9-308.2) us; d = median(B-A) -24.58 us (-8.23 %); band 75.78 us | gate: R1 B<=A (d<=0); R2 [B<=A (d<=0) / B<=A (inside band)]; R3 B<=A (d<=0); all originals R1 B<=A (d<=0) (d -8.70 / band 75.78); post hoc i/s/r n/n/n | **B <= A** |
| DM1-EC-vb-edit100 | as above | per-frame median over 220 frames; A n=4, B n=3, 3 complete pairs | A 789.4 (IQR 14.2, SE 5.4, 781.8-798.2) us; B 793.8 (IQR 11.5, SE 8.8, 775.6-798.6) us; d = median(B-A) +2.42 us (+0.31 %); band 23.01 us | gate: R1 B<=A (inside band); R2 [B<=A (inside band) / RED: B-A above band]; R3 B<=A (inside band); all originals R1 B<=A (d<=0) (d -3.62 / band 23.01); post hoc i/s/r n/n/n | **B <= A (R2 half RED, degenerate)** |
| DM1-VB_RUN-vb-edit100 | as above | per-frame median over 220 frames; A n=4, B n=3, 3 complete pairs | A 859.6 (IQR 14.1, SE 5.4, 852.5-868.9) us; B 864.3 (IQR 11.5, SE 8.9, 845.3-868.4) us; d = median(B-A) +2.05 us (+0.24 %); band 23.04 us | gate: R1 B<=A (inside band); R2 [B<=A (inside band) / B<=A (inside band)]; R3 B<=A (inside band); all originals R1 B<=A (d<=0) (d -4.35 / band 23.04); post hoc i/s/r n/n/n | **B <= A** |
| DM1-VB_PRODUCE_NET-vb-edit100 | as above | per-frame median over 220 frames; A n=4, B n=3, 3 complete pairs | A 301.3 (IQR 67.7, SE 25.6, 259.1-334.8) us; B 263.2 (IQR 28.7, SE 21.8, 250.9-308.2) us; d = median(B-A) -25.60 us (-8.56 %); band 75.78 us | gate: R1 B<=A (d<=0); R2 [B<=A (d<=0) / B<=A (inside band)]; R3 B<=A (d<=0); all originals R1 B<=A (d<=0) (d -8.45 / band 75.78); post hoc i/s/r n/n/n | **B <= A** |

Readings of the DM1 gate:
- R1 (primary): the pairs are (A1,B1) (A2,B2) (A3,B3) (A4,B4) by ABBA position. d is the median over the complete
  pairs. The band is max(range of the A repeats, range of the B repeats).
- R2: window 8's rule verbatim, applied to each ABBA half.
- R3: the pairs as in R1, with the band = the largest within-half |A1-A2|, |B1-B2|, |A3-A4| or |B3-B4|.
- "All originals" is a sensitivity reading that ignores the slot rule.

## Implied decisions

1. **L10 C1b (sleeping ON by default) is supported by its pre-registered bar.**
   - Ruling 8 asks for "no measurable awake cost". The awake cost is NOT CLAIMED at W1 or at W8, disarmed or armed.
   - The product row on the TREE is CLAIMED STRONG faster at W1 and W8, in [100,500) and in [274,500).
   - Nothing in this block argues for moving C1b out of the place ruling 8 gave it: after the pin-moving-free lanes,
     because it moves every pose pin.
2. **Window 8's open point on the sleeping result is closed for the tree.**
   - Window 8 said the settled-pile floor was dominated by the AllPairs broadphase (1.96 / 2.01 ms), which sleeping does
     not reduce, and that the product row was unmeasured.
   - On the tree, the floor is 0.86 % of the OFF step at W1 and 3.46 % at W8.
   - The W8 gain over [100,500) is now -55.24 %. Window 8 had -35.65 % on AllPairs.
3. **The awake-cost bound at W8 is weak.** A cost below ~13 % of T(8) over [0,100) could not have been claimed. The
   armed spans bound the L10 serial pieces directly at 21.16 us/step = 0.50 % of the OFF step. If C1b needs a tighter
   W8 awake bound, it has to come from the spans or from a lower-noise awake window (post hoc; see anomaly 2), not
   from this wall cell.
4. **DM1 C6: KEEP by ruling 9's rule. No revert commit.**
   - VB_EARLY_CULL's B-A did not stay above its band in either row, under R1 or R3, and under the all-originals
     sensitivity reading.
   - The one red R2 half is degenerate (unsure item 9).
   - VB_RUN agrees: -6.14 us idle and +2.05 us inside a 23.04 us band with the edit.
   - Window 8's "B slower in all four pairs" did not reproduce. With the idle row, B is faster in all 3 complete pairs
     under the slot rule and in all 4 pairs with all originals.
5. **The DM1 gate still has no power on VB_SHADE / VB_PRODUCE_NET / GBUF_DEFERRED_RESOLVE.**
   - Their bands are 21.7-25.2 % of the zone, so a C6 regression below that is invisible, as in window 8.
   - The added zones VB_EARLY_CULL and VB_RUN have bands of 1.9-2.9 %. Only there does the ABBA x 2 re-read have the
     power ruling 9 asked for.
   - Unchanged: the grow frame is NOT MEASURED, as ruling 9 says; no non-FIFO present mode exists (PREP); and only
     1920x1080 is measured.

## Anomalies and validity notes

1. **J-Son-T selection.**
   - 85 records: 72 slots plus 13 re-runs. 70 slots used, 2 dropped, 11 re-runs used.
   - Dropped: pass 0 round 0 JSoffT@W8 (original witness 2.73 %, re-run receipt-after 5.51 %) and pass 0 round 0
     JSoffT@W1 (original receipt-after 14.77 %, re-run witness 2.01 %). So both disarmed OFF cells have n = 8, and K = 2
     in pass 0 (unsure item 1).
   - Validity: 0 problems. The independent re-check covers exit, sha256 = bin/SHA256SUMS, pose = fixture and expect
     match, void 0, workers, msvc, the armed flags, TreeDiag = the row's expectation, first_frozen_step = 274 in every
     ON record, an awake column that is zero from CSV row 273 on (ON) and empty (OFF), and the window mean = SUMMARY.
   - Poses: ON 0x3db47fae414b655c, OFF 0x30c5438bc6ad9ffa. TreeDiag: ON has a sleeper rebuild and 1241 members; OFF
     has 1 member.
   - Used receipts: before <= 4.79 %, after <= 4.02 %, witness <= 1.93 %.
2. **W8 awake phase is noisy.**
   - The W8 cells in [0,100) have IQRs of 4.26 / 4.86 % (disarmed) and 6.73 / 2.29 % (armed), against 0.27-0.78 % at
     W1. This is why W8 can resolve nothing below ~13 %; window 8's W8 [0,100) bars were 1.54 / 0.93 % at K = 6.
   - Nothing in the record assigns it. The disarmed per-process values span 3.82-4.18 ms in both the ON and OFF rows. The used
     witnesses are <= 1.88 %. The top CPU is logical 10 in 35 of 35 used W8 processes, with top_share_of_proc
     0.156-0.203.
   - At W1, main_share_top_est = 1.000 in every process that carries it (33 of 35 used W1 processes).
3. **Freeze alignment.**
   - SUMMARY first_frozen_step is 274, and the CSV awake column is 0 from row 273 (CSV row = step, 0-based).
   - stdout says "every dynamic row held from step Some(276)".
   - The pre-registered [274,500) is the driver's slice, CSV rows 274..499, window 8's convention. It therefore
     contains the transitional steps 274-277: W1 medians 1900 / 1771 / 393 / 671 us against ~100-106 us afterwards.
4. **DM1 selection.**
   - 30 processes: 24 originals and 6 re-runs. 22 of 24 slots used.
   - Dropped: seq 8 = vb-idle A4 (witness 2.08 %, re-run 2.10 %) and seq 22 = vb-edit100 B3 (witness 2.10 %, re-run
     receipt-after 5.36 %). Hence vb-idle has A n=3 and vb-edit100 has B n=3, each with 3 complete pairs.
   - 4 re-runs were used: vb-idle A1, deferred B2 and A3, edit100 A2. All ran 20:58:39-20:59:33, after the ABBA
     sequence, so they are not time-adjacent to their pair partners.
   - Validity 0. Present mode fifo everywhere. Zone n = 220 everywhere. One process (seq 14, deferred-idle B3) prints
     frames=240 against 241 for the others; its zones still have n = 220, and it was used.
   - The witness's top process is claude.exe in all 23 unclean J-Son-T + DM1 processes (ruling 11's hygiene point).
5. **DM1 bimodality (as in window 8).**
   - Over all 20 vb processes, re-runs included, the VB_SHADE per-frame medians sit in two states: 249.9-268.3 us
     (7 processes: 5 A, 2 B) and 295.9-335.9 us (13 processes: 6 A, 7 B).
   - GBUF_DEFERRED_RESOLVE B sits at 440.3-445.4 us (3 processes) or 569.9-570.4 us (2), while A sits at 592.4-594.9 us
     (5). The per-zone detail is in states.txt.
   - The state a process lands in decides the sign of d on these zones. VB_SHADE idle reads -23.04 us under the slot
     rule and +43.52 us (inside a 72.70 us band) with all originals. In the all-originals reading, ABBA half 1 of
     vb-idle is also red for VB_SHADE / VB_PRODUCE_NET under R2: +47.10 against a 3.58 band.
   - VB_EARLY_CULL and VB_RUN do not show the two states. Over all 20 processes they span only 774.6-800.0 us and
     843.8-869.9 us.

## Post hoc observations (none is a pre-registered claim)

- **Whole window [0,500), disarmed:** -44.00 % at W1 and -42.94 % at W8, both CLAIMED STRONG by the same rule.
- **Awake after step 100, [100,274):** -0.61 % at W1 and -1.31 % at W8, NOT CLAIMED. The smallest claimable effects
  are 2.66 % and 17.66 %. The armed twins read -0.30 % and -3.06 %. So no awake cost resolves in either awake window.
- **L10 serial pieces (armed, JSonT-a):** sleep_begin + freeze + end + classify.
  - [0,100): 19.60 us/step at W1 (0.123 % of the OFF step) and 21.16 us/step at W8 (0.501 %).
  - On the frozen pile [274,500): 25.16 us at W1 and 27.01 us at W8. The largest piece is sleep_classify, at
    19.1 / 20.2 us.
- **Steady hold [277,500):** ON costs 0.1085 ms at W1 and 0.1177 ms at W8 (disarmed), against 14.6148 / 3.8608 ms OFF
  (-99.26 % / -96.95 %).
  - Armed composition at W1 (us/step): wall 111.2 = bp 35.9, gather 28.1, g 23.5, sleep_classify 19.1, solve 12.7,
    graph 8.7, sleep_end 4.1, u 2.7, sleep_begin 2.0, apply 1.5, np 0.2.
  - At W8: wall 117.7, with bp 38.1, gather 29.8, g 24.0, classify 20.2, solve 13.9, graph 9.5.
  - Counters on [274,500), OFF -> ON: bp queried rows 1240 -> 16.46 per step; np pairs 9545 -> 126.7; held rows
    1234.5; waves 96 -> 0; colours 11 -> 0.049.
  - The W8 floor sits above W1 (0.1337 vs 0.1254 ms on [274,500)). This was not tested.
- **Against the L10 design rev 2 prediction** (a reference, not a gate of this block): 04-DESIGN-REV2.md "Expected
  gain" predicted the C3c J-Son tail floor at 0.11-0.23 ms (W1) and 0.08-0.18 ms (W8), on [264,1000) in the P0 era.
  The measured 0.1254 / 0.1337 ms lie inside both brackets.
- **Per-manifold normalisation changes nothing.** The ON and OFF manifold counts agree: 4467.00 vs 4468.11 on
  [274,500), a ratio of 1.00025, and they are identical over [0,100) and [100,274).
- **Pose.** The tree ON pose 0x3db47fae414b655c equals window 8's AllPairs J-Son pose (win8/analysis.md section 7). The
  OFF pose equals the JT500/JA500 fixture. Pose is broadphase-independent on this scene.
- **DM1 post hoc statistics.**
  - Per-frame means: VB_EARLY_CULL idle -26.25 us (-3.34 %); edit100 +0.57 us, inside a 41.84 us band.
  - Frame span (the largest end_off_ns): vb idle -16.22 us (-1.30 %); edit100 -3.39 us (-0.27 %). Both B <= A.
  - With all originals, VB_EARLY_CULL idle is -16.83 us (-2.12 %), and B is faster in 4 of 4 pairs (i/s/r Y/Y/n on the
    K 4/4 cells).
  - GBUF_DEFERRED_RESOLVE: -87.81 us (-14.78 %) under the slot rule. It is driven by B's low state (anomaly 5), not a
    resolved gain.

## Where the pre-registered rule was ambiguous (how it was applied)

1. **K = 2 per-pass cells.**
   - Pass 0 of JSoffT at W1 and W8 has K = 2 after the dropped slots. Ruling 1's text and window 8's written
     definitions of i/s/r compute at K = 2, and the analysis applies them. Window 8's lib8.cmp_ refuses any cell with
     K < 3.
   - Under that guard, JST-G-W1/W8 and JST-F-W1/W8 (disarmed) become NOT CLAIMED on the guard alone. Their pass-0 bars
     clear by a wide margin: i <= 7.56 %, s <= 6.50 % against effects of 52.7-99.2 %.
   - The armed twins, K = 3 everywhere, are CLAIMED STRONG under both readings. The C1b conclusion does not depend on
     the choice.
2. **"Every clean block"** is read as every pass. 0 passes were voided, so all 3 count.
3. **STRONG** requires r to clear in every pass and pooled, not only pooled. A claim also requires one sign across the
   passes and the pooled cell.
4. **Primary row.** The disarmed rows are primary, as in window 8, and the armed rows are confirmation. The rows file
   pre-registers all four without naming a primary. They agree on every verdict.
5. **C1b's bar.**
   - The listed sources give no numeric bar beyond ruling 1's rule and ruling 8's "no measurable awake cost". The
     bar applied: [0,100) not claimed slower, plus [100,500) and [274,500) claimed faster.
   - The L10 design rev 2 carries other lines: a realised-gain gate >= 0.6 x the lower predicted delta, which is for
     C3c Off-vs-Sets, a different comparator; and "J parent vs commit, K = 12, not claimed slower" for C1b's lane.
     These are not this block's sources and were not applied.
   - "Not claimed" at W8 means "below ~13 %", not "absent".
6. **Window indexing.** [274,500) is taken as CSV rows 274..499 (the driver's col_stats slice, window 8's
   convention). It is not the rows from the first zero-awake row (273), and not the rows from the full hold (276).
   [277,500) is reported post hoc.
7. **DM1 ABBA x 2.** Ruling 9 says "ABBA x 2 per row" but does not extend the pairing or the band. R1 is primary: the
   paired median over all ABBA positions against the range of the repeats. R2 and R3 are reported, and every one of
   them keeps C6.
8. **DM1 re-runs.** Re-runs fill ABBA slots out of time order, because they ran after the sequence. The protocol's
   slot rule was used, and the all-originals sensitivity reading gives the same KEEP.
9. **The R2 red half (edit100 VB_EARLY_CULL, ABBA2) is degenerate.** That half lost B3, so its d is one pair (+2.42 us)
   and its band is |A3-A4| alone (2.02 us). "Stays above its band" was judged on R1/R3. R2 is not red in both halves.
   Window 8's red was above the band in both rows.
10. **DM1 "per frame"** is taken as the artifact's per-frame statistics: the median over 220 frames is the gated
    number, and the mean is post hoc. No per-frame record exists, and the grow frame is NOT MEASURED. The frame span
    (the largest end_off_ns) is reported post hoc.
11. **Ruling 1's rule does not govern DM1.** The protocol's own_k gives DM1 ruling 9's ABBA x 2, so i/s/r there is
    post hoc only.

## Scripts and outputs (analysis/jsondm1/)

- libjd.py: selection, validity, statistics. Its definitions are copied from win8/analysis/lib8.py.
- jsont.py -> jsont.txt, jsont.json, jsont_procs.json.
- jsont_post.py -> jsont_post.txt.
- dm1.py -> dm1.txt, dm1.json.
- states.py -> states.txt.
- checks.py -> checks.txt.
- summary.py -> summary.txt.
- table_md.py -> table_md.txt (the verdict table above).
