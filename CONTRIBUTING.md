# Contributing to LumenCast

Thank you for your interest in contributing to LumenCast! This document provides guidelines and information for contributors.

## Code of Conduct

This project adheres to a [Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code.

## How to Contribute

### Reporting Bugs

Before creating a bug report, please check the existing issues to avoid duplicates. When filing a bug report, include:

- **Clear title** describing the issue
- **Steps to reproduce** the problem
- **Expected behavior** vs actual behavior
- **System information**: OS, GPU model, driver version
- **Logs** from the terminal output if applicable

### Suggesting Features

Feature requests are welcome! Please:

- Check existing issues and discussions first
- Clearly describe the use case
- Explain why this would benefit other users

### Pull Requests

1. **Fork the repository** and create your branch from `main`
2. **Follow the code style** of the project
3. **Write clear commit messages**
4. **Test your changes** thoroughly
5. **Update documentation** if needed
6. **Submit the PR** with a clear description

## Development setup

You need Rust (the version is pinned in `rust-toolchain.toml`), Node.js with
`pnpm`, and the development packages of WebKitGTK 4.1, GTK 3 and
libayatana-appindicator. See "Building from source" in the
[README](README.md).

```sh
git clone https://github.com/MAlexVR/LumenCast.git
cd LumenCast
pnpm install && pnpm --dir ui install
pnpm tauri build --debug --no-bundle
WEBKIT_DISABLE_COMPOSITING_MODE=1 ./target/debug/lumencast
```

## Before you open a pull request

Run the same checks that every commit here passes:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm --dir ui check && pnpm --dir ui test
```

- Write a test first for a behaviour change, and keep tests next to the code.
  Tests that need hardware are `#[ignore]` and say how to run them.
- Use [Conventional Commits](https://www.conventionalcommits.org/) (`feat(audio): ...`,
  `fix(h264): ...`), with the tests and the docs of a change in the same commit.
- Code, comments, commit messages and documentation are in English; texts the
  user sees go through the i18n dictionaries (`ui/src/lib/i18n`), Spanish and
  English both.
- Do not add NVIDIA code or model files, or models whose licence does not allow
  redistribution. New third party code or models go in `THIRD_PARTY.md`.
- Add a line to the `[Unreleased]` section of the [CHANGELOG](CHANGELOG.md) for
  anything a user would notice.
