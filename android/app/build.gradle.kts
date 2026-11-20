plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "com.castrolabs.kbstudioscreen"
    compileSdk = 35
    ndkVersion = "27.2.12479018"

    defaultConfig {
        applicationId = "com.castrolabs.kbstudioscreen"
        minSdk = 29 // Android 10
        targetSdk = 35
        versionCode = 8
        versionName = "0.7.1"
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64") }
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            // Firma de pruebas: se usa la clave debug hasta definir la clave de Castro Labs (Fase 5).
            signingConfig = signingConfigs.getByName("debug")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    buildFeatures { compose = true }
}

// Compila el núcleo Rust (libkbcore.so) para las 3 ABIs antes de empaquetar.
val buildRustCore by tasks.registering(Exec::class) {
    description = "cargo ndk build --release -p kb-android"
    workingDir = rootProject.projectDir.resolve("../core")
    environment("ANDROID_NDK_HOME", android.ndkDirectory.absolutePath)
    commandLine(
        "cargo", "ndk",
        "-t", "arm64-v8a", "-t", "armeabi-v7a", "-t", "x86_64",
        "-P", "29",
        "-o", projectDir.resolve("src/main/jniLibs").absolutePath,
        "build", "--release", "-p", "kb-android"
    )
}
tasks.named("preBuild") { dependsOn(buildRustCore) }

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2025.06.00")
    implementation(composeBom)
    implementation("androidx.core:core-ktx:1.16.0")
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.9.1")
    // Lector de QR (Apache-2.0), sin Google Play Services.
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")

    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test:core-ktx:1.6.1")
}
