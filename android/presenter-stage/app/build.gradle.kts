plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "sk.newlevel.presenterstage"
    compileSdk = 34

    defaultConfig {
        applicationId = "sk.newlevel.presenterstage"
        minSdk = 22
        targetSdk = 34
        // versionCode is the stable, monotonic upgrade key the server watchdog
        // compares against the installed app to decide whether to reinstall.
        // Bump it whenever the APK content changes. Keep versionName human-readable.
        versionCode = 1
        versionName = "1.0.0"
    }

    signingConfigs {
        // #740: pin the debug signing key to a COMMITTED keystore so every build
        // signs with the SAME certificate — assembleDebug on ephemeral CI runners
        // AND assembleRelease (which reuses this config below). Without an explicit
        // storeFile, Gradle auto-generates a fresh ~/.android/debug.keystore per
        // run, so each CI APK is signed with a different key → `adb install -r`
        // fails INSTALL_FAILED_UPDATE_INCOMPATIBLE on every upgrade and the stage
        // watchdog tears the running app down mid-event. A debug keystore uses
        // universally-known credentials (not a secret); this app is a LAN-only
        // internal WebView shell, never store-distributed, so a debug key is
        // appropriate and no managed release keystore is needed.
        getByName("debug") {
            storeFile = rootProject.file("debug.keystore")
            storePassword = "android"
            keyAlias = "androiddebugkey"
            keyPassword = "android"
        }
    }

    buildTypes {
        // Debug-signed APK, now signed with the committed stable key above (#740):
        // it installs via `adb install` on any TV without Play, and in-place
        // `adb install -r` upgrades succeed because the signature is stable across
        // builds (no more per-run key → no forced uninstall+reinstall).
        getByName("debug") {
            isMinifyEnabled = false
        }
        getByName("release") {
            isMinifyEnabled = false
            // Reuse the debug signing config so `assembleRelease` is also
            // ADB-installable with the same committed stable key.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    // Intentionally tiny: a single Activity + the platform WebView. No AndroidX
    // UI libs needed — keeps the APK small and the attack/maintenance surface low.
    implementation("androidx.core:core-ktx:1.13.1")
}
