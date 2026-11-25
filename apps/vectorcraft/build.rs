//! Windows only: embed the app icon and version info (VERSIONINFO) into `vectorcraft.exe`. Everywhere,
//! fix the build's channel and the version it reports.
//!
//! `VECTORCRAFT_BUILD_CHANNEL=nightly-v1` makes a Nightly build and needs an exact feed identity in
//! `VECTORCRAFT_VERSION` (the shared nightly workflow's name) or `VECTORCRAFT_OVERRIDE_VERSION` (what
//! this app's own packaging passes to cargo): `X.Y.Z-nightly.YYYYMMDD.<run>.<attempt>.g<sha12>`. Unset,
//! empty or `stable` means the stable build; anything else is an error.
//!
//! On non-Windows targets the resource half does nothing. A missing resource compiler is a warning, so
//! a cross-compile from macOS or Linux still links, unless `VECTORCRAFT_REQUIRE_WINRES=1` turns it into
//! an error (for release and nightly builds).

#[path = "src/build_channel.rs"]
mod build_channel;

fn main() -> Result<(), String> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-check-cfg=cfg(vectorcraft_nightly_v1)");
    println!("cargo:rerun-if-env-changed=VECTORCRAFT_BUILD_CHANNEL");
    println!("cargo:rerun-if-env-changed=VECTORCRAFT_VERSION");
    println!("cargo:rerun-if-env-changed=VECTORCRAFT_OVERRIDE_VERSION");
    let nightly = match std::env::var("VECTORCRAFT_BUILD_CHANNEL").as_deref() {
        Ok("nightly-v1") => {
            println!("cargo:rustc-cfg=vectorcraft_nightly_v1");
            true
        }
        // An empty value (an unset CI input) means the default stable build.
        Ok("stable" | "") | Err(std::env::VarError::NotPresent) => false,
        Ok(other) => return Err(format!("unsupported VECTORCRAFT_BUILD_CHANNEL {other:?} (expected stable or nightly-v1)")),
        Err(std::env::VarError::NotUnicode(_)) => return Err("VECTORCRAFT_BUILD_CHANNEL is not valid Unicode".to_string()),
    };
    // The stable build reports its package version, as it always has.
    let built_version = if nightly {
        match feed_identity() {
            Some(version) if build_channel::is_nightly_version(&version) => version,
            _ => {
                return Err(
                    "nightly-v1 requires an explicit VECTORCRAFT_VERSION feed identity (X.Y.Z-nightly.YYYYMMDD.<run>.<attempt>.g<sha12>)".to_string()
                );
            }
        }
    } else {
        std::env::var("CARGO_PKG_VERSION").unwrap_or_default()
    };
    println!("cargo:rustc-env=VECTORCRAFT_BUILT_VERSION={built_version}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }
    println!("cargo:rerun-if-changed=../../assets/app-icon/vectorcraft.ico");
    println!("cargo:rerun-if-changed=../../assets/app-icon/vectorcraft-nightly.ico");
    println!("cargo:rerun-if-env-changed=VECTORCRAFT_REQUIRE_WINRES");
    let mut res = winresource::WindowsResource::new();
    res.set_icon(build_channel::windows_icon_path(nightly))
        .set("ProductName", if nightly { "VectorCraft Nightly" } else { "VectorCraft" })
        .set("FileDescription", if nightly { "VectorCraft Nightly vector illustration editor" } else { "VectorCraft vector illustration editor" })
        .set("CompanyName", "Learning Machines LLC")
        .set("LegalCopyright", "Copyright (c) the VectorCraft authors. MIT OR Apache-2.0.")
        .set("OriginalFilename", "vectorcraft.exe")
        .set("InternalName", "vectorcraft");
    if nightly {
        // The Launcher verifies the PE ProductVersion against the exact feed identity, not FileVersion.
        res.set("ProductVersion", &built_version);
    }
    if let Err(e) = res.compile() {
        if std::env::var_os("VECTORCRAFT_REQUIRE_WINRES").is_some() {
            return Err(format!("embedding Windows resources failed: {e}"));
        }
        println!("cargo:warning=vectorcraft.exe built without icon/version resources: {e}");
    }
    Ok(())
}

/// The feed identity from `VECTORCRAFT_VERSION`, else `VECTORCRAFT_OVERRIDE_VERSION`: the shared
/// nightly workflow sets the first, this app's own packaging scripts the second.
fn feed_identity() -> Option<String> {
    ["VECTORCRAFT_VERSION", "VECTORCRAFT_OVERRIDE_VERSION"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .next()
}
