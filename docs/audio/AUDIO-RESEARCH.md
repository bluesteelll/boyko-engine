# Audio engine — research corpus

**Date:** 2026-09-25 · **Status:** research complete (rev 2). It feeds
[AUDIO-DESIGN-SPACE.md](AUDIO-DESIGN-SPACE.md), which holds the options, the numbers they are scored
with, the recommendation and the rung plan. Nothing here is ratified.

**Rev 2** adds what the design's first critique pass needed: six primary sources [S63]–[S68],
quotations newly read from [S1], [S2] and [S13], pitfalls 16–19, and the repository facts the
critique relied on (§11). Rev 1's claims are unchanged.

**Trunk read:** `6394bc5e` (branch `u/research-0925`, identical to the trunk). Paths are relative to
the repository root. Code is cited as a path plus a symbol; no line anchors are used.

**Not a delta document.** No audio document exists in this tree before this one, and no audio code
exists (§11). The owner says an audio engine was researched before, outside this repository. That
work was not available to this pass, so nothing is inherited from it. It was researched from
scratch, in the context of the current engine.

## How to read the evidence

| Tag | Meaning |
|---|---|
| **[V]** | A primary page was fetched and read. |
| **[S]** | Search-result snippet only. The page itself returned 403 or 404, or rendered only through JavaScript. |
| **[2]** | Secondary source (a blog, an aggregator, an AI-generated wiki). |
| **[E]** | Derivation or estimate by this document. It is not a measurement, and every one is listed for measurement in the design document's §11. |
| **[R]** | Repository fact, checked at `6394bc5e`. |

**The limits of this pass, stated once.** The session's web-search budget, 200 calls, ran out during
the survey. After that only known primary URLs were fetched directly. Several vendor pages could not
be read:

- audiokinetic.com (Wwise) returned 403.
- The fmod.com 2.02 white papers came back empty (the pages render through JavaScript).
- The unrealengine.com blog and ea.com (Frostbite) were blocked.
- The AES e-library returned 403.

Every claim from those sources is tagged [S]. None of them decides an option in the design document
on its own. Where a vendor number would decide something, the design document measures it instead
(its §11). Five claims were upgraded to [V] by direct fetches in the writing pass: VBAP, FDN history,
the Opus FAQ, the Firewheel design doc and the `windows-sys` module paths.

The rev-2 pass had no search budget left either. It fetched known primary URLs directly: the Rust
`f32` reference, four Microsoft pages, the Linux `getrusage` manual, the cubeb pull request and the
pipewire-alsa plugin source. The freedesktop.org GitLab refused the last one, so it was read from
PipeWire's GitHub mirror.

---

## TL;DR — what the design must know

1. **The rules for the audio thread are the same everywhere.** No allocation, no locks (a mutex
   `try_lock` is not safe either), no blocking system calls, and never waiting on a lower-priority
   thread [S14][S15][V S16]. Memory released on the audio thread is freed elsewhere by a deferred
   collector [V S16][V S43]. On Windows the audio thread is raised by MMCSS: the High category runs
   at priority 23–26, and 20% of the CPU is reserved for low-priority work by default [V S4].
2. **The OS sets output latency, not the mixer.**
   - Windows shared mode defaults to a 10 ms period.
   - `IAudioClient3` goes down to 128 frames (2.67 ms at 48 kHz) on the inbox HDAudio driver, and
     the Windows audio engine adds 1.3 ms [V S1].
   - Exclusive mode takes over the endpoint and needs a shared-mode fallback [V S2].
   - PipeWire's default quantum is 1024 frames at 48 kHz (21.3 ms) [V S8].
3. **The CPU cost of a real voice comes from decoding and resampling, not from mixing.**
   - Opus at 128 kb/s stereo needs about 11 MHz on Haswell [2 S38], about 27 µs per 10 ms at 4 GHz [E].
   - Vorbis costs 1.5–3× ADPCM, and Opus 5–10× ADPCM [S31].
   - A gain-ramped SIMD mix costs about 0.1 µs per voice per 10 ms [E].
   - So shipped engines run many virtual voices and few real ones: FMOD suggests 32–128 real
     software channels [S29], and Apex Legends plays 80–100 real voices [S34].
4. **The acoustics options differ by four orders of magnitude in cost and in effort.** From
   cheapest to most expensive:
   - one raycast driving a low-pass filter and a volume (Unreal) [V S23];
   - graph propagation (Rainbow Six Siege nodes, Wwise rooms and portals) [V S52][S50];
   - ray-traced reflections, real-time or baked (Steam Audio, Apache-2.0 since 2024) [V S47];
   - baked wave simulation (Project Acoustics / Triton): 15 µs per query on a cache hit and 40 µs on
     a miss, on PC [V S48].
5. **Every engine surveyed separates the gameplay side from the render side**, with a command
   channel between them:
   - Unreal: game thread → audio thread → audio render thread [V S20];
   - Unity: `DSPCommandBlock` [V S25];
   - Kira: an `rtrb` command ring [V S41];
   - Firewheel / bevy_seedling: components diffed into events [V S43][V S44].

   Firewheel sends a frame's events as one group, so that events meant for the same block cannot
   land in different blocks [V S43].
6. **What boyko has, in brief** [R]:
   - no audio code;
   - no world ray query;
   - an allocation-free analytic SDF field that runs on the CPU, capped at 16 edits;
   - an SPSC byte ring in `boyko_log`;
   - spare diagnostics lanes that the lane module lets any thread outside the pool claim;
   - a planned kernel contract (Phase D) that already names the storage audio needs:
     registry-free scratch columns, a byte column, and an owning column with epoch-gated retire.

   Details are in §11.

---

## 1. Device output

### 1.1 Windows

**WASAPI** [V S1][V S2][V S3]
- **Event-driven mode.** The stream is created with `AUDCLNT_STREAMFLAGS_EVENTCALLBACK`, and the
  render thread waits on an event handle for each period.
- **Shared mode.**
  - The period is 10 ms by default.
  - `IAudioClient3::GetSharedModeEnginePeriod` reports the default, fundamental, minimum and maximum
    periods, and `InitializeSharedAudioStream` selects one of them.
  - If another client has already fixed the period, the call returns
    `AUDCLNT_E_ENGINE_PERIODICITY_LOCKED`.
  - The inbox HDAudio driver supports 128–480 frames at 48 kHz, and the engine itself adds 1.3 ms
    of latency.
- **Exclusive mode.** Two ping-pong buffers of one device period each. A misaligned size fails with
  `AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED`, and the client retries with the size `GetBufferSize` reports.
  Exclusive mode silences every other application on the endpoint.
- **Microsoft's guidance.** Try low-period shared mode before exclusive mode. Its FAQ states that
  lower latency costs power.
