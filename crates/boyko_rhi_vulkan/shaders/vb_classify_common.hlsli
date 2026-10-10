// VB-P2 classification plan (docs/VB-P2-CLASSIFICATION-PLAN.md), rung P2a (dark infra,
// unwired — see that doc's rung table). The packed `gClassify` byte-address buffer shared by
// the classify/shade compute family (`vb_classify_count/scan/scatter.comp.hlsl`,
// `vb_shade.comp.hlsl`) — Set 0 binding 7 on the SAME `vb_layout0` every other VB Set-0
// consumer binds (`GBufferScene::vb_layout0`'s doc; P2a adds this binding to that ONE shared
// layout object, R5's "no structurally-identical-but-distinct layout" rule).
//
// # HOST<->SHADER SYNC-PIN — the exact `gClassify` sub-region WORD-offset formula
//
// `RWByteAddressBuffer.Load`/`.Store` address BYTES; every accessor below multiplies a WORD
// offset by 4 to get the byte offset. Layout (word = 4 bytes):
//
//   [ counts(MAX) | offsets(MAX) | cursors(MAX) | gbase(MAX) | group_to_mat(G+MAX) | pixel_list(w*h) ]
//
//   MAX = VB_MAX_MATERIAL_ROWS = 65536, mirroring `boyko_render::material_table::
//   MAX_MATERIAL_ROWS` (the plan's P1-2: the M-arrays are pre-sized to the material system's
//   hard 16-bit `MaterialId` addressing cap so this sub-region layout is FIXED and never
//   invalidated by `MaterialTable` growth, the F7 rebind class). This crate cannot depend on
//   `boyko_render` (which sits ABOVE it in the dependency graph — the SAME plain-value
//   boundary crossing `vb_geom_fetch.hlsli`'s Set-numbering doc explains for other host
//   mirrors), so the constant is re-declared here verbatim (the host-side mirror lives in
//   `boyko_rhi_vulkan::present::targets::VB_CLASSIFY_MAX_MATERIAL_ROWS`).
//
//   G = ceil(w*h / 64) — the SAME per-pixel dispatch-group count
//   `GpuSceneBundles::dispatch_group_count_x` computes host-side (`LOCAL_SIZE_X` = 64, the
//   `vb_classify_count`/`vb_classify_scatter`/`vb_shade` group size). Recomputed in-shader from
//   `w`/`h` (the caller's own `img_w_raw`/`img_h_raw` Camera-UBO read, threaded in as plain
//   parameters — see `cls_group_count`'s own doc) rather than threaded as a SEPARATE push
//   constant, so host and shader can never desync on G's value.
//
//   `group_to_mat`'s reserved CAPACITY is `G + MAX` (NOT `G + present_material_count`, the
//   per-frame LIVE length the pipeline's D2 over-dispatch actually walks) — pre-sizing to the
//   material system's hard cap, exactly like the M-arrays, keeps EVERY offset at or past
//   `group_to_mat_off` FIXED across every frame regardless of how many materials are live this
//   frame. This is a P2a design decision (documented, not spelled out verbatim by the plan
//   text): `vb_shade`'s push constant carries only the 64-byte `view_proj` (D3 — its shading
//   tail is character-identical to `vb_resolve.comp.hlsl`'s own, which never declared a
//   `material_count` field), so `cls_pixel`'s offset must be computable from `w`/`h` ALONE, with
//   NO per-frame `material_count` input available at the call site. The cost of the `MAX`-sized
//   (vs a tight `present_material_count`-sized) reservation is `(MAX - present_material_count) *
//   4` bytes of never-addressed tail capacity — at most `MAX * 4` = 256 KiB per FIF, negligible
//   next to `pixel_list`'s own `w*h*4` bytes (~8 MiB at 1080p; P1-2's own "~1 MB total,
//   negligible" precedent for the M-arrays extends cleanly to this region too).
//
//   counts_off        = 0
//   offsets_off       = MAX
//   cursors_off       = 2*MAX
//   gbase_off         = 3*MAX
//   group_to_mat_off  = 4*MAX
//   pixel_list_off    = 4*MAX + (G + MAX) = 5*MAX + G      -- FIXED per extent, not per frame
//
// `present_material_count` (the plan's D2 — the frame's LIVE distinct material-id count, a
// push constant in `vb_classify_scan.comp.hlsl` ONLY) is exclusively a LOOP BOUND (how many
// `[0, material_count)` M-array rows the scan pass folds) — it never participates in any
// `gClassify` OFFSET computation in this file.
//
// # INCLUDE CONTRACT
//
// Self-contained: declares its OWN Set-0 binding 7 (`gClassify`) and needs nothing
// pre-declared by the caller. Every accessor that needs `w`/`h` (`cls_group_count`,
// `cls_pixel`, `cls_pixel_store`) takes them as PLAIN PARAMETERS (the `vb_geom_fetch.hlsli`
// `extent`-threading precedent) rather than reading an ambient `cbuffer Camera` — this file
// therefore does NOT declare a `Camera` binding, so it can be `#include`d by a shader that
// never binds Camera at all (`vb_classify_scan.comp.hlsl`, which touches no `w`/`h`-dependent
// region).

/// The material system's hard 16-bit addressing cap (mirrors
/// `boyko_render::material_table::MAX_MATERIAL_ROWS` — see this file's header sync-pin).
static const uint VB_MAX_MATERIAL_ROWS = 65536u;

