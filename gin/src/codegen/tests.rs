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

fn compile_validation(name: &str, body: &str) -> (Result<(), super::CompilerError>, String) {
    let directory = std::env::temp_dir().join(format!(
        "gin-codegen-validate-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let proto = directory.join("check.proto");
    std::fs::write(
        &proto,
        format!(
            r#"
            syntax = "proto3";
            package check.v1;
            import "buf/validate/validate.proto";
            import "gin.proto";
            import "google/protobuf/wrappers.proto";
            enum State {{ STATE_A = 0; }}
            message Child {{ int32 count = 1; }}
            {body}
            "#
        ),
    )
    .unwrap();
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/validate");
    let output = directory.join("out");
    let mut config = CompileConfig::new()
        .include(fixtures)
        .add_proto_file(&proto);
    #[cfg(feature = "tonic")]
    {
        config = config.skip_services();
    }
    let result = config.compile_into(&output);
    let source = std::fs::read_to_string(output.join("check/v1.rs")).unwrap_or_default();
    let generated_options = output.join("buf").exists() || output.join("google").exists();
    std::fs::remove_dir_all(directory).unwrap();
    assert!(!generated_options, "option packages must not be generated");
    (result, source)
}

#[test]
fn validation_rules_become_derive_attributes() {
    let (result, source) = compile_validation(
        "attributes",
        r#"
        message Sample {
            int32 period = 1 [(buf.validate.field).int32 = {gte: 1, lte: 10000}];
            sint64 offset = 2 [(buf.validate.field).sint64.gt = -3];
            double ratio = 3 [(buf.validate.field).double = {finite: true, lt: inf}];
            string id = 4 [(buf.validate.field).string = {uuid: true, max_len: 36}];
            repeated string names = 5 [(buf.validate.field).repeated = {min_items: 1, unique: true, items: {string: {min_len: 1}}}];
            map<string, uint32> lots = 6 [(buf.validate.field).map = {max_pairs: 4, keys: {string: {min_len: 2}}, values: {uint32: {gte: 1}}}];
            Child child = 7;
            repeated Child children = 8;
            map<int32, Child> by_id = 9;
            google.protobuf.Int32Value wrapped = 10;
            Child custom = 11 [(gin_tonic.v1.rust_type) = "crate::Custom"];
            optional Child maybe = 12;
            State state = 13;
            float scale = 14 [(buf.validate.field).float.finite = false];
            oneof choice {
                string text = 15 [(buf.validate.field).string.min_len = 1];
                Child record = 16;
            }
        }
        message Unwrapped { oneof kind { double score = 1 [(buf.validate.field).double.gte = 0]; Child record = 2; } }
        "#,
    );
    result.unwrap();
    let source = source.split_whitespace().collect::<String>();
    for expected in [
        r#"validate(lte=10000i32,gte=1i32))]#[gin(default)]pubperiod:i32"#,
        r#"validate(gt=-3i64))]#[gin(default)]puboffset:i64"#,
        r#"validate(lt=f64::INFINITY,finite))]"#,
        r#"validate(max_len=36,uuid))]"#,
        r#"validate(min_items=1,unique,items(min_len=1)))]pubnames:Vec<String>"#,
        r#"validate(max_pairs=4,keys(min_len=2),values(gte=1u32)))]"#,
        r#"#[gin(id=7u32,validate(nested))]pubchild:Child"#,
        r#"#[gin(id=8u32,validate(nested))]pubchildren:Vec<Child>"#,
        r#"validate(nested))]pubby_id:"#,
        r#"#[gin(id=10u32)]pubwrapped:i32"#,
        r#"#[gin(id=11u32)]pubcustom:crate::Custom"#,
        r#"#[gin(id=12u32,validate(nested))]pubmaybe:Option<Child>"#,
        r#"#[gin(id=13u32,scalar="int32")]#[gin(default)]pubstate:State"#,
        r#"#[gin(id=14u32,scalar="float")]#[gin(default)]pubscale:f32"#,
        r#"#[gin(id=0,oneof)]pubchoice:sample::Choice"#,
        r#"#[gin(id=1u32,scalar="double",name="score",validate(gte=0f64))]Score(f64)"#,
        r#"#[gin(id=2u32,name="record",validate(nested))]Record(Child)"#,
    ] {
        assert!(source.contains(expected), "missing {expected} in {source}");
    }
}

#[test]
fn unsupported_validation_rules_fail_generation() {
    let cases = [
        (
            r#"message M { int32 a = 1 [(buf.validate.field).cel = {id: "x", expression: "this > 0"}]; }"#,
            "field `check.v1.M.a`: protovalidate rule `cel` is not supported",
        ),
        (
            r#"message M { Child a = 1 [(buf.validate.field).required = true]; }"#,
            "field `check.v1.M.a`: protovalidate rule `required` is not supported",
        ),
        (
            r#"message M { int32 a = 1 [(buf.validate.field).ignore = IGNORE_ALWAYS]; }"#,
            "rule `ignore` is not supported",
        ),
        (
            r#"message M { int32 a = 1 [(buf.validate.field).int32.const = 1]; }"#,
            "rule `int32.const` is not supported",
        ),
        (
            r#"message M { uint64 a = 1 [(buf.validate.field).uint64 = {in: [1, 2]}]; }"#,
            "rule `uint64.in` is not supported",
        ),
        (
            r#"message M { double a = 1 [(buf.validate.field).double.example = 1]; }"#,
            "rule `double.example` is not supported",
        ),
        (
            r#"message M { string a = 1 [(buf.validate.field).string.pattern = "^a$"]; }"#,
            "rule `string.pattern` is not supported",
        ),
        (
            r#"message M { string a = 1 [(buf.validate.field).string.email = true]; }"#,
            "rule `string.email` is not supported",
        ),
        (
            r#"message M { string a = 1 [(buf.validate.field).string.len = 3]; }"#,
            "rule `string.len` is not supported",
        ),
        (
            r#"message M { bytes a = 1 [(buf.validate.field).bytes.min_len = 1]; }"#,
            "rule `bytes.min_len` is not supported",
        ),
        (
            r#"message M { bool a = 1 [(buf.validate.field).bool.const = true]; }"#,
            "rule `bool.const` is not supported",
        ),
        (
            r#"message M { State a = 1 [(buf.validate.field).enum.defined_only = true]; }"#,
            "rule `enum.defined_only` is not supported",
        ),
        (
            r#"message M { repeated Child a = 1 [(buf.validate.field).repeated.unique = true]; }"#,
            "rule `repeated.unique` does not apply to items other than scalars and strings",
        ),
        (
            r#"message M { repeated State a = 1 [(buf.validate.field).repeated.unique = true]; }"#,
            "rule `repeated.unique` does not apply",
        ),
        (
            r#"message M { repeated string a = 1 [(buf.validate.field).repeated.items.string.email = true]; }"#,
            "rule `repeated.items.string.email` is not supported",
        ),
        (
            r#"message M { repeated string a = 1 [(buf.validate.field).repeated.items.required = true]; }"#,
            "rule `repeated.items.required` is not supported",
        ),
        (
            r#"message M { map<string, int32> a = 1 [(buf.validate.field).map.keys.string.pattern = "x"]; }"#,
            "rule `map.keys.string.pattern` is not supported",
        ),
        (
            r#"message M { map<string, Child> a = 1 [(buf.validate.field).map.values.required = true]; }"#,
            "rule `map.values.required` is not supported",
        ),
        (
            r#"message M { int32 a = 1 [(buf.validate.field).int64.gt = 1]; }"#,
            "field `check.v1.M.a`: protovalidate rule `int64` does not apply to int32 fields",
        ),
        (
            r#"message M { int32 a = 1 [(buf.validate.field).repeated.min_items = 1]; }"#,
            "rule `repeated` does not apply to int32 fields",
        ),
        (
            r#"message M { repeated int32 a = 1 [(buf.validate.field).int32.gt = 1]; }"#,
            "rule `int32` does not apply to repeated fields",
        ),
        (
            r#"message M { map<string, int32> a = 1 [(buf.validate.field).repeated.min_items = 1]; }"#,
            "rule `repeated` does not apply to map fields",
        ),
        (
            r#"message M { repeated string a = 1 [(buf.validate.field).repeated.items.int32.gt = 1]; }"#,
            "rule `repeated.items.int32` does not apply to string fields",
        ),
        (
            r#"message M { double a = 1 [(buf.validate.field).double = {gt: nan}]; }"#,
            "rule `double.gt` must not be NaN",
        ),
        (
            r#"message M { string a = 1 [(gin_tonic.v1.rust_type) = "crate::Id", (buf.validate.field).string.min_len = 1]; }"#,
            "rule `string.min_len` does not apply to fields with a custom rust_type",
        ),
        (
            r#"message M { int32 b = 1; oneof k { string a = 2 [(buf.validate.field).string.email = true]; int32 c = 3; } }"#,
            "field `check.v1.M.a`: protovalidate rule `string.email` is not supported",
        ),
        (
            r#"message M { oneof k { string a = 1 [(buf.validate.field).string.email = true]; int32 c = 2; } }"#,
            "field `check.v1.M.a`: protovalidate rule `string.email` is not supported",
        ),
        (
            r#"message M { message Inner { int32 a = 1 [(buf.validate.field).int32.const = 1]; } }"#,
            "field `check.v1.M.Inner.a`: protovalidate rule `int32.const` is not supported",
        ),
        (
            r#"message M { option (buf.validate.message).cel = {id: "x", expression: "true"}; int32 a = 1; }"#,
            "message `check.v1.M`: `(buf.validate.message)` rules are not supported",
        ),
        (
            r#"message M { int32 b = 1; oneof k { option (buf.validate.oneof).required = true; int32 a = 2; string c = 3; } }"#,
            "oneof `check.v1.M.k`: `(buf.validate.oneof)` rules are not supported",
        ),
    ];
    for (index, (body, expected)) in cases.into_iter().enumerate() {
        let (result, _) = compile_validation(&format!("unsupported-{index}"), body);
        let error = result.expect_err(body).to_string();
        assert!(error.contains(expected), "{body}: {error}");
    }
}
