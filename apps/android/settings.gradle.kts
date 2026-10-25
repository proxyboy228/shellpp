// Shell++ for Android (docs/mobile/architecture.md). Dependencies come from
// Google's Maven repository and Maven Central only (run 13 plan, 1.8).
pluginManagement {
    repositories {
        google()
        mavenCentral()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "shellpp-android"
include(":app")
// The vendored Android component of rustls-platform-verifier (D213, D218).
include(":rustls-platform-verifier")
project(":rustls-platform-verifier").projectDir = file("third_party/rustls-platform-verifier")
