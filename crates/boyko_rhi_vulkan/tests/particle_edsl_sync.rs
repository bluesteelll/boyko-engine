//! The GPU particle system's eDSL ↔ HLSL ↔ `.spv` SINGLE-SOURCE GUARD
//! (`docs/PARTICLES-PLAN.md` Rev 4, P0 gate #14).
//!
//! Three layers, each failing independently:
//!
//! 1. **`*_matches_edsl_emit`** — every `// === GENERATED <leaf> BEGIN/END ===` span in the five
//!    committed particle shaders IS the output of `boyko_shaderdsl::emit`, and is inside the
//!    RIGHT function. A hand-edit of a span diverges the GPU math from the `EvalCf` oracle the P0
//!    determinism harness compares a readback against — silently, because the same shader still
//!    compiles and still draws plausible particles.
//! 2. **`particle_*_spv_byte_identical`** — each committed `.spv` is the re-DXC of its own source
//!    under the frozen recipe pinned in that source's header. SKIPS (with an `eprintln`) when no
//!    `dxc` resolves; the byte gate is only as hermetic as the pinned VulkanSDK 1.4.350.0
//!    toolchain that produced the artifacts.
//! 3. **The opcode census** — plan gate #14's artifact-level claims: the workgroup widths, the
//!    exact `OpAtomicIAdd` population of `particle_sim`, zero `OpAtomicUMax` anywhere, zero
//!    atomics in `particle_emit`, and zero `OpFDiv` in the module that carries
//!    `particle_rot_advance`.
//!
//! # Why gate #14's `OpFDiv` clause is asserted MODULE-WIDE
//!
//! The plan words it "no `OpFDiv` in `particle_rot_advance`'s generated span". A per-function scan
//! of the artifact is not merely unbuilt, it is UNREACHABLE: DXC inlines every helper into
//! `%main`, so the committed module carries exactly one `OpFunction` header and there is no span
//! to scope to. The decidable form is therefore "the module that CONTAINS that span carries no
//! `OpFDiv` at all", which is strictly stronger, and `particle_sim.comp.hlsl` is written to
//! satisfy it (the age normalization multiplies by the reciprocal lifetime `particle_emit` stored
//! once at spawn). The source-level half — the span's own text contains no `/` — lives in
//! `boyko_shaderdsl/tests/particle_leaves.rs` and runs on every host, dxc or not.

use std::path::PathBuf;
use std::process::Command;

// ---- Shared plumbing (the `sdf_mesh_shadow_spv_sync` / `vb_batch_cull_spv_sync` idioms) ------

/// The shaders directory (`CARGO_MANIFEST_DIR/shaders`), where the committed `.hlsl` and `.spv`
/// live (and where DXC must run so any `#include` resolves).
fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Reads a committed shader source, LF-normalized so a CRLF checkout does not spuriously mismatch
/// the LF generator output.
fn read_shader(name: &str) -> String {
    let path = shaders_dir().join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("invariant: shaders/{name} must exist next to this crate: {e}"))
        .replace("\r\n", "\n")
}

/// Locates the `dxc` executable: the pinned Vulkan-SDK path, then `$VULKAN_SDK/Bin`, then `PATH`.
/// `None` ⇒ the byte-identity tests SKIP.
fn find_dxc() -> Option<PathBuf> {
    let pinned = PathBuf::from("C:/VulkanSDK/1.4.350.0/Bin/dxc.exe");
    if pinned.exists() {
        return Some(pinned);
    }
    let bare = if cfg!(windows) { "dxc.exe" } else { "dxc" };
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let candidate = PathBuf::from(sdk).join("Bin").join(bare);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if Command::new(bare).arg("--version").output().is_ok() {
        return Some(PathBuf::from(bare));
    }
    None
}

/// Locates `spirv-dis` the same layered way.
fn find_spirv_dis() -> Option<PathBuf> {
    let pinned = PathBuf::from("C:/VulkanSDK/1.4.350.0/Bin/spirv-dis.exe");
    if pinned.exists() {
        return Some(pinned);
    }
    let bare = if cfg!(windows) { "spirv-dis.exe" } else { "spirv-dis" };
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let candidate = PathBuf::from(sdk).join("Bin").join(bare);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if Command::new(bare).arg("--version").output().is_ok() {
        return Some(PathBuf::from(bare));
    }
    None
}

