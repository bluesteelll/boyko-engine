//! VG R3 piece 1 step P1-3: the `hzb_build.comp.spv` byte-identity gate **and its module census**.
//!
//! Two independent things are checked, and — as in `vb_batch_cull_spv_sync.rs`, whose shape this
//! file follows — the second is the one that matters.
//!
//! * **(a) byte identity** — the committed `hzb_build.comp.spv` is the re-DXC of
//!   `hzb_build.comp.hlsl` under the frozen recipe in that file's own header. No `-D` variants, so
//!   no `docs/SHADER-VARIANT-MANIFEST.md` row (the manifest registers `-D` variants only).
//!
//! * **(b) WHAT THE MODULE CONTAINS.** At this step the pyramid is built by nothing and read by
//!   nothing — there is no pipeline, no descriptor set and no pass yet, so **no image anywhere in
//!   this repository can move if this shader is wrong.** That is not a reason to gate it later; it
//!   is the reason the gate has to be structural. Every pin below states a property that the
//!   FUNCTIONAL gates (G3 at step P1-7, G8 at P1-8) would eventually catch, caught here instead at
//!   the artifact, one step after the source was written rather than five.
//!
//! # The pins, and why each is a claim rather than a count
//!
//! Most of the census exists to detect change. Six entries do more than that, and each has its own
//! named assertion so a failure reports the PROPERTY and not "the census drifted":
//!
//! 1. **`OpExtInst == 0` and `OpExtInstImport == 0` — the NaN policy, structurally.** HLSL's
//!    `min(a, b)` lowers to `GLSL.std.450 NMin`/`FMin`, and **NaN under `NMin` does not propagate:
//!    it silently selects the OTHER operand.** This repository has a recorded incident on exactly
//!    that (`clamp(NaN, 0, 1)` collapsing to `0`). `hzb_build.comp.hlsl` therefore spells its
//!    reduce as `isnan` plus a compare-and-select and calls no intrinsic at all — so the module
//!    importing NO extended instruction set is the artifact-level proof that no `NMin` can be
//!    hiding in it. Nothing else in the tree checks this.
//! 2. **`OpControlBarrier == 4`** — the LDS reduce chain writes five levels behind exactly four
//!    barriers (INVARIANT HZB-LDS-DISJOINT). A missing one is a workgroup data race; a race in a
//!    `min` reduce produces a value that is merely too large somewhere, which under reverse-Z is
//!    the direction that DELETES geometry, and on a 512×512 golden it may not move a pixel.
//! 3. **`OpImageWrite == 6`** — one store per destination mip. A level nobody writes keeps the boot
//!    clear `0.0`, which under reverse-Z is the FAR plane: the pyramid would report "nothing is
//!    there" at that level and occlude nothing, invisibly.
//! 4. **Both variant arms survive.** `hzb_build` has ONE entry point with a uniform branch on
//!    `pc.base_level == 0`: the base arm fetches the source depth, the reduce arm reads mip `d-1`.
//!    Rung R2d-3 MEASURED that DXC **strips a declared-but-unloaded resource**, so `gSrcDepth` and
//!    `gFine` appearing in the binding set — and one `OpImageFetch` beside one `OpImageRead` — is
//!    what states that both arms compiled rather than one being folded away.
//! 5. **`OpUDiv == 4`** — the base map's four `first_source` divisions (`x_lo`, `x_hi`, `y_lo`,
//!    `y_hi`). The map is `⌈t·S/P⌉` in INTEGERS and its denominator `P` is a push constant, so it
//!    cannot lawfully become a shift. Were one substituted, the map would still be exact at every
//!    power-of-two extent — which is every golden pin in this tree — and wrong at 7×3, 511×1023 and
//!    1920×1080.
//! 6. **The push-constant member offsets.** `HzbBuildPush` is the ONE contract between this shader
//!    and the host that no other test can see. A drift makes the shader read a level extent from
//!    the wrong bytes; the likely outcome is a pyramid that writes nothing (every extent test
//!    fails) or reduces over the wrong footprint, with no validation message either way.
//!
//! SKIPS (with an eprintln) when no `dxc` / `spirv-dis` resolves on the host — the byte gate is
//! only as hermetic as the pinned VulkanSDK 1.4.350.0 toolchain that produced the committed
//! artifact. The fixture control at the bottom runs unconditionally and cannot skip.

use std::path::PathBuf;
use std::process::Command;

/// The shaders directory (`CARGO_MANIFEST_DIR/shaders`), where the committed `.hlsl` and `.spv`
/// live (and where DXC must run so any `#include` resolves).
fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Locates the `dxc` executable: pinned Vulkan-SDK path, then `$VULKAN_SDK/Bin`, then `PATH`.
/// `vb_batch_cull_spv_sync.rs`'s `find_dxc` verbatim.
fn find_dxc() -> Option<PathBuf> {
    find_tool("dxc")
}

/// Locates `spirv-dis` the same layered way.
fn find_spirv_dis() -> Option<PathBuf> {
    find_tool("spirv-dis")
}

/// The shared three-step lookup behind both finders above: the pinned SDK path, then
/// `$VULKAN_SDK/Bin`, then bare on `PATH`.
fn find_tool(stem: &str) -> Option<PathBuf> {
    let bare = if cfg!(windows) { format!("{stem}.exe") } else { stem.to_string() };
    let pinned = PathBuf::from(format!("C:/VulkanSDK/1.4.350.0/Bin/{bare}"));
    if pinned.exists() {
        return Some(pinned);
    }
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let candidate = PathBuf::from(sdk).join("Bin").join(&bare);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if Command::new(&bare).arg("--version").output().is_ok() {
        return Some(PathBuf::from(bare));
    }
    None
}

