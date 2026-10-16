fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
    gin_tonic::CompileConfig::new()
        .include(".")
        .add_proto_file("sample.proto")
        .add_attribute("#[derive(serde::Serialize, serde::Deserialize)]", ".")
        .field_attribute(
            ".fixture.v1.Values.counts",
            "#[serde(rename = \"weights\")]",
        )
        .field_attribute(".fixture.v1.Values.counts", "#[serde(default)]")
        .btree_map([".fixture.v1.Values.counts"])
        .compile_into(&out)?;
    std::fs::write(
        out.join("bindings.rs"),
        format!("#[path = {:?}] mod fixture;", out.join("fixture.rs")),
    )?;
    println!("cargo:rerun-if-changed=sample.proto");
    Ok(())
}
