import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// Short commit hash baked into the human-readable versionName (forgerig +
// gatekeeper-demo pattern: GITHUB_SHA first so CI resolves without git
// history, `git rev-parse --short=10 HEAD` fallback for local builds, `dev`
// fallback so fresh clones without git history and offline builds still
// configure).
val gitCommitHash: String = System.getenv("GITHUB_SHA")?.take(10)
    ?.takeIf { it.matches(Regex("[0-9a-f]{10}")) }
    ?: try {
        val proc = ProcessBuilder("git", "rev-parse", "--short=10", "HEAD")
            .directory(rootProject.projectDir)
            .redirectErrorStream(true)
            .start()
        val sha = proc.inputStream.bufferedReader().readText().trim()
        proc.waitFor()
        if (sha.matches(Regex("[0-9a-f]{10}"))) sha else "dev"
    } catch (e: Exception) {
        "dev"
    }

android {
    namespace = "com.example.irohapp"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.example.irohapp"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        // Human-readable: the committing SHA beats a constant for debugging
        // (forgerig + gatekeeper-demo pattern with `dev` fallback).
        versionName = "1.0-$gitCommitHash"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    // Deterministic sideload signing: CI runners are ephemeral, so an
    // auto-generated debug key would give every build a different cert and
    // rolling installs would fail with UPDATE_INCOMPATIBLE. The committed
    // keystore below signs every build with the same cert (sideload key,
    // not a Play key — see keystore.properties).
    signingConfigs {
        create("sideload") {
            val props = Properties()
            val f = rootProject.file("keystore.properties")
            if (f.exists()) {
                f.inputStream().use { props.load(it) }
            }
            storeFile = rootProject.file(props.getProperty("storeFile", "keystore/sideload.keystore"))
            storePassword = props.getProperty("storePassword", "")
            keyAlias = props.getProperty("keyAlias", "")
            keyPassword = props.getProperty("keyPassword", "")
        }
    }

    buildTypes {
        release {
            signingConfig = signingConfigs.getByName("sideload")
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

dependencies {
    implementation(project(":irohbridge"))
    implementation("androidx.core:core-ktx:1.12.0")
    implementation("androidx.appcompat:appcompat:1.6.1")
    implementation("com.google.android.material:material:1.11.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.5")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.1")
    androidTestImplementation("androidx.test:core:1.5.0")
    androidTestImplementation("androidx.test:rules:1.5.0")
}
