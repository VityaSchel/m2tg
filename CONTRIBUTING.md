# Contributing to m2tg

CI runs `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked` and `shellcheck build.sh`. [lefthook](https://github.com/evilmartians/lefthook) formats staged Rust files and runs clippy, tests and shellcheck before each commit after `lefthook install`.

## Release builds

`./build.sh` builds reproducible static musl binaries for x86_64 and aarch64 and prints their SHA-256. It needs cargo-zigbuild 0.22.3 and zig 0.16.0 from the [official tarball](https://ziglang.org/download/). The release.yml workflow builds each tag on Vultr and Hetzner and attaches the binaries to its release only when their hashes match.