- **A small period is endpoint-wide.** The same FAQ: when one application requests small buffers,
  "all applications that use the same endpoint and mode will automatically switch to that small
  buffer size", until the low-latency application exits [V S1]. The setting therefore reaches beyond
  the requesting process.
- **Bounded waits in Microsoft's own sample.** The event-driven sample waits with
  `WaitForSingleObject(hEvent, 2000)`, and on a timeout stops the stream and returns
  `ERROR_TIMEOUT` [V S2].
- **Engine-side conversion.** `AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM` inserts a channel matrixer and a
  sample-rate converter between the client's format and the engine mix format;
  `AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY` selects a better converter at a higher cost [V S64].
- **Device loss.** Many WASAPI methods return `AUDCLNT_E_DEVICE_INVALIDATED` when the endpoint is
  unplugged, reconfigured, disabled or removed. For a default-device client, Microsoft's recovery is
  to release every WASAPI interface, get the current default endpoint, and activate a new
  `IAudioClient` on it. When a session is disconnected, WASAPI closes its streams, and a later call
  such as `GetCurrentPadding` fails with that error [V S65].
- **A shipped raw-WASAPI client.** Firefox's cubeb added `IAudioClient3` support in 2019 (merged
  pull request 530). It embeds the `IAudioClient3` definitions in its own source and requests lower
  shared-mode periods through `InitializeSharedAudioStream`. The PR reports web-audio latency of
  10 ms [V S13].

**MMCSS** [V S4]
- A thread registers with `AvSetMmThreadCharacteristicsW` under a task name: `"Pro Audio"`,
  `"Audio"` or `"Games"`.
- Categories: High runs at priority 23–26, Medium at 16–22, Low at 8–15.
- The `SystemResponsiveness` registry value reserves 20% of the CPU for low-priority work by default.
- WASAPI tags its own transport threads "Pro Audio" when the device period is under 10 ms [V S2].

**XAudio2** (in the box on Windows) [V S5]
- Offers source, submix and mastering voices, a filter per voice, implicit sample-rate conversion,
  multirate graphs and ADPCM.
- Microsoft documents that its methods contain critical sections and can block, and that
  `DestroyVoice` can block the engine.
- It is Windows-only.

**Windows Spatial Sound (`ISpatialAudioClient`)** [V S6]
- A static bed of up to 8.1.4.4 channels plus dynamic objects.
- Current dynamic-object limits: Dolby Atmos over HDMI allows 20; Atmos, DTS and Windows Sonic
  headphones allow 128. With spatial sound off, no dynamic objects are available.
- The page states that Doppler, distance attenuation, occlusion and reverb stay the game's job.

### 1.2 Linux

- PipeWire speaks its native protocol over a UNIX socket [V S9]. Its default quantum is 1024 frames
  at 48 kHz (21.3 ms), and the minimum is 32 [V S8].
- The ALSA plugins of PulseAudio and PipeWire ignore ALSA periods and run on OS timers. An
  ALSA-level period setting therefore does not set latency on those systems [V S10].
- SDL3 prefers PipeWire when it is present [2 S11]. miniaudio loads its backends at runtime by
  default; `MA_NO_RUNTIME_LINKING` turns that off [V S12].
- **Plugin threads start inside the open call.** pipewire-alsa's `snd_pcm_pipewire_open` calls
  `pw_thread_loop_new("alsa-pipewire", NULL)` and then `pw_thread_loop_start`, so its loop thread is
  created on the thread that calls `snd_pcm_open` [V S66].

### 1.3 Raw FFI feasibility

- **`windows-sys` has functions, structs, constants and GUIDs but no COM vtables** [V S7].
  - `IMMDeviceEnumerator`, `IAudioClient`/`IAudioClient3`, `IAudioRenderClient`, `IAudioClock` and
    `IMMNotificationClient` would be declared by hand, as the Vulkan surface is.
  - `AvSetMmThreadCharacteristicsW` is `windows_sys::Win32::System::Threading` [V S62].
  - `WAVEFORMATEXTENSIBLE` is `windows_sys::Win32::Media::Audio::WAVEFORMATEXTENSIBLE`
    (`#[repr(C, packed(1))]`) [V S62].
  - The module holding the IEEE-float subformat GUID was not checked. Declaring the 16-byte GUID by
    hand is the fallback.
- **Repository precedent** [R]:
  - `windows-sys` 0.61 is a workspace pin whose base feature is `Win32_UI_WindowsAndMessaging` (root
    `Cargo.toml`, `[workspace.dependencies]`).
  - `crates/boyko_rhi_vulkan/Cargo.toml` and `crates/boyko_input/Cargo.toml` record it as explicitly
    approved for the OS layer, with features added per crate.
  - `crates/boyko_app/src/timer_resolution.rs` hand-declares `winmm` without any binding crate.

---

## 2. The real-time audio thread

### 2.1 The rules

- Never block the callback on a mutex, the disk or the network [S14].
- `std::mutex` is unsafe on the audio thread even through `try_lock`, because a contended unlock can
  force a system call. Spinlocks with back-off are the recommended alternative [S15].
- **Freeing is deferred.**
  - basedrop's `Owned`/`Shared` push released contents onto an MPSC linked-list queue, and a
    `Collector` frees them on another thread. Its reclamation channel cannot fill, by construction
    [V S16].
  - Firewheel keeps a simple garbage collector for resources dropped on the real-time thread
    [V S43].
- **Allocation detection.** `assert_no_alloc` wraps the global allocator and aborts or warns on any
  allocation inside a guarded section. It is off in release builds by default [V S18], and Kira
  exposes it as a feature [V S41]. A global-allocator hook sees heap calls only, not memory an
  engine commits from its own virtual reservations (§11).
- **Residency.** `VirtualLock` keeps pages in physical memory, but "the maximum number of pages that
  a process can lock is equal to the number of pages in its minimum working set minus a small
  overhead", and raising that takes `SetProcessWorkingSetSize` [V S68]. On Linux,
  `getrusage(RUSAGE_THREAD)` (since 2.6.26) reports minor (`ru_minflt`) and major (`ru_majflt`)
  page faults for the calling thread [V S67].

### 2.2 Communication

- `rtrb` is a wait-free SPSC ring. It allocates nothing after construction and pads its cursors in
  the crossbeam style [V S17]. Kira sends its commands through it [V S41].
- Firewheel's update flushes the queued events to the audio thread as one group, so events meant for
  the same processing cycle cannot be split across two cycles [V S43].
- Unity DSPGraph batches graph edits into a `DSPCommandBlock` that is submitted atomically; its jobs
  are Burst-compiled [V S25].

### 2.3 Thread layouts in shipped engines

