#!/bin/bash
set -euo pipefail

readonly targets=(x86_64-unknown-linux-musl aarch64-unknown-linux-musl)
readonly zig_required=0.16.0
readonly clang_required="clang version 21.1.0"
readonly name=m2tg

root=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
readonly root
cd "$root"

fail() {
	printf '%s\n' "$@" >&2
	exit 1
}

require_tools() {
	for tool in zig cargo-zigbuild; do
		command -v "$tool" >/dev/null 2>&1 || fail "$tool not found; see CONTRIBUTING.md"
	done
}

require_pinned_zig() {
	local found
	found=$(zig version)
	[ "$found" = "$zig_required" ] || fail \
		"zig $zig_required is pinned, found $found" \
		"another zig links another musl and moves the hash"
}

require_zig_tarball_clang() {
	local banner found
	banner=$(zig cc -target x86_64-linux-musl -v -E -x c /dev/null 2>&1) || true
	found=${banner%%$'\n'*}
	[ "$found" = "$clang_required" ] || fail \
		"zig $zig_required must carry '$clang_required', found '$found'" \
		"install the tarball from https://ziglang.org/download/" \
		"a packaged zig reports the same version but links the system LLVM and moves the hash"
}

use_fresh_zig_cache_owned_by_this_build() {
	export ZIG_GLOBAL_CACHE_DIR="$root/target/zig-cache"
	rm -rf "$ZIG_GLOBAL_CACHE_DIR"
}

remap_build_paths() {
	local sysroot=$1 cargo_home=$2
	for path in "$sysroot" "$cargo_home" "$root"; do
		[[ $path != *" "* ]] || fail "cannot remap '$path': cargo splits RUSTFLAGS on spaces"
	done
	RUSTFLAGS="--remap-path-prefix=$sysroot=/rust"
	RUSTFLAGS+=" --remap-path-prefix=$cargo_home=/cargo"
	RUSTFLAGS+=" --remap-path-prefix=$root=/build"
	export RUSTFLAGS
}

require_no_build_paths() {
	local binary=$1 sysroot=$2 cargo_home=$3
	if LC_ALL=C grep -aq -e "$sysroot" -e "$cargo_home" -e "$root" "$binary"; then
		fail "a build path survived into $binary; the build is not reproducible"
	fi
}

print_sha256() {
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum "$1"
	else
		shasum -a 256 "$1"
	fi
}

require_tools
require_pinned_zig
require_zig_tarball_clang
use_fresh_zig_cache_owned_by_this_build

sysroot=$(rustc --print sysroot)
cargo_home=${CARGO_HOME:-$HOME/.cargo}
remap_build_paths "$sysroot" "$cargo_home"

cargo clean
for target in "${targets[@]}"; do
	cargo zigbuild --target "$target" --release --locked
done

for target in "${targets[@]}"; do
	binary=target/$target/release/$name
	require_no_build_paths "$binary" "$sysroot" "$cargo_home"
	print_sha256 "$binary"
done