/// Re-DXCs `hzb_build.comp.hlsl` under the EXACT frozen recipe pinned in its header
/// (`-spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3`, no `-O`) into a fresh temp `.spv`, and
/// returns the bytes. Never overwrites the committed artifact.
fn redxc(dxc: &PathBuf, dir: &PathBuf) -> Vec<u8> {
    let out_spv = std::env::temp_dir().join("hzb_build.comp.redxc.spv");
    let status = Command::new(dxc)
        .current_dir(dir)
        .args([
            "-spirv",
            "-T",
            "cs_6_0",
            "-E",
            "main",
            "-fspv-target-env=vulkan1.3",
            "hzb_build.comp.hlsl",
            "-Fo",
        ])
        .arg(&out_spv)
        .status()
        .expect("invariant: dxc was located and must run");
    assert!(status.success(), "dxc failed re-compiling hzb_build.comp.hlsl under the frozen recipe");
    let bytes = std::fs::read(&out_spv).expect("invariant: dxc wrote the re-DXC .spv");
    let _ = std::fs::remove_file(&out_spv); // best-effort tidy
    bytes
}

/// Disassembles `spv_path` via `spirv-dis`. Panics on a non-zero exit — a malformed committed
/// `.spv` is a build-integrity bug, not a skip.
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

/// The module census. Every field is counted by EXACT whole-token match on a whitespace-split
/// line — see [`the_hzb_census_uses_whole_token_matching`] for the near-misses that make this
/// non-negotiable rather than stylistic. (`OpExtInst` is a strict PREFIX of `OpExtInstImport`, and
/// that pair alone would invert this file's strongest pin under substring matching.)
#[derive(Debug, PartialEq, Eq)]
struct SpvCensus {
    /// ⚠️ THE NaN PIN, half one. Any use of an extended instruction set — which is where `NMin`
    /// lives. Must be zero.
    op_ext_inst: usize,
    /// ⚠️ THE NaN PIN, half two: the IMPORT itself. Stated separately from the uses because a
    /// module can import `GLSL.std.450` and not use it, and the claim this file makes is the
    /// stronger one — the set is not even reachable.
    op_ext_inst_import: usize,
    /// ⚠️ The LDS chain's barrier count. Exactly four (INVARIANT HZB-LDS-DISJOINT).
    op_control_barrier: usize,
    /// ⚠️ One store per destination mip: exactly six.
    op_image_write: usize,
    /// The BASE arm's source tap (`gSrcDepth.Load`). Non-zero ⟺ the arm compiled.
    op_image_fetch: usize,
    /// The REDUCE arm's fine tap (`gFine[...]`, a storage-image read). Non-zero ⟺ the arm compiled.
    op_image_read: usize,
    /// ⚠️ The base map's four integer divisions.
    op_udiv: usize,
    /// The `isnan(a) || isnan(b)` half of every `hzb_min`: two per site.
    op_is_nan: usize,
    /// The `b < a` half of every `hzb_min`: one per site. Half of [`Self::op_is_nan`] by
    /// construction, which the census's shape makes checkable rather than assumed.
    op_ford_less_than: usize,
    /// The `b < a ? b : a` selection of every `hzb_min`.
    op_select: usize,
    /// Must be zero: this pass has no atomics. It compacts nothing, allocates no slot and shares no
    /// counter — every write is to a texel this lane alone owns. An atomic appearing here would
    /// mean someone made the reduce shared state.
    op_atomic_iadd: usize,
    /// The declared workgroup size, read off `OpExecutionMode ... LocalSize <x> <y> <z>`.
    local_size: [usize; 3],
    /// The module's DECLARED BINDING SET — every `OpDecorate %x Binding <n>`, sorted and deduped.
    /// Eight entries: the source tap, the fine tap and the six destinations.
    binding_set: Vec<usize>,
    /// ⚠️ Every `OpMemberDecorate %type_PushConstant_HzbBuildPush <i> Offset <n>`, in member order.
    /// THE host/shader push layout contract.
    push_offsets: Vec<usize>,
    /// The declared `OpCapability` names, sorted. Exactly `["Shader"]` — see the named assertion.
    capabilities: Vec<String>,
    /// The TOTAL `groupshared` word count: the sum over EVERY Workgroup-storage variable of its
    /// array length. 340 floats = 1360 bytes, in one array.
    ///
    /// A SUM rather than "the array's length", because the pin's stated purpose is to catch a
    /// reshaping of the LDS chain and the natural way to reshape it is to add a SECOND
    /// `groupshared` array. Reading one variable — the last one, at that — would leave `340` green
    /// while total shared memory grew.
    lds_words: usize,
    /// The number of Workgroup-storage variables. Pinned at 1 beside [`Self::lds_words`]: the sum
    /// alone cannot distinguish one 340-word array from two whose lengths happen to add to 340,
    /// and the region offsets the reduce chain indexes are only meaningful inside ONE array.
    lds_vars: usize,
    /// Every `OpLoopMerge`'s loop-control token, in emission order. The base-texel loop carries
    /// `Unroll`; see the named assertion for what was measured about it.
    loop_controls: Vec<String>,
}

