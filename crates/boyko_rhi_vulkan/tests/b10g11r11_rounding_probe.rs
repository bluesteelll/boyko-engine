//! Render wave PX0 (H3 Scope 3 / 3b; `px0/cut.md` PX0-6, D-P1..D-P3) — **THE PROBE**: the device's
//! float-format rounding classified per (format, path), the content arms run on both formats, the
//! model ↔ device agreement per arm, the display metric per arm, and the decision rows applied.
//!
//! The pre-registered model is `tests/lit_format/` (frozen at the model commit — the record's §6
//! prints the freeze and ancestry checks). This file holds only what the device run needs: the
//! upload packing, the dispatches, the classifier and the decision rows; it CALLS the model and
//! never edits it.
//!
//! # Two tests
//!
//! - [`classify_selftest`] (host, not ignored): each class's simulated outputs classify as that
//!   class; a 50/50 `N`/`Z` mixture classifies `Unmodelled` with a maximum error ≥ ½ ULP; an input
//!   set two classes both match is a RED (`Ambiguous`), never a verdict.
//! - [`b10g11r11_rounding_probe`] (`gpu:`): the premises (B10G11R11 and RGBA16F STORAGE,
//!   COLOR_ATTACHMENT, BLEND — each a named RED, AD-1 requires them), the classification on five
//!   paths × two formats, the content arms on both formats, the decision, a TOML block to stdout
//!   and to `$BOYKO_PX0_PROBE_OUT`.
//!
//! The paths, as the device runs them: **store** = the compute kernel's `imageStore`; **attachment**
//! = the raster FS's colour write, no blend; **blend_add** / **blend_over** = the same draw through
//! `ONE + ONE` / premultiplied over onto `dst` uploaded as packed bits; **clear** = an attachment
//! `loadOp = CLEAR` over a ONE-texel render area per value (the route the engine's own `lit` clears
//! take; `px0/critique.md` open question 1).
//!
//! Run (GL-PX0-P): `BOYKO_DISABLE_VALIDATION=1 BOYKO_PX0_PROBE_OUT=<probe.toml> cargo test -p
//! boyko_rhi_vulkan --test b10g11r11_rounding_probe --locked -- --ignored --test-threads=1`.

mod lit_format;

use std::fmt::Write as _;

use boyko_rhi::enums::{BarrierAccess, BarrierStage, DescriptorKind};
use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, BlendState, BufferDesc,
    BufferImageCopy, BufferUsage, ComputePipelineDesc, CullMode, Format, GraphicsPipelineDesc,
    ImageAspect, ImageBarrierDesc, ImageLayout, ImageSubresourceRange, ImageUsage, LoadOp, MemoryLocation,
    PrimitiveTopology, RenderArea, RenderingAttachment, RenderingDesc, RhiCommandEncoder, RhiDevice,
    RhiQueue, ShaderStage, StoreOp, TextureDesc, TextureDimension, Viewport,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_shaderdsl::pixfmt::{Channel, Member, Path, PathClass, Target, classes_for, decode};

use lit_format::{
    Arm, ClassInput, Curve, Family, REGION_H, REGION_PX, REGION_W, arms, background_code, class_inputs,
    decode_image, exact, exact_display, metric, predict, predict_class, simulate,
};

// ---- The classifier (host) --------------------------------------------------------------------------

/// What a path classified as.
#[derive(Clone, Debug, PartialEq)]
enum Verdict {
    /// The unique matching class (an `Unmodelled` class carries its maximum error).
    Class(PathClass),
    /// More than one class matched every output: the input set does not separate them — a RED of
    /// the set, never a device verdict.
    Ambiguous(Vec<PathClass>),
}

/// The ULP of `ch` at `v` (the subnormal spacing below the smallest normal).
fn ulp_at(ch: Channel, v: f64) -> f64 {
    let m = ch.mantissa_bits() as i32;
    let e = if v > 0.0 { v.log2().floor() as i32 } else { -14 };
    2f64.powi(e.max(-14) - m)
}

