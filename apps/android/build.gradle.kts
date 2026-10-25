plugins {
    // AGP 9 builds Kotlin itself (Kotlin 2.2.10): no separate Kotlin plugin,
    // only the Compose compiler of the same Kotlin version (D213, M1).
    id("com.android.application") version "9.4.1" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.2.10" apply false
}
