# Summary

[Introduction](introduction.md)

# Getting Started

- [Installation](getting-started/installation.md)
- [Your First App](getting-started/first-app.md)

# Core ECS

- [Components](concepts/components.md)
- [Dense Components](concepts/dense-components.md)
- [Bundles](concepts/bundles.md)
- [Entities](concepts/entities.md)
- [Cloning & Prefabs](concepts/cloning-and-prefabs.md)
- [Resources](concepts/resources.md)
- [Assets & Handles](concepts/assets.md)
- [Queries](concepts/queries.md)
- [Iteration — Chunked & Parallel](concepts/iteration.md)
- [Systems](concepts/systems.md)
- [Commands](concepts/commands.md)
- [Events](concepts/events.md)

# Tags

- [Tags](concepts/tags.md)
- [Dynamic Tags](concepts/dynamic-tags.md)
- [Enable Tags (Bitset Storage)](concepts/enable-tags.md)

# Reactivity & Relationships

- [Lifecycle Hooks, Observers & Triggers](concepts/hooks-and-observers.md)
- [Hierarchies](concepts/hierarchies.md)
- [Relations](concepts/relations.md)

# Scheduling

- [Parallel Scheduler](scheduler.md)
- [System Ordering & Sets](scheduling/ordering-and-sets.md)
- [Run Conditions](scheduling/run-conditions.md)
- [States](scheduling/states.md)
- [State Charts](scheduling/state-charts.md)
- [Change Detection](change_detection.md)

# Application

- [App & Plugins](app/plugins.md)
- [Windowed Host (`boyko_app`)](app/windowed-host.md)
- [Time & Fixed Timestep](app/time.md)
- [Input](app/input.md)
- [Multiple Worlds](app/multi-world.md)

# Aether DSL

- [Overview](aether/overview.md)
- [Data Constructs](aether/data-constructs.md)
- [Systems & Plugins](aether/systems-and-plugins.md)
- [State Machines](aether/state-machines.md)
- [Materials](aether/materials.md)
- [Scenes](aether/scenes.md)
- [Diagnostics](aether/diagnostics.md)
- [Language Reference](aether/reference.md)

# Architecture

- [Design Principles](architecture/principles.md)
- [Storage Trade-offs: Tags, Churn, and Fragmentation](architecture/storage-tradeoffs.md)
- [Entities & Generations](architecture/entities-and-generations.md)
- [Per-Pool Virtual Memory](memory/arena.md)

# Rendering

- [Overview](rendering/overview.md)
- [Render Paths & Visibility Buffer](rendering/render-paths.md)
- [RHI & Vulkan Backend](rendering/rhi.md)
- [Framegraph](rendering/framegraph.md)
- [GPU-Resident Columns](rendering/gpu-columns.md)
- [Meshes & Loaders](rendering/meshes-and-loaders.md)
- [Materials & Textures](rendering/materials-and-textures.md)
- [Lighting](rendering/lighting.md)
- [Shadows & Ambient Occlusion](rendering/shadows-and-ao.md)
- [Global Illumination (SDF DDGI)](rendering/global-illumination.md)
- [Anti-Aliasing](rendering/anti-aliasing.md)
- [GPU Particles](rendering/particles.md)
- [SDF Rendering](rendering/sdf.md)
- [Shader eDSL](rendering/shader-edsl.md)

# Simulation

- [Physics](simulation/physics.md)
- [Transforms & Scene](simulation/transforms.md)
- [Math](simulation/math.md)

# User Interface

- [Overview](ui/overview.md)
- [Text & MSDF](ui/text-msdf.md)
- [Sprites & Animation](ui/sprites-and-animation.md)

# Diagnostics

- [Logging & Error Codes](diagnostics/logging.md)
- [Profiling](diagnostics/profiling.md)

# Persistence

- [Serialization](persistence/serialization.md)
- [Reflection (Editor Builds)](persistence/reflection.md)

# Reference

- [Glossary](reference/glossary.md)
- [Benchmarks](reference/benchmarks.md)

---

[Contributing](contributing.md)
