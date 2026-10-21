//! BOOT-1 F-A device gates: one `VkPipelineCache` per context reaches every `vkCreate*Pipelines`
//! site, and the creation feedback counts every pipeline.
//!
//! The evidence is the driver's own: `VkPipelineCreationFeedback` with
//! `VK_PIPELINE_CREATION_FEEDBACK_APPLICATION_PIPELINE_CACHE_HIT_BIT` set says the pipeline came
//! out of the cache this create passed. A second identical create through a site that passes the
//! cache is a hit; through a site that passes `VK_NULL_HANDLE` it can never be one.
//!
//! Run: `cargo test -p boyko_rhi_vulkan --test pipeline_cache_persist -- --ignored --test-threads=1`
//! with `BOYKO_DISABLE_VALIDATION=1`. Single-threaded: the singleton tests and the env switch test
//! share process state.

use core::slice;

use boyko_rhi::{
    BindGroupLayoutDesc, BindGroupLayoutEntry, ComputePipelineDesc, CullMode, DescriptorKind, Format,
    GraphicsPipelineDesc, PrimitiveTopology, RhiDevice, ShaderStage,
};
use boyko_rhi_vulkan::compute::write_pattern_spirv;
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::pipeline_cache::{CacheLoad, PipelineCreationStats};

/// A 4-byte-aligned SPIR-V blob wrapper (the `graphics_triangle.rs` idiom).
#[repr(C, align(4))]
struct SpirvBlob<const N: usize>([u8; N]);

impl<const N: usize> SpirvBlob<N> {
    fn as_words(&self) -> &[u32] {
        const { assert!(N.is_multiple_of(4), "SPIR-V byte length must be a multiple of 4") };
        // SAFETY: the `align(4)` wrapper makes `self.0`'s address a valid `*const u32`; `N` is a
        // 4-byte multiple (const-asserted above); the `'static` blob outlives the slice; any bit
        // pattern is a valid `u32`.
        unsafe { slice::from_raw_parts(self.0.as_ptr().cast::<u32>(), N / 4) }
    }
}

static TRIANGLE_VS_SPV: SpirvBlob<700> =
    SpirvBlob(*include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/triangle.vs.spv")));
static TRIANGLE_FS_SPV: SpirvBlob<336> =
    SpirvBlob(*include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/triangle.fs.spv")));

fn headless() -> InstanceConfig {
    InstanceConfig { enable_validation: false, windowed: false }
}

fn boot_or_skip(test: &str) -> Option<VulkanContext> {
    match VulkanContext::boot(headless()) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP {test}: no Vulkan device ({e:?})");
            None
        }
    }
}

/// The five creation entry points, by the site each reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Site {
    /// `RhiDevice::create_compute_pipeline` (`rhi_impl/device.rs`, the device-shared layout).
    Compute,
    /// `RhiDevice::create_graphics_pipeline` -> `build_graphics_pipeline`.
    Graphics,
    /// `VulkanContext::create_compute_pipeline_forward` (2 sets).
    ComputeForward,
    /// `VulkanContext::create_compute_pipeline_vb` (3 sets).
    ComputeVb,
    /// `VulkanContext::create_compute_pipeline_vb_textured` (4 sets).
    ComputeVbTextured,
}

const SITES: [Site; 5] = [Site::Compute, Site::Graphics, Site::ComputeForward, Site::ComputeVb, Site::ComputeVbTextured];

/// Creates (and destroys) one pipeline through `site`; the shaders' bindings are a subset of every
/// layout (`write_pattern` reads set 0 binding 0, a storage buffer; the triangle pair binds nothing).
fn create_once(ctx: &VulkanContext, site: Site) {
    match site {
        Site::Graphics => {
            let vs = ctx.create_shader_module(TRIANGLE_VS_SPV.as_words()).expect("triangle VS module");
            let fs = ctx.create_shader_module(TRIANGLE_FS_SPV.as_words()).expect("triangle FS module");
            let pipeline = ctx
                .create_graphics_pipeline(&GraphicsPipelineDesc {
                    vertex_module: &vs,
                    vertex_entry: c"main",
                    fragment_module: &fs,
                    fragment_entry: c"main",
                    color_formats: &[Format::R8G8B8A8Unorm],
                    depth_format: None,
                    topology: PrimitiveTopology::TriangleList,
                    vertex_layout: None,
                    push_constant_bytes: 0,
                    bind_group_layout: None,
                    blend: None,
                    cull_mode: CullMode::None,
                    depth_bias: None,
                })
                .expect("graphics pipeline");
            // SAFETY: the pipeline and modules were created on `ctx` above, nothing was submitted,
            // and each is destroyed exactly once.
            unsafe {
                ctx.destroy_graphics_pipeline(pipeline);
                ctx.destroy_shader_module(vs);
                ctx.destroy_shader_module(fs);
            }
        }
        _ => {
            let module = ctx.create_shader_module(write_pattern_spirv()).expect("write_pattern module");
            let bgl = ctx
                .create_bind_group_layout(&BindGroupLayoutDesc {
                    entries: &[BindGroupLayoutEntry {
                        binding: 0,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::COMPUTE,
                    }],
                })
                .expect("one STORAGE_BUFFER layout");
            let raw = bgl.set_layout();
            let desc = ComputePipelineDesc {
                module: &module,
                entry: c"main",
                push_constant_bytes: 4,
                bind_group_layout: if site == Site::Compute { None } else { Some(&bgl) },
                spec_constants: &[],
            };
            let pipeline = match site {
                Site::Compute => ctx.create_compute_pipeline(&desc),
                Site::ComputeForward => ctx.create_compute_pipeline_forward(&desc, raw),
                Site::ComputeVb => ctx.create_compute_pipeline_vb(&desc, raw, raw),
                Site::ComputeVbTextured => ctx.create_compute_pipeline_vb_textured(&desc, raw, raw, raw),
                Site::Graphics => unreachable!("invariant: handled above"),
            }
            .expect("compute pipeline");
            // SAFETY: as above.
            unsafe {
                ctx.destroy_compute_pipeline(pipeline);
                ctx.destroy_bind_group_layout(bgl);
                ctx.destroy_shader_module(module);
            }
        }
    }
}

