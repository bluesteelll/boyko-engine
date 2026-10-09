# Third-party notices

Licensed under the Apache License, Version 2.0 (LICENSE). Some files are third-party and remain
under their own licenses; they are listed in NOTICE and THIRD-PARTY-NOTICES.md.

Nothing below is licensed under Apache-2.0 by this project. Each section names the files, their
upstream, the license they stay under, the upstream copyright notice and the full license text,
copied verbatim from the upstream URL the section names.

No dependency crate is vendored in this repository and no binary is published from it, so the crates
named in `Cargo.lock` carry no notice here.

## SMAA

- **Files:**
  - `crates/boyko_rhi_vulkan/shaders/smaa_common.hlsli`
  - `crates/boyko_rhi_vulkan/shaders/smaa_edge.fs.hlsl` and `crates/boyko_rhi_vulkan/shaders/smaa_edge.fs.spv`
  - `crates/boyko_rhi_vulkan/shaders/smaa_weight.fs.hlsl` and `crates/boyko_rhi_vulkan/shaders/smaa_weight.fs.spv`
  - `crates/boyko_rhi_vulkan/shaders/smaa_blend.fs.hlsl` and `crates/boyko_rhi_vulkan/shaders/smaa_blend.fs.spv`
  - `crates/boyko_render/assets/smaa/AreaTex.bin` and `crates/boyko_render/assets/smaa/SearchTex.bin`
    (extracted byte-for-byte from upstream `Textures/AreaTex.h` and `Textures/SearchTex.h`; see
    `crates/boyko_render/assets/smaa/NOTICE`)
- **Upstream:** <https://github.com/iryoku/smaa>
- **License:** MIT, with the upstream clarification that binary distributions need not carry the
  notice. A source distribution, such as this repository, must.
- **Copyright:**
  - Copyright (C) 2013 Jorge Jimenez (jorge@iryoku.com)
  - Copyright (C) 2013 Jose I. Echevarria (joseignacioechevarria@gmail.com)
  - Copyright (C) 2013 Belen Masia (bmasia@unizar.es)
  - Copyright (C) 2013 Fernando Navarro (fernandn@microsoft.com)
  - Copyright (C) 2013 Diego Gutierrez (diegog@unizar.es)

