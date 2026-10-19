//! The render-wave PXe emitters: the `post_final` generator (the traced
//! [`crate::post::post_final_main`] body, the declarations and constants printed from the
//! single-source tables of [`crate::post`], and the whole-file template) and the per-facet probes
//! that pin each resource facet's HLSL spelling.
//!
//! `use super::*` surfaces the private recorder state and printer plumbing — the same registration
//! shape as the [`super::depth`] / [`super::prims`] siblings. The texture handles name their
//! resources through the printer's existing `Names::res_in` table (the Increment-5c resource
//! table), so no printer name table is widened.

use super::*;

use crate::cf::Cf;
use crate::post::{self, PostBindingKind, PostParams, PostRes, PostUboKind};
use crate::scalar::FieldScalar;

/// The shared reset/seed/pop/print harness (the `emit_depth_leaf` shape): clears the recorder,
/// seeds the function-body block, runs `body` and prints the recorded block at depth 1 against the
/// given parameter-name tables (the named-literal and mutable-local tables are the ones the trace
/// interned).
fn emit_post_leaf<F: FnOnce()>(
    float_in: &[&str],
    uint_in: &[&str],
    vec_in: &[&str],
    vec4_in: &[&str],
    res_in: &[&str],
    body: F,
) -> String {
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    VAR_TYPES.with(|t| t.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    CALLS.with(|c| c.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    body();

    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    let named_lit = NAMED_LITS.with(|n| n.borrow().clone());
    let vars = VARS.with(|v| v.borrow().clone());
    let names = Names {
        float_in,
        uint_in,
        vec_in,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: &named_lit,
        vars: &vars,
        vec4_in,
        call_in: NO_CALL_INPUTS,
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut span = String::new();
        print_block(&body_block, &arena, names, 1, &mut span);
        span
    })
}

/// One resource facet of the post body, for its emit pin ([`emit_hlsl_post_facet`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostFacet {
    /// `Cf::tex2_load` over a `Load`-only `gTex` at `(int)px`.
    Tex2Load,
    /// `Cf::tex2_sample_linear` over `gTex` + `gLinearClamp` at `uv`.
    Tex2SampleLinear,
    /// `Cf::tex3_sample_linear` over `gLut` + `gLinearClamp` at `uvw`.
    Tex3SampleLinear,
    /// `Cf::vec4_xyz` of a `float4 v`, materialised and returned with alpha 1.
    Vec4Xyz,
}

/// The probe's resource-name table: `res_in[0] = gTex`, `[1] = gLinearClamp`, `[2] = gLut`.
const FACET_RES: &[&str] = &["gTex", "gLinearClamp", "gLut"];

/// Traces one facet in a one-statement probe body and returns the printed span — the facet's
/// HLSL spelling, pinned by `boyko_shaderdsl/tests/post_facets.rs`.
pub fn emit_hlsl_post_facet(facet: PostFacet) -> String {
    emit_post_leaf(&[], &["px.x", "px.y"], &["uv", "uvw"], &["v"], FACET_RES, || {
        let ret = RetCellV4;
        let value = match facet {
            PostFacet::Tex2Load => {
                let t = TexParam { tex: 0, smp: TexParam::NO_SAMPLER };
                let x = EmitCf::int_from_uint(Emit::uint_input(0));
                let y = EmitCf::int_from_uint(Emit::uint_input(1));
                EmitCf::tex2_load(t, x, y)
            }
            PostFacet::Tex2SampleLinear => {
                let t = TexParam { tex: 0, smp: 1 };
                EmitCf::tex2_sample_linear(t, Emit(push(Node::Vec2Param(0))))
            }
            PostFacet::Tex3SampleLinear => {
                let t = TexParam { tex: 2, smp: 1 };
                EmitCf::tex3_sample_linear(t, Emit(push(Node::Vec3Param(1))))
            }
            PostFacet::Vec4Xyz => {
                let rgb = EmitCf::temp_vec3("rgb", EmitCf::vec4_xyz(Emit(push(Node::Vec4Param(0)))));
                EmitCf::vec4_from_scalars(
                    EmitCf::vec3_x(rgb),
                    EmitCf::vec3_y(rgb),
                    EmitCf::vec3_z(rgb),
                    <Emit as FieldScalar>::lit(1.0),
                )
            }
        };
        let _ = EmitCf::ret_vec4(&ret, value);
    })
}

