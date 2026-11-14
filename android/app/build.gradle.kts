plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}
android {
    namespace = "dev.shrimply.tablet"
    compileSdk = 37
    ndkVersion = "28.2.13676358"
    defaultConfig {
        applicationId = "dev.shrimply.tablet"
        minSdk = 33
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += providers.gradleProperty("nativeAbi").orElse("arm64-v8a").get() }
    }
    buildFeatures { compose = true }
    buildTypes {
        release {
            isMinifyEnabled = false
            if (providers.gradleProperty("devSigning").orNull == "true") {
                signingConfig = signingConfigs.getByName("debug")
            }
        }
    }
    packaging { jniLibs.useLegacyPackaging = false }
}
dependencies {
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.compose.foundation:foundation:1.12.1")
    implementation("androidx.compose.material3:material3:1.4.0")
}