/// The class of a path from its device outputs over its input set.
fn classify(target: Target, path: Path, inputs: &[ClassInput], outputs: &[[u32; 3]]) -> Verdict {
    let matching: Vec<PathClass> = classes_for(target, path)
        .iter()
        .copied()
        .filter(|c| inputs.iter().zip(outputs).all(|(x, o)| predict_class(target, path, *c, x) == Some(*o)))
        .collect();
    match matching.len() {
        1 => Verdict::Class(matching[0]),
        0 => {
            let chans = target.channels();
            let mut worst = 0.0f64;
            for (x, o) in inputs.iter().zip(outputs) {
                for c in 0..3 {
                    let v = f64::from(x.value[c]);
                    let err = (f64::from(decode(chans[c], o[c])) - v).abs() / ulp_at(chans[c], v);
                    worst = worst.max(err);
                }
            }
            Verdict::Class(PathClass::Unmodelled { max_err_milli_ulp: (worst * 1000.0).ceil() as u32 })
        }
        _ => Verdict::Ambiguous(matching),
    }
}

#[test]
fn classify_selftest() {
    for target in [Target::B10G11R11, Target::Rgba16f] {
        for path in Path::ALL {
            let inputs = class_inputs(target, path);
            for c in classes_for(target, path) {
                let out: Vec<[u32; 3]> = inputs.iter().map(|x| predict_class(target, path, *c, x).expect("modelled")).collect();
                assert_eq!(classify(target, path, &inputs, &out), Verdict::Class(*c), "{target:?}/{path:?}/{c:?}");
            }
            // A 50/50 N / Z mixture is no class: Unmodelled, at least half a ULP off somewhere.
            let mixed: Vec<[u32; 3]> = inputs
                .iter()
                .enumerate()
                .map(|(i, x)| predict(target, path, if i % 2 == 0 { Member::N } else { Member::Z }, x))
                .collect();
            match classify(target, path, &inputs, &mixed) {
                Verdict::Class(PathClass::Unmodelled { max_err_milli_ulp }) => {
                    assert!(max_err_milli_ulp >= 500, "{target:?}/{path:?}: {max_err_milli_ulp}");
                }
                other => panic!("{target:?}/{path:?}: a mixture classified {other:?}"),
            }
        }
    }
    // An input set that does not separate two classes is a RED, not a verdict: the exact midpoints
    // alone tie the same way under N and N16.
    let only_mids: Vec<ClassInput> = class_inputs(Target::B10G11R11, Path::Store).into_iter().step_by(4).collect();
    let out: Vec<[u32; 3]> = only_mids.iter().map(|x| predict(Target::B10G11R11, Path::Store, Member::N, x)).collect();
    assert!(
        matches!(classify(Target::B10G11R11, Path::Store, &only_mids, &out), Verdict::Ambiguous(_)),
        "N and N16 coincide on the midpoints alone: the classifier must say so"
    );
}

// ---- The device --------------------------------------------------------------------------------------

const FEAT_STORAGE: u32 = 0x0000_0002;
const FEAT_COLOR: u32 = 0x0000_0080;
const FEAT_BLEND: u32 = 0x0000_0100;

static STORE_B10_SPV: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/px0_probe_store_r11g11b10f.comp.spv"));
static STORE_F16_SPV: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/px0_probe_store_rgba16f.comp.spv"));
static RASTER_FS_SPV: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/px0_probe_raster.fs.spv"));
static FULLSCREEN_VS_SPV: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/fullscreen_sample.vs.spv"));