// ---- The `post_final` generator --------------------------------------------------------------------

/// The `float` / `uint` input-name tables of the traced body, built from the UBO layout (each
/// `PostFinal` member is read by its bare name on Emit; a `uint2` member as `<name>.x` / `<name>.y`),
/// with the pixel coordinate `px.x` / `px.y` first in the `uint` table.
fn post_input_names() -> (Vec<String>, Vec<String>) {
    let mut floats = Vec::new();
    let mut uints = vec!["px.x".to_string(), "px.y".to_string()];
    for f in post::POST_FINAL_UBO_LAYOUT {
        match f.kind {
            PostUboKind::U32 => uints.push(f.name.to_string()),
            PostUboKind::F32 => floats.push(f.name.to_string()),
            PostUboKind::Uint2 => {
                uints.push(format!("{}.x", f.name));
                uints.push(format!("{}.y", f.name));
            }
        }
    }
    (floats, uints)
}

/// The index of `name` in an input-name table (a generator-input invariant: the body reads only
/// declared members).
fn input_id(table: &[String], name: &str) -> u32 {
    table
        .iter()
        .position(|n| n == name)
        .unwrap_or_else(|| panic!("invariant: `{name}` is a POST_FINAL_UBO_LAYOUT member")) as u32
}

/// The `res_in` names of the fetching binding rows, in table order.
fn post_resource_names() -> Vec<&'static str> {
    post::POST_FINAL_BINDINGS
        .iter()
        .filter(|b| {
            matches!(b.kind, PostBindingKind::Tex2Load | PostBindingKind::Tex2Linear | PostBindingKind::Tex3Linear)
        })
        .map(|b| b.name)
        .collect()
}

/// The texture handle of the binding row `name` (a `Load`-only texture at PXe).
fn post_texture(res: &[&str], name: &str) -> TexParam {
    let tex = res
        .iter()
        .position(|n| *n == name)
        .unwrap_or_else(|| panic!("invariant: `{name}` is a texture row of POST_FINAL_BINDINGS")) as u32;
    TexParam { tex, smp: TexParam::NO_SAMPLER }
}