| Engine | Layout | Source |
|---|---|---|
| Unreal Audio Mixer | Audio thread (sound selection, parameters; fenced during GC) → audio render thread (DSP, mixing) → hardware callback thread. Renders N buffers ahead, N set per platform. Compressed sources are decoded in async tasks. MetaSounds render on the same async-task architecture that decoding uses | [V S20][V S21] |
| Firewheel | One audio thread. A multithreaded graph is an explicit non-goal, called overkill for games | [V S43] |
| Godot 4 | Mixing in `AudioServer`, over a lock-free `SafeList` and atomics. Fixed 1024-frame mix blocks, about 23 ms at 44.1 kHz | [V S26][V S27][2 S28] |
| Unity DSPGraph | Burst jobs driven by command blocks. Still `0.1.0-preview.22` | [V S25] |
| XAudio2 | Engine thread inside the OS component; the API takes critical sections | [V S5] |
| miniaudio | Device thread plus a resource-manager job queue: fixed-capacity MPMC, spinlock-protected | [V S12] |

### 2.4 Denormals

Operations on denormal floats cost about two orders of magnitude more on both AMD and Intel
[V S19]. Reverb and filter tails decay into that range. The industry fix is flush-to-zero and
denormals-are-zero on the audio thread. See §11 for why boyko must confine that setting to one
thread. No figure for the reference CPU's generation (Zen 3) was found in this pass, so the design
measures it before it sets a gate threshold on it.

### 2.5 Clocks and sample-accurate scheduling

- Firewheel has three clocks. The seconds clock is an f64 count of stream time, the sample clock
  counts processed samples, and the musical clock is advanced manually in beats (f64). Events carry
  delays in those units [V S43].
- Kira has clocks and tweens [V S41].
- MetaSounds triggers fire at an exact sample index inside a block, a 0.02 ms resolution at 48 kHz
  [V S21].

### 2.6 Floating-point determinism in Rust

The Rust reference for `f32` separates two classes of operation [V S63]:
- **Guaranteed.** Addition, subtraction, multiplication and division round ties-to-even per IEEE
  754-2008. `sqrt` and `mul_add` return the correctly rounded infinite-precision result, "guaranteed
  not to change".
- **Not guaranteed.** `sin`, `cos`, `exp` and `ln` carry the note "Unspecified precision": their
  precision "varies by platform, Rust version, and can even differ within the same execution from
  one invocation to the next".

So an audio kernel is bit-reproducible across hosts and toolchains only if it keeps to the first
class. DSP coefficient formulas (biquad, one-pole), sine-cosine pan laws and logarithmic attenuation
curves all reach for the second class unless it is replaced.

---

## 3. Decoding and formats

| Codec | Evidence |
|---|---|
| PCM | No decode. The per-instance cost is the resampler and the mix. FMOD recommends PCM for short sounds [S30]. |
| IMA / MS ADPCM, FMOD FADPCM | The cheapest compressed option. FMOD recommends FADPCM as its primary format [S30]. FMOD's per-instance memory: ADPCM 3,128 B, Vorbis 23,256 B [S30]. Phoboslab (2023): 4-bit ADPCM is still used where many different effects must play at once [V S35]. |
| QOA | 3.2 bits per sample; a sign-sign LMS predictor with 4 weights; 20-sample slices; about 400 lines of C. Its author: slower than ADPCM, and does not compress as well as MP3 [V S35]. Godot 4 is reported to import it [unverified in this pass]. |
| Vorbis | 1.5–3× the CPU of ADPCM in Wwise's guidance [S31]. Symphonia takes 317.6 ms against FFmpeg's 302.9 ms on the same file (i7-4790K). The benchmark page does not state file durations, so only ratios are usable [V S36]. The pure-Rust `lewton` was 1.30–1.44× slower than libvorbis in its PR history [S37]. |
| Opus | About 11 MHz for 128 kb/s stereo on Haswell with the float build [2 S38]. 5–10× ADPCM [S31]. The FAQ: complexity varies a great deal with settings; the default build is float; decoding is internal at 48 kHz; seeking needs 80 ms of pre-roll to converge [V S38]. Specified by RFC 6716 (SILK + CELT + hybrid) [S40]. |
| FLAC | RFC 9639 (2024) [V S39]. Symphonia decodes it in 0.71× FFmpeg's time [V S36]. |
| Bink Audio | Epic says it decodes closer to ADPCM speed than to MP3 or Vorbis, at about 10:1. Closed source [S34]. |

**Streaming** [V S12][S32][V S38]:
- miniaudio decodes streams in 1-second pages and keeps two pages per stream.
- Wwise needs Vorbis seek tables for virtual voices set to "play from elapsed time".
- Opus seeking needs 80 ms of pre-roll.

---

## 4. Mixer, DSP graph, voice management

- **Buses and sends.**
  - Unreal's submix graph renders child submixes before sources. Every source carries a high-pass
    and a low-pass filter for distance and occlusion [V S20].
  - Kira has main, sub, send and spatial tracks [V S41].
  - In bevy_seedling a bus is only a label on an ordinary node [V S44].
- **Virtualization and priority.**
  - FMOD: priority 0–256, then the quietest audibility is stolen first [S29].
  - Wwise: a voice below threshold is killed, continued or sent to virtual. Resume modes are "play
    from beginning", "play from elapsed time" and "resume"; ties go to discard-oldest or
    discard-newest [S32][S33].
  - Unreal concurrency rules: Prevent New, Stop Oldest, Stop Farthest then Oldest, Stop Lowest
    Priority, Stop Quietest, and others, plus ducking and a retrigger time [V S22].
- **Loudness-driven culling against importance.**
  - Frostbite's HDR audio measures loudness at the listener and keeps a sliding window within a
    range of up to about 130 dB, culling below it. It says this is not compression [S54].
  - Overwatch rejected HDR in favour of weighted importance factors ("Shot At" 0.6, "Damaged" 0.5)
    and buckets 1 / 2 / 4–10 / the rest, mapped to HIGH / NORMAL / LOW / CULL. It drives makeup
    gain, filter and pitch from the resulting importance through an RTPC [V S51].
- **Snapshots, ducking, modulation.** Unreal Audio Modulation provides Control Buses, Control Bus
  Mixes and generators (LFO, envelope follower), evaluated on the audio render thread [V S24].
- **No virtual dispatch in the graph.** MetaSounds compile a graph into a static, non-virtual C++
  object with no data copies between nodes [V S21].
- **Sample-rate conversion.**
  - Godot 3 used linear interpolation for WAV and cubic for other streams; its redesign batches
    resampling by sample rate [V S26].
  - XAudio2 runs sub-graphs at the source rate (multirate) [V S5].
  - The theory is J.O. Smith's bandlimited windowed-sinc method [V S58].
- **Click safety.**
  - Every parameter that affects volume must change smoothly over many frames [V S27].
  - In Godot 3, freeing a node while it played caused a pop [V S26].

---

## 5. Spatialization

