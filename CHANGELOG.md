# Changelog

All notable changes to LumenCast are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] - 2026-11-02

First release of LumenCast: a rewrite of Blucast as a native Rust, Tauri 2 and
Svelte 5 program.

### Added
- Camera capture (MJPG, YUYV and H264 through the system OpenH264) with the
  negotiated mode shown, and a live preview.
- Effects: background blur, removal and replacement, with a built-in model on
  the CPU or NVIDIA's model on the GPU in an optional container; studio light
  with warmth, strength and an optional light picture (HDR); auto framing;
  video noise reduction; vignette; manual zoom and pan; preview mirror.
- Virtual camera through v4l2loopback, with optional Lanczos upscaling.
- Audio through PipeWire: a virtual microphone and a virtual output with noise
  removal (RNNoise), room echo removal and studio voice.
- System tray: closing the window keeps the program running; the menu shows or
  hides the window, toggles the virtual camera and the autostart, looks for
  updates and quits. A second launch brings the running one to the front.
- Autostart with the session, minimised to the tray.
- In-program updates from GitHub Releases, verified by SHA-256 and a minisign
  signature before anything is installed.
- RPM and DEB packages with icons, a menu entry and the v4l2loopback and udev
  configuration for the virtual camera.
- Spanish and English interface; first-run import of Blucast's settings.

### Notes
- Eye contact was tried (a landmark based redraw and NVIDIA's own model through
  its Triton server) and removed; the code is kept in the git tag
  `eye-contact-nvidia-and-gaze`.
