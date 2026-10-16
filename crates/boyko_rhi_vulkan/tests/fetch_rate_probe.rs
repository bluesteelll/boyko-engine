//! Render wave PX0 (H3 Scope 2; `px0/cut.md` D-P5) — **the fetch-rate probe**: ns per fetch of
//! RGBA8 / RGBA16F / B10G11R11 images, filtered (`SampleLevel`, linear) and unfiltered (`Load`), at
//! three sizes, measured by a raw timestamp pair around one dispatch of `px0_fetch_rate.comp`.
//!
//! K15: POSTFX's texel model assumes a 64-bit filter rate nobody measured; half of its gates rest on
//! it. This probe CALIBRATES it and asserts nothing about time: it is a WR-1 row, read in the
//! orchestrator's quiet window (owner consent, the Claude app closed), never a verdict here.
//!
//! Run (WR-1): `cargo test --release -p boyko_rhi_vulkan --test fetch_rate_probe --locked --
//! --ignored --test-threads=1`.

use boyko_rhi::enums::{BarrierAccess, BarrierStage, DescriptorKind};
use boyko_rhi::{
    AddressMode, BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc,
    BufferUsage, ComputePipelineDesc, Filter, Format, ImageBarrierDesc, ImageLayout, ImageSubresourceRange,
    ImageUsage, MemoryLocation, MipMode, QueryPoolDesc, RhiCommandEncoder, RhiDevice, RhiQueue, SamplerDesc,
    ShaderStage, TextureDesc, TextureDimension, TimestampStage,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

static FETCH_SPV: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/px0_fetch_rate.comp.spv"));

/// Invocations per dispatch and fetches per invocation.
const INVOCATIONS: u32 = 65_536;
/// Fetches per invocation.
const ITERS: u32 = 64;

#[test]
#[ignore = "gpu: needs a Vulkan device — the PX0 fetch-rate probe (a WR-1 row, no verdict); run --release --ignored --test-threads=1 in the orchestrator's quiet window"]
fn fetch_rate_probe() {
    let ctx = VulkanContext::boot(InstanceConfig { enable_validation: false, ..InstanceConfig::default() })
        .unwrap_or_else(|e| panic!("the fetch-rate probe needs a Vulkan device: {e:?}"));
    println!("Vulkan device: {}", ctx.device_name());
    let device: &VulkanContext = &ctx;
    let queue = ctx.rhi_queue();
    let module = device.create_shader_module(&FETCH_SPV.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect::<Vec<_>>()).expect("module");
    let bgl = device
        .create_bind_group_layout(&BindGroupLayoutDesc {
            entries: &[
                BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::SampledImage, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            ],
        })
        .expect("layout");
    let pipe = device
        .create_compute_pipeline(&ComputePipelineDesc { module: &module, entry: c"main", push_constant_bytes: 16, bind_group_layout: Some(&bgl), spec_constants: &[] })
        .expect("pipeline");
    let sampler = device
        .create_sampler(&SamplerDesc { mag_filter: Filter::Linear, min_filter: Filter::Linear, address_mode: AddressMode::ClampToEdge, mip: MipMode::None, ..SamplerDesc::default() })
        .expect("sampler");
    let sink = device
        .create_buffer(&BufferDesc { size: u64::from(INVOCATIONS) * 4, usage: BufferUsage::STORAGE, location: MemoryLocation::DeviceLocal })
        .expect("sink");
    let pool = device.create_query_pool(&QueryPoolDesc { count: 2 }).expect("query pool");
    println!("format              size  filtered  ns/fetch");
    for (name, format) in [("RGBA8", Format::R8G8B8A8Unorm), ("RGBA16F", Format::R16G16B16A16Sfloat), ("B10G11R11", Format::B10G11R11UfloatPack32)] {
        for size in [256u32, 1024, 2048] {
            let tex = device
                .create_texture(&TextureDesc {
                    width: size,
                    height: size,
                    depth: 1,
                    format,
                    dimension: TextureDimension::D2,
                    usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST,
                    array_layers: 1,
                    mip_levels: 1,
                    view_format: None,
                })
                .expect("texture");
            let group = device
                .create_bind_group(&BindGroupDesc {
                    layout: &bgl,
                    entries: &[BindGroupEntry::SampledImage { texture: &tex, sampler: &sampler }, BindGroupEntry::StorageBuffer { buffer: &sink }],
                })
                .expect("group");
            for filtered in [0u32, 1] {
                let fence = device.create_fence(false).expect("fence");
                let mut enc = device.create_command_encoder().expect("encoder");
                enc.begin().expect("begin");
                enc.image_barrier(&ImageBarrierDesc {
                    texture: &tex,
                    src_stage: BarrierStage::TOP_OF_PIPE,
                    dst_stage: BarrierStage::COMPUTE_SHADER,
                    src_access: BarrierAccess::NONE,
                    dst_access: BarrierAccess::SHADER_READ,
                    old_layout: ImageLayout::Undefined,
                    new_layout: ImageLayout::ShaderReadOnlyOptimal,
                    range: ImageSubresourceRange::COLOR,
                });
                enc.reset_query_pool(&pool, 0, 2);
                enc.bind_compute_pipeline(&pipe);
                enc.bind_descriptor_set_compute(&group, &pipe);
                let push: Vec<u8> = [size, ITERS, filtered, 0].iter().flat_map(|v| v.to_le_bytes()).collect();
                enc.push_compute_constants(&pipe, ShaderStage::COMPUTE, 0, &push);
                enc.write_timestamp(&pool, TimestampStage::BottomOfPipe, 0);
                enc.dispatch(INVOCATIONS / 64, 1, 1);
                enc.write_timestamp(&pool, TimestampStage::BottomOfPipe, 1);
                enc.end().expect("end");
                queue.submit(&enc, &fence).expect("submit");
                device.wait_fence(&fence, u64::MAX).expect("wait");
                let mut scratch = [0u64; 4];
                let mut ns = [0f64; 1];
                device.read_query_pool_ns(&pool, 1, &mut scratch, &mut ns).expect("timestamps");
                let per = ns[0] / f64::from(INVOCATIONS * ITERS);
                println!("{name:<18} {size:>5} {filtered:>9} {per:>9.4}");
                // SAFETY: the fence-waited submission is complete; both objects were created on
                // `device` and are destroyed once.
                unsafe {
                    device.destroy_command_encoder(enc);
                    device.destroy_fence(fence);
                }
            }
            // SAFETY: no submission that used them is in flight (every one was fence-waited).
            unsafe {
                device.destroy_bind_group(group);
                device.destroy_texture(tex);
            }
        }
    }
    // SAFETY: as above — every submission completed; each object is destroyed once.
    unsafe {
        device.destroy_query_pool(pool);
        device.destroy_buffer(sink);
        device.destroy_sampler(sampler);
        device.destroy_compute_pipeline(pipe);
        device.destroy_bind_group_layout(bgl);
        device.destroy_shader_module(module);
    }
}
