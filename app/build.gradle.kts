plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.example.irohapp"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.example.irohapp"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }
    packaging {
        jniLibs {
            useLegacyPackaging = true
        }
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

tasks.register<Exec>("buildNativeEngine") {
    group = "build"
    description = "Cross-compiles native_iroh_engine into app/src/main/jniLibs via build_android.sh."
    workingDir = file("../native_iroh_engine")
    commandLine("./build_android.sh")
    onlyIf {
        val requested = gradle.startParameter.taskNames
        val unitTestsOnly = requested.any { it.contains("test", ignoreCase = true) } &&
            requested.none {
                it.contains("assemble", ignoreCase = true) ||
                    it.contains("install", ignoreCase = true) ||
                    it.contains("bundle", ignoreCase = true)
            }
        if (unitTestsOnly) {
            logger.warn("buildNativeEngine: unit-test-only invocation; skipping native build (JVM tests do not need the .so).")
            false
        } else if (nativeToolchainAvailable()) {
            true
        } else {
            logger.warn("buildNativeEngine: cargo/NDK not available; skipping native build (APK will not bundle libnative_iroh_engine.so).")
            false
        }
    }
}

tasks.named("preBuild") {
    dependsOn("buildNativeEngine")
}

dependencies {
    implementation("androidx.core:core-ktx:1.12.0")
    implementation("androidx.appcompat:appcompat:1.6.1")
    implementation("com.google.android.material:material:1.11.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.5")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.1")
}
