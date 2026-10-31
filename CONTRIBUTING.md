# Contributing to Search CLI

Thanks for your interest in contributing. Here's how to get started.

## Setup

```bash
git clone https://github.com/alepar/search-cli
cd search-cli
cargo build
cargo test
```

Requires Rust 1.89+ and Cargo.

The optional `stealth` feature (`cargo test --features stealth`) builds
BoringSSL: it needs Rust 1.98+, cmake, clang and a C++ compiler. CI checks it
on Linux.

## Making Changes

1. Fork the repo and create a branch from `master`
2. Make your changes
3. Run `cargo fmt`, `cargo test` and `cargo clippy --all-targets -- -D warnings`
4. Open a pull request with a clear description of what you changed and why

## What We're Looking For

- New search provider integrations
- Bug fixes and performance improvements
- Better error messages and documentation
- Test coverage improvements

## Code Style

- Follow existing patterns in the codebase
- Keep functions focused and small
- Add tests for new functionality
- Use meaningful variable names

## Reporting Issues

Open a GitHub issue with:
- What you expected to happen
- What actually happened
- Steps to reproduce
- Your OS and Rust version

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
