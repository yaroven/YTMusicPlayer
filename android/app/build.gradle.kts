plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

// Same version as the desktop player and the iPhone app.
// (`-PappVersion=0.9.0` builds an older-looking app, to try self-update.)
val appVersion: String = (findProperty("appVersion") as String?) ?: rootDir.resolve("../Cargo.toml").readLines()
    .first { it.startsWith("version = ") }.substringAfter('"').substringBefore('"')
val buildNumber: Int = (System.getenv("BUILD_NUMBER") ?: "1").toInt()

android {
    namespace = "dev.ytmplayer.android"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.ytmplayer.android"
        minSdk = 26
        targetSdk = 35
        versionCode = buildNumber
        versionName = appVersion
        // English and Ukrainian only (no translations of the libraries' strings).
        resourceConfigurations += listOf("en", "uk")
    }

    signingConfigs {
        // CI signs with a keystore from secrets (so updates install over
        // each other); without one, the debug key.
        create("release") {
            val store = System.getenv("ANDROID_KEYSTORE")
            if (store != null) {
                storeFile = file(store)
                storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("ANDROID_KEY_ALIAS")
                keyPassword = System.getenv("ANDROID_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = if (System.getenv("ANDROID_KEYSTORE") != null) {
                signingConfigs.getByName("release")
            } else {
                signingConfigs.getByName("debug")
            }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    buildFeatures {
        compose = true
        buildConfig = true
    }
    lint {
        // Versions are pinned on purpose (and bumped by hand).
        disable += "GradleDependency"
    }
    packaging {
        resources.excludes += listOf("META-INF/*.kotlin_module", "DebugProbesKt.bin", "kotlin/**")
    }
}

dependencies {
    implementation(project(":core"))
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")

    val compose = platform("androidx.compose:compose-bom:2024.12.01")
    implementation(compose)
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.core:core-ktx:1.15.0")

    val media3 = "1.5.1"
    implementation("androidx.media3:media3-exoplayer:$media3")
    implementation("androidx.media3:media3-exoplayer-hls:$media3")
    implementation("androidx.media3:media3-session:$media3")

    implementation("androidx.webkit:webkit:1.12.1")
    implementation("io.coil-kt:coil-compose:2.7.0")

    testImplementation("junit:junit:4.13.2")
}