fn words(b: &[u8]) -> Vec<u32> {
    b.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

fn format_of(t: Target) -> Format {
    match t {
        Target::B10G11R11 => Format::B10G11R11UfloatPack32,
        Target::Rgba16f => Format::R16G16B16A16Sfloat,
    }
}

fn texel_bytes(t: Target) -> usize {
    match t {
        Target::B10G11R11 => 4,
        Target::Rgba16f => 8,
    }
}

/// Packs three codes into one texel's bytes (RGBA16F alpha = 1.0).
fn pack_codes(t: Target, c: [u32; 3]) -> Vec<u8> {
    match t {
        Target::B10G11R11 => (c[0] | (c[1] << 11) | (c[2] << 22)).to_le_bytes().to_vec(),
        Target::Rgba16f => [c[0] as u16, c[1] as u16, c[2] as u16, 0x3C00].iter().flat_map(|h| h.to_le_bytes()).collect(),
    }
}

/// Unpacks one texel's bytes into three codes.
fn unpack_codes(t: Target, b: &[u8]) -> [u32; 3] {
    match t {
        Target::B10G11R11 => {
            let w = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            [w & 0x7FF, (w >> 11) & 0x7FF, (w >> 22) & 0x3FF]
        }
        Target::Rgba16f => core::array::from_fn(|k| u32::from(u16::from_le_bytes([b[2 * k], b[2 * k + 1]]))),
    }
}

/// The probe's device objects.
struct Rig<'a> {
    ctx: &'a VulkanContext,
}

/// One job's inputs: the image extent, the dst texels to upload first (packed bits; `None` = no
/// upload), and the per-layer float4 inputs.
struct Job<'a> {
    w: u32,
    h: u32,
    dst: Option<Vec<[u32; 3]>>,
    layers: u32,
    input: &'a [[f32; 4]],
}

