// The Android component of rustls-platform-verifier, vendored (README.md).
plugins {
    id("com.android.library")
}

android {
    namespace = "org.rustls.platformverifier"
    compileSdk = 37

    defaultConfig {
        minSdk = 26
        consumerProguardFiles("consumer-rules.pro")
        // Upstream's test hooks (a mock key store) stay off.
        buildConfigField("boolean", "TEST", "false")
    }

    buildFeatures {
        buildConfig = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
