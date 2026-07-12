//! Textured-PBR rung T6c — the NonUniformResourceIndex SPIR-V descriptor-indexing PROOF.
//!
//! A hermetic (no-GPU) test: loads the COMMITTED `shaders/gbuffer_mrt_tex.fs.spv` and walks
//! its raw SPIR-V word stream, asserting DXC actually emitted the descriptor-indexing
//! machinery the bindless sample (`gTextures[NonUniformResourceIndex(slot)].Sample(...)`,
//! `gbuffer_mrt.fs.hlsl`'s `#ifdef TEXTURED` block) requires:
//!
//! 1. `OpCapability ShaderNonUniform` (enum 5301) + `OpCapability RuntimeDescriptorArray`
//!    (enum 5302) — the two capabilities `SPV_EXT_descriptor_indexing` promotes into SPIR-V
//!    1.5/Vulkan 1.2 core, required for a runtime-sized `Texture2D[]` indexed by a
//!    non-uniform value.
//! 2. At least one `OpDecorate <id> NonUniform` (decoration enum 5300) — DXC's lowering of
//!    `NonUniformResourceIndex(slot)`.
//!
//! Ground truth for the enum numbers was extracted directly from the compiled binary (not
//! from memory) — see the T6c developer report. Compiled offline with:
//!   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//!       -fspv-target-env=vulkan1.3 -D TEXTURED=1 gbuffer_mrt.fs.hlsl -Fo gbuffer_mrt_tex.fs.spv
//! Disassembly spot-check: `spirv-dis gbuffer_mrt_tex.fs.spv | grep NonUniform` shows both the
//! `OpCapability` lines and 20 `OpDecorate %N NonUniform` lines (one per bindless `.Sample`
//! call site the TEXTURED fragment emits — albedo/normal/metal-rough/AO/emissive across the
//! gAlbedo/gNormal/gPbr writes).

/// The SPIR-V magic number (little-endian word order — the ONLY byte order DXC emits on this
/// toolchain; `boyko_rhi_vulkan::compute::SpirvBlob` makes the same little-endian assumption
/// reinterpreting `include_bytes!` as `&[u32]`).
const SPIRV_MAGIC: u32 = 0x0723_0203;

/// `OpCapability`'s opcode (SPIR-V spec §3.32.2).
const OP_CAPABILITY: u32 = 17;
/// `OpDecorate`'s opcode (SPIR-V spec §3.32.10).
const OP_DECORATE: u32 = 71;

/// `Capability ShaderNonUniform` (enum 5301 — `SPV_EXT_descriptor_indexing`, promoted to
/// SPIR-V 1.5 core). Ground-truthed against the committed `gbuffer_mrt_tex.fs.spv`.
const CAPABILITY_SHADER_NON_UNIFORM: u32 = 5301;
/// `Capability RuntimeDescriptorArray` (enum 5302). Required for the unbounded
/// `Texture2D gTextures[]` declaration (`[[vk::binding(0, 1)]] Texture2D gTextures[] :
/// register(t0, space1);`).
const CAPABILITY_RUNTIME_DESCRIPTOR_ARRAY: u32 = 5302;
/// `Decoration NonUniform` (enum 5300). DXC's lowering of `NonUniformResourceIndex(slot)`.
const DECORATION_NON_UNIFORM: u32 = 5300;

/// Reads the committed textured gbuffer fragment SPIR-V as a `u32` word stream.
///
/// # Panics
/// Panics if the file is missing/misaligned/truncated — a build-time invariant (the
/// hermetic `.spv` is checked into the repo, mirroring every other `embed_spirv!` asset).
fn textured_fs_words() -> Vec<u32> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/gbuffer_mrt_tex.fs.spv");
    let bytes = std::fs::read(path)
        .expect("invariant: shaders/gbuffer_mrt_tex.fs.spv must exist next to this crate");
    assert!(
        bytes.len().is_multiple_of(4),
        "invariant: a SPIR-V binary is a whole number of 4-byte words"
    );
    bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Walks the SPIR-V instruction stream (skipping the 5-word header:
/// magic/version/generator/bound/schema), invoking `visit(opcode, operand_words)` per
/// instruction. `operand_words` excludes the leading `(word_count << 16) | opcode` word.
fn for_each_instruction(words: &[u32], mut visit: impl FnMut(u32, &[u32])) {
    assert!(
        words.len() >= 5 && words[0] == SPIRV_MAGIC,
        "invariant: the file must be a valid little-endian SPIR-V binary (magic 0x07230203)"
    );
    let mut i = 5usize;
    while i < words.len() {
        let word_count = (words[i] >> 16) as usize;
        let opcode = words[i] & 0xFFFF;
        assert!(
            word_count > 0 && i + word_count <= words.len(),
            "invariant: a well-formed SPIR-V instruction's word count stays in bounds"
        );
        visit(opcode, &words[i + 1..i + word_count]);
        i += word_count;
    }
}

/// The descriptor-indexing PROOF: the committed `gbuffer_mrt_tex.fs.spv` carries BOTH
/// `OpCapability ShaderNonUniform` + `OpCapability RuntimeDescriptorArray` AND at least one
/// `OpDecorate <id> NonUniform` — DXC actually lowered
/// `gTextures[NonUniformResourceIndex(slot)].Sample(...)` into real descriptor-indexing
/// SPIR-V, not a silently-uniform index that would be UB against a non-uniformly-varying
/// bindless slot across a wave.
#[test]
fn textured_fragment_spirv_has_nonuniform_descriptor_indexing() {
    let words = textured_fs_words();

    let mut has_shader_non_uniform_cap = false;
    let mut has_runtime_descriptor_array_cap = false;
    let mut non_uniform_decoration_count = 0u32;

    for_each_instruction(&words, |opcode, operands| {
        if opcode == OP_CAPABILITY {
            let cap = operands[0];
            has_shader_non_uniform_cap |= cap == CAPABILITY_SHADER_NON_UNIFORM;
            has_runtime_descriptor_array_cap |= cap == CAPABILITY_RUNTIME_DESCRIPTOR_ARRAY;
        }
        if opcode == OP_DECORATE {
            // OpDecorate <target-id> <decoration-enum> [<extra operands>...]
            let decoration = operands[1];
            if decoration == DECORATION_NON_UNIFORM {
                non_uniform_decoration_count += 1;
            }
        }
    });

    assert!(
        has_shader_non_uniform_cap,
        "gbuffer_mrt_tex.fs.spv is missing OpCapability ShaderNonUniform (5301) — \
         NonUniformResourceIndex was not lowered into real descriptor-indexing SPIR-V"
    );
    assert!(
        has_runtime_descriptor_array_cap,
        "gbuffer_mrt_tex.fs.spv is missing OpCapability RuntimeDescriptorArray (5302) — \
         the unbounded `Texture2D gTextures[]` declaration did not compile to a runtime array"
    );
    assert!(
        non_uniform_decoration_count > 0,
        "gbuffer_mrt_tex.fs.spv has no OpDecorate <id> NonUniform (5300) — \
         NonUniformResourceIndex(slot) produced no NonUniform-decorated id"
    );
}