/// One site's row: (hits gained by the first create, by the second).
fn hits_per_create(ctx: &VulkanContext, site: Site) -> (u32, u32) {
    let s0 = ctx.pipeline_stats();
    create_once(ctx, site);
    let s1 = ctx.pipeline_stats();
    create_once(ctx, site);
    let s2 = ctx.pipeline_stats();
    assert_eq!(s1.total(), s0.total() + 1, "{site:?}: the first create was not counted");
    assert_eq!(s2.total(), s1.total() + 1, "{site:?}: the second create was not counted");
    (s1.hits - s0.hits, s2.hits - s1.hits)
}

#[test]
#[ignore = "gpu: needs a Vulkan device; run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn every_create_site_passes_the_cache() {
    let Some(ctx) = boot_or_skip("every_create_site_passes_the_cache") else { return };
    assert_eq!(ctx.pipeline_cache_load(), CacheLoad::Ephemeral, "VulkanContext::boot is ephemeral");
    assert!(!ctx.pipeline_cache().is_null(), "an ephemeral context has a cache");
    let mut report = String::new();
    let mut red = 0;
    for site in SITES {
        let (first, second) = hits_per_create(&ctx, site);
        let ok = first == 0 && second == 1;
        red += usize::from(!ok);
        report.push_str(&format!(
            "  {:<18} {}  first create hits {first}, second create hits {second}\n",
            format!("{site:?}"),
            if ok { "GREEN" } else { "RED" }
        ));
    }
    eprint!("every_create_site_passes_the_cache:\n{report}");
    assert_eq!(
        red, 0,
        "{red} create site(s) do not pass the cache (the second identical create must carry VALID | \
         APPLICATION_PIPELINE_CACHE_HIT, the first must miss):\n{report}"
    );
}

/// Restores `BOYKO_PIPELINE_CACHE` on every exit path, including a failed assertion.
struct EnvGuard(&'static str);

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: this binary runs `--test-threads=1` (module doc) and nothing else in it reads
        // the environment concurrently; the variable is removed after the test that set it.
        unsafe { std::env::remove_var(self.0) };
    }
}

#[test]
#[ignore = "gpu: needs a Vulkan device; run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn cache_off_passes_null() {
    // SAFETY: as `EnvGuard::drop` — single-threaded binary, the guard removes the variable.
    unsafe { std::env::set_var("BOYKO_PIPELINE_CACHE", "off") };
    let _guard = EnvGuard("BOYKO_PIPELINE_CACHE");
    let Some(ctx) = boot_or_skip("cache_off_passes_null") else { return };
    assert_eq!(ctx.pipeline_cache_load(), CacheLoad::Off, "BOYKO_PIPELINE_CACHE=off creates no cache");
    assert!(ctx.pipeline_cache().is_null(), "off means VK_NULL_HANDLE at every site");
    for site in SITES {
        let (first, second) = hits_per_create(&ctx, site);
        assert_eq!((first, second), (0, 0), "{site:?}: a cache hit with BOYKO_PIPELINE_CACHE=off");
    }
}

#[test]
#[ignore = "gpu: needs a Vulkan device; run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn stats_count_every_creation() {
    let Some(ctx) = boot_or_skip("stats_count_every_creation") else { return };
    let before: PipelineCreationStats = ctx.pipeline_stats();
    for site in SITES {
        create_once(&ctx, site);
    }
    let s = ctx.pipeline_stats();
    assert_eq!(s.compute - before.compute, 4, "four compute sites ran once each");
    assert_eq!(s.graphics - before.graphics, 1, "one graphics site ran once");
    assert_eq!(s.feedback_invalid, 0, "every create came back with VALID feedback");
    assert!(s.driver_ns > before.driver_ns, "the driver reported creation time");
    let tops: Vec<u64> = s.top.iter().map(|e| e.ns).collect();
    assert!(tops.windows(2).all(|w| w[0] >= w[1]), "top-N sorted descending: {tops:?}");
    for e in s.top.iter().filter(|e| e.ns > 0) {
        let site = e.site.expect("a ranked entry names its site");
        assert!(
            site.file().ends_with("pipeline_cache_persist.rs"),
            "#[track_caller] must name the caller of the entry point, got {}:{}",
            site.file(),
            site.line()
        );
    }
}
