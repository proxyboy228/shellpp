import java.io.File
import java.util.Properties
import javax.xml.parsers.DocumentBuilderFactory
import org.gradle.api.artifacts.component.ModuleComponentIdentifier

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

// The commit the APK is built from, for About (D223 item 5): `-dirty` when
// the tree has changes not committed; "unknown" without git.
val buildCommit: String = runCatching {
    val head = providers.exec { commandLine("git", "rev-parse", "--short=10", "HEAD") }.standardOutput.asText.get().trim()
    val dirty = providers.exec { commandLine("git", "status", "--porcelain", "--untracked-files=no") }.standardOutput.asText.get().isNotBlank()
    if (dirty) "$head-dirty" else head
}.getOrDefault("unknown")

// The release signing key is the owner's (D218, D223): the path of its
// file and its alias come from a properties file outside the repository
// (SHELLPP_ANDROID_SIGNING names it), its passwords from the environment of
// scripts\build-android-release.ps1, which never writes them to disk.
val releaseSigning: Properties? = System.getenv("SHELLPP_ANDROID_SIGNING")?.let { path ->
    Properties().apply { File(path).inputStream().use { load(it) } }
}

android {
    namespace = "dev.shellpp.android"
    compileSdk = 37
    buildToolsVersion = "36.0.0"
    ndkVersion = "30.0.16248370"

    defaultConfig {
        // The owner's id (D218). The code's package stays dev.shellpp.android
        // (the JNI exports are named after it).
        applicationId = "io.github.proxyboy228.shellpp"
        minSdk = 26
        targetSdk = 37
        // The Android app's own line of versions (D223 item 17).
        versionCode = 3
        versionName = "1.0.0-rc1"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        buildConfigField("String", "COMMIT", "\"$buildCommit\"")
    }

    signingConfigs {
        if (releaseSigning != null) {
            create("release") {
                storeFile = File(releaseSigning.getProperty("storeFile") ?: error("storeFile is missing in SHELLPP_ANDROID_SIGNING"))
                keyAlias = releaseSigning.getProperty("keyAlias") ?: error("keyAlias is missing in SHELLPP_ANDROID_SIGNING")
                storePassword = System.getenv("SHELLPP_ANDROID_STORE_PASSWORD")
                keyPassword = System.getenv("SHELLPP_ANDROID_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        getByName("release") {
            // No R8 and no resource shrinking in the first version: the JNI
            // names must stay, and the Rust library is most of the size
            // (D224) [decision].
            isMinifyEnabled = false
            isShrinkResources = false
            signingConfig = signingConfigs.findByName("release")
        }
    }

    // `verify-android.ps1 -Release` runs the emulator tests on the release
    // build (D223 item 15); every other run tests the debug build.
    testBuildType = providers.gradleProperty("shellppTestBuildType").getOrElse("debug")

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets {
        getByName("main") {
            jniLibs.directories.add(layout.buildDirectory.dir("rustJniLibs").get().asFile.path)
            // The licenses screen's list, made from THIRD_PARTY_NOTICES.md.
            assets.directories.add(layout.buildDirectory.dir("generated/notices").get().asFile.path)
        }
    }

    testOptions {
        unitTests.isReturnDefaultValues = true
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }
}

dependencies {
    // Jetpack Compose for the screens (architecture 1, D213); the terminal
    // stays a View of its own.
    implementation(platform("androidx.compose:compose-bom:2026.09.00"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-core")
    // Material Icons (Apache-2.0) for the screens of run 15: folders, keys,
    // keyboard, upload and the like (plan 1.3: Material icons or our own glyphs).
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.activity:activity-compose:1.13.0")
    // Android's trust manager for the sync server's certificate (architecture 5).
    implementation(project(":rustls-platform-verifier"))
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20250517")
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test:rules:1.7.0")
    androidTestImplementation("androidx.test:core:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
    androidTestImplementation("androidx.test.uiautomator:uiautomator:2.4.0")
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

// The licenses screen (Settings → About, D223 item 5) shows the Android
// app's part of THIRD_PARTY_NOTICES.md: the same list as the repository's,
// copied into the app's assets at build time.
val noticesFile = File(repoRoot, "THIRD_PARTY_NOTICES.md")
val noticesAsset = tasks.register("noticesAsset") {
    group = "build"
    description = "Copies the Android part of THIRD_PARTY_NOTICES.md into the app's assets."
    val out = layout.buildDirectory.dir("generated/notices").get().asFile
    inputs.file(noticesFile)
    outputs.dir(out)
    doLast {
        val text = noticesFile.readText()
        val start = text.indexOf("## The Android app")
        check(start >= 0) { "THIRD_PARTY_NOTICES.md has no section of the Android app" }
        val end = text.indexOf("\n## ", start + 1).let { if (it < 0) text.length else it }
        val dir = File(out, "notices").apply { mkdirs() }
        File(dir, "android.md").writeText(text.substring(start, end))
    }
}

tasks.named("preBuild") {
    dependsOn(buildRust, noticesAsset)
}

// The "Android libraries" table of THIRD_PARTY_NOTICES.md (D223 item 14):
// the modules of the release build's runtime classpath with the licenses
// their POMs (or their parents') declare. `gradlew androidNotices` checks
// it (scripts\verify-android.ps1); `-PwriteNotices` rewrites it.
val librariesStart = "<!-- android-libraries:start (generated by the androidNotices task of apps/android/app) -->"
val librariesEnd = "<!-- android-libraries:end -->"

/** The POM of a module, through the build's own repositories. */
fun pomOf(group: String, module: String, version: String): File =
    configurations.detachedConfiguration(dependencies.create("$group:$module:$version@pom"))
        .apply { isTransitive = false }
        .singleFile

/** The license names and the project URL a POM declares, a parent's when it declares none. */
fun licensesOf(pom: File, depth: Int = 0): Pair<List<String>, String?> {
    val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(pom)
    val root = doc.documentElement
    fun child(parent: org.w3c.dom.Element, name: String): org.w3c.dom.Element? {
        val nodes = parent.childNodes
        for (i in 0 until nodes.length) {
            val node = nodes.item(i)
            if (node is org.w3c.dom.Element && node.tagName == name) return node
        }
        return null
    }
    val url = child(root, "url")?.textContent?.trim()?.takeIf { it.isNotEmpty() && !it.contains("\${") }
    val licenses = child(root, "licenses")?.let { list ->
        val nodes = list.getElementsByTagName("license")
        (0 until nodes.length).mapNotNull { (nodes.item(it) as org.w3c.dom.Element).let { license -> child(license, "name")?.textContent?.trim() } }
    }.orEmpty()
    if (licenses.isNotEmpty() || depth > 4) return licenses to url
    val parent = child(root, "parent") ?: return licenses to url
    val (inherited, parentUrl) = licensesOf(
        pomOf(child(parent, "groupId")!!.textContent.trim(), child(parent, "artifactId")!!.textContent.trim(), child(parent, "version")!!.textContent.trim()),
        depth + 1,
    )
    return inherited to (url ?: parentUrl)
}

tasks.register("androidNotices") {
    group = "verification"
    description = "Checks (or with -PwriteNotices rewrites) the Android libraries table of THIRD_PARTY_NOTICES.md."
    val write = providers.gradleProperty("writeNotices").isPresent
    doLast {
        val modules = configurations.getByName("releaseRuntimeClasspath").incoming.resolutionResult.allComponents
            .mapNotNull { it.id as? ModuleComponentIdentifier }
            .distinctBy { "${it.group}:${it.module}:${it.version}" }
            .sortedWith(compareBy<ModuleComponentIdentifier>({ it.group }, { it.module }))
        val rows = modules.map { id ->
            val (licenses, url) = licensesOf(pomOf(id.group, id.module, id.version))
            check(licenses.isNotEmpty()) { "${id.group}:${id.module}:${id.version} declares no license; check its license and add it by hand" }
            val source = url ?: "https://maven.google.com/web/index.html#${id.group}:${id.module}:${id.version}"
            "| ${id.group}:${id.module} | ${id.version} | ${licenses.joinToString(" / ")} | <$source> |"
        }
        val table = (listOf("| Library | Version | License | Source |", "|---|---|---|---|") + rows).joinToString("\n")
        val text = noticesFile.readText()
        val from = text.indexOf(librariesStart)
        val to = text.indexOf(librariesEnd, from)
        check(from >= 0 && to > from) { "the markers of the Android libraries table are missing in THIRD_PARTY_NOTICES.md" }
        val expected = text.substring(0, from + librariesStart.length) + "\n\n" + table + "\n\n" + text.substring(to)
        when {
            expected == text -> println("THIRD_PARTY_NOTICES.md: the Android libraries table is up to date (${rows.size} libraries).")
            write -> {
                noticesFile.writeText(expected)
                println("THIRD_PARTY_NOTICES.md: ${rows.size} Android libraries recorded.")
            }
            else -> throw GradleException("THIRD_PARTY_NOTICES.md: the Android libraries table is out of date; run gradlew androidNotices -PwriteNotices")
        }
    }
}