/// The symbols the traced body interned, each with its value spelled for a `static const`
/// (`(symbol, hlsl type, value text)`, interning order) — read back from the recorded nodes, so the
/// constants span declares exactly what the body references.
fn traced_constants(arena: &[Node], symbols: &[&'static str]) -> Vec<(&'static str, &'static str, String)> {
    symbols
        .iter()
        .enumerate()
        .map(|(id, &sym)| {
            let id = id as u32;
            let node = arena
                .iter()
                .find(|n| matches!(n, Node::NamedLit { sym_id, .. } | Node::NamedUint { sym_id, .. } if *sym_id == id))
                .unwrap_or_else(|| panic!("invariant: the interned symbol `{sym}` has a node"));
            match *node {
                Node::NamedUint { val, .. } if val > 255 => (sym, "uint", format!("{val:#X}u")),
                Node::NamedUint { val, .. } => (sym, "uint", format!("{val}u")),
                Node::NamedLit { val, .. } => (sym, "float", fmt_lit(val)),
                _ => unreachable!("matched a NamedLit / NamedUint above"),
            }
        })
        .collect()
}

/// Traces [`crate::post::post_final_main`] over [`EmitCf`] and returns `(body span, constants)`.
fn trace_post_final_main() -> (String, Vec<(&'static str, &'static str, String)>) {
    let (floats, uints) = post_input_names();
    let float_in: Vec<&str> = floats.iter().map(String::as_str).collect();
    let uint_in: Vec<&str> = uints.iter().map(String::as_str).collect();
    let res = post_resource_names();
    let mut consts = Vec::new();
    let span = emit_post_leaf(&float_in, &uint_in, NO_VEC_INPUTS, NO_VEC4_INPUTS, &res, || {
        let px = [Emit::uint_input(input_id(&uints, "px.x")), Emit::uint_input(input_id(&uints, "px.y"))];
        let params = PostParams::<EmitCf> {
            mode_words: Emit::uint_input(input_id(&uints, "mode_words")),
            exposure_ratio: Emit::input(input_id(&floats, "exposure_ratio")),
            extent: [
                Emit::uint_input(input_id(&uints, "extent.x")),
                Emit::uint_input(input_id(&uints, "extent.y")),
            ],
        };
        let resources = PostRes::<EmitCf> { scene: post_texture(&res, "gScene") };
        let _ = post::post_final_main::<EmitCf>(px, &params, &resources, &RetCellV4);
        let symbols = NAMED_LITS.with(|n| n.borrow().clone());
        consts = ARENA.with(|a| traced_constants(&a.borrow(), &symbols));
    });
    (span, consts)
}

/// Generates the **`post_final_main`** BODY span by tracing [`crate::post::post_final_main`] over
/// [`EmitCf`] — the whole per-pixel body: the one scene fetch, the four stage markers, the
/// `[branch]` tonemapper dispatch, the OETF and the `float4` return.
pub fn emit_hlsl_post_final_main() -> String {
    trace_post_final_main().0
}

/// The `post_final_bindings` span: one declaration per [`crate::post::POST_FINAL_BINDINGS`] row, the
/// `cbuffer` members from [`crate::post::POST_FINAL_UBO_LAYOUT`].
pub fn emit_hlsl_post_final_bindings() -> String {
    let mut out = String::new();
    for b in post::POST_FINAL_BINDINGS {
        let at = format!("[[vk::binding({}, {})]]", b.binding, b.set);
        match b.kind {
            PostBindingKind::Tex2Load | PostBindingKind::Tex2Linear => {
                out.push_str(&format!("{at} Texture2D<float4> {};\n", b.name));
            }
            PostBindingKind::Tex3Linear => out.push_str(&format!("{at} Texture3D<float4> {};\n", b.name)),
            PostBindingKind::Buf => out.push_str(&format!("{at} StructuredBuffer<uint> {};\n", b.name)),
            PostBindingKind::Store => {
                let fmt = b.image_format.expect("invariant: a Store row carries its storage format");
                out.push_str(&format!("{at} [[vk::image_format(\"{fmt}\")]] RWTexture2D<float4> {};\n", b.name));
            }
            PostBindingKind::Ubo => {
                out.push_str(&format!("{at} cbuffer {} {{\n", b.name));
                for f in post::POST_FINAL_UBO_LAYOUT {
                    let ty = match f.kind {
                        PostUboKind::U32 => "uint",
                        PostUboKind::F32 => "float",
                        PostUboKind::Uint2 => "uint2",
                    };
                    out.push_str(&format!("    {ty} {}; // offset {}\n", f.name, f.offset));
                }
                out.push_str("};\n");
            }
        }
    }
    out
}

/// The `post_final_consts` span: a `static const` for every symbol the traced body references.
pub fn emit_hlsl_post_final_consts() -> String {
    trace_post_final_main()
        .1
        .iter()
        .map(|(sym, ty, val)| format!("static const {ty} {sym} = {val};\n"))
        .collect()
}

/// Wraps a generated span in its sentinel pair.
fn sentinel(name: &str, body: &str) -> String {
    format!("// === GENERATED {name} BEGIN ===\n{body}// === GENERATED {name} END ===\n")
}

/// Generates the WHOLE committed `boyko_rhi_vulkan/shaders/post_final.comp.hlsl`: the header with the
/// frozen recipe, the three generated spans (`post_final_bindings`, `post_final_consts`,
/// `post_final_main`) and the skeleton — which is ONLY `numthreads`, the extent guard, one call and
/// the store. No leaf is printed as a standalone function: the traced body inlines every leaf, so a
/// printed leaf would be dead text DXC strips and a reader would mistake for the executed code.
///
/// The template lives HERE (not in the bin) so the sync test compares the whole committed file.
pub fn emit_post_final_comp() -> String {
    let main_fn = format!("float4 post_final_main(uint2 px) {{\n{}}}\n", emit_hlsl_post_final_main());
    format!(
        r#"// post_final.comp -- render wave PXe: the post chain's final pass, GENERATED WHOLE and not yet
// wired (PX1a creates its pipeline). Per pixel: one scene fetch, then hdr_sanitize -> pre_ratio_apply
// -> tonemap_select -> oetf_gamma22 (`boyko_shaderdsl::post`), stored to the display image.
//
// GENERATED by `cargo run -p boyko_shaderdsl --features emit --bin emit_post`. The whole file is the
// generator's output (`boyko_shaderdsl::emit::emit_post_final_comp`); a hand-edit fails
// `boyko_rhi_vulkan/tests/post_edsl_sync.rs`, which also byte-gates the committed .spv. The
// declarations are printed from `post::POST_FINAL_BINDINGS` / `POST_FINAL_UBO_LAYOUT`; the body is
// the trace of `post::post_final_main`, the SAME function PX0's host oracle evaluates. A stage is
// added in `post.rs`, never here: this skeleton holds no fetch and no arithmetic.
//
// # Compile (offline + hermetic; the committed .spv is byte-gated)
//
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 post_final.comp.hlsl -Fo post_final.comp.spv

{bindings}
{consts}
{main_fn}
[numthreads(8, 8, 1)]
void main(uint3 dtid : SV_DispatchThreadID) {{
    if (any(dtid.xy >= extent)) {{
        return;
    }}
    gDisplay[dtid.xy] = post_final_main(dtid.xy);
}}
"#,
        bindings = sentinel("post_final_bindings", &emit_hlsl_post_final_bindings()),
        consts = sentinel("post_final_consts", &emit_hlsl_post_final_consts()),
        main_fn = sentinel("post_final_main", &main_fn),
    )
}

// ---- Render wave PX1a: the HDR boundary spans (`px1a/cut.md` D-S2; orchestrator ruling 67 W5) ----

/// A traced symbol's `static const` row: `(symbol, hlsl type, value text)`.
type ConstRow = (&'static str, &'static str, String);

/// Prints one `float3` leaf as a standalone HLSL function `float3 <name>(float3 <p0>[, …]) { return
/// …; }`: the leaf is traced over [`Node::Vec3Param`]s named `params` and its value recorded as the
/// one `return`. Returns the function text and the named constants it references.
fn emit_vec3_leaf_fn(
    name: &str,
    params: &[&'static str],
    leaf: impl FnOnce(&[Emit]) -> Emit,
) -> (String, Vec<ConstRow>) {
    let mut consts = Vec::new();
    let body = emit_post_leaf(&[], &[], params, NO_VEC4_INPUTS, &[], || {
        let args: Vec<Emit> = (0..params.len() as u32).map(|i| Emit(push(Node::Vec3Param(i)))).collect();
        let value = leaf(&args);
        record_stmt(Stmt::Return(value.0));
        let symbols = NAMED_LITS.with(|n| n.borrow().clone());
        consts = ARENA.with(|a| traced_constants(&a.borrow(), &symbols));
    });
    let sig: Vec<String> = params.iter().map(|p| format!("float3 {p}")).collect();
    (format!("float3 {name}({}) {{\n{body}}}\n", sig.join(", ")), consts)
}

/// The constants of several traced functions, each symbol once, in first-reference order.
fn const_block(sets: &[Vec<ConstRow>]) -> String {
    let mut seen: Vec<&'static str> = Vec::new();
    let mut out = String::new();
    for (sym, ty, val) in sets.iter().flatten() {
        if !seen.contains(sym) {
            seen.push(*sym);
            out.push_str(&format!("static const {ty} {sym} = {val};\n"));
        }
    }
    out
}

/// The `post_hdr_boundary` span body: [`crate::post::hdr_sanitize`] and
/// [`crate::post::hdr_history_guard`] printed as HLSL functions (`float3 hdr_sanitize(float3 c)`,
/// `float3 hdr_history_guard(float3 hist, float3 cur)`), after the constants they reference. Spliced
/// into `taa_resolve.comp.hlsl` (every `lit` tap, the history write) and `ssaa_downsample.fs.hlsl`
/// (every tap of the HDR box) by `emit_post --splice`, so the two hand-written files' HDR boundary
/// is the eDSL's — the SAME leaves `post_final`'s traced body inlines.
pub fn emit_hlsl_post_hdr_boundary() -> String {
    let (sanitize, c1) = emit_vec3_leaf_fn("hdr_sanitize", &["c"], |a| post::hdr_sanitize::<EmitCf>(a[0]));
    let (guard, c2) =
        emit_vec3_leaf_fn("hdr_history_guard", &["hist", "cur"], |a| post::hdr_history_guard::<EmitCf>(a[0], a[1]));
    format!("{}\n{sanitize}\n{guard}", const_block(&[c1, c2]))
}

/// The `lit_store_saturate` span body: [`crate::post::lit_store_saturate`] printed as the HLSL
/// function `float3 lit_store_saturate(float3 c)`, after `HDR_MAX`. Spliced into
/// `shaders/lit_format.hlsli`, which every `lit` producer includes; every `lit` store passes
/// through it (`tests/lit_store_clamp_census.rs` rule 3).
pub fn emit_hlsl_lit_store_saturate() -> String {
    let (f, c) = emit_vec3_leaf_fn("lit_store_saturate", &["c"], |a| post::lit_store_saturate::<EmitCf>(a[0]));
    format!("{}\n{f}", const_block(&[c]))
}

/// One generated span inside a hand-written shader: `(file under boyko_rhi_vulkan/shaders, span
/// name, the emitter's text)`.
pub type PostSplice = (&'static str, &'static str, String);

/// Every span `emit_post --splice` rewrites — the table the bin applies and
/// `boyko_rhi_vulkan/tests/post_chain_spv_sync.rs` pins each committed span against.
pub fn post_splices() -> [PostSplice; 3] {
    [
        ("lit_format.hlsli", "lit_store_saturate", emit_hlsl_lit_store_saturate()),
        ("taa_resolve.comp.hlsl", "post_hdr_boundary", emit_hlsl_post_hdr_boundary()),
        ("ssaa_downsample.fs.hlsl", "post_hdr_boundary", emit_hlsl_post_hdr_boundary()),
    ]
}

/// `src` with the body of its ONE `// === GENERATED <name> BEGIN/END ===` span replaced by `body`;
/// `Err` names a missing or repeated sentinel. The sentinels themselves are kept.
pub fn splice_span(src: &str, name: &str, body: &str) -> Result<String, String> {
    let begin = format!("// === GENERATED {name} BEGIN ===\n");
    let end = format!("// === GENERATED {name} END ===\n");
    let (Some(b), Some(e)) = (src.find(&begin), src.find(&end)) else {
        return Err(format!("no `{}` / `{}` sentinel pair", begin.trim_end(), end.trim_end()));
    };
    if src.matches(&begin).count() != 1 || src.matches(&end).count() != 1 || e < b {
        return Err(format!("the `{name}` span is not exactly one ordered sentinel pair"));
    }
    Ok(format!("{}{begin}{body}{}", &src[..b], &src[e..]))
}

/// The committed body of the ONE `<name>` span of `src`, or `None`.
pub fn span_body<'a>(src: &'a str, name: &str) -> Option<&'a str> {
    let begin = format!("// === GENERATED {name} BEGIN ===\n");
    let end = format!("// === GENERATED {name} END ===\n");
    let b = src.find(&begin)? + begin.len();
    let e = src[b..].find(&end)? + b;
    Some(&src[b..e])
}
