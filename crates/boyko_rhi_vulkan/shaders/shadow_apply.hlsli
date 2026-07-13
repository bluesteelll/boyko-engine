// Combined shadow-source apply (`shadow_apply.hlsli`) — Multi-Paradigm Render-Path
// Decision 7 (resolves W2): SCAFFOLD ONLY, no code yet.
//
// FUTURE ROLE (not implemented this rung): shadow at a shade site is multi-source —
// CSM PCF + punctual atlas + SDF soft-march + optional HW-RT vis — exactly as
// `deferred_pbr.hlsl` combines them today inline. This header will model the armed
// sources as `ShadowSources` bitflags (not a single mode) and combine whichever are
// bound at a given shade site into ONE `vis` value fed into `eval_pbr_direct`
// (`pbr_lighting.hlsli`), keeping the BRDF itself single-source. It restores
// Deferred's default non-hwrt SDF soft shadow to Forward/ForwardPlus/VisibilityBuffer.
//
// Planned includes (per the plan's shader inventory, §C): `light_table.hlsli`,
// `sdf_soft_shadow_ranged` (eDSL-generated). Planned consumers: `forward_opaque.fs.hlsl`,
// `vb_resolve.hlsl`/`vb_shade.hlsl`, `sdf_forward_march.hlsl`/`sdf_shade.hlsl`.
//
// Not `#include`d anywhere yet — this file has no effect on any compiled shader until a
// later rung populates it.
