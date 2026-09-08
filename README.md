# iroh-android-native

Shared P2P registry transport for Android: a native Iroh engine (`native_iroh_engine`, Rust, iroh 1.1.0) exposed over JNI (`IrohBridge`), plus a foreground daemon service.

It includes:
- A pure native Iroh engine integrated via JNI (`native_iroh_engine`).
- Kotlin JNI Bridge and Android daemon service (`:irohbridge` library module).
- Build scripts and GitHub Actions workflows for testing, packaging the APK, and publishing the bridge AAR.

## Building

The GitHub Actions workflow in `.github/workflows/android.yml` automatically builds a debug APK on pushes to `main` and pull requests. To build the native library locally:

```
cd native_iroh_engine
./build_android.sh   # needs ANDROID_NDK_HOME, rustup + cargo-ndk; outputs ../irohbridge/src/main/jniLibs/<ABI>/
```

## Consuming the AAR

The `:irohbridge` module packages the JNI bridge, the daemon service and the
native `.so` into an AAR published to GitHub Packages on every `engine-v*` tag:

```kotlin
repositories {
    maven { url = uri("https://maven.pkg.github.com/1337farm/iroh-android-native") }
}
dependencies {
    implementation("com.example.irohapp:irohbridge:0.3.0")
}
```

The AAR declares the `IrohDaemonService` (dataSync foreground service) and the
needed permissions; consumer manifests pick them up via manifest merge. minSdk
is 23 to match the lowest consumer.

## JNI surface (`com.example.irohapp.IrohBridge`)

Lifecycle: `initialize(storageDir, secretKey?)` (persists blobs in `storageDir`, stable endpoint id when a 32-byte secret is passed; null generates) → `shutdown()`. `endpointId()` / `secretKey()` for identity backup.

Blobs: `blobAdd(data): hash`, `blobGet(hash)`, `blobHas(hash)`, `ticketFor(hash): BlobTicket string`, `ticketInfo(ticket): "hash_hex:peer_hex"`, `blobFetch(ticket): bytes` (one-shot download of a standalone blob). Null return = check logcat; most calls throw `RuntimeException` with a message on error.

Models: `modelPublish(metadataJson, files[]): ticket`. The engine injects `hashes`/`tickets`/`sizes` arrays into the metadata before storing, so fetchers need only the one ticket. `modelFetch(ticket, dir, cb)` downloads metadata + files, writes them to `dir` (names are traversal-checked), and reports via `IrohTransferListener`: `onTransferProgress(status, pct, downloadedBytes, totalBytes, msg)` with status 1 routing / 2 connecting / 3 connected / 4 progress / 5 done / negative error (-3 cancelled), plus `onModelMetadata(json, namesJson)` and `onFetchComplete(dir)`. `cancelFetch()` cancels the active fetch.

Search (local index, no hub): `searchQuery(keyword): hashHex[]`, `searchGetMetadata(hashHex): json`. The index learns from every publish and fetch.

Sync (epidemic, no hub): `syncAnnounce(): ticket` covers up to 200 indexed models; `syncMerge(ticket): {"new_models":n,"model_tickets":[...]}` fetches and indexes up to 100 unknown models. `knownPeers(): idHex[]` lists peers we have downloaded from.

Metadata JSON contract (consumers): `title`, `description`, `designer{name,pubkey}`, `license{spdx,url}`, `category`, `tags[]`, `images[]`, `files[]`, `printSettings{}`, plus engine-added `hashes[]`, `tickets[]`, `sizes[]` aligned with `files[]`.

## Versioning

`native_iroh_engine` follows `engine-vX.Y.Z` tags (see releases). Dependents should pin a tag, not `main`.
