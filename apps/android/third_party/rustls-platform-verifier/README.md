# rustls-platform-verifier: the Android component (vendored)

The Kotlin part that `rustls-platform-verifier` calls through JNI to check a
server certificate with Android's own trust manager (`docs/mobile/
architecture.md` 5). Its published Maven repository is on GitHub, which is
not one of the app's dependency sources (Google Maven, Maven Central), so
its source is vendored here (D213, D218).

- Upstream: https://github.com/rustls/rustls-platform-verifier
- Tag `v/0.7.1` (the crate version `Cargo.lock` holds), commit
  `252e25161a91af476cbea620e29d277592d27ec7`; the component's own version
  there is 0.2.0 (`android-release-support/Cargo.toml`, the
  `rustls-platform-verifier-android` crate in `Cargo.lock`).
- License: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, copied from
  the repository's root).
- Files, copied unchanged from `android/rustls-platform-verifier/src/main/`:
  `java/org/rustls/platformverifier/CertificateVerifier.kt`,
  `AndroidManifest.xml`, `res/xml/network_security_config.xml` (the
  library's network security config: cleartext only to the listed CRL
  hosts).
- Ours: `build.gradle.kts` (a library module of the app's Gradle build, its
  `BuildConfig.TEST` always `false`) and `consumer-rules.pro` (the classes
  Rust calls are kept as they are).

SHA-256 of the copied files as the repository holds them (LF line ends;
the same as the upstream blobs at the commit above):

```text
c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4  LICENSE-APACHE
1c7cf76689c837a68ed8d704994e52a0f2940c087958f860d17f3186afbdcc0c  LICENSE-MIT
a5c52c41bc88c56b64219e557c289b960dd6cf5e69ac8bc0a21abaefe95b6124  src/main/AndroidManifest.xml
251031aec27214398328c4755744cd3274c54e9d51e10952712bcd5ba64bc4ed  src/main/java/org/rustls/platformverifier/CertificateVerifier.kt
a81434f9139f3780412c358d1cc782fe25339182d3dc141b35413f23d2b29f6a  src/main/res/xml/network_security_config.xml
```

When the crate moves to another version, take this directory from that
version's tag the same way and update the commit, the hashes and
THIRD_PARTY_NOTICES.