impl Rig<'_> {
    /// Runs `path` on `target` over `job` and returns the image's codes, row-major.
    #[allow(clippy::too_many_lines)] // one linear recording; splitting it would scatter the barriers
    fn run(&self, target: Target, path: Path, job: &Job<'_>, rmw: bool, clear_values: Option<&[[f32; 3]]>) -> Vec<[u32; 3]> {
        let ctx = self.ctx;
        let queue = ctx.rhi_queue();
        let n = (job.w * job.h) as usize;
        let tb = texel_bytes(target);
        let tex = ctx
            .create_texture(&TextureDesc {
                width: job.w,
                height: job.h,
                depth: 1,
                format: format_of(target),
                dimension: TextureDimension::D2,
                usage: ImageUsage::STORAGE | ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_SRC | ImageUsage::TRANSFER_DST,
                array_layers: 1,
                mip_levels: 1,
                view_format: None,
            })
            .expect("probe target texture");
        let input_bytes: Vec<u8> = job.input.iter().flat_map(|v| v.iter().flat_map(|f| f.to_le_bytes())).collect();
        let input = ctx
            .create_buffer(&BufferDesc {
                size: input_bytes.len().max(16) as u64,
                usage: BufferUsage::STORAGE,
                location: MemoryLocation::HostVisibleCoherent,
            })
            .expect("probe input buffer");
        let p = ctx.buffer_mapped_ptr(&input).expect("mapped input");
        // SAFETY: the mapping is `input_bytes.len()` bytes (at least) of host-coherent memory owned
        // by `input`; no submission reads it yet; the source and the mapping do not overlap.
        unsafe { core::ptr::copy_nonoverlapping(input_bytes.as_ptr(), p.as_ptr(), input_bytes.len()) };
        let staging = ctx
            .create_buffer(&BufferDesc {
                size: (n * tb) as u64,
                usage: BufferUsage::TRANSFER_DST | BufferUsage::TRANSFER_SRC,
                location: MemoryLocation::HostVisibleCoherent,
            })
            .expect("probe staging buffer");
        let sp = ctx.buffer_mapped_ptr(&staging).expect("mapped staging");
        if let Some(dst) = &job.dst {
            let bytes: Vec<u8> = dst.iter().flat_map(|c| pack_codes(target, *c)).collect();
            // SAFETY: the staging mapping is `n * tb` bytes and `bytes` is `dst.len() * tb <= n * tb`;
            // no submission is in flight.
            unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), sp.as_ptr(), bytes.len()) };
        }
        let region = [BufferImageCopy {
            buffer_offset: 0,
            buffer_row_length: 0,
            buffer_image_height: 0,
            aspect: ImageAspect::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
            image_offset_x: 0,
            image_offset_y: 0,
            image_offset_z: 0,
            image_extent_w: job.w,
            image_extent_h: job.h,
            image_extent_d: 1,
        }];
        let fence = ctx.create_fence(false).expect("fence");
        let mut enc = ctx.create_command_encoder().expect("encoder");
        enc.begin().expect("begin");
        let barrier = |enc: &mut _, ss, ds, sa, da, ol, nl| {
            RhiCommandEncoder::image_barrier(
                enc,
                &ImageBarrierDesc { texture: &tex, src_stage: ss, dst_stage: ds, src_access: sa, dst_access: da, old_layout: ol, new_layout: nl, range: ImageSubresourceRange::COLOR },
            );
        };
        let mut layout = ImageLayout::Undefined;
        if job.dst.is_some() {
            barrier(&mut enc, BarrierStage::TOP_OF_PIPE, BarrierStage::TRANSFER, BarrierAccess::NONE, BarrierAccess::TRANSFER_WRITE, ImageLayout::Undefined, ImageLayout::TransferDstOptimal);
            enc.copy_buffer_to_image(&staging, &tex, ImageLayout::TransferDstOptimal, &region);
            layout = ImageLayout::TransferDstOptimal;
        }
        let compute = path == Path::Store;
        // The objects every branch may need, destroyed after the fence.
        let mut modules = Vec::new();
        let mut cpipes = Vec::new();
        let mut gpipes = Vec::new();
        let mut layouts = Vec::new();
        let mut groups = Vec::new();
        if compute {
            barrier(&mut enc, BarrierStage::TRANSFER, BarrierStage::COMPUTE_SHADER, BarrierAccess::TRANSFER_WRITE, BarrierAccess::SHADER_WRITE, layout, ImageLayout::General);
            let module = ctx.create_shader_module(&words(if target == Target::B10G11R11 { STORE_B10_SPV } else { STORE_F16_SPV })).expect("store module");
            let bgl = ctx
                .create_bind_group_layout(&BindGroupLayoutDesc {
                    entries: &[
                        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                    ],
                })
                .expect("store layout");
            let group = ctx
                .create_bind_group(&BindGroupDesc {
                    layout: &bgl,
                    entries: &[BindGroupEntry::StorageImage { texture: &tex }, BindGroupEntry::StorageBuffer { buffer: &input }],
                })
                .expect("store group");
            let pipe = ctx
                .create_compute_pipeline(&ComputePipelineDesc { module: &module, entry: c"main", push_constant_bytes: 16, bind_group_layout: Some(&bgl), spec_constants: &[] })
                .expect("store pipeline");
            enc.bind_compute_pipeline(&pipe);
            enc.bind_descriptor_set_compute(&group, &pipe);
            for l in 0..job.layers {
                let push: Vec<u8> = [job.w, job.h, u32::from(rmw), l].iter().flat_map(|v| v.to_le_bytes()).collect();
                enc.push_compute_constants(&pipe, ShaderStage::COMPUTE, 0, &push);
                enc.dispatch(job.w.div_ceil(8), job.h.div_ceil(8), 1);
                barrier(&mut enc, BarrierStage::COMPUTE_SHADER, BarrierStage::COMPUTE_SHADER, BarrierAccess::SHADER_WRITE, BarrierAccess::SHADER_READ, ImageLayout::General, ImageLayout::General);
            }
            barrier(&mut enc, BarrierStage::COMPUTE_SHADER, BarrierStage::TRANSFER, BarrierAccess::SHADER_WRITE, BarrierAccess::TRANSFER_READ, ImageLayout::General, ImageLayout::TransferSrcOptimal);
            modules.push(module);
            cpipes.push(pipe);
            layouts.push(bgl);
            groups.push(group);
        } else {
            let src_stage = if job.dst.is_some() { BarrierStage::TRANSFER } else { BarrierStage::TOP_OF_PIPE };
            let src_access = if job.dst.is_some() { BarrierAccess::TRANSFER_WRITE } else { BarrierAccess::NONE };
            barrier(&mut enc, src_stage, BarrierStage::COLOR_ATTACHMENT_OUTPUT, src_access, BarrierAccess::COLOR_ATTACHMENT_WRITE, layout, ImageLayout::ColorAttachmentOptimal);
            let full = RenderArea { x: 0, y: 0, width: job.w, height: job.h };
            if let Some(values) = clear_values {
                for (i, v) in values.iter().enumerate() {
                    let (x, y) = ((i as u32) % job.w, (i as u32) / job.w);
                    let att = [RenderingAttachment { texture: &tex, layout: ImageLayout::ColorAttachmentOptimal, load_op: LoadOp::Clear, store_op: StoreOp::Store, clear_color: [v[0], v[1], v[2], 1.0] }];
                    enc.begin_rendering(&RenderingDesc { render_area: RenderArea { x: x as i32, y: y as i32, width: 1, height: 1 }, colors: &att, depth: None });
                    enc.end_rendering();
                }
            } else {
                let vs = ctx.create_shader_module(&words(FULLSCREEN_VS_SPV)).expect("vs");
                let fs = ctx.create_shader_module(&words(RASTER_FS_SPV)).expect("fs");
                let bgl = ctx
                    .create_bind_group_layout(&BindGroupLayoutDesc {
                        entries: &[BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::FRAGMENT }],
                    })
                    .expect("raster layout");
                let group = ctx
                    .create_bind_group(&BindGroupDesc { layout: &bgl, entries: &[BindGroupEntry::StorageBuffer { buffer: &input }] })
                    .expect("raster group");
                let blend = match path {
                    Path::BlendAdd => Some(BlendState::ADDITIVE),
                    Path::BlendOver => Some(BlendState::PREMULTIPLIED_ALPHA),
                    _ => None,
                };
                let formats = [format_of(target)];
                // A FRAGMENT-only push range: `fullscreen_sample.vs` declares no push block and the
                // FS reads the whole range, so the push below names FRAGMENT alone, and
                // `VUID-vkCmdPushConstants-offset-01796` needs the range to be exactly that (the
                // default builder's `VERTEX | FRAGMENT` range made the push invalid — triage r1 F1).
                let pipe = ctx
                    .create_graphics_pipeline_fragment_push(&GraphicsPipelineDesc {
                        vertex_module: &vs,
                        vertex_entry: c"main",
                        fragment_module: &fs,
                        fragment_entry: c"main",
                        color_formats: &formats,
                        depth_format: None,
                        topology: PrimitiveTopology::TriangleList,
                        vertex_layout: None,
                        push_constant_bytes: 16,
                        bind_group_layout: Some(&bgl),
                        blend,
                        cull_mode: CullMode::None,
                        depth_bias: None,
                    })
                    .expect("raster pipeline");
                let load = if job.dst.is_some() { LoadOp::Load } else { LoadOp::Clear };
                let att = [RenderingAttachment { texture: &tex, layout: ImageLayout::ColorAttachmentOptimal, load_op: load, store_op: StoreOp::Store, clear_color: [0.0; 4] }];
                enc.begin_rendering(&RenderingDesc { render_area: full, colors: &att, depth: None });
                enc.bind_graphics_pipeline(&pipe);
                enc.bind_descriptor_set(&group, &pipe);
                enc.set_viewport(&Viewport { x: 0.0, y: 0.0, width: job.w as f32, height: job.h as f32, min_depth: 0.0, max_depth: 1.0 });
                enc.set_scissor(&full);
                for l in 0..job.layers {
                    let push: Vec<u8> = [job.w, job.h, l, 0].iter().flat_map(|v| v.to_le_bytes()).collect();
                    enc.push_graphics_constants(&pipe, ShaderStage::FRAGMENT, 0, &push);
                    enc.draw(3, 1, 0, 0);
                }
                enc.end_rendering();
                modules.push(vs);
                modules.push(fs);
                gpipes.push(pipe);
                layouts.push(bgl);
                groups.push(group);
            }
            barrier(&mut enc, BarrierStage::COLOR_ATTACHMENT_OUTPUT, BarrierStage::TRANSFER, BarrierAccess::COLOR_ATTACHMENT_WRITE, BarrierAccess::TRANSFER_READ, ImageLayout::ColorAttachmentOptimal, ImageLayout::TransferSrcOptimal);
        }
        enc.copy_image_to_buffer(&tex, ImageLayout::TransferSrcOptimal, &staging, &region);
        enc.end().expect("end");
        queue.submit(&enc, &fence).expect("submit");
        ctx.wait_fence(&fence, u64::MAX).expect("wait");
        let mut out = vec![0u8; n * tb];
        // SAFETY: the fence-waited submission is complete; the copy's TRANSFER_WRITE was made
        // host-visible by the barrier the encoder records for a mapped destination; the mapping is
        // `n * tb` bytes; `out` does not overlap it.
        unsafe { core::ptr::copy_nonoverlapping(sp.as_ptr(), out.as_mut_ptr(), out.len()) };
        // SAFETY: every object below was created on `ctx` and is destroyed exactly once, after the
        // only submission that used it completed (fence-waited above).
        unsafe {
            ctx.destroy_command_encoder(enc);
            ctx.destroy_fence(fence);
            for p in gpipes {
                ctx.destroy_graphics_pipeline(p);
            }
            for p in cpipes {
                ctx.destroy_compute_pipeline(p);
            }
            for g in groups {
                ctx.destroy_bind_group(g);
            }
            for l in layouts {
                ctx.destroy_bind_group_layout(l);
            }
            for m in modules {
                ctx.destroy_shader_module(m);
            }
            ctx.destroy_buffer(staging);
            ctx.destroy_buffer(input);
            ctx.destroy_texture(tex);
        }
        out.chunks_exact(tb).map(|b| unpack_codes(target, b)).collect()
    }
}

