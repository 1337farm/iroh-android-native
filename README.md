# iroh-android-native

This repository contains the Android implementation for iroh-android-native.

It includes:
- A pure native Iroh engine integrated via JNI (`native_iroh_engine`).
- Kotlin JNI Bridge and Android daemon service.
- Build scripts and GitHub Actions workflows for testing and packaging the APK.

## Building

The GitHub Actions workflow in `.github/workflows/android.yml` automatically builds a debug APK on pushes to `main` and pull requests.