Full license text (`LICENSE.txt`, <https://github.com/iryoku/smaa/blob/master/LICENSE.txt>):

```text
Copyright (C) 2013 Jorge Jimenez (jorge@iryoku.com)
Copyright (C) 2013 Jose I. Echevarria (joseignacioechevarria@gmail.com)
Copyright (C) 2013 Belen Masia (bmasia@unizar.es)
Copyright (C) 2013 Fernando Navarro (fernandn@microsoft.com)
Copyright (C) 2013 Diego Gutierrez (diegog@unizar.es)

Permission is hereby granted, free of charge, to any person obtaining a copy
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software. As clarification, there is no
requirement that the copyright notice and permission be included in binary
distributions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

## FXAA shader (Rendu)

- **Files:** `crates/boyko_rhi_vulkan/shaders/fxaa.fs.hlsl` and `crates/boyko_rhi_vulkan/shaders/fxaa.fs.spv`
- **Upstream:** <https://github.com/kosua20/Rendu> (`resources/common/shaders/screens/fxaa.frag`).
  The HLSL shader is adapted from that GLSL shader. FXAA itself is Timothy Lottes' algorithm; no
  NVIDIA source text is present in this repository.
- **License:** MIT
- **Copyright:** Copyright (c) 2017 Simon Rodriguez

Full license text (`LICENSE`, <https://github.com/kosua20/Rendu/blob/master/LICENSE>):

```text
MIT License

Copyright (c) 2017 Simon Rodriguez

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Jolt Physics-derived material

- **Files:**
  - `crates/boyko_physics/benches/jolt_parity/pyramid_scene.patch` (a patch against Jolt's
    `PerformanceTest/` sources; it carries their context and removed lines)
  - the 202 files matching `docs/measurements/2026-09-2[5-7]*/**/profile_chart_*.html` (Jolt
    profiler output; each one embeds the HTML/JavaScript chart template of Jolt's
    `Jolt/Core/Profiler.cpp`)
  - `docs/measurements/2026-09-27-physics-window8/tools/patch_jolt_w8.py` (its search strings are
    taken from Jolt sources)
- **Upstream:** <https://github.com/jrouwe/JoltPhysics>
- **License:** MIT
- **Copyright:** Copyright 2021 Jorrit Rouwe

Full license text (`LICENSE`, <https://github.com/jrouwe/JoltPhysics/blob/master/LICENSE>):

```text
Copyright 2021 Jorrit Rouwe

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

## vis-network

- **File:** `tools/arch-graph/vendor/vis-network.min.js`, vis-network 9.1.9, identical to the
  upstream npm artifact `vis-network@9.1.9/standalone/umd/vis-network.min.js`. Its license header
  is intact.
- **Upstream:** <https://github.com/visjs/vis-network>
- **License:** dual Apache-2.0 / MIT ("vis.js may be distributed under either license"). It is used
  here under Apache-2.0, whose full text is `LICENSE` at the repository root.
- **Copyright:**
  - (c) 2011-2017 Almende B.V, http://almende.com
  - (c) 2017-2019 visjs contributors, https://github.com/visjs
- **Bundled:** the file inlines Hammer.JS 2.0.17-rc (the egjs fork by NAVER), MIT. Its in-file
  header ("Copyright (c) hammerjs / Licensed under the MIT license") is intact. Upstream:
  <https://github.com/naver/hammer.js>.
  Copyright:
  - Copyright (c) 2018-present NAVER Corp.
  - Copyright (C) 2011-2017 by Jorik Tangelder (Eight Media)

Full Hammer.JS license text (`LICENSE.md`, <https://github.com/naver/hammer.js/blob/master/LICENSE.md>):

```text
The MIT License (MIT)

Copyright (c) 2018-present NAVER Corp.
Copyright (C) 2011-2017 by Jorik Tangelder (Eight Media)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```

## mermaid

The documentation site (the mdBook under `book/`) loads these two scripts to draw its diagrams. No
crate uses them.

- **Files:**
  - `book/theme/mermaid.min.js`: mermaid 11.2.0. It is byte-identical to
    `src/bin/assets/mermaid.min.js` in the `mdbook-mermaid` 0.14.0 crate, the version that
    `.github/workflows/docs.yml` pins.
  - `book/theme/mermaid-init.js`: byte-identical to `src/bin/assets/mermaid-init.js` in the same
    crate.
- **Upstream:** <https://github.com/mermaid-js/mermaid> (mermaid) and
  <https://github.com/badboy/mdbook-mermaid> (mdbook-mermaid).
- **License:**
  - `mermaid.min.js`: MIT.
  - `mermaid-init.js`: Mozilla Public License 2.0, the license of mdbook-mermaid (`license =
    "MPL-2.0"` in its `Cargo.toml`). The file is unmodified and stays under MPL-2.0; it is its own
    source form. The license text is at <https://mozilla.org/MPL/2.0/>.
- **Copyright:**
  - mermaid: Copyright (c) 2014 - 2022 Knut Sveidqvist
  - mdbook-mermaid: Jan-Erik Rediger (the `authors` field of the crate manifest)
- **Bundled:** `mermaid.min.js` inlines third-party libraries. The license comments that the
  upstream build kept are intact at the end of the file, under "Bundled license information". They
  name DOMPurify 3.1.6 (Apache-2.0 or MPL-2.0, (c) Cure53 and other contributors), Lodash (MIT,
  Copyright OpenJS Foundation and other contributors), cytoscape (MIT) and js-yaml 4.1.0 (MIT).

Full mermaid license text (`LICENSE` at the tag `mermaid@11.2.0`,
<https://github.com/mermaid-js/mermaid/blob/mermaid%4011.2.0/LICENSE>):

```text
The MIT License (MIT)

Copyright (c) 2014 - 2022 Knut Sveidqvist

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Font test fixtures

The two fonts are test fixtures of `boyko_fontbake`. They remain under their own font licences,
which forbid distributing them under any other licence.

- `crates/boyko_fontbake/fixtures/SourceCodePro-Regular.otf`: SIL Open Font License 1.1.
  © 2023 Adobe (http://www.adobe.com/), with Reserved Font Name 'Source'. Source is a trademark of
  Adobe in the United States and/or other countries. The full license text is
  `crates/boyko_fontbake/fixtures/SourceCodePro-OFL.txt`, beside the font.
- `crates/boyko_fontbake/fixtures/Ubuntu-Light.ttf`: Ubuntu Font Licence 1.0. Copyright 2011
  Canonical Ltd. The full licence text is `crates/boyko_fontbake/fixtures/Ubuntu-UFL-1.0.txt`,
  beside the font. Ubuntu and Canonical are registered trademarks of Canonical Ltd.

## Acknowledgements (no license obligation)

The items below carry no license obligation: they are re-derivations, numeric data or
algorithms, credited as sources.

- **AMD FidelityFX CAS** (Contrast-Adaptive Sharpening, MIT, Advanced Micro Devices):
  `crates/boyko_rhi_vulkan/shaders/rcas.comp.hlsl` is a simplified re-derivation, not a copy.
- **Stephen Hill's ACES fit**, with the coefficients as published in MJP's BakingLab (MIT): the
  `ACES_IN` / `ACES_OUT` matrices and `aces_fitted` in
  `crates/boyko_rhi_vulkan/shaders/pbr_lighting.hlsli`, mirrored by the host oracle in
  `crates/boyko_rhi_vulkan/src/goldens.rs`.
- **msdfgen** (Viktor Chlumský, MIT): `crates/boyko_fontbake/src/msdf/color.rs` follows the
  behaviour of msdfgen's `edge-coloring.h` in an independent Rust implementation.
- **Algorithms cited in code comments**, among them Karis/Lottes TAA, Salvi's variance clipping,
  the Hillis-Steele scan and DDGI.
- **Sample models:** the assets listed in `assets/vg_corpus/CORPUS.toml` are fetched by users with
  `scripts/fetch_corpus.ps1` and are not redistributed in this repository. Each entry there records
  its own licence and attribution.