/// Every `OpConstant %uint <n>` the module declares, sorted and deduped.
///
/// Deliberately NOT a [`SpvCensus`] field. As a whole-set pin it would churn on every unrelated
/// edit (it currently holds the LDS offsets, the loop bounds and the level indices); its value is
/// as a targeted QUERY, which is what the tile assertion uses it for.
fn declared_uint_constants(dis: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for line in dis.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        // `%uint_32 = OpConstant %uint 32` — take the trailing literal, never the `%uint_32` name,
        // so a renamed id cannot inject a phantom value.
        if toks.get(1) == Some(&"=")
            && toks.get(2) == Some(&"OpConstant")
            && toks.get(3) == Some(&"%uint")
            && let Some(v) = toks.get(4).and_then(|t| t.parse::<usize>().ok())
        {
            out.push(v);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Counts the census tokens in a `spirv-dis` disassembly by whole-token match.
fn census(dis: &str) -> SpvCensus {
    let mut c = SpvCensus {
        op_ext_inst: 0,
        op_ext_inst_import: 0,
        op_control_barrier: 0,
        op_image_write: 0,
        op_image_fetch: 0,
        op_image_read: 0,
        op_udiv: 0,
        op_is_nan: 0,
        op_ford_less_than: 0,
        op_select: 0,
        op_atomic_iadd: 0,
        local_size: [0; 3],
        binding_set: Vec::new(),
        push_offsets: Vec::new(),
        capabilities: Vec::new(),
        lds_words: 0,
        lds_vars: 0,
        loop_controls: Vec::new(),
    };

    for line in dis.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        for tok in &toks {
            match *tok {
                "OpExtInst" => c.op_ext_inst += 1,
                "OpExtInstImport" => c.op_ext_inst_import += 1,
                "OpControlBarrier" => c.op_control_barrier += 1,
                "OpImageWrite" => c.op_image_write += 1,
                "OpImageFetch" => c.op_image_fetch += 1,
                "OpImageRead" => c.op_image_read += 1,
                "OpUDiv" => c.op_udiv += 1,
                "OpIsNan" => c.op_is_nan += 1,
                "OpFOrdLessThan" => c.op_ford_less_than += 1,
                "OpSelect" => c.op_select += 1,
                "OpAtomicIAdd" => c.op_atomic_iadd += 1,
                _ => {}
            }
        }
        // `OpExecutionMode %main LocalSize 16 16 1` — the three extents follow the token.
        if let Some(i) = toks.iter().position(|t| *t == "LocalSize") {
            for axis in 0..3 {
                if let Some(v) = toks.get(i + 1 + axis).and_then(|t| t.parse::<usize>().ok()) {
                    c.local_size[axis] = v;
                }
            }
        }
        // `OpDecorate %gSrcDepth Binding 0` — the number AFTER `Binding`. Whole-token, so the
        // sibling `DescriptorSet 0` cannot contribute a phantom @0.
        if let Some(i) = toks.iter().position(|t| *t == "Binding")
            && let Some(b) = toks.get(i + 1).and_then(|t| t.parse::<usize>().ok())
        {
            c.binding_set.push(b);
        }
        // `OpMemberDecorate %type_PushConstant_HzbBuildPush 3 Offset 24` — scoped to the push
        // struct BY NAME, so a member decoration on any other struct cannot join this list.
        if toks.first() == Some(&"OpMemberDecorate")
            && toks.get(1) == Some(&"%type_PushConstant_HzbBuildPush")
            && let Some(i) = toks.iter().position(|t| *t == "Offset")
            && let Some(off) = toks.get(i + 1).and_then(|t| t.parse::<usize>().ok())
        {
            c.push_offsets.push(off);
        }
        if toks.first() == Some(&"OpCapability")
            && let Some(name) = toks.get(1)
        {
            c.capabilities.push((*name).to_string());
        }
        // `%gs = OpVariable %_ptr_Workgroup__arr_float_uint_340 Workgroup` names its element count
        // in the pointer type's own name, so the length is recovered from the declaration rather
        // than by chasing the type graph. ACCUMULATED over every Workgroup variable, never
        // overwritten — see `lds_words`' doc for the second-array hole that overwriting leaves.
        if toks.last() == Some(&"Workgroup")
            && let Some(i) = toks.iter().position(|t| *t == "OpVariable")
            && let Some(ty) = toks.get(i + 1)
        {
            c.lds_vars += 1;
            if let Some(tail) = ty.rsplit('_').next()
                && let Ok(n) = tail.parse::<usize>()
            {
                c.lds_words += n;
            }
        }
        // `OpLoopMerge %90 %88 Unroll` — the loop-control token is the LAST one on the line.
        if toks.first() == Some(&"OpLoopMerge")
            && let Some(ctrl) = toks.last()
        {
            c.loop_controls.push((*ctrl).to_string());
        }
    }
    // Sorted + deduped so the pin is on the SET, not on DXC's emission order.
    c.binding_set.sort_unstable();
    c.binding_set.dedup();
    c.capabilities.sort();
    c.capabilities.dedup();
    c
}

/// Gate (a): the committed artifact byte-equals its own re-DXC under the frozen recipe.
#[test]
fn hzb_build_spv_byte_identical() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "hzb_build_spv_sync: dxc not found (no C:/VulkanSDK/.../dxc.exe, no $VULKAN_SDK/Bin, \
             not on PATH) — SKIPPING the re-DXC byte-identity check on this host."
        );
        return;
    };
    let dir = shaders_dir();
    let committed_path = dir.join("hzb_build.comp.spv");
    let committed = std::fs::read(&committed_path)
        .unwrap_or_else(|e| panic!("missing committed {}: {e}", committed_path.display()));
    let fresh = redxc(&dxc, &dir);
    assert!(
        committed == fresh,
        "hzb_build.comp.spv ({} bytes committed, {} bytes fresh) is NOT the re-DXC of \
         hzb_build.comp.hlsl under the frozen recipe — either the committed .spv is stale (re-run \
         the recipe in the shader's header and commit it) or the host dxc is not the pinned \
         VulkanSDK 1.4.350.0 toolchain.",
        committed.len(),
        fresh.len(),
    );
}

