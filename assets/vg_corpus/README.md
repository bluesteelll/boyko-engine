# `assets/vg_corpus/` — the VG-R0 density-census corpus

**Tracked here: `CORPUS.toml` and this file. Everything else is gitignored payload.**

The rule in `.gitignore` (`/assets/vg_corpus/*` with `!CORPUS.toml` / `!README.md` escapes) mirrors
the `assets/materials/` convention exactly. A high-poly corpus is not small by any definition that
keeps this repo cloneable, there is no `.gitattributes` so Git LFS is not configured, and git
history is immutable — a corpus committed once is carried by every clone forever.

## Getting the payload

```powershell
scripts\fetch_corpus.ps1
```

It reads `CORPUS.toml`, downloads each `source_url`, verifies the archive against `archive_sha256`
**before** extracting, extracts, and verifies each extracted `.glb` against `glb_sha256`. A hash
mismatch is a hard stop, never a warning: the pin is what makes "the corpus" a fixed object rather
than whatever a URL served today.

## What is checked, and what is not

`crates/boyko_app/tests/vg_corpus_ingest.rs` is the ingest gate. Two of its parts read only tracked
files and therefore run on **every** checkout, payload or no payload:

* **(a0)** — every owner VALUES call listed in `[gating].r0b_blocked_by` resolves and is answered.
* **(e)** — the manifest enumerates at least `[k1].committed_paths_min` distinct camera-path ids.

The other four need the payload and **skip by a recorded policy** when it is absent, naming what was
not run rather than passing silently. That asymmetry is deliberate: the payload is gitignored, so
the branch every fresh checkout takes must still assert whatever can be asserted from tracked files.

**Not checked anywhere:** that the committed camera paths are *representative* of the content class,
and that a path's *definition* (a test constant) has not been re-aimed — no digest in R0 hashes
those. Both are recorded in the plan's §9.1 as residuals rather than reasoned away.
