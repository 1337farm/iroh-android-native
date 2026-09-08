#!/usr/bin/env bash
set -euo pipefail

export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"

if [ -z "${ANDROID_NDK_HOME:-}" ]; then
    if [ -d "$ANDROID_HOME/ndk" ]; then
        LATEST_NDK=$(ls -1d "$ANDROID_HOME/ndk/"* | sort -V | tail -n1)
        export ANDROID_NDK_HOME="$LATEST_NDK"
    else
        echo "ERROR: ANDROID_NDK_HOME is not set and no NDK directory found."
        exit 1
    fi
fi

echo "Using Android NDK: $ANDROID_NDK_HOME"

# Shared compile cache across all farm Rust builds (native_iroh_engine
# here, farm-iroh in 1337farm/flashforge-farm): same $CARGO_HOME registry
# plus sccache object cache at $SCCACHE_DIR. Skip silently if sccache
# is not installed.
if command -v sccache >/dev/null 2>&1; then
    export RUSTC_WRAPPER=sccache
    export CARGO_INCREMENTAL=0
    export SCCACHE_DIR="${SCCACHE_DIR:-$HOME/.cache/sccache}"
    export SCCACHE_CACHE_SIZE="${SCCACHE_CACHE_SIZE:-10G}"
fi

# Build for all Android ABIs: arm64-v8a, armeabi-v7a, x86_64
TARGETS=("aarch64-linux-android" "armv7-linux-androideabi" "i686-linux-android")
JNI_FOLDERS=("arm64-v8a" "armeabi-v7a" "x86")

echo "Verifying toolchains..."
for target in "${TARGETS[@]}"; do
    rustup target add "$target"
done

if ! command -v cargo-ndk &> /dev/null; then
    echo "Installing cargo-ndk linker helper..."
    cargo install cargo-ndk
fi

APP_JNI_DIR="../app/src/main/jniLibs"

# Clean previous builds
rm -rf "$APP_JNI_DIR"/*

for i in "${!TARGETS[@]}"; do
    TARGET="${TARGETS[$i]}"
    ABI="${JNI_FOLDERS[$i]}"

    echo "Building for target $TARGET ($ABI)..."
    cargo ndk -t "$ABI" --platform 26 build --release

    DEST_DIR="$APP_JNI_DIR/$ABI"
    mkdir -p "$DEST_DIR"
    cp "target/$TARGET/release/libnative_iroh_engine.so" "$DEST_DIR/"
done

echo "SUCCESS: Cross-compilation complete for ABIs: ${JNI_FOLDERS[*]}"
echo "Binaries installed in $APP_JNI_DIR"
