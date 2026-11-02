# Third-party components (LumenCast UI)

| Component | Use | License | Source |
|---|---|---|---|
| Sora (`@fontsource-variable/sora`) | titles and numerals, bundled locally | SIL OFL 1.1 | https://github.com/sora-xor/sora-font |
| Geist (`@fontsource-variable/geist`) | UI text, bundled locally | SIL OFL 1.1 | https://github.com/vercel/geist-font |
| Svelte, Vite, Vitest | UI build and tests | MIT | https://svelte.dev, https://vite.dev, https://vitest.dev |
| Tauri | desktop shell | MIT or Apache-2.0 | https://tauri.app |
| `zune-jpeg` | JPEG decoding of camera frames | MIT, Apache-2.0 or Zlib | https://crates.io/crates/zune-jpeg |
| `jpeg-encoder` | JPEG encoding of preview frames | (MIT or Apache-2.0) and IJG | https://crates.io/crates/jpeg-encoder |
| SeetaFace frontal face detector model (`assets/models/seeta_fd_frontal_v1.0.bin`, SHA-256 `c4619d066ed35e84d9a8e842860b0dff567aba0cbb139881075538761db3ff5d`), embedded in the binary | auto framing | BSD-2-Clause, Copyright (c) 2016 Visual Information Processing and Learning (VIPL) group, Institute of Computing Technology, Chinese Academy of Sciences | https://github.com/seetaface/SeetaFaceEngine |
| `rustface` crate | runs that model, pure Rust | BSD-2-Clause | https://crates.io/crates/rustface |
| `v4l` crate | V4L2 access on Linux | MIT | https://crates.io/crates/v4l |

The design tokens (`ui/src/lib/design/tokens.css`, `base.css`) and the icon drawing style
come from the owner's LumenGet project (MIT) so both applications look alike.

This software is based in part on the work of the Independent JPEG Group (`jpeg-encoder` is derived from IJG code).

## OpenH264 (system library, not shipped)

H264 preview loads the user's own `libopenh264.so.8` (Cisco's binary, installed from the distribution, e.g. `sudo dnf install openh264`) at run time. LumenCast bundles no H264 code; the `openh264` Rust crate (BSD-2-Clause) is only the binding.

## Selfie segmentation model (embedded)

`crates/lumencast-segment/assets/selfie_segmentation_landscape_v1.onnx` (462,338 bytes, SHA-256 `7a0adcfdb1715d3b0ff0f61486d8a181c33f4a208343a5abfccbc6540e872d24`) is MediaPipe's landscape selfie segmentation model (256x144) in ONNX form, published by the `onnx-community/mediapipe_selfie_segmentation_landscape` repository on Hugging Face under the Apache-2.0 license (original model: Google, Apache-2.0). It is run with `tract-onnx` (MIT or Apache-2.0). PNG pictures for the background are read with the `png` crate (MIT or Apache-2.0).

## NVIDIA Maxine VideoFX (not in this repository)

The optional NVIDIA background engine runs NVIDIA's proprietary Maxine VideoFX 0.7.2 libraries and models inside a container the user builds locally from their own Blucast image (`sidecar/nvidia-matte`). Nothing of it is stored or redistributed here; the API headers in `app/third_party` are NVIDIA's MIT-licensed public ones.

## Audio cleaning

Noise removal uses RNNoise (a recurrent neural network for speech enhancement, BSD-3-Clause, Copyright (c) 2017 Mozilla, Jean-Marc Valin) through the `nnnoiseless` crate (BSD-3-Clause), a pure Rust port that embeds the trained weights. The room echo removal and the studio voice are LumenCast's own signal processing; the spectral transforms use the `realfft` crate (MIT). Audio devices are driven with the user's own PipeWire tools (`pw-cat`, `pw-loopback`, `pw-dump`, `pw-link`); nothing of PipeWire is bundled. NVIDIA Broadcast's audio models are not used, copied or redistributed.

## Updates, tray and packaging

The in-program update uses `ureq` (MIT or Apache-2.0) for HTTPS, `sha2` (MIT or Apache-2.0) for checksums, `base64` (MIT or Apache-2.0) and `minisign-verify` (MIT) to verify the signature of a download against the public key in `crates/lumencast-update/update.pub`. The tray icon and the single instance lock come from Tauri's `tray-icon` feature and `tauri-plugin-single-instance` (MIT or Apache-2.0). The RPM and DEB packages are made by Tauri's bundler. The icon and logo (`assets/lumencast.svg`, `assets/lumencast.png`) are LumenCast's own.

## Licenses of what is shipped or built in

The full texts are in [`licenses/`](licenses/) and are installed with the packages in `/usr/share/doc/lumen-cast/`.

| What | License | Text |
|---|---|---|
| MediaPipe selfie segmentation model (embedded), run with `tract` | Apache-2.0 | `licenses/Apache-2.0.txt` |
| SeetaFace frontal face detector model (embedded) and the `rustface` crate | BSD-2-Clause, Copyright (c) 2016 VIPL group, Institute of Computing Technology, Chinese Academy of Sciences | `licenses/BSD-2-Clause-SeetaFace.txt` |
| RNNoise (trained weights, through the `nnnoiseless` crate) | BSD-3-Clause, Copyright (c) 2017 Mozilla, 2007-2017 Jean-Marc Valin, 2005-2017 Xiph.Org Foundation, 2003-2004 Mark Borgerding, 2020 Joe Neeman | `licenses/BSD-3-Clause-RNNoise-nnnoiseless.txt` |
| Sora and Geist fonts (bundled in the interface) | SIL Open Font License 1.1 | `licenses/OFL-1.1-Sora.txt`, `licenses/OFL-1.1-Geist.txt` |
| NVIDIA Maxine API headers in `sidecar/nvidia-matte/headers`, used only to build the optional container | MIT, Copyright (c) 2020 NVIDIA Corporation (the notice is in each file) | in the files |

## Rust libraries

Every Rust dependency is under a permissive license (MIT, Apache-2.0, BSD, Zlib, Unlicense, Unicode-3.0, or a choice between them), checked from the licenses declared in `cargo metadata`, with one family under the file-level copyleft **MPL-2.0**: `cssparser`, `cssparser-macros`, `dtoa-short`, `option-ext` and `selectors` (pulled in by Tauri). They are used unmodified, from crates.io, and their source is available there under the same names and versions as in `Cargo.lock`. WebKitGTK and GTK are linked dynamically from the system (LGPL) and are not distributed with LumenCast.

## What is not here

LumenCast contains no NVIDIA code, model, library or installer, and no code from Blucast beyond what is named above. OpenH264 is not bundled: the program loads the copy installed by the user's distribution. The optional NVIDIA background engine runs NVIDIA's own libraries and models from the container the user builds from their own Blucast image; nothing of them is stored or redistributed here.

