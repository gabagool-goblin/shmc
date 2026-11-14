#!/usr/bin/env sh
set -eu

repo_url="https://github.com/gabagool-goblin/shmc"
default_branch="main"
dest=""
version=""
source_only=""

usage() {
    cat <<'EOF'
Usage: install.sh [DIR] [--version TAG] [--source]

Install shmc: the prebuilt binary for this platform, or a build from source.

Arguments:
  DIR              install directory (default: $PREFIX, else ~/.local/bin)

Options:
  --version TAG    install release TAG (for example 0.1.0) instead of the latest
  --source         build from source even when a prebuilt binary exists
  -h, --help       show this help and exit

Prebuilt binaries: macOS on Apple Silicon, Linux on x86_64 and arm64. Each
download is checked against the release's SHA256SUMS. Inside a shmc checkout
the script builds in place; anywhere else it builds from a shallow clone of
https://github.com/gabagool-goblin/shmc in a temporary directory.
EOF
}

die() {
    echo "$1" >&2
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        -h | --help)
            usage
            exit 0
            ;;
        --version)
            [ $# -ge 2 ] || die "error: --version needs a tag, e.g. --version 0.1.0"
            version="$2"
            shift 2
            ;;
        --version=*)
            version="${1#--version=}"
            shift
            ;;
        --source)
            source_only=1
            shift
            ;;
        -*)
            echo "error: unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
        *)
            [ -z "$dest" ] || die "error: only one install directory may be given"
            dest="$1"
            shift
            ;;
    esac
done

# $0 is the shell when piped, so look at the working directory instead.
is_project_dir() {
    [ -f "$1/Cargo.toml" ] && grep -q '^name = "shmc"' "$1/Cargo.toml"
}

need_cargo() {
    command -v cargo >/dev/null 2>&1 || die "error: cargo not found.

Building shmc from source needs a Rust toolchain. Install one from
https://rustup.rs and re-run this script."
}

platform_target() {
    case "$(uname -s)/$(uname -m)" in
        Darwin/arm64) echo aarch64-apple-darwin ;;
        Linux/x86_64) echo x86_64-unknown-linux-gnu ;;
        Linux/aarch64 | Linux/arm64) echo aarch64-unknown-linux-gnu ;;
    esac
}

latest_release() {
    command -v curl >/dev/null 2>&1 || return 0
    curl -fsSI "$repo_url/releases/latest" 2>/dev/null | tr -d '\r' |
        sed -n 's#^[Ll]ocation: .*/releases/tag/##p'
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    fi
}

# Sets $binary, or returns 1 when there is no usable prebuilt binary.
fetch_binary() {
    target="$(platform_target)"
    [ -n "$target" ] || return 1
    command -v curl >/dev/null 2>&1 || return 1
    asset="shmc-$target.tar.gz"
    base="$repo_url/releases/download/$version"
    echo "==> downloading $asset ($version)"
    if ! curl -fsSL -o "$tmp/$asset" "$base/$asset" ||
        ! curl -fsSL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS"; then
        echo "note: no prebuilt binary for $target in $version; building from source" >&2
        return 1
    fi
    got="$(sha256_of "$tmp/$asset")"
    if [ -z "$got" ]; then
        echo "note: no sha256sum or shasum to check the download; building from source" >&2
        return 1
    fi
    want="$(awk -v f="$asset" '$2 == f { print $1 }' "$tmp/SHA256SUMS")"
    [ -n "$want" ] && [ "$got" = "$want" ] ||
        die "error: $asset does not match the release's SHA256SUMS; not installing it"
    tar -xzf "$tmp/$asset" -C "$tmp"
    if ! "$tmp/shmc-$target/shmc" --version >/dev/null 2>&1; then
        echo "note: the prebuilt binary does not run here (an older glibc?); building from source" >&2
        return 1
    fi
    binary="$tmp/shmc-$target/shmc"
}

build_dir=""
if is_project_dir "."; then
    build_dir="$(pwd)"
elif is_project_dir ".."; then
    build_dir="$(cd .. && pwd)"
fi

tmp=""
cleanup() {
    if [ -n "$tmp" ]; then
        rm -rf "$tmp"
    fi
}
trap cleanup EXIT
trap 'exit 130' INT HUP TERM

binary=""
if [ -n "$build_dir" ]; then
    need_cargo
    echo "==> building shmc (release) from $build_dir"
else
    tmp="$(mktemp -d "${TMPDIR:-/tmp}/shmc-install.XXXXXX")"
    [ -n "$version" ] || version="$(latest_release)"
    if [ -z "$source_only" ] && [ -n "$version" ]; then
        fetch_binary || true
    fi
    if [ -z "$binary" ]; then
        need_cargo
        command -v git >/dev/null 2>&1 ||
            die "error: git not found. It is needed to fetch the shmc sources;
install git and re-run this script."
        if [ -n "$version" ]; then
            echo "==> cloning $repo_url at $version"
            git clone --quiet --depth 1 --branch "$version" "$repo_url" "$tmp/src" ||
                die "error: could not clone $repo_url at '$version'. Is that a tag or
branch, and is the network up?"
        else
            echo "==> cloning $repo_url ($default_branch)"
            git clone --quiet --depth 1 "$repo_url" "$tmp/src" ||
                die "error: could not clone $repo_url (branch '$default_branch'). Is the
network up?"
        fi
        build_dir="$tmp/src"
    fi
fi

if [ -z "$binary" ]; then
    cd "$build_dir"
    cargo build --release --locked
    binary="$build_dir/target/release/shmc"
fi

if [ -z "$dest" ]; then
    dest="${PREFIX:-$HOME/.local/bin}"
fi

case "$dest" in
    /opt/homebrew/bin)
        echo "note: $dest is Homebrew's prefix; ~/.local/bin (the default) is safer" >&2
        ;;
esac

mkdir -p "$dest"
install -m755 "$binary" "$dest/shmc"
echo "==> installed $dest/shmc"

case ":$PATH:" in
    *":$dest:"*) ;;
    *)
        echo
        echo "note: $dest is not on your PATH. Add this to your shell profile:"
        echo "    export PATH=\"$dest:\$PATH\""
        ;;
esac

echo
echo "next: run 'shmc config' — it checks your host and edits the settings"
echo "      ('shmc setup' is the short server/token wizard)"
