#![cfg(feature = "generator")]
//! Exercise the public generator from a separate crate, including compiled output.

#[test]
fn generated_proto_compiles_and_runs_in_an_external_consumer()
-> Result<(), Box<dyn std::error::Error>> {
    let crate_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let directory = std::env::temp_dir().join(format!(
        "gin-generated-consumer-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos(),
    ));
    std::fs::create_dir_all(&directory)?;
    // Keep this target separate: the parent Cargo process may still hold its build lock.
    let target = crate_root
        .parent()
        .ok_or("workspace parent missing")?
        .join("target/generated-consumer");
    let path = crate_root
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    std::fs::write(
        directory.join("Cargo.toml"),
        format!(
            r#"
[package]
name = "generated-consumer"
version = "0.0.0"
edition = "2024"
[workspace]
[lib]
path = "lib.rs"
[dependencies]
gin-tonic = {{ path = "{path}", default-features = false, features = ["derive"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
[build-dependencies]
gin-tonic = {{ path = "{path}", default-features = false, features = ["generator"] }}
"#
        ),
    )?;
    for (name, contents) in [
        (
            "sample.proto",
            include_str!("fixtures/consumer/sample.proto"),
        ),
        ("build.rs", include_str!("fixtures/consumer/build.rs")),
        ("lib.rs", include_str!("fixtures/consumer/lib.rs")),
    ] {
        std::fs::write(directory.join(name), contents)?;
    }
    let output =
        std::process::Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
            .current_dir(&directory)
            .env("CARGO_TARGET_DIR", target)
            .args(["test", "--offline", "--quiet"])
            .output()?;
    assert!(
        output.status.success(),
        "consumer failed in {}:\n{}\n{}",
        directory.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
