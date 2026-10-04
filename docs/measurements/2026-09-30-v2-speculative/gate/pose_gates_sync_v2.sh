#!/usr/bin/env bash
# V2 (C6): generated from l10/pose_gates_sync.sh by make_v2_scripts.py - the fixtures and pinned
# hashes are the V2 recordings (D:/wt/merge/docs/measurements/2026-09-30-v2-speculative).
# L10 pose gates G1-G3 on the tree synced with L9 C4 (contact reuse on by default), untimed.
# Derived from the lane's pose_gates_c3c.sh: every row runs twice, flagless (the default, reuse
# on) against the reuse-on fixtures, and with --contact-reuse off against the pre-C4 fixtures.
# G3's sleeping-on rows run under --sleep-skip {unset, off, sets} x --broadphase {unset, tree,
# grid} in both arms. G4: window 6's 1000/800-step sleeping rows (item 1) in both arms and every
# sleep-skip mode, and its Off' rows (explicit --contact-reuse on), which must not move.
# usage: pose_gates_sync.sh <exe> <outdir>
set -u
EXE=$1
OUT=$2
SON="--sleeping on"
L9=D:/wt/merge/docs/measurements/2026-09-30-v2-speculative/l9-reuse-off
L9C4=D:/wt/merge/docs/measurements/2026-09-30-v2-speculative/l9-reuse-on
L10=D:/wt/merge/docs/measurements/2026-09-30-v2-speculative/l10-reuse-on
L10OFF=D:/wt/merge/docs/measurements/2026-09-30-v2-speculative/l10-reuse-off
W6=D:/wt/merge/docs/measurements/2026-09-30-v2-speculative/w6
mkdir -p "$OUT"
T="$OUT/pose_gates.tsv"
: > "$T"
chk() { # label W expected-hash-or-dash flags...
  local label=$1 w=$2 want=$3; shift 3
  "$EXE" "$@" --workers "$w" > "$OUT/${label}_W${w}.out" 2>&1
  local rc=$?
  local h; h=$(grep -o 'pose_hash [0-9a-fx]*' "$OUT/${label}_W${w}.out" | awk '{print $2}')
  local m; m=$(grep -o '"expect_pose":"[a-z]*"' "$OUT/${label}_W${w}.out" | cut -d'"' -f4)
  local verdict=PASS
  if [ "$rc" != 0 ]; then verdict=FAIL; fi
  if [ "$want" != - ] && [ "$h" != "$want" ]; then verdict=FAIL; fi
  printf '%s\tW%s\texit=%s\t%s\t%s\t%s\t%s\n' "$label" "$w" "$rc" "$h" "${m:-none}" "$verdict" "$*" >> "$T"
}
for w in 1 8; do
  if [ "$w" = 8 ]; then PS=--parallel-solve; else PS=; fi
  for arm in on off; do
    if [ "$arm" = on ]; then RU=; J=0x441a568e91a4f9c9; F9=$L9C4; F10=$L10; else RU="--contact-reuse off"; J=0xbc09a1faf7b413a8; F9=$L9; F10=$L10OFF; fi
    # G1: the J pose, 500 steps, cfg default/as x allpairs/tree.
    for c in default as; do for bp in allpairs tree; do
      chk "G1_${arm}_J500_${c}_${bp}" "$w" $J --scene jolt --gap 0.5 --cfg $c --broadphase $bp --steps 500 $RU
    done; done
    # G2: L9's C0-family rows, 600 steps.
    chk G2_${arm}_J-A "$w" - --scene jolt --gap 0.5 --cfg a --steps 600 $RU --expect-pose $F9/J-A_W$w.pose
    chk G2_${arm}_J-D "$w" - --scene jolt --gap 0.5 --cfg default --steps 600 $RU --expect-pose $F9/J-D_W$w.pose
    chk G2_${arm}_R "$w" - --scene rest --solver colored $PS --steps 600 $RU --expect-pose $F9/R_W$w.pose
    chk G2_${arm}_R-S "$w" - --scene rest $SON --steps 600 $RU --expect-pose $F9/R-S_W$w.pose
    chk G2_${arm}_J-Son "$w" - --scene jolt --gap 0.5 --cfg a $SON --steps 600 $RU --expect-pose $F9/J-Son_W$w.pose
    # G3: the L10 lane fixtures, 600 steps.
    chk G3_${arm}_R "$w" - --scene rest --solver colored $PS --steps 600 $RU --expect-pose $F10/R_W$w.pose
    for bp in "" tree grid; do
      if [ -z "$bp" ]; then BP=; BTAG=; else BP="--broadphase $bp"; BTAG="_bp-$bp"; fi
      for skip in "" off sets; do
        if [ -z "$skip" ]; then SK=; TAG=; else SK="--sleep-skip $skip"; TAG="_skip-$skip"; fi
        chk G3_${arm}_R-S$TAG$BTAG "$w" - --scene rest $SON $SK $BP --steps 600 $RU --expect-pose $F10/R-S_W$w.pose
        chk G3_${arm}_J-Son$TAG$BTAG "$w" - --scene jolt --gap 0.5 --cfg a $SON $SK $BP --steps 600 $RU --expect-pose $F10/J-Son_W$w.pose
        chk G3_${arm}_J-A-on$TAG$BTAG "$w" - --scene jolt --gap 0.5 --cfg a $SON $SK $BP --steps 600 $RU --expect-pose $F10/J-A-on_W$w.pose
        chk G3_${arm}_R-on$TAG$BTAG "$w" - --scene rest --solver colored $PS $SON $SK $BP --steps 600 $RU --expect-pose $F10/R-on_W$w.pose
        chk G3_${arm}_J-D-on$TAG$BTAG "$w" - --scene jolt --gap 0.5 --cfg default $SON $SK $BP --steps 600 $RU --expect-pose $F10/J-D-on_W$w.pose
      done
    done
    # G4: window 6 item 1 (Son-J1000 / RS800), every sleep-skip mode, both arms.
    if [ "$arm" = on ]; then HS=0x130c76cb6b463ab8; HR=0xfcf3d49356b45a08; else HS=0xc671831894394df6; HR=0xe99ed04348f5d898; fi
    for skip in "" off sets; do
      if [ -z "$skip" ]; then SK=; TAG=; else SK="--sleep-skip $skip"; TAG="_skip-$skip"; fi
      chk G4_${arm}_Son-J1000$TAG "$w" $HS --scene jolt --gap 0.5 --cfg a --sleeping $SK --steps 1000 $RU
      chk G4_${arm}_RS800$TAG "$w" $HR --scene rest --sleeping $SK --steps 800 $RU
    done
  done
  # G4': window 6's Off' rows (explicit --contact-reuse on, tree): must not move.
  for skip in "" off sets; do
    if [ -z "$skip" ]; then SK=; TAG=; else SK="--sleep-skip $skip"; TAG="_skip-$skip"; fi
    chk G4p_Offp-J1000$TAG "$w" 0x130c76cb6b463ab8 --scene jolt --gap 0.5 --cfg a --sleeping --broadphase tree --contact-reuse on $SK --steps 1000 --expect-pose $W6/Offp-J1000.pose
    chk G4p_RSOffp800$TAG "$w" 0xfcf3d49356b45a08 --scene rest --sleeping --broadphase tree --contact-reuse on $SK --steps 800 --expect-pose $W6/RSOffp800.pose
  done
  # G4 fixtures (window 6's files) for the flagless and reuse-off 1000/800-step rows.
  chk G4f_on_Son-J1000 "$w" 0x130c76cb6b463ab8 --scene jolt --gap 0.5 --cfg a --sleeping --steps 1000 --expect-pose $W6/Offp-J1000.pose
  chk G4f_off_Son-J1000 "$w" 0xc671831894394df6 --scene jolt --gap 0.5 --cfg a --sleeping --steps 1000 --contact-reuse off --expect-pose $W6/Son-J1000.pose
  chk G4f_on_RS800 "$w" 0xfcf3d49356b45a08 --scene rest --sleeping --steps 800 --expect-pose $W6/RSOffp800.pose
  chk G4f_off_RS800 "$w" 0xe99ed04348f5d898 --scene rest --sleeping --steps 800 --contact-reuse off --expect-pose $W6/RS800.pose
done
# Negative controls: the flagless row against its reuse-off file, and the reverse, must exit 4.
"$EXE" --scene rest --sleeping --workers 1 --steps 600 --expect-pose $L10OFF/R-S_W1.pose > "$OUT/neg_flagless_vs_off.out" 2>&1; echo "neg flagless R-S W1 vs fixtures-reuse-off: exit=$? (want 4)" | tee "$OUT/neg.txt"
"$EXE" --scene rest --sleeping --contact-reuse off --workers 1 --steps 600 --expect-pose $L10/R-S_W1.pose > "$OUT/neg_off_vs_flagless.out" 2>&1; echo "neg reuse-off R-S W1 vs fixtures: exit=$? (want 4)" | tee -a "$OUT/neg.txt"
echo "runs $(wc -l < "$T"), pass $(grep -c 'PASS' "$T"), fail $(grep -c 'FAIL' "$T")"
