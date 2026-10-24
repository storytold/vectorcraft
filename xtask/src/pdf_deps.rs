//! Verify that PDF reading and writing use one immutable shared crate and patched backends.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

const REV: &str = "2eca9b3a53a98ac52efed2a27353c6a6927ead10";
const PACKAGES: &[(&str, &str)] = &[
    ("craft-pdf", "0.1.0"),
    ("krilla", "0.8.2"),
    ("hayro-syntax", "0.7.2"),
    ("hayro-interpret", "0.7.0"),
    ("hayro-jbig2", "0.3.0"),
    ("hayro-cmap", "0.1.0"),
    ("hayro-postscript", "0.1.0"),
];

pub fn run(root: &Path) -> Result<(), String> {
    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .current_dir(root)
        .args(["metadata", "--locked", "--format-version", "1"])
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(format!("cargo metadata: {}", String::from_utf8_lossy(&output.stderr)));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout).map_err(|e| format!("cargo metadata: {e}"))?;
    validate(&metadata)?;
    println!("PDF: shared crate and patched backends resolve once to {REV}");
    Ok(())
}

fn validate(metadata: &Value) -> Result<(), String> {
    let packages = metadata.get("packages").and_then(Value::as_array).ok_or("cargo metadata has no packages")?;
    let source = format!("git+https://github.com/storytold/craft-libs?rev={REV}#{REV}");
    for &(name, version) in PACKAGES {
        let mut found = packages.iter().filter(|p| p.get("name").and_then(Value::as_str) == Some(name));
        let package = found.next().ok_or_else(|| format!("{name}: missing resolved package"))?;
        if found.next().is_some() {
            return Err(format!("{name}: expected one resolved package"));
        }
        if package.get("version").and_then(Value::as_str) != Some(version) || package.get("source").and_then(Value::as_str) != Some(source.as_str()) {
            return Err(format!("{name}: expected {version} from {source}; got {package}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn metadata() -> Value {
        let packages: Vec<_> = PACKAGES
            .iter()
            .map(|(name, version)| {
                json!({
                    "name":name, "version":version,
                    "source":format!("git+https://github.com/storytold/craft-libs?rev={REV}#{REV}")
                })
            })
            .collect();
        json!({"packages": packages})
    }

    #[test]
    fn accepts_pinned_pdf_packages_without_changing_printer_transport() {
        let mut m = metadata();
        m["packages"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"craft-print", "version":"0.1.0", "source":"git+https://github.com/storytold/craft-libs?rev=d991bf6792f8ade99c51358bf3c1d902c8cd96b5#d991bf6792f8ade99c51358bf3c1d902c8cd96b5"}));
        assert!(validate(&m).is_ok());
    }

    #[test]
    fn rejects_missing_or_duplicate_pdf_packages() {
        for index in 0..PACKAGES.len() {
            let mut m = metadata();
            m["packages"].as_array_mut().unwrap().remove(index);
            assert!(validate(&m).is_err());
            let mut m = metadata();
            let duplicate = m["packages"][index].clone();
            m["packages"].as_array_mut().unwrap().push(duplicate);
            assert!(validate(&m).is_err());
        }
    }

    #[test]
    fn rejects_registry_local_version_and_revision_drift() {
        for index in 0..PACKAGES.len() {
            for (field, value) in [
                ("source", Value::Null),
                ("source", json!("registry+https://github.com/rust-lang/crates.io-index")),
                ("source", json!("git+https://github.com/storytold/craft-libs?branch=main#other")),
                ("version", json!("0.99.0")),
            ] {
                let mut m = metadata();
                m["packages"][index][field] = value;
                assert!(validate(&m).is_err());
            }
        }
    }
}
