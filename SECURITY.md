# Security Policy

## Reporting a vulnerability

Please report it privately through
[GitHub Security Advisories](https://github.com/MAlexVR/LumenCast/security/advisories/new)
rather than a public issue. Describe the problem, include steps to reproduce
it and, if you have one, a fix. Receipt is acknowledged within 48 hours, and a
fix for a critical issue is aimed at within 7 days.

## Security considerations

### What the program touches
- It reads your camera and microphone and writes a virtual camera
  (`v4l2loopback`) and two PipeWire devices. Nothing is sent anywhere.
- The only network access is the update check and download, to GitHub's own
  hosts (`api.github.com`, `github.com` and `*.githubusercontent.com`), on
  start (which can be turned off) or when you ask. It sends no data of yours.
- Settings are stored in `~/.config/lumencast/`.

### Updates
- A downloaded update is installed only after its SHA-256 matches
  `SHA256SUMS.txt` **and** its minisign signature verifies against the public
  key built into the program (`crates/lumencast-update/update.pub`). Either
  check failing installs nothing.
- The program re-reads the release itself when installing: the window never
  sends an address or a file name. Installing a package uses `pkexec` (polkit),
  with argument lists and no shell.
- A private repository needs an access token in
  `~/.config/lumencast/update-token`; it is only sent to `api.github.com`.
- The signing key stays with the maintainer, outside the repository.

### The optional NVIDIA container
- It runs on your machine with GPU access (`--device nvidia.com/gpu=all`,
  `--security-opt label=disable`) and publishes its port on the loopback only.
  It is built by you from your own Blucast image; LumenCast ships no NVIDIA code
  or model.

### The program itself
- The Tauri window has no filesystem, shell or opener access: only the program's
  own commands, events and the native file picker.