- **Panning.**
  - VBAP is the standard amplitude panning method for arbitrary loudspeaker layouts: V. Pulkki,
    "Virtual Sound Source Positioning Using Vector Base Amplitude Panning", *J. Audio Eng. Soc.*
    45(6):456–466, 1997 [V S60].
  - Stereo engines use a constant-power law: Kira pans left/right per spatial track [V S41], and
    Unreal offers either panning or a binaural plugin [V S23].
- **Distance attenuation.**
  - Unreal shapes: linear, logarithmic, inverse, log-reverse, natural [V S23].
  - miniaudio: none, inverse (equivalent to `AL_INVERSE_DISTANCE_CLAMPED`), linear, exponential;
    plus cones and Doppler [V S12].
- **Doppler and propagation delay.** oddio models both [V S42].
- **Per-source HRTF.**
  - Steam Audio's binaural effect interpolates nearest-neighbour or bilinearly. Bilinear carries
    relatively high CPU overhead [V S47].
  - HRTF datasets use SOFA, standardised as AES69-2015 and reaffirmed in 2020 and 2022 [V S57].
    SOFA files are netCDF-4/HDF5 containers.
  - Fyrox uses IRCAM HRIR spheres [V S46].
- **Ambisonic bus.** Resonance Audio projects every source into one high-order ambisonic field and
  applies the HRTF once, so the cost per source stays minimal [S49][V S49 concepts].
- **OS object rendering.** Windows Spatial Sound, with the object limits in §1.1 [V S6].

---

## 6. Environmental acoustics

| Method | Shipped example and numbers |
|---|---|
| **Raycast occlusion** | **Unreal:** a line trace on the Visibility channel, simple collision by default. It applies a low-pass cutoff and volume scaling with an interpolation time [V S23]. **Overwatch:** a single raycast is "usually on/off", improved with path-diversion percentages [V S51]. **Steam Audio:** raycast, or volumetric occlusion with `numOcclusionSamples` plus `numTransmissionRays`. Direct simulation must not run on the audio thread when occlusion is on [V S47]. |
| **Graph propagation** | **Rainbow Six Siege:** propagation-node path cost from length, accumulated angles and a destruction penalty. The sound is virtually repositioned to simulate diffraction [V S52]. **Wwise rooms and portals:** diffraction angle maps to obstruction, transmission to occlusion [S50]. **The Last of Us:** reverb propagated through portals [S53]. |
| **Geometric ray tracing** | **Steam Audio** real-time reflections carry significant CPU cost, controlled by `maxNumRays`, `numDiffuseSamples`, duration, ambisonic order and `numThreads`. It also offers baked reflections and pathing, and convolution, parametric, hybrid or TrueAudio Next (GPU) reverb [V S47][S59]. |
| **Baked wave simulation** | **Triton / Project Acoustics** shipped in Gears of War 4 and 5, Sea of Thieves and Borderlands 3. It encodes obstruction, occlusion, portaling, reverberance and decay [V S48], with data compressed to about 100 MB scale [S48 paper]. Version 3.0 (vendor numbers): ACE files 2–3× smaller; a query costs 15 µs on a cache hit and 40 µs on a miss on PC; HRTF data went from 32 MB to 0.23 MB; "150 sources ≈ 3% CPU" [V S48]. |
| **Reverb DSP** | **FDN history** [V S61]: Gerzon proposed an orthogonal-matrix feedback reverberator in 1971. Stautner and Puckette gave a four-channel FDN and its stability conditions in 1982. Jot (1991–92) developed a systematic design with reverberation time set largely independently per frequency band. Also convolution, and hybrid schemes (Steam Audio, with an example transition at 1.0 s and a 0.25 overlap) [V S47]. |

---

## 7. Music

- The iMUSE patent (US 5,315,057, LucasArts, 1994) uses composer-placed decision points [V S56].
- Modern equivalents are clock-quantized scheduling:
  - Kira clocks [V S41];
  - Firewheel's musical clock [V S43];
  - MetaSounds' sample-exact triggers [V S21].
- The body of the Unreal Quartz page could not be retrieved in this pass.

---

## 8. Gameplay API, tooling, determinism

- **Bevy / bevy_seedling.** A sound is an entity: `commands.spawn(SamplePlayer::new(...))`
  [V S44].
- **Kira.** `AudioManager` returns `ResourceLimitReached` when a fixed capacity is exhausted, so
  capacities are set up front [V S41].
- **Overwatch.** Wwise events are triggered from the TED editor [V S51].
- **CryEngine ATL.** Middleware (Wwise, FMOD, ADX2, SDL_mixer) is abstracted behind "Audio
  Controls" [S55].
- **id Tech.** No reliable public information was found.
- **Determinism.** No surveyed source ties audio to replay. boyko's replay design excludes audio from
  its promise explicitly (§11) [R].

---

## 9. Engine-by-engine summary

| Engine / library | Threads and graph | Voices and spatial | Integration and licence | What boyko can take |
|---|---|---|---|---|
| **Wwise** | Not verified in this pass | Virtual voices (kill / continue / virtual; resume modes) [S32]; rooms and portals [S50] | Closed SDK with a licence; conflicts with owner rule 5 | The virtual-voice resume modes; rooms and portals as a later rung |
| **FMOD** | Not verified in this pass | Priority 0–256 plus audibility [S29] | Closed SDK with a licence | The priority-then-audibility steal order |
| **Unreal Audio Mixer + MetaSounds** | Three threads, N buffers ahead, async decode tasks [V S20]; static non-virtual graphs [V S21] | Concurrency rules [V S22]; attenuation shapes; line-trace occlusion [V S23]; modulation [V S24] | Engine source (EULA) | Thread split, decode-ahead, concurrency vocabulary, per-source HPF/LPF, compile-to-static |
| **Unity DSPGraph** | Burst jobs, command blocks [V S25] | — | Still a preview package | Atomic command batches |
| **Godot 4** | `AudioServer`, `SafeList` [V S27]; 1024-frame blocks [2 S28] | Positional audio | Open source (licence not checked in this pass) | The click-safety lessons [V S26][V S27] |
| **miniaudio** | Device thread plus spinlocked job queue [V S12] | Attenuation models, cones, Doppler [V S12] | Public domain / MIT-0; one C file | The attenuation vocabulary; paged streaming |
| **Kira** | `rtrb` command ring; tracks; clocks [V S41] | Spatial tracks (attenuation, pan) | Rust crate (licence not checked) | Clocks, tweens, fixed capacities |
| **oddio** | — | Doppler and propagation delay [V S42] | Rust crate | Propagation delay as a start offset |
| **bevy_audio** | rodio 0.22 on Bevy main [V S45] | Basic | Bevy | The Bevy team's own list of rodio's gaps (no buses, a type per effect, data hidden from the ECS) [V S45] |
| **Firewheel / bevy_seedling** | One audio thread, no mutexes, grouped event flush, three clocks [V S43] | HRTF on its roadmap | Rust crates; its readiness notes mention "nontrivial UB" [V S45] | Components diffed into events; grouped flush |
| **bevy_kira_audio** | Not fetched in this pass; known to wrap Kira [unverified] | — | Rust crate | — |
| **Fyrox (fyrox-sound)** | — | HRTF from IRCAM HRIR spheres [V S46] | Rust crate (licence not checked) | — |
| **Steam Audio** | Its own threads | Binaural, occlusion, transmission, reflections, pathing, bake [V S47] | Apache-2.0 since 2024, C++ | The occlusion and transmission model; the hybrid reverb split |
| **Resonance Audio** | — | Ambisonic bus plus one HRTF [S49] | Open source (licence not checked in this pass) | The ambisonic-bus design |
| **Project Acoustics (Triton)** | Lookup only | Baked wave acoustics [V S48] | Proprietary, cloud bake | Its parameter set (obstruction, occlusion, portal direction, decay) as a target vocabulary |
| **CryEngine ATL** | — | — | Middleware abstraction [S55] | — |
| **Frostbite** | — | HDR audio window [S54] | — | Taken as a counter-example (Overwatch §4) |
| **XAudio2** | OS engine; the API takes critical sections [V S5] | Filters, submixes | OS component, Windows only | Nothing; §1.1 explains why |

