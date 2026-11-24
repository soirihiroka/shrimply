plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}
val licenseAssetsDirectory = layout.buildDirectory.dir("generated/licenseAssets")
val licenseAssets = tasks.register<Sync>("licenseAssets") {
    mapOf(
        "vendor/vision/LICENSE" to "OpenCV-Vision.txt",
        "crates/math/color/src/css/LICENSE" to "GTK-Pango-Color.txt",
        "crates/math/color/src/css/NOTICE" to "GTK-Pango-Color-NOTICE.txt",
    ).forEach { (source, name) ->
        from(rootProject.file("../$source")) {
            into("licenses")
            rename { name }
        }
    }
    into(licenseAssetsDirectory)
}
android {
    namespace = "dev.shrimply.app"
    compileSdk = 37
    ndkVersion = "28.2.13676358"
    defaultConfig {
        applicationId = "dev.shrimply.app"
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
    sourceSets["main"].assets.directories.add(licenseAssetsDirectory.get().asFile.path)
}
tasks.named("preBuild") { dependsOn(licenseAssets) }
dependencies {
    // The existing Rust TLS verifier requires its matching Android component.
    val verifierVersion = providers.fileContents(rootProject.layout.projectDirectory.file("../Cargo.lock")).asText.map { lock ->
        Regex("""name = "rustls-platform-verifier-android"\s+version = "([^"]+)"""").find(lock)?.groupValues?.get(1)
            ?: error("rustls-platform-verifier-android is missing from Cargo.lock")
    }
    implementation(verifierVersion.map { "org.rustls:rustls-platform-verifier:$it" })
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.compose.foundation:foundation:1.12.1")
    implementation("androidx.compose.material3:material3:1.4.0")
}
