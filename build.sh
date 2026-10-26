#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
WORKSPACE_DIR="$(dirname -- "$ROOT_DIR")"
SUBMODULE_DIR="$ROOT_DIR/USB_SampleRate_Changer"
PATCH_SERIES="$ROOT_DIR/patches/series"
TARGET="${RUST_TARGET:-aarch64-linux-android}"
ANDROID_API_LEVEL="${ANDROID_API_LEVEL:-24}"
OUTPUT_DIR="${OUTPUT_DIR:-$WORKSPACE_DIR/output}"
MODULE_ARCHIVE_NAME="USB_SampleRate_Changer_WebUI"
VERSION_FILE="$ROOT_DIR/VERSION"

if [ ! -r "$VERSION_FILE" ]; then
    printf 'Missing version file: %s\n' "$VERSION_FILE" >&2
    exit 1
fi
MODULE_VERSION="$(sed -n '1p' "$VERSION_FILE" | tr -d '\r')"
case "$MODULE_VERSION" in
    '' | */* | *[!0-9.]* | .* | *.)
        printf 'Invalid version in %s: %s\n' "$VERSION_FILE" "$MODULE_VERSION" >&2
        exit 1
        ;;
esac

# Homebrew's Rust package cannot install extra standard-library targets. When
# rustup is available, prefer its cargo/rustc proxies so the pinned Android
# target and the selected compiler always come from the same toolchain.
if command -v rustup >/dev/null 2>&1; then
    # Resolve the active rustup toolchain explicitly.  Homebrew can install a
    # standalone cargo/rustc next to rustup; preferring that pair would hide
    # an otherwise installed Android target.
    RUSTUP_CARGO_PATH="$(rustup which cargo 2>/dev/null || true)"
    if [ -n "$RUSTUP_CARGO_PATH" ]; then
        RUSTUP_TOOLCHAIN_BIN="$(dirname -- "$RUSTUP_CARGO_PATH")"
        if [ -x "$RUSTUP_TOOLCHAIN_BIN/cargo" ] && [ -x "$RUSTUP_TOOLCHAIN_BIN/rustc" ]; then
            PATH="$RUSTUP_TOOLCHAIN_BIN:$PATH"
            export PATH
        fi
    else
        RUSTUP_BIN_DIR="$(dirname -- "$(command -v rustup)")"
        if [ -x "$RUSTUP_BIN_DIR/cargo" ] && [ -x "$RUSTUP_BIN_DIR/rustc" ]; then
            PATH="$RUSTUP_BIN_DIR:$PATH"
            export PATH
        fi
    fi
fi

for command_name in cargo git npm sed tar zip; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        printf 'Required command not found: %s\n' "$command_name" >&2
        exit 1
    fi
done

if [ ! -r "$PATCH_SERIES" ]; then
    printf 'Missing patch series: %s\n' "$PATCH_SERIES" >&2
    exit 1
fi

if [ ! -e "$SUBMODULE_DIR/.git" ]; then
    printf 'Initializing USB_SampleRate_Changer submodule...\n'
    if ! git -C "$ROOT_DIR" submodule update --init --recursive -- USB_SampleRate_Changer; then
        printf 'Unable to initialize submodule: %s\n' "$SUBMODULE_DIR" >&2
        exit 1
    fi
fi

if ! git -C "$SUBMODULE_DIR" rev-parse --verify HEAD >/dev/null 2>&1; then
    printf 'Invalid or uninitialized submodule: %s\n' "$SUBMODULE_DIR" >&2
    exit 1
fi

UPSTREAM_COMMIT="$(git -C "$SUBMODULE_DIR" rev-parse HEAD)"
UPSTREAM_STATE=clean
if [ -n "$(git -C "$SUBMODULE_DIR" status --porcelain)" ]; then
    UPSTREAM_STATE=modified
    printf 'Including local USB_SampleRate_Changer worktree changes in this build.\n'
fi

BUILD_DIR="$(mktemp -d "${TMPDIR:-/tmp}/usb-samplerate-webui.XXXXXX")"
UPSTREAM_DIR="$BUILD_DIR/upstream"
STAGING_DIR="$BUILD_DIR/module"
trap 'rm -rf "$BUILD_DIR"' EXIT HUP INT TERM

mkdir -p "$UPSTREAM_DIR"
(cd "$SUBMODULE_DIR" && tar --exclude='.git' -cf - .) | (cd "$UPSTREAM_DIR" && tar -xf -)

while IFS= read -r patch_name || [ -n "$patch_name" ]; do
    case "$patch_name" in
        '' | \#*) continue ;;
        /* | *..*)
            printf 'Unsafe patch path in %s: %s\n' "$PATCH_SERIES" "$patch_name" >&2
            exit 1
            ;;
    esac

    patch_path="$ROOT_DIR/patches/$patch_name"
    if [ ! -r "$patch_path" ]; then
        printf 'Patch listed in series does not exist: %s\n' "$patch_path" >&2
        exit 1
    fi

    git -C "$UPSTREAM_DIR" apply --check "$patch_path"
    git -C "$UPSTREAM_DIR" apply "$patch_path"
done < "$PATCH_SERIES"

mkdir -p "$STAGING_DIR/core"
for upstream_file in USB_SampleRate_Changer.sh functions3.shlib LICENSE; do
    if [ ! -e "$UPSTREAM_DIR/$upstream_file" ]; then
        printf 'Expected upstream file is missing: %s\n' "$upstream_file" >&2
        exit 1
    fi
    case "$upstream_file" in
        USB_SampleRate_Changer.sh | functions3.shlib)
            cp "$UPSTREAM_DIR/$upstream_file" "$STAGING_DIR/core/$upstream_file"
            ;;
        LICENSE)
            cp "$UPSTREAM_DIR/$upstream_file" "$STAGING_DIR/$upstream_file"
            ;;
    esac
done

for upstream_directory in templates extras; do
    if [ ! -d "$UPSTREAM_DIR/$upstream_directory" ]; then
        printf 'Expected upstream directory is missing: %s\n' "$upstream_directory" >&2
        exit 1
    fi
    cp -R "$UPSTREAM_DIR/$upstream_directory" "$STAGING_DIR/core/$upstream_directory"
done

sed "s/^version=.*/version=$MODULE_VERSION/" "$ROOT_DIR/module/module.prop" > "$STAGING_DIR/module.prop"
cp "$ROOT_DIR/module/customize.sh" \
    "$ROOT_DIR/module/uninstall.sh" "$ROOT_DIR/module/service.sh" \
    "$ROOT_DIR/module/skip_mount" "$STAGING_DIR/"