---

## 10. Pitfalls documented by practitioners

1. A lock, an allocation, a free or a blocking system call on the audio thread [S14][S15][V S16].
   `try_lock` on a mutex is not safe either [S15].
2. Priority inversion: the real-time thread waiting on a normal-priority worker [S14][S15]. Unreal
   decodes asynchronously ahead of time instead of waiting [V S20]. This applies directly to
   boyko's pool, which has no priority lanes (§11).
3. Denormal stalls of about 100× [V S19].
4. Clicks and zipper noise from unramped parameters, and pops when a playing voice is removed
   [V S26][V S27].
5. Exclusive mode silences other applications; another client can lock the engine period; low
   latency costs power [V S1][V S2].
6. PipeWire's 21 ms default quantum, and timer-driven ALSA plugins [V S8][V S10].
7. A virtual voice resumed at its elapsed time needs a seekable codec: Vorbis seek tables [S32],
   Opus 80 ms pre-roll [V S38].
8. Resampling quality that differs by source type [V S26].
9. The rodio model: no buses, a new type per effect, component data hidden from the ECS [V S45].
10. Single-ray occlusion is binary and needs interpolation [V S51][V S23]. Real-time ray-traced
    reflections are CPU-heavy [V S47].
11. Spatial Sound object caps (20 on Atmos over HDMI) force prioritisation [V S6].
12. HDR mixing can hide threat cues; Overwatch rejected it [V S51].
13. XAudio2's critical sections and blocking `DestroyVoice` [V S5].
14. **Repository-specific.** A thread holding no diagnostics lane that logs at `Warn` or `Error`
    takes the synchronous output channel, which is bounded by a 50 ms acquire deadline
    (`crates/boyko_log/src/lane.rs`, `emit_to` and `emit_unlaned`) [R]. An audio thread must hold a
    lane or must not log at those levels.
15. **Repository-specific.** On Linux a new thread inherits its creator's MXCSR
    (`crates/boyko_threadpool/tests/mxcsr_uniform.rs`, module doc) [R]. A flush-to-zero setting
    applied on a thread that later spawns threads, or on the main thread before the pool starts,
    spreads to those threads.
16. **Threads that a library creates for you.** An ALSA plugin can start its own loop thread inside
    `snd_pcm_open` [V S66]. On Linux that thread inherits the caller's MXCSR, even when the caller
    itself never spawns anything.
17. **Transcendental functions are not reproducible.** Rust leaves the precision of `sin`, `cos`,
    `exp` and `ln` unspecified across platforms, versions and even calls [V S63]. Goldens over code
    that uses them can red on another host or toolchain with no code change.
18. **Small periods reach other applications.** One client's low-latency request switches every
    application on the endpoint to the small buffer [V S1].
19. **Repository-specific: a heap counter is blind to kernel columns.** The G2 census states that
    its `#[global_allocator]` sees the Rust heap only, and that "a column that committed one more
    granule every frame would read 0" (`crates/boyko_physics/tests/alloc_frame_census.rs`, section
    "Coverage boundary") [R]. An allocation-free proof for a thread that touches kernel columns
    needs a commit-side check as well.

---

## 11. What boyko has — repository inventory at `6394bc5e`

All [R]. The design document assigns each fact a role.

**Absent**
- **Audio code.** None: no crate, backend, decoder, mixer or component. A grep of `crates/` for
  `audio|wasapi|xaudio` finds doc-comment mentions only.
- **A world ray query.**
  - `crates/boyko_math/src/ray.rs` has `Ray`, `ray_sphere` and `ray_aabb`, used by the world-space UI
    cursor pick. Nothing casts a ray against the collider set or a spatial index.
  - The SI1 row of `docs/unification/UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md` lifts the physics tree
    broadphase into a kernel feature with sphere, AABB and user-predicate arms. It has no ray arm.

**Threads and scheduling**
- **Pool.** `crates/boyko_threadpool/src/thread_pool.rs`:
  - `MAX_WORKERS = 64`;
  - the default worker count is `available_parallelism()`;
  - worker affinity is a no-op stub.

  No thread priority is set anywhere in the tree.
- **Waits.** The scheduler dispatcher parks with `PARK_TIMEOUT = 100 µs`
  (`crates/boyko_ecs/src/ecs/core/schedule/schedule.rs`).
- **Timer resolution.** `timeBeginPeriod(1)` is held for the run by the host only
  (`crates/boyko_app/src/timer_resolution.rs`, `ENGINE_PERIOD_MS`). The module states the rule that a
  library may not change process-wide state; only the host may.
- **Fixed step.** The default fixed step is 15.625 ms (64 Hz)
  (`crates/boyko_ecs/src/ecs/core/time/fixed_time.rs`, `DEFAULT_FIXED_TIMESTEP`). `CoreSchedule`
  has `Main` and `Fixed` (`crates/boyko_ecs/src/ecs/core/app/app.rs`). At the defaults the fixed
  loop runs at most 16 substeps per frame, and a paused frame runs 0
  (`crates/boyko_ecs/src/ecs/core/time/fixed_loop.rs`, `fixed_advance` doc).
- **Virtual and real time.** `Time` (`crates/boyko_ecs/src/ecs/core/time/time.rs`) keeps a virtual
  delta that is clamped by `max_delta`, scaled by `relative_speed` and zero while paused (`pause`,
  `unpause`, `is_paused`), beside a raw real delta that ignores pause. So virtual time and a
  wall-clock or QPC-derived clock diverge whenever the game pauses or changes speed.
