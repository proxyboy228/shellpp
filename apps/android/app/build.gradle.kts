import java.io.File

plugins {
    id("com.android.application")
}

android {
    namespace = "dev.shellpp.android"
    compileSdk = 37
    buildToolsVersion = "36.0.0"
    ndkVersion = "30.0.16248370"

    defaultConfig {
        // The prototype's id; the release id is the owner's choice.
        applicationId = "dev.shellpp.android"
        minSdk = 26
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0-prototype"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets {
        getByName("main") {
            jniLibs.directories.add(layout.buildDirectory.dir("rustJniLibs").get().asFile.path)
        }
    }

    testOptions {
        unitTests.isReturnDefaultValues = true
    }
}

dependencies {
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test:rules:1.7.0")
    androidTestImplementation("androidx.test:core:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
}

// The Rust library (apps/android/native, crate shellpp-android), built by
// cargo with the NDK's clang for the phone (arm64-v8a) and the emulator
// (x86_64), and copied where the APK takes its native libraries from.
val rustTargets = mapOf(
    "arm64-v8a" to "aarch64-linux-android",
    "x86_64" to "x86_64-linux-android",
)
val repoRoot: File = rootProject.projectDir.parentFile.parentFile
val rustOut: File = layout.buildDirectory.dir("rustJniLibs").get().asFile

val ndkDir: File = System.getenv("ANDROID_NDK_HOME")?.let { File(it) }
    ?: File(androidComponents.sdkComponents.sdkDirectory.get().asFile, "ndk/${android.ndkVersion}")

val buildRust = tasks.register<Exec>("buildRust") {
    group = "build"
    description = "Builds libshellpp_android.so with cargo for every ABI."
    val windows = System.getProperty("os.name").startsWith("Windows")
    val hostTag = if (windows) "windows-x86_64" else "linux-x86_64"
    val llvm = File(ndkDir, "toolchains/llvm/prebuilt/$hostTag/bin")
    val clangSuffix = if (windows) ".cmd" else ""
    val arName = if (windows) "llvm-ar.exe" else "llvm-ar"
    workingDir = repoRoot
    val targetDir = System.getenv("CARGO_TARGET_DIR")?.let { File(it) } ?: File(repoRoot, "target")
    val args = mutableListOf("cargo", "build", "--locked", "--profile", "android", "-p", "shellpp-android")
    for (triple in rustTargets.values) {
        args += listOf("--target", triple)
        val upper = triple.uppercase().replace('-', '_')
        val lower = triple.replace('-', '_')
        val clang = File(llvm, "${triple}26-clang$clangSuffix").absolutePath
        environment("CC_$lower", clang)
        environment("AR_$lower", File(llvm, arName).absolutePath)
        environment("CARGO_TARGET_${upper}_LINKER", clang)
    }
    commandLine(args)
    inputs.dir(File(repoRoot, "apps/android/native/src"))
    inputs.dir(File(repoRoot, "crates"))
    outputs.dir(rustOut)
    doLast {
        for ((abi, triple) in rustTargets) {
            val built = File(targetDir, "$triple/android/libshellpp_android.so")
            val into = File(rustOut, abi)
            into.mkdirs()
            built.copyTo(File(into, "libshellpp_android.so"), overwrite = true)
        }
    }
}

tasks.named("preBuild") {
    dependsOn(buildRust)
}
