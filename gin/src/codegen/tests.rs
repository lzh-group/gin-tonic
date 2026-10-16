use super::CompileConfig;

#[test]
fn generates_proto3_shapes_and_accumulates_configuration() {
    let directory = std::env::temp_dir().join(format!("gin-codegen-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let proto = directory.join("sample.proto");
    std::fs::write(
        &proto,
        r#"
        syntax = "proto3";
        package sample.v1;
        enum State { STATE_UNSPECIFIED = 0; STATE_READY = 7; }
        message Foreign { int32 count = 1; }
        message Point { double x = 1; double y = 2; }
        message Sample {
            State state = 1;
            bytes data = 2;
            repeated bytes chunks = 3;
            map<string, int32> counts = 4;
            optional Foreign foreign = 5;
        }
        message Choice { oneof kind { double score = 1; Point point = 2; } }
        message Mixed { int32 id = 1; oneof outcome { string failed = 2; Point point = 3; } }
    "#,
    )
    .unwrap();
    let output = directory.join("out");
    let mut config = CompileConfig::new()
        .add_proto_file(&proto)
        .add_attribute("#[doc = \"first\"]", ".sample.v1.Sample")
        .add_attribute("#[doc = \"second\"]", ".sample.v1.Sample")
        .field_attribute(".sample.v1.Sample.state", "#[doc = \"field-first\"]")
        .field_attribute(".sample.v1.Sample.state", "#[doc = \"field-second\"]")
        .btree_map([".sample.v1.Sample.counts"])
        .boxed(".sample.v1.Choice.kind.point")
        .extern_path(".sample.v1.Foreign", "::other::Foreign");
    #[cfg(feature = "tonic")]
    {
        config = config.skip_services();
    }
    config.compile_into(&output).unwrap();
    let source = std::fs::read_to_string(output.join("sample/v1.rs")).unwrap();
    assert!(source.contains("Unspecified = 0"));
    assert!(source.contains("Ready = 7"));
    assert!(source.contains("pub state: State"));
    assert!(source.contains("pub data: Vec<u8>"));
    assert!(source.contains("pub chunks: Vec<Vec<u8>>"));
    assert!(source.contains("pub counts: ::std::collections::BTreeMap<String, i32>"));
    assert!(source.contains("pub foreign: Option<::other::Foreign>"));
    assert!(!source.contains("pub struct Foreign"));
    assert!(
        source.contains("Point(::std::boxed::Box<super::Point>)")
            || source.contains("Point(::std::boxed::Box<Point>)")
    );
    for marker in ["first", "second", "field-first", "field-second"] {
        assert!(source.contains(&format!("///{marker}")), "{source}");
    }
    assert!(source.contains("#[gin(default)]"));
    assert!(source.contains("#[gin(id = 0, oneof)]"));
    let syntax = syn::parse_file(&source).unwrap();
    let point = syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == "Point" => Some(item),
            _ => None,
        })
        .unwrap();
    let attrs = &point.attrs;
    let point_attributes = quote::quote!(#(#attrs)*).to_string();
    assert!(point_attributes.contains("Copy"));
    assert!(point_attributes.contains("Default"));
    std::fs::remove_dir_all(directory).unwrap();
}
