plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
    id("maven-publish")
}

android {
    namespace = "com.example.irohapp"
    compileSdk = 34

    defaultConfig {
        minSdk = 23
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    sourceSets {
        getByName("main") {
            jniLibs.srcDirs("src/main/jniLibs")
        }
    }
    packaging {
        jniLibs {
            useLegacyPackaging = true
        }
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.12.0")
    implementation("androidx.appcompat:appcompat:1.6.1")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.5")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.1")
}

fun nativeToolchainAvailable(): Boolean {
    val cargo = runCatching {
        ProcessBuilder("cargo", "--version")
            .redirectErrorStream(true)
            .start()
            .waitFor() == 0
    }.getOrDefault(false)
    val ndkHome = System.getenv("ANDROID_NDK_HOME")
    val sdkHome = System.getenv("ANDROID_HOME") ?: System.getenv("ANDROID_SDK_ROOT")
    val ndkDir = ndkHome?.let { File(it) }
        ?: sdkHome?.let { File(it, "ndk") }
    val hasNdk = ndkDir?.let { dir ->
        if (dir.name == "ndk") {
            dir.isDirectory && (dir.list()?.isNotEmpty() == true)
        } else {
            dir.isDirectory
        }
    } ?: false
    return cargo && hasNdk
}

fun nativeLibsAlreadyBuilt(): Boolean {
    val jniLibsDir = File("src/main/jniLibs")
    if (!jniLibsDir.isDirectory) return false
    val abiDirs = listOf("arm64-v8a", "armeabi-v7a", "x86")
    return abiDirs.all { abi ->
        val lib = File(jniLibsDir, "$abi/libnative_iroh_engine.so")
        lib.exists()
    }
}

tasks.register<Exec>("buildNativeEngine") {
    group = "build"
    description = "Cross-compiles native_iroh_engine into irohbridge/src/main/jniLibs via build_android.sh."
    workingDir = file("../native_iroh_engine")
    commandLine("./build_android.sh")
    onlyIf {
        val requested = gradle.startParameter.taskNames
        val unitTestsOnly = requested.any { it.contains("test", ignoreCase = true) } &&
            requested.none {
                it.contains("assemble", ignoreCase = true) ||
                    it.contains("install", ignoreCase = true) ||
                    it.contains("bundle", ignoreCase = true) ||
                    it.contains("publish", ignoreCase = true)
            }
        if (unitTestsOnly) {
            logger.warn("buildNativeEngine: unit-test-only invocation; skipping native build (JVM tests do not need the .so).")
            false
        } else if (nativeLibsAlreadyBuilt()) {
            logger.info("buildNativeEngine: native libs already built; skipping.")
            false
        } else if (nativeToolchainAvailable()) {
            true
        } else {
            logger.warn("buildNativeEngine: cargo/NDK not available; skipping native build (AAR/APK will not bundle libnative_iroh_engine.so).")
            false
        }
    }
}

tasks.named("preBuild") {
    dependsOn("buildNativeEngine")
}

afterEvaluate {
    publishing {
        publications {
            create<MavenPublication>("release") {
                from(components["release"])
                groupId = "com.example.irohapp"
                artifactId = "irohbridge"
                version = (System.getenv("IROH_VERSION") ?: project.findProperty("irohVersion") as String?) ?: "0.3.0-SNAPSHOT"
            }
        }
        repositories {
            maven {
                name = "GitHubPackages"
                url = uri("https://maven.pkg.github.com/1337farm/iroh-android-native")
                credentials {
                    username = System.getenv("GITHUB_ACTOR")
                    password = System.getenv("GITHUB_TOKEN")
                }
            }
        }
    }
}