- **Ordering sets.** `crates/boyko_scene/src/sets.rs` defines `FixedSet::{Gameplay, Snapshot}` and
  `CameraSet::{Control, Resolve}`. `CameraSet::Resolve` is where `GlobalTransform` is propagated.
- **Host on Linux.** The non-Windows `run_windowed` is a stub that reports "windowing unsupported"
  (`crates/boyko_app/src/runner.rs`).

**Transport and diagnostics**
- **SPSC byte ring.** `LogLane` in `crates/boyko_log/src/lane.rs`:
  - it caches the opposite cursor and pads three partitions apart;
  - its module doc cites one published measurement in which caching plus padding moved a ring from
    about 32 to about 440 M ops/s, while padding alone made it slower;
  - it is `pub(crate)` to `boyko_log`.
- **Dedicated-thread precedent.** The `boyko-log-sink` thread is spawned with
  `std::thread::Builder` (`crates/boyko_log/src/lifecycle.rs`). The consumer role is a CAS'd token
  (`crates/boyko_log/src/drain_owner.rs`, `DRAIN_OWNER`).
- **Diagnostics lanes.** `crates/boyko_diag/src/lane.rs`:
  - `LANE_COUNT = 80`, with 14 claimable spares from `LANE_SPARE_BASE = 66`;
  - `claim_lane` and `release_lane`. `claim_lane`'s doc names "an OS or driver callback" as a
    claimant, and on exhaustion it returns `None` and never blocks.
- **Profiling scope.** The profiling plan reserves an engine-scope bit named "Audio" in `ARM_MASK`
  bits 8..31 ([PROFILING-SYSTEM-PLAN.md](../PROFILING-SYSTEM-PLAN.md), D20).
- **Engine packages.** A new engine crate must be added to `ENGINE_PACKAGES`
  (`crates/boyko_diag/src/sample.rs`); `tests/engine_packages_census.rs` enforces both directions.
- **Ignored-test classes.** The vocabulary is closed and CLAUDE.md-owned (`IGNORE_CLASSES` in
  `tests/ignore_reasons_census.rs`). It has no class for "needs an audio endpoint".

**Storage and assets**
- **Kernel storage today.**
  - `ComponentPool`: address-stable, VM-backed (`crates/boyko_ecs/src/ecs/memory/component_pool.rs`).
  - `ScratchColumn<T>`: Copy-only, with a per-element `row_ptr` view for workers
    (`crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs`). It needs a registered
    `ComponentId`; physics reserves a band of them (`crates/boyko_physics/src/scratch_ids.rs`).
  - `VmColumn<T>`, which backs `LogRing`'s byte arena, is `pub(crate)`
    (`crates/boyko_ecs/src/ecs/memory/vm_column.rs`, `crates/boyko_ecs/src/ecs/core/log/ring.rs`).
  - Growth is a VM commit on the pushing thread: `ComponentPool` grows in place by committing fresh
    pages at the frontier of the same reservation (the `ScratchColumn` doc in `scratch_column.rs`).
    `ScratchColumn::solve_base` hands out the raw base pointer. The commit call itself is
    `VmReservation::commit` (`crates/boyko_ecs/src/ecs/memory/vm.rs`), `pub(crate)`.
  - Under Miri, `VmReservation` falls back to `alloc_zeroed`; the module doc says Miri can validate
    only that fallback arm (`crates/boyko_ecs/src/ecs/memory/vm.rs`, module doc).
  - A "committed does not move" check already exists as a gate pattern: clause (d) of T-RED-1,
    "`committed_slots()` does not move across the window"
    (`crates/boyko_ecs/tests/em_deferred_recycle.rs`).
- **Kernel storage planned** ([UNIFIED-SYSTEM-PLAN-01-KERNEL-CONTRACT.md](../unification/UNIFIED-SYSTEM-PLAN-01-KERNEL-CONTRACT.md)):
  - KC-03: `ByteColumn = VmColumn<MaybeUninit<u8>>` in `boyko_memory`, with push/pop stack helpers,
    `ensure_len_zeroed(n)` and `AtomicU64::from_ptr` reader views (KF-42); its threading contract is
    "single writer under `&mut`";
  - KC-10: `ScratchColumn::<T>::for_type(rows)`, registry-free, lands at D-S2. Its `Default` is an
    unreserved column that reserves at the first push;
  - KC-15: `SegmentedColumn<T: Copy>` (ED15 in
    [ENGINE-RUNTIME-ECS-DESIGN.md](../unification/ENGINE-RUNTIME-ECS-DESIGN.md)): power-of-two
    classes, LIFO class free lists, no compaction. A span relocates only when it grows past its
    class, and the base never moves, so "a `SpanRef` stays valid until its own span relocates or is
    freed". Its internal slack is at most 2× per span;
  - KC-16: `OwnedColumn<T>` with `retain_ready(epoch)`. It retires `T` rows, not byte ranges;
  - KC-26: event lanes with per-type overflow policy, and `OsEventSink<E>`, which is dispatcher-only
    and "Copy but not Send" ([ENGINE-RUNTIME-ECS-DESIGN.md](../unification/ENGINE-RUNTIME-ECS-DESIGN.md), §11);
  - KC-04: an engine thread context in which a non-worker thread makes one cold claim.
- **Assets.** `Assets<T>` is a VM-native, `ComponentPool`-backed store with refcount, `RetireTicket`
  and generations (`crates/boyko_ecs/src/ecs/core/asset/assets.rs`). `AssetServer::load` reads and
  decodes synchronously on the caller's thread (`crates/boyko_ecs/src/ecs/core/asset/server.rs`).
  No IO thread exists.
- **Gaia.** "sound" is already one of Gaia's asset reference kinds, resolved to a path-name hash and
  looked up through `PathIndex::lookup` ([gaia/DECISIONS.md](../gaia/DECISIONS.md)).
- **Aether.** An Aether machine's push event lane feeds "sound, telemetry, netcode — one frame later"
  ([aether-v2/MACHINES.md](../aether-v2/MACHINES.md)).

**SDF geometry on the CPU**
- **The analytic field.** `crates/boyko_sdf_math/src/lib.rs`:
  - `no_std` (it links `std` only for `sqrt` on stable) and allocation-free;
  - `MAX_SDF_EDITS = 16`;
  - `sdf_edit_list` is the one CPU↔GPU oracle.
- **The 8-wide evaluator.** `sdf_edit_list_x8` in `crates/boyko_physics/src/sdf_simd.rs` evaluates 8
  points bit-identically to the scalar oracle, with no FMA. Its body is 210 instructions
  ([physics/FMA-DETERMINISM-MEASUREMENTS.md](../physics/FMA-DETERMINISM-MEASUREMENTS.md)).