/// The image width the classification inputs are laid out at.
const CLASS_W: u32 = 256;

/// Runs one path's classification set and returns its outputs (in input order).
fn classify_on_device(rig: &Rig<'_>, target: Target, path: Path) -> (Vec<ClassInput>, Vec<[u32; 3]>) {
    let inputs = class_inputs(target, path);
    let h = (inputs.len() as u32).div_ceil(CLASS_W);
    let n = (CLASS_W * h) as usize;
    let input: Vec<[f32; 4]> = (0..n)
        .map(|i| inputs.get(i).map_or([0.0, 0.0, 0.0, 0.0], |x| [x.src[0], x.src[1], x.src[2], x.alpha]))
        .collect();
    let dst = path.is_blend().then(|| (0..n).map(|i| inputs.get(i).map_or([0; 3], |x| x.dst)).collect());
    let clears: Option<Vec<[f32; 3]>> = (path == Path::Clear).then(|| inputs.iter().map(|x| x.value).collect());
    let job = Job { w: CLASS_W, h, dst, layers: 1, input: &input };
    let out = rig.run(target, path, &job, false, clears.as_deref());
    (inputs.clone(), out[..inputs.len()].to_vec())
}

/// Runs one content arm on `target` and returns the device image's codes.
fn arm_on_device(rig: &Rig<'_>, target: Target, arm: &Arm) -> Vec<[u32; 3]> {
    let chans = target.channels();
    let bg: [u32; 3] = core::array::from_fn(|c| background_code(chans[c], arm.background));
    let mut input = vec![[0.0f32; 4]; REGION_PX * arm.k as usize];
    for px in 0..REGION_PX {
        for (l, layer) in arm.layers(px).iter().enumerate() {
            input[l * REGION_PX + px] = [layer.src[0], layer.src[1], layer.src[2], layer.alpha];
        }
    }
    let job = Job { w: REGION_W, h: REGION_H, dst: Some(vec![bg; REGION_PX]), layers: arm.k, input: &input };
    rig.run(target, arm.path, &job, arm.family == Family::A3, None)
}

