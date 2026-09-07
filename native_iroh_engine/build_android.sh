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

TARGETS=("aarch64-linux-android" "armv7-linux-androideabi" "x86_64-linux-android")
JNI_FOLDERS=("arm64-v8a" "armeabi-v7a" "x86_64")

echo "Verifying toolchains..."
for target in "${TARGETS[@]}"; do
    rustup target add "$target"
done

if ! command -v cargo-ndk &> /dev/null; then
    echo "Installing cargo-ndk linker helper..."
    cargo install cargo-ndk
fi

APP_JNI_DIR="../app/src/main/jniLibs"

for i in "${!TARGETS[@]}"; do
    TARGET="${TARGETS[$i]}"
    ABI="${JNI_FOLDERS[$i]}"

    echo "Building for target $TARGET ($ABI)..."
    cargo ndk -t "$ABI" --platform 26 build --release

    DEST_DIR="$APP_JNI_DIR/$ABI"
    mkdir -p "$DEST_DIR"
    cp "target/$TARGET/release/libnative_iroh_engine.so" "$DEST_DIR/"
done

echo "SUCCESS: Cross-compilation complete. Binaries installed in $APP_JNI_DIR"