- **Where the edit list comes from.** The render authoring component is `SdfPrimitive`, gathered
  once at startup into `SdfEditStaging` (`crates/boyko_render/src/sdf_edit.rs`); dynamic per-frame
  edits are deferred. Physics holds its own CPU copy as the `SdfField` resource
  (`crates/boyko_physics/src/sdf_query.rs`).
  - The gather `collect_sdf_edits` clamps to `MAX_SDF_EDITS`: "excess primitives are silently
    dropped, matching the shader's edit-count clamp".
  - The module doc defers three things to a separate campaign: dynamic per-frame edits, the
    streaming brick clipmap accelerator, and raising `MAX_SDF_EDITS`. Content that lands through
    either of the last two would not be in the CPU analytic field.
- **The eDSL's host oracle.** The `f32` backend of `boyko_shaderdsl` forwards `sin` and `cos` to
  `f32::sin` and `f32::cos`, or to the nightly intrinsics (`crates/boyko_shaderdsl/src/cf.rs`;
  `crates/boyko_shaderdsl/src/interp.rs`). It is not a source of host-independent transcendentals.
- **Mesh distance fields.** A mesh→SDF baker exists (`crates/boyko_sdf_math/src/mesh_sdf.rs`,
  `MeshSdfField`). Its output is uploaded to a GPU texture (`crates/boyko_rhi_vulkan/src/mesh_sdf_texture.rs`).
  Whether a CPU copy is kept at runtime was not established.
- **Collider shapes.** Physics colliders are `ColliderShape::{Sphere, Box}` only
  (`crates/boyko_physics/src/components.rs`).

**Floating point and determinism**
- **MXCSR.**
  - No source writes MXCSR, and gate G-FP pins every pool worker to the IEEE default
    (`crates/boyko_threadpool/tests/mxcsr_uniform.rs`). Its module doc records the platform split: "on
    Linux a thread inherits its creator's MXCSR, so such a mutation reds only on Windows". That
    implies a new Windows thread takes the state the OS gives it rather than its creator's.
  - [physics/FMA-DETERMINISM.md](../physics/FMA-DETERMINISM.md) names a third-party runtime that sets
    flush-to-zero on its init thread ("some audio and graphics runtimes document exactly this") as a
    hazard to the inline-versus-worker bit-identity of the physics colour solve.
  - The ISA baseline is `x86-64-v3` (`crates/boyko_physics/src/sdf_simd.rs`, module doc).
- **Replay.** Replay promises exactly the scope of its digest: "Not rendering, not audio"
  ([editor/EDITOR-REPLAY.md](../editor/EDITOR-REPLAY.md), point 3). The replay crate's tick-boundary
  checks include MXCSR (KC-37 / RP-2 in the unified plan).

**GPU**
- **RHI.** One GRAPHICS|COMPUTE queue family (`find_queue_family` in
  `crates/boyko_rhi_vulkan/src/device.rs`). `vkCmdDispatchIndirect` is present
  (`cmd_dispatch_indirect`). `DrawIndirectCount` and timeline semaphores are absent.
  `FRAMES_IN_FLIGHT = 2` (`crates/boyko_rhi_vulkan/src/present/mod.rs`).
- **Particles.** Whether gameplay or audio may read GPU particle counts back is open
  ([PARTICLES-RESEARCH.md](../PARTICLES-RESEARCH.md), open question 8).

**Allocation gates**
- G2 exists and is green (`crates/boyko_physics/tests/alloc_frame_census.rs`). Its "Coverage
  boundary" section states that it counts the Rust heap only: kernel columns grow by
  `VirtualAlloc(MEM_COMMIT)`, which it does not see.
- The deny-after-steady gate allocator (`DenyAfterSteady`) is designed in
  [memory/ALLOCATOR-DESIGN-SPACE.md](../memory/ALLOCATOR-DESIGN-SPACE.md). Its Miri arm counts
  instead of aborting, and the deny itself is `cfg(not(miri))`.
- The opt-in counting shim is `crates/boyko_app/src/profiling/alloc_shim.rs`.
- No shipped image installs a global allocator.

**Reference machine.** The owner's workstation, a Ryzen 9 5900HS (8C/16T), is the in-tree CPU
reference (for example `docs/measurements/2026-09-25-physics-window7/README.md`). Its L2 is 512 KiB
per core, shared only by the two SMT siblings
(`docs/measurements/2026-09-19-physics-p0/p0b/diag_report.md`).

---

## 12. Academic and industry works

- V. Pulkki, "Virtual Sound Source Positioning Using Vector Base Amplitude Panning", *JAES* 45(6),
  456–466, 1997 [V S60].
- J.-M. Jot, the FDN design methodology of 1991–92; M. Gerzon 1971; J. Stautner and M. Puckette
  1982 — all as summarised by J.O. Smith [V S61].
- N. Raghuvanshi and J. Snyder, "Parametric wave field coding for precomputed sound propagation",
  SIGGRAPH 2014, doi 10.1145/2601097.2601184. The 2018 follow-up is "Parametric directional coding
  for precomputed sound propagation", doi 10.1145/3197517.3201339. The PDFs could not be parsed, so
  their tables are unread [S48].
- M. Gorzel et al., "Efficient Encoding and Decoding of Binaural Sound with Resonance Audio", AES
  2019 [S49].
- M. Wittmann et al., arXiv:1506.03997, 2015, on denormal costs [V S19].
- Land and McConnell, US 5,315,057, 1994 (iMUSE) [V S56].
- N. Taylor, "Understanding Wwise Virtual Voices", *Game Audio Programming 2* [S33].
- J.O. Smith, *Physical Audio Signal Processing*, and the resampling notes [V S58].

---

## Sources