// ---- The decision ------------------------------------------------------------------------------------

/// One arm's device result on one target.
struct ArmResult {
    arm: Arm,
    target: Target,
    class: PathClass,
    agreement: Option<f64>,
    agx: lit_format::ArmMetric,
    aces: lit_format::ArmMetric,
}

/// The decision table of the record (§5), applied: `(row, lit)`.
fn decide(classes: &[(Target, Path, Verdict)], results: &[ArmResult]) -> (u32, &'static str, bool) {
    let class_of = |p: Path| {
        classes.iter().find(|(t, q, _)| *t == Target::B10G11R11 && *q == p).map(|(_, _, v)| v.clone()).expect("every path classified")
    };
    let decisive = [Path::BlendAdd, Path::BlendOver, Path::Store];
    let unmodelled = decisive.iter().any(|p| matches!(class_of(*p), Verdict::Class(PathClass::Unmodelled { .. })));
    let b10: Vec<&ArmResult> = results.iter().filter(|r| r.target == Target::B10G11R11 && r.arm.family.decisive()).collect();
    let agree_all = b10.iter().all(|r| r.agreement.is_some_and(|a| a >= 0.999));
    let pass = |f: &dyn Fn(&&ArmResult) -> bool| b10.iter().filter(|r| f(r)).all(|r| r.agx.passes());
    let all_pass = pass(&|_| true);
    let row = if unmodelled { 3 } else if agree_all { 1 } else { 2 };
    let a3_alone = !all_pass && pass(&|r| r.arm.family != Family::A3);
    let rgba_fails = results.iter().filter(|r| r.target == Target::Rgba16f && r.arm.family.decisive()).any(|r| !r.agx.passes());
    if all_pass {
        return (row, "b10g11r11", false);
    }
    if a3_alone {
        let blend_ok = results.iter().filter(|r| r.target == Target::B10G11R11 && r.arm.family == Family::A3Blend).all(|r| r.agx.passes());
        if blend_ok {
            return (7, "b10g11r11", false);
        }
    }
    (if rgba_fails { 4 } else { row }, "rgba16f", rgba_fails)
}