/// Extracts a `<ret> NAME(...) { ... }` function — the signature line through its MATCHING closing
/// brace — out of a shader source.
///
/// A BRACE COUNTER, not a first-`\n}` scan: `main` and the generated leaves both nest (`for`,
/// `if`), so the first `}` on its own line closes an inner block. Counting `{`/`}` from the
/// signature's opening brace to its balanced close extracts the whole function for any body.
/// String and comment braces do not occur inside the particle shaders' function bodies, so a raw
/// brace count is exact.
fn extract_fn(src: &str, sig: &str) -> String {
    let start = src
        .find(sig)
        .unwrap_or_else(|| panic!("the committed shader is missing `{sig}`"));
    let after = &src[start..];
    let open = after
        .find('{')
        .expect("a function must have an opening brace");
    let mut depth = 0i32;
    for (i, ch) in after[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return after[..open + i + 1].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces extracting `{sig}` — the function never closed");
}

/// Asserts that `span` — a freshly generated eDSL body — is the body of the committed `sig`
/// function in `file`.
///
/// The `extract_fn` step is what makes this a REAL pin rather than a substring search: a `span`
/// that had been pasted into some other function, or into a comment, would satisfy a bare
/// `src.contains(span)` while the function the sim actually calls carried something else.
fn assert_span_is_the_body_of(file: &str, sig: &str, leaf: &str, span: &str) {
    let src = read_shader(file);
    let func = extract_fn(&src, sig);
    assert!(
        func.contains(span),
        "{file} `{leaf}` DRIFTED from boyko_shaderdsl::emit — the committed body no longer \
         matches the generator. Re-run `cargo run -p boyko_shaderdsl --features emit --bin \
         emit_particles`, re-DXC the affected `.spv` with the recipe in the shader's header, and \
         commit both.\n--- expected (eDSL-generated) ---\n{span}\n--- committed function ---\n{func}"
    );
}

// ---- Layer 1: the per-leaf generator pins ---------------------------------------------------

#[test]
fn particle_integrate_matches_edsl_emit() {
    // The integrator is the whole per-substep physics. A hand-edit that, say, advanced the
    // position with the ENTRY velocity instead of the damped one would keep compiling, keep
    // drawing plausible particles, and diverge from the `EvalCf` oracle that gates determinism.
    let span = boyko_shaderdsl::emit::emit_hlsl_particle_integrate().replace("\r\n", "\n");
    assert_span_is_the_body_of(
        "particle_sim.comp.hlsl",
        "void particle_integrate(inout float3 pos, inout float3 vel, inout float life,",
        "particle_integrate",
        &span,
    );
}

#[test]
fn particle_rng_matches_edsl_emit() {
    // The PCG hash is bit-exact by construction, which makes it the ONE leaf whose device output
    // the CPU oracle can be compared to with `assert_eq!` and no tolerance — but only while the
    // committed text IS the generator's.
    let span = boyko_shaderdsl::emit::emit_hlsl_particle_rng().replace("\r\n", "\n");
    assert_span_is_the_body_of(
        "particle_emit.comp.hlsl",
        "uint particle_rng(uint state) {",
        "particle_rng",
        &span,
    );
}

#[test]
fn particle_spawn_state_matches_edsl_emit() {
    // The cone sample's unit length is an ALGEBRAIC identity (the Lambert lift), not a
    // normalization — so any hand-edit of this span produces non-unit directions with no
    // renormalization downstream to hide them.
    let span = boyko_shaderdsl::emit::emit_hlsl_particle_spawn_state().replace("\r\n", "\n");
    assert_span_is_the_body_of(
        "particle_emit.comp.hlsl",
        "void particle_spawn_state(float3 basis_x, float3 basis_y, float3 basis_z,",
        "particle_spawn_state",
        &span,
    );
}

#[test]
fn particle_curve_eval_matches_edsl_emit() {
    let span = boyko_shaderdsl::emit::emit_hlsl_particle_curve_eval().replace("\r\n", "\n");
    assert_span_is_the_body_of(
        "particle_sim.comp.hlsl",
        "float particle_curve_eval(uint keys_lo, uint keys_hi, float t) {",
        "particle_curve_eval",
        &span,
    );
}

#[test]
fn particle_billboard_corner_matches_edsl_emit() {
    // The VS's snorm16 decode is the SAME `snorm16_lane` helper `particle_rot_advance` uses, so
    // this pin and the next are jointly what keep the sim's encode and the draw's decode from
    // forking — the failure mode being billboards that spin at a rate nothing else in the frame
    // agrees with.
    let span = boyko_shaderdsl::emit::emit_hlsl_particle_billboard_corner().replace("\r\n", "\n");
    assert_span_is_the_body_of(
        "particle_draw.vs.hlsl",
        "void particle_billboard_corner(float3 center, float3 cam_right, float3 cam_up,",
        "particle_billboard_corner",
        &span,
    );
}

#[test]
fn particle_rot_advance_matches_edsl_emit() {
    let span = boyko_shaderdsl::emit::emit_hlsl_particle_rot_advance().replace("\r\n", "\n");
    assert_span_is_the_body_of(
        "particle_sim.comp.hlsl",
        "uint particle_rot_advance(uint rot_cs, float mul_cos, float mul_sin) {",
        "particle_rot_advance",
        &span,
    );
    // Plan gate #14's SOURCE-level half for this leaf, restated at the committed file (the
    // generator-side half is in `boyko_shaderdsl/tests/particle_leaves.rs`). Plan M7: a division
    // inside a leaf drags `OpFDiv`'s 2.5 ULP into that leaf's oracle, and the renormalization
    // that would have needed one is deliberately absent.
    assert!(
        !span.contains('/'),
        "particle_rot_advance's committed span must contain NO divide (plan gate #14 / M7):\n{span}"
    );
}

// ---- The generator-input pins (plan gate #8, shader half) ------------------------------------

/// `offset_of!(ParticleDrawArgs, additive.instance_count)` — plan D4 pins it at 4.
const ADDITIVE_INSTANCE_COUNT_OFFSET: u32 = 4;
/// `offset_of!(ParticleDrawArgs, alpha.instance_count)` — plan D4 pins it at 28 (P2's slot).
const ALPHA_INSTANCE_COUNT_OFFSET: u32 = 28;
/// The emit/sim workgroup width (R4 / plan D4).
const PARTICLE_LOCAL_SIZE: usize = 256;
/// `PARTICLE_SUBSTEP_CEILING` (plan M3) — the shader's `min` is the F25 hang guard only.
const PARTICLE_SUBSTEP_CEILING: u32 = 64;

#[test]
fn draw_arg_instance_count_word_indices_are_generator_derived() {
    // Plan gate #8. The two byte offsets are `offset_of!` chains host-side; the SHADERS must
    // spell the WORD indices DERIVED from them, never typed. This asserts the derivation landed:
    // `4 / 4 == 1` and `28 / 4 == 7`.
    //
    // Both offsets must be 4-aligned for a word index to exist at all — checked here rather than
    // assumed, because a struct edit that broke the alignment would otherwise silently truncate.
    assert_eq!(ADDITIVE_INSTANCE_COUNT_OFFSET % 4, 0);
    assert_eq!(ALPHA_INSTANCE_COUNT_OFFSET % 4, 0);
    let additive_word = ADDITIVE_INSTANCE_COUNT_OFFSET / 4;
    let alpha_word = ALPHA_INSTANCE_COUNT_OFFSET / 4;

    let kickoff = read_shader("particle_kickoff.comp.hlsl");
    assert!(
        kickoff.contains(&format!(
            "static const uint DRAW_ADDITIVE_INSTANCE_WORD = {additive_word}u;"
        )),
        "particle_kickoff must derive the additive instanceCount WORD index from byte offset \
         {ADDITIVE_INSTANCE_COUNT_OFFSET}"
    );
    assert!(
        kickoff.contains(&format!(
            "static const uint DRAW_ALPHA_INSTANCE_WORD    = {alpha_word}u;"
        )),
        "particle_kickoff must derive the alpha instanceCount WORD index from byte offset \
         {ALPHA_INSTANCE_COUNT_OFFSET}"
    );

    let sim = read_shader("particle_sim.comp.hlsl");
    assert!(
        sim.contains(&format!(
            "static const uint DRAW_ADDITIVE_INSTANCE_WORD = {additive_word}u;"
        )),
        "particle_sim's render counter must sit at the word derived from byte offset \
         {ADDITIVE_INSTANCE_COUNT_OFFSET} — this is the address the returning InterlockedAdd \
         allocates render positions from"
    );
    // The sim must NOT name the alpha counter at P0: the alpha class is unreachable here, and a
    // reference to it would mean a class select shipped before the pass that needs one.
    assert!(
        !sim.contains("DRAW_ALPHA_INSTANCE_WORD"),
        "particle_sim must not reference the alpha render counter at P0 (additive-only)"
    );
}

#[test]
fn substep_ceiling_is_the_shader_side_hang_guard() {
    // Plan M3: the clamp happens ONCE, on the host. The shader's own `min` survives solely as the
    // F25 guard against a corrupt push constant driving an unbounded device loop. It must be
    // PRESENT (or a bad push hangs the device) and must carry the plan's value (or it silently
    // becomes a second, disagreeing clamp — the exact two-numbers defect M3 closed).
    let sim = read_shader("particle_sim.comp.hlsl");
    assert!(
        sim.contains(&format!(
            "static const uint SUBSTEP_CEILING = {PARTICLE_SUBSTEP_CEILING}u;"
        )),
        "particle_sim must carry PARTICLE_SUBSTEP_CEILING = {PARTICLE_SUBSTEP_CEILING} (plan M3)"
    );
    assert!(
        sim.contains("uint steps = min(pc.steps, SUBSTEP_CEILING);"),
        "particle_sim's substep loop must be bounded by the hang guard (plan M3 / F25)"
    );
}

#[test]
fn every_particle_shader_pins_its_own_frozen_recipe() {
    // A committed `.spv` is only gated if the recipe the gate re-runs is the recipe the header
    // claims. Pinning the `-T` profile per file keeps those one text: a header that drifted to a
    // different stage would make the byte gate below assert against an artifact nobody builds.
    for (file, profile) in PARTICLE_MODULES {
        let src = read_shader(&format!("{file}.hlsl"));
        assert!(
            src.contains(&format!("-T {profile} -E main")),
            "{file}.hlsl must pin its frozen dxc recipe (`-T {profile} -E main`) in its own header"
        );
        assert!(
            src.contains("-fspv-target-env=vulkan1.3"),
            "{file}.hlsl's frozen recipe must target vulkan1.3"
        );
        assert!(
            !src.contains(" -O3") && !src.contains(" -D "),
            "{file}.hlsl's frozen recipe must carry no -O and no -D (plan D12)"
        );
    }
}

// ---- Layer 2: the re-DXC byte gate ----------------------------------------------------------

/// The five committed particle modules and the DXC target profile each one's frozen recipe names.
const PARTICLE_MODULES: [(&str, &str); 5] = [
    ("particle_kickoff.comp", "cs_6_0"),
    ("particle_emit.comp", "cs_6_0"),
    ("particle_sim.comp", "cs_6_0"),
    ("particle_draw.vs", "vs_6_0"),
    ("particle_draw.fs", "ps_6_0"),
];

/// Re-DXCs `<stem>.hlsl` under the EXACT frozen recipe its header pins (`-spirv -T <profile> -E
/// main -fspv-target-env=vulkan1.3`, no `-O`, no `-D`) into a fresh temp `.spv` and returns the
/// bytes. Never overwrites a committed artifact.
fn redxc(dxc: &PathBuf, dir: &PathBuf, stem: &str, profile: &str) -> Vec<u8> {
    let out_spv = std::env::temp_dir().join(format!("{stem}.spv.redxc.spv"));
    let mut cmd = Command::new(dxc);
    cmd.current_dir(dir)
        .args(["-spirv", "-T", profile, "-E", "main"]);
    cmd.args([
        "-fspv-target-env=vulkan1.3",
        &format!("{stem}.hlsl"),
        "-Fo",
    ])
    .arg(&out_spv);
    let status = cmd
        .status()
        .expect("invariant: dxc was located and must run");
    assert!(
        status.success(),
        "dxc failed re-compiling {stem}.hlsl under the frozen recipe"
    );
    let bytes = std::fs::read(&out_spv).expect("invariant: dxc wrote the re-DXC .spv");
    let _ = std::fs::remove_file(&out_spv); // best-effort tidy
    bytes
}

/// Asserts one module's committed `.spv` byte-equals its own re-DXC.
fn assert_spv_byte_identical(stem: &str, profile: &str) {
    let Some(dxc) = find_dxc() else {
        eprintln!("SKIP {stem}_spv_byte_identical: no dxc on this host");
        return;
    };
    let dir = shaders_dir();
    let committed_path = dir.join(format!("{stem}.spv"));
    let committed = std::fs::read(&committed_path)
        .unwrap_or_else(|e| panic!("missing committed {}: {e}", committed_path.display()));
    let fresh = redxc(&dxc, &dir, stem, profile);
    assert!(
        committed == fresh,
        "{stem}.spv ({} bytes committed, {} bytes fresh) is NOT the re-DXC of its source under \
         the frozen recipe — either the committed .spv is stale (re-run the recipe in the \
         shader's header and commit it) or the host dxc is not the pinned VulkanSDK 1.4.350.0 \
         toolchain.",
        committed.len(),
        fresh.len(),
    );
}

#[test]
fn particle_kickoff_spv_byte_identical() {
    assert_spv_byte_identical("particle_kickoff.comp", "cs_6_0");
}

#[test]
fn particle_emit_spv_byte_identical() {
    assert_spv_byte_identical("particle_emit.comp", "cs_6_0");
}

#[test]
fn particle_sim_spv_byte_identical() {
    assert_spv_byte_identical("particle_sim.comp", "cs_6_0");
}

#[test]
fn particle_draw_vs_spv_byte_identical() {
    assert_spv_byte_identical("particle_draw.vs", "vs_6_0");
}

#[test]
fn particle_draw_fs_spv_byte_identical() {
    assert_spv_byte_identical("particle_draw.fs", "ps_6_0");
}

// ---- Layer 3: the opcode census (plan gate #14) ----------------------------------------------

/// One committed module's opcode census. Every field is counted by EXACT whole-token match on a
/// whitespace-split line — the `vb_batch_cull_spv_sync` discipline, whose own fixture control
/// documents the near-miss (`OpSelectionMerge` read as an `OpSelect`) that makes whole-token
/// matching non-negotiable rather than stylistic.
#[derive(Debug, Default, PartialEq, Eq)]
struct SpvCensus {
    /// The returning `InterlockedAdd`s. In `particle_sim` this is THE machinery claim: exactly
    /// the three wave-leader sites (plan D5's budget), and nowhere else.
    op_atomic_iadd: usize,
    /// Plan M1 DELETED the `InterlockedMax` mirror once the two counters turned out to be
    /// different quantities. Zero everywhere, forever — a nonzero count means a mirror came back.
    op_atomic_umax: usize,
    /// EVERY atomic opcode, by prefix. `particle_emit` must carry zero of these (plan A3): its
    /// zero-atomic property is the whole reason kickoff is a one-thread pass.
    op_atomic_any: usize,
    /// Plan gate #14's decidable `OpFDiv` clause — see this file's header for why it is
    /// module-wide rather than span-scoped.
    op_fdiv: usize,
    /// The declared workgroup width, read off `OpExecutionMode … LocalSize <x> 1 1`. Zero for the
    /// graphics stages, which declare none.
    local_size_x: usize,
    /// The module's declared binding numbers, sorted and deduped. DXC STRIPS a
    /// declared-but-unread resource, so this doubles as a "every declared binding is actually
    /// read" claim.
    binding_set: Vec<usize>,
}

/// Disassembles `spv_path` with `spirv-dis`.
fn disassemble(spirv_dis: &PathBuf, spv_path: &PathBuf) -> String {
    let out = Command::new(spirv_dis)
        .arg(spv_path)
        .output()
        .expect("invariant: spirv-dis was located and must run");
    assert!(
        out.status.success(),
        "spirv-dis failed on {}: {}",
        spv_path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("invariant: spirv-dis emits UTF-8 disassembly")
}

/// Counts [`SpvCensus`]'s fields over a disassembly.
fn census(dis: &str) -> SpvCensus {
    let mut c = SpvCensus::default();
    for line in dis.lines() {
        for tok in line.split_whitespace() {
            match tok {
                "OpAtomicIAdd" => {
                    c.op_atomic_iadd += 1;
                    c.op_atomic_any += 1;
                }
                "OpAtomicUMax" => {
                    c.op_atomic_umax += 1;
                    c.op_atomic_any += 1;
                }
                "OpFDiv" => c.op_fdiv += 1,
                // The PREFIX selector for "any atomic at all". `OpAtomicLoad`, `OpAtomicStore`,
                // `OpAtomicExchange`, `OpAtomicCompareExchange`, `OpAtomicUMin`, … all count; the
                // two named above are excluded here because their own arms already counted them.
                t if t.starts_with("OpAtomic") => c.op_atomic_any += 1,
                _ => {}
            }
        }
        let toks: Vec<&str> = line.split_whitespace().collect();
        // `OpExecutionMode %main LocalSize 256 1 1` — the width is the token AFTER `LocalSize`.
        if let Some(i) = toks.iter().position(|t| *t == "LocalSize")
            && let Some(x) = toks.get(i + 1).and_then(|t| t.parse::<usize>().ok())
        {
            c.local_size_x = x;
        }
        // `OpDecorate %p_counters Binding 0` — whole-token, so the sibling `DescriptorSet 0` line
        // cannot contribute a phantom binding 0.
        if let Some(i) = toks.iter().position(|t| *t == "Binding")
            && let Some(b) = toks.get(i + 1).and_then(|t| t.parse::<usize>().ok())
        {
            c.binding_set.push(b);
        }
    }
    c.binding_set.sort_unstable();
    c.binding_set.dedup();
    c
}

/// Censuses one committed module, or `None` when `spirv-dis` does not resolve on this host.
fn census_of(stem: &str) -> Option<SpvCensus> {
    let spirv_dis = find_spirv_dis()?;
    let path = shaders_dir().join(format!("{stem}.spv"));
    Some(census(&disassemble(&spirv_dis, &path)))
}

#[test]
fn particle_compute_workgroup_widths_are_the_plan_s() {
    // Plan D4: 256 for emit and sim (R4's number), 1 for kickoff. This is the ONE number the host
    // and the shader must agree on that nothing else checks — the host dispatches
    // `ceil(n / LOCAL_SIZE)` groups, and a shader whose `[numthreads]` were wider would leave the
    // tail unvisited while every image still looked plausible.
    let Some(kickoff) = census_of("particle_kickoff.comp") else {
        eprintln!("SKIP particle_compute_workgroup_widths_are_the_plan_s: no spirv-dis on this host");
        return;
    };
    assert_eq!(
        kickoff.local_size_x, 1,
        "particle_kickoff is a ONE-THREAD pass — its pre-decrement/pre-increment of the two \
         counters is only atomic-free because exactly one lane runs it"
    );
    let emit = census_of("particle_emit.comp").expect("spirv-dis resolved above");
    assert_eq!(emit.local_size_x, PARTICLE_LOCAL_SIZE);
    let sim = census_of("particle_sim.comp").expect("spirv-dis resolved above");
    assert_eq!(sim.local_size_x, PARTICLE_LOCAL_SIZE);
}

#[test]
fn particle_emit_carries_zero_atomics() {
    // Plan A3/D5: emit's zero-atomic property is STRUCTURAL, not incidental — kickoff published
    // `dead_base` and `emit_append_base` precisely so every lane could compute both of its
    // indices arithmetically. A future edit that reaches for an `InterlockedAdd` here has
    // reintroduced the per-lane serialization the one-thread pass exists to delete, and this is
    // where that shows up.
    let Some(emit) = census_of("particle_emit.comp") else {
        eprintln!("SKIP particle_emit_carries_zero_atomics: no spirv-dis on this host");
        return;
    };
    assert_eq!(
        emit.op_atomic_any, 0,
        "particle_emit must carry ZERO atomic opcodes (plan A3); census: {emit:?}"
    );
}

/// The wave-leader `InterlockedAdd` sites in `particle_sim`, enumerated: the LIST counter
/// (`alive_count_next`), the additive class's RENDER counter (`additive.instanceCount`), and the
/// dying path's free-list push (`dead_count`).
///
/// Three, not two and not four. Two would mean a counter lost its reservation; four would mean
/// P2's alpha counter (or a re-introduced mirror) shipped early.
const SIM_WAVE_LEADER_ATOMIC_SITES: usize = 3;

#[test]
fn particle_sim_atomic_census_is_exactly_the_wave_leader_sites() {
    let Some(sim) = census_of("particle_sim.comp") else {
        eprintln!(
            "SKIP particle_sim_atomic_census_is_exactly_the_wave_leader_sites: no spirv-dis on \
             this host"
        );
        return;
    };
    assert_eq!(
        sim.op_atomic_iadd, SIM_WAVE_LEADER_ATOMIC_SITES,
        "particle_sim must carry exactly the three wave-leader InterlockedAdd sites (plan D5's \
         per-wave budget). A HIGHER count is the per-lane form the wave aggregation exists to \
         delete — ~0.5 ms/frame at 1M against ~32 us; a LOWER one means a reservation went \
         missing. Census: {sim:?}"
    );
    assert_eq!(
        sim.op_atomic_any, SIM_WAVE_LEADER_ATOMIC_SITES,
        "particle_sim must carry NO atomic beyond those three; census: {sim:?}"
    );
}

#[test]
fn no_particle_module_carries_an_atomic_max() {
    // Plan M1 deleted the `InterlockedMax` mirror: it existed only because ONE counter was trying
    // to serve two different quantities, and once the list count and the render count were
    // recognized as different numbers, the mirror had nothing to mirror. This asserts across ALL
    // FIVE modules rather than just the sim, because "the mirror came back" is a class of edit,
    // not a location.
    if find_spirv_dis().is_none() {
        eprintln!("SKIP no_particle_module_carries_an_atomic_max: no spirv-dis on this host");
        return;
    }
    for (stem, _) in PARTICLE_MODULES {
        let c = census_of(stem).expect("spirv-dis resolved above");
        assert_eq!(
            c.op_atomic_umax, 0,
            "{stem} carries an OpAtomicUMax — plan M1 deleted that mirror; census: {c:?}"
        );
    }
}

#[test]
fn particle_sim_carries_no_float_divide() {
    // Plan gate #14's `OpFDiv` clause, in its decidable module-wide form (see this file's header
    // for why span-scoping is unreachable). `particle_sim` is the module that carries
    // `particle_rot_advance`, and it is written to have no divide anywhere: the age normalization
    // multiplies by the reciprocal lifetime `particle_emit` computed once at spawn.
    let Some(sim) = census_of("particle_sim.comp") else {
        eprintln!("SKIP particle_sim_carries_no_float_divide: no spirv-dis on this host");
        return;
    };
    assert_eq!(
        sim.op_fdiv, 0,
        "particle_sim must carry ZERO OpFDiv (plan gate #14 / M7 — a divide inside a leaf drags \
         2.5 ULP into that leaf's bit-exact contract); census: {sim:?}"
    );
}

#[test]
fn particle_modules_declare_the_bindings_they_read() {
    // DXC STRIPS a declared-but-unloaded resource (measured, `vb_batch_cull_spv_sync` rung
    // R2d-3), so this census reads back the bindings the module actually TOUCHES. Pinning the set
    // therefore catches two opposite defects with one assertion: a resource silently dropped from
    // the shader's flow (the set shrinks) and one bound for a feature that has not landed (the
    // set grows). The host's `PARTICLE_LAYOUT_ENTRIES` table must be the union of these.
    if find_spirv_dis().is_none() {
        eprintln!("SKIP particle_modules_declare_the_bindings_they_read: no spirv-dis on this host");
        return;
    }
    let expected: [(&str, &[usize]); 5] = [
        // counters, dispatch args, draw args.
        ("particle_kickoff.comp", &[0, 1, 2]),
        // counters, dead, alive_read, particle, emit requests, effects.
        ("particle_emit.comp", &[0, 3, 4, 6, 8, 9]),
        // counters, draw args, dead, alive_read, alive_write, particle, render, effects.
        ("particle_sim.comp", &[0, 2, 3, 4, 5, 6, 7, 9]),
        // render records + the camera UBO (set 0).
        ("particle_draw.vs", &[0, 1]),
        // the bindless texture array + its sampler (set 1).
        ("particle_draw.fs", &[0, 1]),
    ];
    for (stem, want) in expected {
        let c = census_of(stem).expect("spirv-dis resolved above");
        assert_eq!(
            c.binding_set, want,
            "{stem}'s declared-and-read binding set moved; census: {c:?}"
        );
    }
}