/// `group_to_mat[g] == VB_GROUP_SENTINEL` marks a dispatch group past `total_groups` (the
/// `fill` pass's `vkCmdFillBuffer` sentinel-fill, P1-1) — `vb_shade` returns immediately on
/// this value. Host mirror: `boyko_rhi_vulkan::present::targets::VB_GROUP_SENTINEL`.
static const uint VB_GROUP_SENTINEL = 0xFFFFFFFFu;

static const uint VB_CLS_COUNTS_OFF = 0u;
static const uint VB_CLS_OFFSETS_OFF = VB_MAX_MATERIAL_ROWS;
static const uint VB_CLS_CURSORS_OFF = 2u * VB_MAX_MATERIAL_ROWS;
static const uint VB_CLS_GBASE_OFF = 3u * VB_MAX_MATERIAL_ROWS;
static const uint VB_CLS_GROUP_TO_MAT_OFF = 4u * VB_MAX_MATERIAL_ROWS;

// binding 7: the packed classify buffer (Set 0, `vb_layout0`).
[[vk::binding(7, 0)]] RWByteAddressBuffer gClassify;

/// `G = ceil(w*h / 64)` — the per-pixel dispatch-group count (see this file's header sync-pin
/// for why this is recomputed here rather than threaded as a push constant).
uint cls_group_count(uint w, uint h) {
    return (w * h + 63u) / 64u;
}

/// `pixel_list`'s WORD offset — the only region whose offset depends on the extent (via `G`).
uint cls_pixel_list_word_off(uint w, uint h) {
    return 5u * VB_MAX_MATERIAL_ROWS + cls_group_count(w, h);
}

/// Reads `counts[mat]` (the `count` pass's per-material pixel tally).
uint cls_count(uint mat) {
    return gClassify.Load(VB_CLS_COUNTS_OFF * 4u + mat * 4u);
}

/// `InterlockedAdd(counts[mat], 1)` — the `count` pass's per-pixel contribution.
void cls_count_add(uint mat) {
    uint prev;
    gClassify.InterlockedAdd(VB_CLS_COUNTS_OFF * 4u + mat * 4u, 1u, prev);
}

/// Reads `offsets[mat]` (material `mat`'s `pixel_list` region START, written by `scan`).
uint cls_offset(uint mat) {
    return gClassify.Load(VB_CLS_OFFSETS_OFF * 4u + mat * 4u);
}

/// Writes `offsets[mat]` (the `scan` pass's Phase-1 exclusive-prefix-sum output).
void cls_offset_store(uint mat, uint value) {
    gClassify.Store(VB_CLS_OFFSETS_OFF * 4u + mat * 4u, value);
}

/// Seeds `cursors[mat] = offsets[mat]` (the `scan` pass's Phase-1 write; `scatter`'s per-pixel
/// atomics then walk it forward).
void cls_cursor_store(uint mat, uint value) {
    gClassify.Store(VB_CLS_CURSORS_OFF * 4u + mat * 4u, value);
}

/// `InterlockedAdd(cursors[mat], 1)`, returning the PRE-increment value — the `scatter` pass's
/// per-pixel claimed slot (a `pixel_list` write index, relative to `offsets[mat]`).
uint cls_scatter(uint mat) {
    uint prev;
    gClassify.InterlockedAdd(VB_CLS_CURSORS_OFF * 4u + mat * 4u, 1u, prev);
    return prev;
}

/// Reads `gbase[mat]` (material `mat`'s first `group_to_mat` row index, written by `scan`).
uint cls_gbase(uint mat) {
    return gClassify.Load(VB_CLS_GBASE_OFF * 4u + mat * 4u);
}

/// Writes `gbase[mat]` (the `scan` pass's Phase-2 exclusive-prefix-sum output over
/// `ceil(counts[mat]/64)`).
void cls_gbase_store(uint mat, uint value) {
    gClassify.Store(VB_CLS_GBASE_OFF * 4u + mat * 4u, value);
}

/// Reads `group_to_mat[g]` — the material id `vb_shade`'s group `g` shades (or
/// `VB_GROUP_SENTINEL` past `total_groups`).
uint cls_g2m(uint g) {
    return gClassify.Load(VB_CLS_GROUP_TO_MAT_OFF * 4u + g * 4u);
}

/// Writes `group_to_mat[g] = mat` (the `scan` pass's Phase-2 fill; NEVER touches the
/// `fill`-owned SENTINEL tail past `total_groups`, P1-1).
void cls_g2m_store(uint g, uint mat) {
    gClassify.Store(VB_CLS_GROUP_TO_MAT_OFF * 4u + g * 4u, mat);
}

/// Reads `pixel_list[idx]` (a linear `py*w+px` pixel index) — `idx` is the CALLER's own
/// `offsets[mat] + slot` sum (mirrors `vb_shade.comp.hlsl`'s prologue).
uint cls_pixel(uint idx, uint w, uint h) {
    return gClassify.Load(cls_pixel_list_word_off(w, h) * 4u + idx * 4u);
}

/// Writes `pixel_list[idx] = value` (the `scatter` pass's per-pixel linear-index store).
void cls_pixel_store(uint idx, uint value, uint w, uint h) {
    gClassify.Store(cls_pixel_list_word_off(w, h) * 4u + idx * 4u, value);
}