/// Gate (b): the module carries the pyramid build this step designed — the NaN-safe reduce, the
/// four-barrier LDS chain, six destination stores, both variant arms, the integer base map and the
/// push layout the host will mirror.
///
/// Every count below was READ OFF the built artifact, never predicted. The rule the cull's census
/// states applies here verbatim: do not edit these literals to make a failing run pass — they say
/// what the module DOES, and a change in them is a change in the pyramid.
#[test]
fn hzb_build_module_carries_the_pyramid_build() {
    let Some(spirv_dis) = find_spirv_dis() else {
        eprintln!(
            "hzb_build_spv_sync: spirv-dis not found — SKIPPING the module census on this host. \
             NOTHING about what the pyramid build contains is checked by this run."
        );
        return;
    };
    let dir = shaders_dir();
    let committed_path = dir.join("hzb_build.comp.spv");
    assert!(committed_path.exists(), "missing committed {}", committed_path.display());
    let actual = census(&disassemble(&spirv_dis, &committed_path));

    // ⚠️ THE STRONGEST SINGLE CHECK IN THE STEP, stated FIRST so it names itself instead of
    // arriving as two differing fields inside a whole-struct diff.
    //
    // `GLSL.std.450 NMin` does not propagate NaN — it silently takes the OTHER operand — so a
    // single `min()` where `hzb_min` belongs would make an unknown depth read as the KNOWN one.
    // Under reverse-Z that is the direction that deletes geometry, and it has no visual tell on any
    // scene whose depth buffer holds no NaN (which is every scene, until one does).
    assert_eq!(
        (actual.op_ext_inst, actual.op_ext_inst_import),
        (0, 0),
        "the module reaches an extended instruction set ({} import(s), {} use(s)). \
         `hzb_build.comp.hlsl` calls NO intrinsic: its reduce is `isnan` plus a compare-and-select, \
         precisely so `NMin`/`FMin` cannot appear. A non-zero here means someone wrote `min(a, b)`, \
         under which a NaN operand is silently DISCARDED rather than propagated.",
        actual.op_ext_inst_import,
        actual.op_ext_inst,
    );

    // ---- The properties, each stated BEFORE the whole-struct compare -------------------------
    //
    // Ordering is deliberate and was MEASURED: with the struct compare first, dropping a barrier
    // reported itself as a 16-field `SpvCensus { … }` diff in which `op_control_barrier: 3` sits
    // among fifteen identical fields, and the named message below — the one that says what a
    // missing barrier DOES — never ran. Each property therefore fires on its own first, and the
    // struct compare stays behind them as the catch-all for everything nobody named.

    assert_eq!(
        actual.op_control_barrier, 4,
        "invariant: the LDS reduce chain is four barriers — one publishing each of the regions for \
         levels d+1..d+4 (INVARIANT HZB-LDS-DISJOINT). A missing barrier is a workgroup data race \
         whose damage is a `min` that came out too LARGE, which under reverse-Z rejects geometry \
         that is visible; an extra one means the chain grew a step this census has not been told \
         about."
    );
    assert_eq!(
        actual.op_image_write,
        boyko_rhi_vulkan::compute::HZB_LEVELS_PER_PASS as usize,
        "invariant: exactly one store per destination mip, `gDst0 .. gDst5`. A level nobody writes \
         keeps the boot clear `0.0` — the FAR plane under reverse-Z — so the pyramid would report \
         that nothing occludes anything, silently and at full speed. Compared against the HOST \
         constant rather than the literal 6, so this is a real cross-language tie for \
         `HZB_LEVELS_PER_PASS` and not two spellings that merely happen to agree."
    );

    // ⚠️ THE TILE, and the reason this assertion is shaped so oddly.
    //
    // What stood here was `actual.local_size[0] * 2 == HZB_BUILD_TILE` — which CANNOT FAIL. The
    // assertion above it already establishes `local_size[0] == HZB_BUILD_LOCAL_SIZE`, and
    // `compute.rs` makes `HZB_BUILD_TILE == HZB_BUILD_LOCAL_SIZE * 2` a COMPILE-TIME truth, so both
    // sides were the same expression and nothing was read from the module at all. A review found
    // it, and the failing case it named is real: set `TILE = 64u` in the shader, leave
    // `[numthreads(16,16,1)]` alone, re-DXC. Byte gate green (the .spv honestly matches the edited
    // source). Census green — every opcode count, the LocalSize, the bindings, the push offsets,
    // the capabilities and the LDS length are untouched. And the shader now writes level `d` at
    // `gid*64 + tid*2` while the host dispatches `ceil(E/32)` groups, so half of every level is
    // never written and keeps the boot clear.
    //
    // No census field can name a "tile": it exists in the module only as the anonymous multipliers
    // `TILE >> k`. So the tie is made through the CONSTANTS those multipliers declare, in both
    // directions — `%uint_TILE` must be present (a SMALLER tile drops it) and `%uint_(2*TILE)` must
    // be absent (a LARGER tile introduces it, since `TILE >> 0` is the largest of the five).
    let consts = declared_uint_constants(&disassemble(&spirv_dis, &committed_path));
    let tile = boyko_rhi_vulkan::compute::HZB_BUILD_TILE as usize;
    assert!(
        consts.contains(&tile) && !consts.contains(&(tile * 2)),
        "invariant: the shader's tile edge must equal the host's HZB_BUILD_TILE ({tile}). The \
         module declares uint constants {consts:?}: it must contain {tile} (the `gid * TILE` \
         multiplier) and must NOT contain {} (which only a doubled tile introduces). This is the \
         only check in the tree that reads the tile out of the module — the host-side \
         `const _: () = assert!(HZB_BUILD_TILE == HZB_BUILD_LOCAL_SIZE * 2)` relates two HOST \
         constants and says nothing about the shader.",
        tile * 2
    );
    assert!(
        actual.op_image_fetch > 0 && actual.op_image_read > 0,
        "invariant: BOTH arms of the `pc.base_level == 0` branch must survive compilation — the \
         base arm's source `OpImageFetch` and the reduce arm's `OpImageRead` on mip d-1. One at \
         zero means an arm folded away, and since the arm's binding would then be stripped too \
         (DXC strips a declared-but-unloaded resource — MEASURED at rung R2d-3) the pass would \
         either build only level 0 or only levels 1+."
    );
    assert_eq!(
        actual.op_udiv, 4,
        "invariant: the base map keeps its FOUR integer divisions (`x_lo`, `x_hi`, `y_lo`, `y_hi` \
         of `hzb_first_source`). `⌈t·S/P⌉` divides by a PUSH CONSTANT and cannot lawfully become a \
         shift; a substituted shift stays exact at every power-of-two extent — which is every \
         golden pin in this tree — and is wrong at 7×3, 511×1023 and 1920×1080."
    );
    assert_eq!(
        actual.op_is_nan,
        2 * actual.op_ford_less_than,
        "invariant: every `hzb_min` site is TWO `isnan` tests and ONE `b < a`. A ratio other than \
         2:1 means some site lost its NaN guard while keeping the comparison — the half that turns \
         an unknown depth into a rejecting one."
    );
    assert_eq!(
        actual.op_atomic_iadd, 0,
        "invariant: the pyramid build shares nothing. Every texel is written by exactly one lane, \
         so an atomic appearing here means someone made the reduce shared state and changed the \
         cost model of the whole pass without changing any image."
    );
    assert_eq!(
        (actual.lds_words, actual.lds_vars),
        (340, 1),
        "invariant: ONE `groupshared` array of 340 floats — 256 + 64 + 16 + 4, the DISJOINT regions \
         for levels d+1..d+4. 256 alone is the single-reused-region layout whose steps RACE (a lane \
         reads the entry another lane is overwriting), and it is the shape someone 'simplifying' \
         this would reach for. The variable COUNT is pinned beside the total because the sum alone \
         cannot tell one 340-word array from two that add to 340, and the region offsets the chain \
         indexes are only meaningful inside one array."
    );
    assert_eq!(
        actual.loop_controls,
        vec![
            "Unroll".to_string(),
            "None".to_string(),
            "None".to_string(),
            "None".to_string(),
            "None".to_string()
        ],
        "invariant: the level-`d` loop keeps its `Unroll` loop control and the other four keep \
         `None`. This was MEASURED to be the ENTIRE difference the `[unroll]` attribute makes: \
         removing it changes `OpLoopMerge %90 %88 Unroll` to `... None` and moves nothing else in \
         15856 bytes. So the attribute does not unroll anything in DXC — it records the request for \
         the DRIVER's backend, which is where the unroll happens and where `q[4]`'s dynamic \
         indexing gets promoted out of function-scope memory. It was once read as decoration and \
         deleted; this pin is why that is now a red test rather than a silent handover."
    );

    // Stated separately because it is a HOST/SHADER CONTRACT, not a property of the module alone.
    // The host dispatches `ceil(E_a(d) / HZB_BUILD_TILE)` groups of `HZB_BUILD_LOCAL_SIZE²` threads,
    // each thread owning a 2×2 block. A `[numthreads]` narrower than the host believes leaves tail
    // texels unvisited — holding the boot clear `0.0`, the far plane — and a wider one runs lanes
    // whose output texels the extent test discards.
    assert_eq!(
        actual.local_size,
        [
            boyko_rhi_vulkan::compute::HZB_BUILD_LOCAL_SIZE as usize,
            boyko_rhi_vulkan::compute::HZB_BUILD_LOCAL_SIZE as usize,
            1
        ],
        "invariant: the shader's [numthreads] must equal the host's HZB_BUILD_LOCAL_SIZE on both \
         axes"
    );

    // What the capability list proves is what is ABSENT from it, which is why it is pinned as a
    // whole rather than probed for one entry:
    //   * no `StorageImageWriteWithoutFormat` — every RW view carries `[[vk::image_format("r32f")]]`,
    //     so the writes need no optional device feature;
    //   * no `StorageImageArrayDynamicIndexing` — the six destinations are six separate bindings
    //     rather than an array DXC might have declined to unroll;
    //   * no `Int64` — the base map's overflow argument is carried by the `t == p` case in 32-bit
    //     arithmetic, not by widening.
    // Each absence is a device feature this pass does NOT require, and each would otherwise be
    // discovered at pipeline creation on some other machine.
    assert_eq!(
        actual.capabilities,
        vec!["Shader".to_string()],
        "invariant: the pyramid build requires the BASE capability and nothing else. A new entry \
         here is a new device-feature dependency — and it would surface as a pipeline-creation \
         failure on hardware this machine cannot test."
    );

    // ---- …and the catch-all, for every count no property above names ------------------------
    let expected = SpvCensus {
        // ---- MEASURED off the built artifact --------------------------------------------------
        op_ext_inst: 0,
        op_ext_inst_import: 0,
        // The four barriers of the LDS chain: levels d+2, d+3, d+4 and d+5 each read a region the
        // previous step published.
        op_control_barrier: 4,
        // One per destination mip — DERIVED from the host constant, so the field is a tie and not a
        // coincidence.
        op_image_write: boyko_rhi_vulkan::compute::HZB_LEVELS_PER_PASS as usize,
        // The base arm's `gSrcDepth.Load` and the reduce arm's `gFine[...]`. Both non-zero is what
        // states that BOTH arms of the uniform branch survived compilation.
        //
        // ⚠️ These do NOT corroborate the rolled `main` loop, though an earlier comment here said
        // they did. They share the FULL-INLINING premise with the 17-derivation below: had DXC kept
        // `hzb_base_texel`/`hzb_fine_texel` as `OpFunction`, the fetch and the read would each
        // appear once inside the callee however many times an unrolled loop called it.
        // `op_image_write` is the count that needs no such premise — that store is written directly
        // in `main`.
        op_image_fetch: 1,
        op_image_read: 1,
        // `x_lo`, `x_hi`, `y_lo`, `y_hi` — the four `hzb_first_source` divisions.
        op_udiv: 4,
        // Two per `hzb_min` site …
        op_is_nan: 34,
        // … and one `b < a` per site, so exactly half of the above.
        //
        // 17 is MEASURED, but it is also DERIVABLE, and the derivation is written down because a
        // number one can only measure is a number nobody can reason about when it moves:
        //   1  `hzb_base_texel` — its source loop has DYNAMIC bounds (`x_lo .. x_hi`), so it rolls
        //   1  `hzb_fine_texel` — constant-bound, but DXC kept it rolled too
        //   3  the per-thread fold of `q[0..4]` into the level-`d+1` value
        //  12  `hzb_fold_lds` inlined at its four call sites, 3 apiece
        // The `main` loop over `i < 4` also stayed ROLLED, and the field that corroborates that
        // INDEPENDENTLY is `op_image_write == 6`: `gDst0[t] = m` sits directly in that loop and is
        // the only write to `gDst0`, so an unrolled body would read 4 + 5 = 9.
        op_ford_less_than: 17,
        // … lowered as a select rather than a branch at every site.
        op_select: 17,
        // No atomics: every write is to a texel this lane alone owns.
        op_atomic_iadd: 0,
        // NOT a measurement: READ FROM the host constant, so this field states the CONTRACT. The
        // separately-named assertion below is what reports a violation.
        local_size: [
            boyko_rhi_vulkan::compute::HZB_BUILD_LOCAL_SIZE as usize,
            boyko_rhi_vulkan::compute::HZB_BUILD_LOCAL_SIZE as usize,
            1,
        ],
        // All eight: @0 the source tap, @1 the fine tap, @2..@7 the six destinations. DXC strips a
        // declared-but-unloaded resource (MEASURED at rung R2d-3), so a SHORT set here is the
        // signature of an arm that compiled away.
        //
        // DERIVED (`2 + HZB_LEVELS_PER_PASS`) rather than spelled `[0..=7]`, for the same reason
        // `op_image_write` is: a literal beside a host constant is two spellings that agree today,
        // not a tie.
        binding_set: (0..(2 + boyko_rhi_vulkan::compute::HZB_LEVELS_PER_PASS as usize)).collect(),
        // ⚠️ THE HOST CONTRACT. Eight `uint2` back to back, then two `uint`.
        push_offsets: vec![0, 8, 16, 24, 32, 40, 48, 56, 64, 68],
        // Exactly one. See the named assertion below for what each ABSENT capability proves.
        capabilities: vec!["Shader".to_string()],
        // 340 floats = 1360 bytes: 256 + 64 + 16 + 4 for levels d+1 .. d+4 in disjoint regions,
        // in exactly ONE array.
        lds_words: 340,
        lds_vars: 1,
        // The level-`d` loop's `[unroll]`, then the four `hzb_min` / fold loops DXC left alone.
        loop_controls: vec![
            "Unroll".to_string(),
            "None".to_string(),
            "None".to_string(),
            "None".to_string(),
            "None".to_string(),
        ],
    };
    assert_eq!(
        actual, expected,
        "hzb_build.comp.spv's census diverged. Expected {expected:?}, got {actual:?}."
    );
}

