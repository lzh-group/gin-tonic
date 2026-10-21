fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
    gin_tonic::CompileConfig::new()
        .include(".")
        .add_proto_file("shared.proto")
        .compile_into(out.join("shared"))?;
    gin_tonic::CompileConfig::new()
        .include(".")
        .add_proto_file("sample.proto")
        .extern_path(".fixture.shared.v1", "crate::shared_fixture::shared::v1")
        .boxed(".fixture.validate.v1.Node.child")
        .compile_into(out.join("sample"))?;
    std::fs::write(
        out.join("bindings.rs"),
        format!(
            "#[path = {:?}] mod shared_fixture;\n#[path = {:?}] mod sample;",
            out.join("shared/fixture.rs"),
            out.join("sample/fixture.rs"),
        ),
    )?;
    println!("cargo:rerun-if-changed=shared.proto");
    println!("cargo:rerun-if-changed=sample.proto");
    Ok(())
}
