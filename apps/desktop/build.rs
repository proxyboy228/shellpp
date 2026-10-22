//! Windows: the version resource of `shellpp.exe` (run 4, plan 9.5), made
//! from the package version of Cargo.toml, so Explorer's Details and the
//! installer see the same version as `--version`. Other platforms: nothing.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let mut numbers = version
        .split(['.', '-', '+'])
        .map(|part| part.parse::<u16>().unwrap_or(0));
    let mut next = || numbers.next().unwrap_or(0);
    let (major, minor, patch) = (next(), next(), next());
    // In an .rc string a quote is written twice.
    let authors = std::env::var("CARGO_PKG_AUTHORS")
        .unwrap_or_default()
        .replace('"', "\"\"");
    let rc = format!(
        r#"#include <winver.h>
VS_VERSION_INFO VERSIONINFO
 FILEVERSION {major},{minor},{patch},0
 PRODUCTVERSION {major},{minor},{patch},0
 FILEFLAGSMASK VS_FFI_FILEFLAGSMASK
 FILEFLAGS 0x0L
 FILEOS VOS_NT_WINDOWS32
 FILETYPE VFT_APP
 FILESUBTYPE VFT2_UNKNOWN
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "FileDescription", "Shell++"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "shellpp"
      VALUE "LegalCopyright", "{authors}, Apache License 2.0"
      VALUE "OriginalFilename", "shellpp.exe"
      VALUE "ProductName", "Shell++"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("version.rc");
    std::fs::write(&out, rc).expect("version.rc");
    // Linked into the binaries only (the library stays without it). No
    // resource compiler is a warning, so an artifact without the version
    // shows up in the build log.
    if let Err(error) = embed_resource::compile(&out, embed_resource::NONE).manifest_required() {
        println!("cargo:warning=the version resource was not embedded: {error}");
    }
}
