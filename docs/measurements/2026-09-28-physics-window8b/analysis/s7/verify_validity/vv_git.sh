#!/bin/sh
# Read-only provenance of P (16191fda) against the cut's P (54a7714f) and of T (a3adc827); no checkout, no edit.
cd D:/claude/BoykoEngine || exit 1
echo "== git diff --stat 54a7714f 16191fda"; git diff --stat 54a7714f 16191fda
echo "== git diff --stat 16191fda a3adc827"; git diff --stat 16191fda a3adc827
git merge-base --is-ancestor 54a7714f 16191fda && echo "54a7714f is an ancestor of 16191fda"
git merge-base --is-ancestor 16191fda a3adc827 && echo "16191fda is an ancestor of a3adc827"