fn verdict_spelling(v: &Verdict) -> String {
    match v {
        Verdict::Class(c) => {
            let mut s = String::new();
            c.write_spelling(&mut s).expect("a String writes");
            s
        }
        Verdict::Ambiguous(cs) => format!("AMBIGUOUS:{cs:?}"),
    }
}

#[test]
#[ignore = "gpu: needs a Vulkan device — the PX0 float-format probe and the lit format decision's device arms (GL-PX0-P); run --ignored --test-threads=1 with BOYKO_PX0_PROBE_OUT"]
fn b10g11r11_rounding_probe() {
    let ctx = match VulkanContext::boot(InstanceConfig { enable_validation: std::env::var_os("BOYKO_DISABLE_VALIDATION").is_none(), ..InstanceConfig::default() }) {
        Ok(c) => c,
        Err(e) => panic!("the probe needs a Vulkan device (GL-PX0-P is a device leg): {e:?}"),
    };
    println!("Vulkan device: {}", ctx.device_name());

    // The premises: AD-1 requires every path on both formats, so a missing feature is a RED.
    let mut missing = Vec::new();
    for (name, fmt) in [("B10G11R11_UFLOAT_PACK32", 122), ("R16G16B16A16_SFLOAT", 97)] {
        let f = ctx.optimal_tiling_features(fmt);
        for (bit, what) in [(FEAT_STORAGE, "STORAGE_IMAGE"), (FEAT_COLOR, "COLOR_ATTACHMENT"), (FEAT_BLEND, "COLOR_ATTACHMENT_BLEND")] {
            if f & bit == 0 {
                missing.push(format!("{name}: {what}"));
            }
        }
    }
    assert!(missing.is_empty(), "PREMISE RED — AD-1 requires these format features:\n{}", missing.join("\n"));

    let rig = Rig { ctx: &ctx };
    let mut toml = String::from("# PX0 GL-PX0-P — written by b10g11r11_rounding_probe\n");
    let _ = writeln!(toml, "device = \"{}\"", ctx.device_name());

    // 1. Classification, five paths x two formats.
    let mut classes = Vec::new();
    let _ = writeln!(toml, "\n[classes]");
    for target in [Target::B10G11R11, Target::Rgba16f] {
        for path in Path::ALL {
            let (inputs, out) = classify_on_device(&rig, target, path);
            let v = classify(target, path, &inputs, &out);
            assert!(!matches!(v, Verdict::Ambiguous(_)), "{target:?}/{path:?}: the input set does not separate {v:?} — a RED of the set");
            let _ = writeln!(toml, "{}_{} = \"{}\"", target.spelling(), path.spelling(), verdict_spelling(&v));
            println!("class {:>9} {:>10}: {}", target.spelling(), path.spelling(), verdict_spelling(&v));
            classes.push((target, path, v));
        }
    }

    // 2. The content arms, both formats.
    let mut results = Vec::new();
    for arm in arms() {
        for target in [Target::B10G11R11, Target::Rgba16f] {
            let class = match classes.iter().find(|(t, p, _)| *t == target && *p == arm.path).map(|(_, _, v)| v.clone()) {
                Some(Verdict::Class(c)) => c,
                other => panic!("invariant: {target:?}/{:?} classified ({other:?})", arm.path),
            };
            let dev = arm_on_device(&rig, target, &arm);
            let agreement = Member::of_class(class).map(|m| {
                let sim = simulate(&arm, target, m);
                sim.iter().zip(&dev).filter(|(a, b)| a == b).count() as f64 / REGION_PX as f64
            });
            let ex = exact(&arm, target);
            let dev_lin = decode_image(&dev, target);
            let agx = metric(Curve::Agx, &dev_lin, &exact_display(Curve::Agx, &ex));
            let aces = metric(Curve::AcesHill, &dev_lin, &exact_display(Curve::AcesHill, &ex));
            results.push(ArmResult { arm, target, class, agreement, agx, aces });
        }
    }
    let _ = writeln!(toml, "\n# arm | target | class | agreement | agx max|d| | agx chroma | aces max|d| | aces chroma");
    let _ = writeln!(toml, "arms = [");
    for r in &results {
        let _ = writeln!(
            toml,
            "  [\"{}\", \"{}\", \"{}\", {}, {:.3}, {:.3}, {:.3}, {:.3}],",
            r.arm.label(),
            r.target.spelling(),
            r.class.name(),
            r.agreement.map_or("-1.0".to_owned(), |a| format!("{a:.6}")),
            r.agx.max_delta,
            r.agx.chroma,
            r.aces.max_delta,
            r.aces.chroma
        );
    }
    let _ = writeln!(toml, "]");

    // 3. The decision.
    let (row, lit, stop) = decide(&classes, &results);
    let _ = writeln!(toml, "\n[decision]\nrow = {row}\nlit = \"{lit}\"\nstop_the_wave = {stop}");
    print!("{toml}");
    if let Ok(path) = std::env::var("BOYKO_PX0_PROBE_OUT") {
        std::fs::write(&path, &toml).unwrap_or_else(|e| panic!("{path}: {e}"));
        println!("probe.toml -> {path}");
    }
    if ctx.validation_enabled()
        && let Some(state) = ctx.debug_state()
    {
        assert_eq!(state.total(), 0, "validation reported {} message(s) during the probe", state.total());
    }
}