(cd "$ROOT_DIR/webui" && npm ci && WEBUI_OUT_DIR="$STAGING_DIR/webroot" npm run build)

if [ "$TARGET" = "aarch64-linux-android" ] && \
        [ -z "${CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER:-}" ]; then
    NDK_ROOT="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
    if [ -z "$NDK_ROOT" ] && command -v sdkmanager >/dev/null 2>&1; then
        SDK_ROOT="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-/opt/homebrew/share/android-commandlinetools}}"
        NDK_ROOT="$(find "$SDK_ROOT/ndk" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort -V | tail -n 1)"
    fi

    HOST_TAG=darwin-x86_64
    if [ "$(uname -s)" = Linux ]; then
        HOST_TAG=linux-x86_64
    fi

    LINKER="$NDK_ROOT/toolchains/llvm/prebuilt/$HOST_TAG/bin/aarch64-linux-android${ANDROID_API_LEVEL}-clang"
    if [ ! -x "$LINKER" ]; then
        printf 'Android NDK linker not found; set ANDROID_NDK_HOME or CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER\n' >&2
        exit 1
    fi
    export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$LINKER"
fi

CARGO_TARGET_DIR="$BUILD_DIR/cargo-target" \
    cargo build --locked --manifest-path "$ROOT_DIR/backend/Cargo.toml" --release --target "$TARGET"
cp "$BUILD_DIR/cargo-target/$TARGET/release/usbsrctl" "$STAGING_DIR/usbsrctl"

chmod 0755 "$STAGING_DIR/customize.sh" "$STAGING_DIR/uninstall.sh" "$STAGING_DIR/service.sh" \
    "$STAGING_DIR/usbsrctl"
chmod 0755 "$STAGING_DIR/core/USB_SampleRate_Changer.sh"
chmod 0644 "$STAGING_DIR/core/functions3.shlib"

STAGED_MODULE_VERSION="$(sed -n 's/^version=//p' "$STAGING_DIR/module.prop")"
if [ "$STAGED_MODULE_VERSION" != "$MODULE_VERSION" ]; then
    printf 'Staged module version mismatch: %s != %s\n' "$STAGED_MODULE_VERSION" "$MODULE_VERSION" >&2
    exit 1
fi

mkdir -p "$OUTPUT_DIR"
ARCHIVE_PATH="$OUTPUT_DIR/$MODULE_ARCHIVE_NAME-$MODULE_VERSION.zip"
TEMP_ARCHIVE="$BUILD_DIR/$MODULE_ARCHIVE_NAME-$MODULE_VERSION.zip"
(cd "$STAGING_DIR" && zip -q -r "$TEMP_ARCHIVE" . -x '*/.DS_Store')
mv -f "$TEMP_ARCHIVE" "$ARCHIVE_PATH"

printf 'Built %s\n' "$ARCHIVE_PATH"
printf 'USB_SampleRate_Changer commit: %s (%s worktree)\n' "$UPSTREAM_COMMIT" "$UPSTREAM_STATE"