/// FIXTURE CONTROL for [`census`]'s selectors, and it is not decorative.
///
/// # The near-miss that inverts this file's strongest pin
///
/// **`OpExtInst` is a strict PREFIX of `OpExtInstImport`.** A substring selector would therefore
/// count the import as a use — and, worse, the pin is `== 0` in BOTH directions, so a module that
/// imports `GLSL.std.450` and calls `NMin` a hundred times would be reported by a broken selector
/// as … also failing, for the wrong reason, while a module that imports the set and uses it zero
/// times would be reported as non-zero. The whole-token form is what separates "reaches the set"
/// from "declares it".
///
/// Runs unconditionally — no `dxc` / `spirv-dis`, so it cannot SKIP the way the artifact gates do.
#[test]
fn the_hzb_census_uses_whole_token_matching() {
    // ---- The prefix pair, both directions. ----
    let import_only = "               %1 = OpExtInstImport \"GLSL.std.450\"\n";
    assert_eq!(
        census(import_only).op_ext_inst,
        0,
        "`OpExtInstImport` was counted as an `OpExtInst`; the NaN pin's two halves would then be \
         unable to tell a declared set from a reachable one"
    );
    assert_eq!(
        census(import_only).op_ext_inst_import,
        1,
        "the selector missed a REAL `OpExtInstImport`; the import half of the NaN pin would be \
         satisfied by blindness"
    );
    let real_use = "         %42 = OpExtInst %float %1 NMin %40 %41\n";
    assert_eq!(
        census(real_use).op_ext_inst,
        1,
        "the selector missed a REAL `OpExtInst` — the `NMin` this whole file exists to exclude"
    );
    assert_eq!(census(real_use).op_ext_inst_import, 0, "an `OpExtInst` use is not an import");

    // ---- The barrier pin must not be satisfied by a MEMORY barrier. ----
    assert_eq!(
        census("               OpControlBarrier %uint_2 %uint_2 %uint_264\n").op_control_barrier,
        1,
        "the selector missed a REAL `OpControlBarrier`; the four-barrier invariant would be \
         satisfied by blindness"
    );
    assert_eq!(
        census("               OpMemoryBarrier %uint_1 %uint_72\n").op_control_barrier,
        0,
        "`OpMemoryBarrier` was counted as a control barrier; the invariant is about workgroup \
         SYNCHRONISATION, not memory ordering, and a chain with four memory barriers and no \
         control barrier races"
    );

    // ---- The image ops are three DIFFERENT claims and must not blur into each other. ----
    let write = "               OpImageWrite %53 %52 %51\n";
    let fetch = "         %60 = OpImageFetch %v4float %59 %58 Lod %int_0\n";
    let read = "         %70 = OpImageRead %v4float %69 %68\n";
    assert_eq!(census(write).op_image_write, 1, "the selector missed a REAL `OpImageWrite`");
    assert_eq!(census(write).op_image_read, 0, "`OpImageWrite` must not count as a read");
    assert_eq!(census(fetch).op_image_fetch, 1, "the selector missed the BASE arm's `OpImageFetch`");
    assert_eq!(census(fetch).op_image_read, 0, "a fetch is not a storage-image read");
    assert_eq!(census(read).op_image_read, 1, "the selector missed the REDUCE arm's `OpImageRead`");
    assert_eq!(census(read).op_image_fetch, 0, "a storage-image read is not a fetch");

    // ---- The division pin must not be satisfied by the SIGNED opcode. ----
    assert_eq!(
        census("         %80 = OpUDiv %uint %78 %79\n").op_udiv,
        1,
        "the selector missed a REAL `OpUDiv`; the integer-base-map pin would be vacuous"
    );
    assert_eq!(
        census("         %80 = OpSDiv %int %78 %79\n").op_udiv,
        0,
        "`OpSDiv` was counted as the unsigned base-map division; the map is UNSIGNED and a signed \
         one would round toward zero on a negative it can never legitimately see"
    );

    // ---- The comparison pin, and the longer mnemonic that contains it. ----
    assert_eq!(
        census("         %33 = OpFOrdLessThan %bool %31 %32\n").op_ford_less_than,
        1,
        "the selector missed a REAL `OpFOrdLessThan`"
    );
    assert_eq!(
        census("         %33 = OpFOrdLessThanEqual %bool %31 %32\n").op_ford_less_than,
        0,
        "`OpFOrdLessThanEqual` false-matched `OpFOrdLessThan`; a reduce spelled with `<=` would \
         then read as the `<` the oracle uses, and the two disagree on which of two EQUAL operands \
         is returned — invisible in a `min`, but the same selector guards the ordering claim"
    );
    // `OpIsNan` and `OpIsInf` are different questions about a float.
    assert_eq!(census("         %20 = OpIsNan %bool %19\n").op_is_nan, 1, "missed a real `OpIsNan`");
    assert_eq!(
        census("         %20 = OpIsInf %bool %19\n").op_is_nan,
        0,
        "`OpIsInf` was counted as a NaN test — and `+INFINITY` is the min IDENTITY this shader \
         writes deliberately, so conflating the two would report the identity as an unknown depth"
    );

    // ---- LocalSize reads all three extents, in order, and is not a constant. ----
    assert_eq!(
        census("               OpExecutionMode %main LocalSize 16 16 1\n").local_size,
        [16, 16, 1],
        "the selector missed the real LocalSize triple"
    );
    assert_eq!(
        census("               OpExecutionMode %main LocalSize 8 4 2\n").local_size,
        [8, 4, 2],
        "the selector is returning a constant rather than reading the three extents in order — a \
         16×16 pin would then be satisfied by a 4×64 workgroup, which tiles differently and reduces \
         the wrong 2×2 blocks"
    );

    // ---- Bindings: the number after `Binding`, never the sibling `DescriptorSet`. ----
    let decorations = "               OpDecorate %gSrcDepth DescriptorSet 0
               OpDecorate %gSrcDepth Binding 0
               OpDecorate %gDst5 DescriptorSet 0
               OpDecorate %gDst5 Binding 7
";
    assert_eq!(
        census(decorations).binding_set,
        vec![0, 7],
        "the binding-set selector must read the number after `Binding` only — a `DescriptorSet 0` \
         counted as a binding would put a phantom @0 in every module's set and make the \
         both-arms-survived pin unable to see a stripped `gSrcDepth`"
    );
    assert!(
        census("               OpDecorate %x DescriptorSet 3\n").binding_set.is_empty(),
        "a lone `DescriptorSet` decoration declares no binding"
    );

    // ---- The push offsets are SCOPED BY STRUCT NAME and keep MEMBER ORDER. ----
    let members = "               OpMemberDecorate %type_PushConstant_HzbBuildPush 0 Offset 0
               OpMemberDecorate %type_PushConstant_HzbBuildPush 1 Offset 8
               OpMemberDecorate %SomeOtherStruct 0 Offset 999
               OpMemberDecorate %type_PushConstant_HzbBuildPush 9 Offset 68
";
    assert_eq!(
        census(members).push_offsets,
        vec![0, 8, 68],
        "the push-offset selector must take ONLY `%type_PushConstant_HzbBuildPush` members, in \
         emission order. A member decoration on another struct joining the list would make the \
         host-contract pin fail for a reason that has nothing to do with the push layout"
    );
    // Order-SENSITIVE by design: unlike the binding set, these are positional and must not be
    // sorted — member 3's offset means nothing except as member 3's.
    let swapped = "               OpMemberDecorate %type_PushConstant_HzbBuildPush 0 Offset 8
               OpMemberDecorate %type_PushConstant_HzbBuildPush 1 Offset 0
";
    assert_eq!(
        census(swapped).push_offsets,
        vec![8, 0],
        "the push offsets must be recorded in member order, not sorted — two members whose offsets \
         were SWAPPED are a real layout change and must not compare equal to the correct layout"
    );

    // ---- Capabilities are collected by name. ----
    let caps = "               OpCapability Shader
               OpCapability StorageImageWriteWithoutFormat
";
    assert_eq!(
        census(caps).capabilities,
        vec!["Shader".to_string(), "StorageImageWriteWithoutFormat".to_string()],
        "the capability selector must see EVERY declared capability — the pin's whole value is \
         that a NEW device-feature dependency shows up in it"
    );

    // ---- The LDS length comes out of the Workgroup variable's type name. ----
    assert_eq!(
        census("         %gs = OpVariable %_ptr_Workgroup__arr_float_uint_340 Workgroup\n")
            .lds_words,
        340,
        "the selector missed the `groupshared` array length"
    );
    assert_eq!(
        census("         %gs = OpVariable %_ptr_Workgroup__arr_float_uint_256 Workgroup\n")
            .lds_words,
        256,
        "the selector is returning a constant rather than reading the array length — a 256-word \
         pin would then be satisfied by the 340-word layout and vice versa, and 256 is exactly the \
         single-region layout whose steps RACE"
    );
    assert_eq!(
        census("         %x = OpVariable %_ptr_Private_float Private\n").lds_words,
        0,
        "a Private-storage variable is not the `groupshared` array"
    );
    // ⚠️ The hole a review found: the selector used to OVERWRITE on each match, so a SECOND
    // `groupshared` array — the natural way to extend the reduce chain — left the 340 pin green
    // while total shared memory grew. It accumulates now, and the variable count is pinned beside
    // the sum so two arrays adding to 340 cannot impersonate one.
    let two_arrays = "         %gs = OpVariable %_ptr_Workgroup__arr_float_uint_340 Workgroup
         %gs2 = OpVariable %_ptr_Workgroup__arr_float_uint_64 Workgroup
";
    assert_eq!(
        (census(two_arrays).lds_words, census(two_arrays).lds_vars),
        (404, 2),
        "the LDS selector must SUM over every Workgroup variable and count them. Overwriting would \
         report 64 here — a SMALLER number than the single-array layout — so a second array would \
         not merely slip past the pin, it would look like a shrink"
    );

    // ---- The loop-control selector reads the token that `[unroll]` actually moves. ----
    assert_eq!(
        census("               OpLoopMerge %90 %88 Unroll\n").loop_controls,
        vec!["Unroll".to_string()],
        "the selector missed a real `Unroll` loop control — the one token the `[unroll]` attribute \
         changes in the whole module"
    );
    assert_eq!(
        census("               OpLoopMerge %90 %88 None\n").loop_controls,
        vec!["None".to_string()],
        "the selector must distinguish `None` from `Unroll`, or dropping the attribute reads as no \
         change at all"
    );
    assert!(
        census("               OpSelectionMerge %30 None\n").loop_controls.is_empty(),
        "`OpSelectionMerge` also ends in `None` and must not be read as a LOOP control — every `if` \
         in the shader emits one, which would bury the single `Unroll` in a list of them"
    );

    // ---- The two selectors that had no fixture, and one of them is the zero-valued pin. ----
    //
    // `op_atomic_iadd == 0` is this file's only pin whose expected value is zero and whose selector
    // was untested — precisely the "satisfied by blindness" failure named three times above.
    assert_eq!(
        census("         %51 = OpAtomicIAdd %uint %50 %uint_1 %uint_0 %uint_1\n").op_atomic_iadd,
        1,
        "the selector missed a REAL `OpAtomicIAdd`; the no-atomics invariant would be green on a \
         module that had made the reduce shared state"
    );
    assert_eq!(
        census("         %51 = OpAtomicIIncrement %uint %50 %uint_1 %uint_0\n").op_atomic_iadd,
        0,
        "`OpAtomicIIncrement` is a different instruction and must not satisfy the pin either way"
    );
    assert_eq!(
        census("         %42 = OpSelect %uint %41 %40 %uint_0\n").op_select,
        1,
        "the selector missed a REAL `OpSelect`"
    );
    assert_eq!(
        census("               OpSelectionMerge %30 None\n").op_select,
        0,
        "`OpSelectionMerge` has `OpSelect` as a strict prefix — the same trap `vb_batch_cull`'s \
         census records, where a substring selector read 3 on a module carrying zero real selects"
    );

    // ---- `declared_uint_constants` — the tile tie's only source. ----
    let consts = "       %uint_32 = OpConstant %uint 32
       %uint_16 = OpConstant %uint 16
       %uint_32 = OpConstant %uint 32
        %int_64 = OpConstant %int 64
";
    assert_eq!(
        declared_uint_constants(consts),
        vec![16, 32],
        "the constant selector must take the TRAILING literal of an `OpConstant %uint`, sorted and \
         deduped, and must ignore a signed constant — a `%int_64` counted here would make the \
         tile's `must NOT contain 2*TILE` half fire on a module that is perfectly correct"
    );
    assert!(
        declared_uint_constants("       %uint_32 = OpTypeInt 32 0\n").is_empty(),
        "an `OpTypeInt` line mentions 32 twice and declares no constant; reading it would inject a \
         phantom tile constant into every module"
    );
}