- [S1] https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/low-latency-audio
- [S2] https://learn.microsoft.com/en-us/windows/win32/coreaudio/exclusive-mode-streams
- [S3] https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient3-initializesharedaudiostream
- [S4] https://learn.microsoft.com/en-us/windows/win32/procthread/multimedia-class-scheduler-service
- [S5] https://learn.microsoft.com/en-us/windows/win32/xaudio2/xaudio2-introduction
- [S6] https://learn.microsoft.com/en-us/windows/win32/coreaudio/spatial-sound
- [S7] https://docs.rs/windows-sys/latest/windows_sys/Win32/Media/Audio/index.html
- [S8] https://docs.pipewire.org/page_man_pipewire_conf_5.html
- [S9] https://docs.pipewire.org/page_overview.html
- [S10] https://nyanpasu64.gitlab.io/blog/low-latency-audio-output-duplex-alsa/
- [S11] https://www.phoronix.com/news/SDL-3.0-Prefer-PipeWire
- [S12] https://github.com/mackron/miniaudio · https://miniaud.io/docs/manual/index.html
- [S13] https://github.com/mozilla/cubeb/pull/530
- [S14] http://www.rossbencina.com/code/real-time-audio-programming-101-time-waits-for-nothing
- [S15] https://timur.audio/using-locks-in-real-time-audio-processing-safely
- [S16] https://micahjohnston.com/posts/basedrop/
- [S17] https://github.com/mgeier/rtrb
- [S18] https://github.com/Windfisch/rust-assert-no-alloc
- [S19] https://arxiv.org/abs/1506.03997
- [S20] https://dev.epicgames.com/documentation/en-us/unreal-engine/audio-mixer-overview-in-unreal-engine
- [S21] https://dev.epicgames.com/documentation/en-us/unreal-engine/metasounds-the-next-generation-sound-sources-in-unreal-engine
- [S22] https://dev.epicgames.com/documentation/en-us/unreal-engine/sound-concurrency-reference-guide
- [S23] https://dev.epicgames.com/documentation/en-us/unreal-engine/sound-attenuation-in-unreal-engine
- [S24] https://dev.epicgames.com/documentation/en-us/unreal-engine/audio-modulation-overview?application_version=4.27
- [S25] https://docs.unity3d.com/Packages/com.unity.audio.dspgraph@0.1/manual/index.html
- [S26] https://github.com/godotengine/godot-proposals/issues/2299
- [S27] https://gist.github.com/ellenhp/0e06c066b83dc30c2281e9f07a67a9b1
- [S28] https://deepwiki.com/godotengine/godot/4.14-audio-system
- [S29] https://www.fmod.com/docs/2.00/api/white-papers-virtual-voices.html
- [S30] https://documentation.help/fmod-studio-api/performance_reference2.html
- [S31] https://www.audiokinetic.com/en/blog/a-guide-for-choosing-the-right-codec/
- [S32] https://audiokinetic.com/library/2016.1.6_5926/?id=understanding_virtual_voice_behavior
- [S33] https://www.taylorfrancis.com/chapters/edit/10.1201/b22247-9/understanding-wwise-virtual-voices-nic-taylor
- [S34] https://www.unrealengine.com/en-US/blog/bink-video-and-bink-audio-now-available-in-unreal-engine-for-free
- [S35] https://phoboslab.org/log/2023/02/qoa-time-domain-audio-compression
- [S36] https://github.com/pdeljanov/Symphonia/blob/master/BENCHMARKS.md
- [S37] https://github.com/RustAudio/lewton/pull/30
- [S38] https://www.xda-developers.com/opus-codec-high-quality-audio-32-kbps/ · https://wiki.xiph.org/OpusFAQ
- [S39] https://www.rfc-editor.org/rfc/rfc9639
- [S40] http://tools.ietf.org/html/rfc6716
- [S41] https://docs.rs/kira/latest/kira/ · https://docs.rs/kira/latest/kira/track/index.html · https://github.com/tesselode/kira
- [S42] https://github.com/Ralith/oddio
- [S43] https://github.com/BillyDM/Firewheel · https://github.com/BillyDM/Firewheel/blob/main/DESIGN_DOC.md · https://docs.rs/firewheel/latest/firewheel/
- [S44] https://docs.rs/bevy_seedling/latest/bevy_seedling/
- [S45] https://hackmd.io/XCdCJwhwQq-I06ouNuStGA · https://hackmd.io/@bevy/firewheel_and_seedling · https://github.com/bevyengine/bevy/blob/main/crates/bevy_audio/Cargo.toml
- [S46] https://docs.rs/fyrox-sound/latest/fyrox_sound/
- [S47] https://valvesoftware.github.io/steam-audio/doc/capi/simulation.html · https://valvesoftware.github.io/steam-audio/doc/capi/binaural-effect.html · https://valvesoftware.github.io/steam-audio/doc/unity/settings.html · https://store.steampowered.com/news/app/596420/view/7745698166044243232
- [S48] https://developer.microsoft.com/en-us/games/articles/2022/08/project-acoustics-30-is-now-available/ · https://www.microsoft.com/en-us/research/project/project-triton/ · https://dl.acm.org/doi/10.1145/2601097.2601184 · https://dl.acm.org/doi/10.1145/3197517.3201339
- [S49] https://resonance-audio.github.io/resonance-audio/discover/concepts.html · https://www.researchgate.net/publication/332179730_Efficient_Encoding_and_Decoding_of_Binaural_Sound_with_Resonance_Audio
- [S50] https://www.audiokinetic.com/en/community/blog/rooms-and-portals-with-wwise-spatial-audio/
- [S51] https://archive.org/stream/GDC2016Lawlor/GDC2016-Lawlor_djvu.txt · https://gdcvault.com/play/1023317/Overwatch-The-Elusive-Goal-Play
- [S52] https://www.gamedeveloper.com/design/game-design-deep-dive-dynamic-audio-in-destructible-levels-in-i-rainbow-six-siege-i-
- [S53] https://www.gdcvault.com/play/1020444/Aural-Immersion-Audio-Technology-in
- [S54] https://www.ea.com/frostbite/news/how-hdr-audio-makes-battlefield-bad-company-go-boom
- [S55] https://docs.cryengine.com/display/SDKDOC2/ATL+-+Audio+Translation+Layer
- [S56] https://patents.google.com/patent/US5315057A/en
- [S57] https://www.sofaconventions.org/mediawiki/index.php/SOFA_(Spatially_Oriented_Format_for_Acoustics)
- [S58] https://ccrma.stanford.edu/~jos/pasp/Feedback_Delay_Networks_FDN.html · https://ccrma.stanford.edu/~jos/resample/
- [S59] https://gpuopen.com/learn/beyond-spatial-audio/
- [S60] https://research.aalto.fi/en/publications/virtual-sound-source-positioning-using-vector-base-amplitude-pann
- [S61] https://ccrma.stanford.edu/~jos/pasp/History_FDNs_Artificial_Reverberation.html
- [S62] https://docs.rs/windows-sys/latest/windows_sys/Win32/System/Threading/fn.AvSetMmThreadCharacteristicsW.html · https://docs.rs/windows-sys/latest/windows_sys/Win32/Media/Audio/struct.WAVEFORMATEXTENSIBLE.html
- [S63] https://doc.rust-lang.org/std/primitive.f32.html (the "Unspecified precision" notes on `sin`, `cos`, `exp`, `ln`; the "Precision" notes on `sqrt` and `mul_add`)
- [S64] https://learn.microsoft.com/en-us/windows/win32/coreaudio/audclnt-streamflags-xxx-constants
- [S65] https://learn.microsoft.com/en-us/windows/win32/coreaudio/recovering-from-an-invalid-device-error
- [S66] https://raw.githubusercontent.com/PipeWire/pipewire/master/pipewire-alsa/alsa-plugins/pcm_pipewire.c (`snd_pcm_pipewire_open`; read from the GitHub mirror because gitlab.freedesktop.org refused the fetch)
- [S67] https://man7.org/linux/man-pages/man2/getrusage.2.html
- [S68] https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtuallock
