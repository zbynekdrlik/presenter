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

    buildTypes {
        // We ship a debug-signed APK: it installs via `adb install` on any TV
        // without Play, and the server watchdog handles signature changes by
        // uninstall+reinstall. No release keystore / Play distribution needed.
        getByName("debug") {
            isMinifyEnabled = false
        }
        getByName("release") {
            isMinifyEnabled = false
            // Reuse the debug signing config so `assembleRelease` is also
            // ADB-installable without a managed keystore.
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
