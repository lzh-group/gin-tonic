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

/// Compile `body` as `check.proto` (importing `gin.proto`) and return every generated source,
/// as types with children land in nested submodule files.
fn compile_options_with(
    name: &str,
    body: &str,
    configure: impl FnOnce(CompileConfig) -> CompileConfig,
) -> (Result<(), super::CompilerError>, Vec<String>) {
    let directory =
        std::env::temp_dir().join(format!("gin-codegen-options-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let proto = directory.join("check.proto");
    std::fs::write(
        &proto,
        format!(
            r#"
            syntax = "proto3";
            package check.v1;
            import "gin.proto";
            {body}
            "#
        ),
    )
    .unwrap();
    let output = directory.join("out");
    let mut config = configure(CompileConfig::new().add_proto_file(&proto));
    #[cfg(feature = "tonic")]
    {
        config = config.skip_services();
    }
    let result = config.compile_into(&output);
    let files = generated_files(&output.join("check"));
    let generated_options = output.join("gin").exists();
    std::fs::remove_dir_all(directory).unwrap();
    assert!(!generated_options, "option packages must not be generated");
    (result, files)
}

fn compile_options_files(
    name: &str,
    body: &str,
) -> (Result<(), super::CompilerError>, Vec<String>) {
    compile_options_with(name, body, |config| config)
}

fn compile_options(name: &str, body: &str) -> (Result<(), super::CompilerError>, String) {
    let (result, files) = compile_options_files(name, body);
    (result, files.join("\n"))
}

fn generated_files(directory: &std::path::Path) -> Vec<String> {
    let mut sources = vec![];
    for entry in std::fs::read_dir(directory).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources.extend(generated_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            sources.push(std::fs::read_to_string(&path).unwrap_or_default());
        }
    }
    sources
}

/// The attributes of the generated struct or enum named `name`, rendered as tokens.
fn item_attributes(files: &[String], name: &str) -> String {
    files
        .iter()
        .filter_map(|file| syn::parse_file(file).ok())
        .flat_map(|file| file.items)
        .find_map(|item| match item {
            syn::Item::Enum(item) if item.ident == name => Some(item.attrs),
            syn::Item::Struct(item) if item.ident == name => Some(item.attrs),
            _ => None,
        })
        .map(|attrs| quote::quote!(#(#attrs)*).to_string())
        .unwrap_or_else(|| panic!("`{name}` is not generated: {}", files.join("\n")))
}

#[test]
fn enum_options_become_strum_and_serde_attributes() {
    let (result, source) = compile_options(
        "enum",
        r#"
        enum Exchange {
            option (gin_tonic.v1.enum) = { strum_case: CASE_UPPER, serde_case: CASE_UPPER };
            EXCHANGE_A = 0;
        }
        enum Side {
            option (gin_tonic.v1.enum) = { serde_case: CASE_SNAKE };
            SIDE_A = 0;
        }
        enum Plain { PLAIN_A = 0; }
        "#,
    );
    result.unwrap();
    assert!(
        source.contains("#[derive(strum::AsRefStr, strum::EnumString)]"),
        "{source}"
    );
    assert!(
        source.contains("#[strum(serialize_all = \"UPPERCASE\")]"),
        "{source}"
    );
    assert!(
        source.contains("#[serde(rename_all = \"UPPERCASE\")]"),
        "{source}"
    );
    assert!(
        source.contains("#[serde(rename_all = \"snake_case\")]"),
        "{source}"
    );
    // An enum without rules keeps the generated defaults only.
    let plain = source.split("pub enum Plain").nth(1).unwrap();
    assert!(!plain.contains("strum"), "{source}");
}

#[test]
fn file_options_are_defaults_overridden_by_the_type() {
    let (result, files) = compile_options_files(
        "file",
        r#"
        option (gin_tonic.v1.file) = {
            enum: { strum_case: CASE_SNAKE }
            message: { deny_unknown_fields: true }
        };
        enum Inherits { INHERITS_A = 0; }
        enum Overrides {
            option (gin_tonic.v1.enum) = { strum_case: CASE_PASCAL };
            OVERRIDES_A = 0;
        }
        message Kept { int32 a = 1; }
        message Wider {
            option (gin_tonic.v1.message) = { serde_default: true };
            int32 a = 1;
        }
        message Cleared {
            option (gin_tonic.v1.message) = { deny_unknown_fields: false };
            int32 a = 1;
        }
        "#,
    );
    result.unwrap();
    let snake = quote::quote!(#[strum(serialize_all = "snake_case")]).to_string();
    let pascal = quote::quote!(#[strum(serialize_all = "PascalCase")]).to_string();
    assert!(item_attributes(&files, "Inherits").contains(&snake));
    assert!(item_attributes(&files, "Overrides").contains(&pascal));
    let deny = quote::quote!(#[serde(deny_unknown_fields)]).to_string();
    assert!(item_attributes(&files, "Kept").contains(&deny));
    // Both container modifiers share a single serde attribute.
    let both = quote::quote!(#[serde(default, deny_unknown_fields)]).to_string();
    assert!(item_attributes(&files, "Wider").contains(&both));
    // An explicit `false` clears the file default.
    let cleared = item_attributes(&files, "Cleared");
    assert!(!cleared.contains("deny_unknown_fields"), "{cleared}");
}

#[test]
fn oneof_options_apply_and_unwrapped_oneofs_take_the_message_rules() {
    let (result, files) = compile_options_files(
        "oneof",
        r#"
        option (gin_tonic.v1.file) = {
            message: { serde_default: true }
        };
        message Signal {
            option (gin_tonic.v1.message) = { deny_unknown_fields: true };
            int32 id = 1;
            oneof signal_outcome {
                option (gin_tonic.v1.oneof) = { serde_case: CASE_SNAKE };
                int32 computed = 2;
                string failed = 3;
            }
        }
        message Outcome {
            option (gin_tonic.v1.message) = {
                serde_default: false
                deny_unknown_fields: true
            };
            oneof outcome {
                option (gin_tonic.v1.oneof) = { serde_case: CASE_SNAKE };
                int32 computed = 1;
                string failed = 2;
            }
        }
        "#,
    );
    result.unwrap();
    // A oneof inside a struct is a separate serde type: only its own rules apply.
    let wrapped = item_attributes(&files, "SignalOutcome");
    assert!(wrapped.contains("rename_all"), "{wrapped}");
    assert!(!wrapped.contains("deny_unknown_fields"), "{wrapped}");
    assert!(!wrapped.contains("default"), "{wrapped}");
    // A message holding only a oneof is generated as the enum, so its rules apply to it.
    let unwrapped = item_attributes(&files, "Outcome");
    let deny = unwrapped.find("deny_unknown_fields").expect(&unwrapped);
    let rename = unwrapped.find("rename_all").expect(&unwrapped);
    assert!(deny < rename, "{unwrapped}");
    assert!(!unwrapped.contains("default"), "{unwrapped}");
}

#[test]
fn field_options_choose_types_and_add_attributes() {
    let (result, source) = compile_options(
        "field",
        r#"
        message Node {
            map<string, int32> counts = 1 [(gin_tonic.v1.field) = { btree_map: true }];
            int32 code = 2 [(gin_tonic.v1.field) = { deserialize_with: "crate::storage::decode" }];
            Node child = 3 [(gin_tonic.v1.field) = { boxed: true }];
            map<string, int32> plain = 4;
            oneof choice {
                int32 tagged = 5 [(gin_tonic.v1.field) = { deserialize_with: "crate::storage::tag" }];
                Node nested = 6 [(gin_tonic.v1.field) = { boxed: true }];
            }
        }
        "#,
    );
    result.unwrap();
    assert!(
        source.contains("pub counts: ::std::collections::BTreeMap<String, i32>"),
        "{source}"
    );
    assert!(
        source.contains("#[serde(deserialize_with = \"crate::storage::decode\")]"),
        "{source}"
    );
    assert!(
        source.contains("pub child: ::std::boxed::Box<Node>"),
        "{source}"
    );
    // A map without the rule keeps the default hasher-backed map.
    assert!(
        source.contains("pub plain: ::gin_tonic::fxhash::FxHashMap<String, i32>"),
        "{source}"
    );
    // Oneof members honour field rules as well.
    assert!(
        source.contains("#[serde(deserialize_with = \"crate::storage::tag\")]"),
        "{source}"
    );
    assert!(source.contains("Nested(::std::boxed::Box<"), "{source}");
}

#[test]
fn field_options_compose_with_configured_paths() {
    let (result, source) = compile_options_with(
        "compose",
        r#"
        message M {
            map<string, int32> by_option = 1 [(gin_tonic.v1.field) = { btree_map: true }];
            map<string, int32> by_config = 2;
        }
        "#,
        |config| config.btree_map([".check.v1.M.by_config"]),
    );
    result.unwrap();
    let source = source.join("\n");
    assert!(
        source.contains("pub by_option: ::std::collections::BTreeMap"),
        "{source}"
    );
    assert!(
        source.contains("pub by_config: ::std::collections::BTreeMap"),
        "{source}"
    );
}

#[test]
fn unsupported_rust_options_fail_generation() {
    let cases = [
        (
            r#"message M { int32 a = 1 [(gin_tonic.v1.field) = { btree_map: true }]; }"#,
            "`(gin_tonic.v1.field.btree_map)` does not apply to fields other than maps",
        ),
        (
            r#"message M { map<string, M> a = 1 [(gin_tonic.v1.field) = { boxed: true }]; }"#,
            "`(gin_tonic.v1.field.boxed)` does not apply to map fields",
        ),
        (
            r#"message M { int32 a = 1 [(gin_tonic.v1.field) = { deserialize_with: "not a path" }]; }"#,
            "`(gin_tonic.v1.field.deserialize_with)` value `not a path` is not a Rust path",
        ),
        (
            r#"message M { int32 b = 1; oneof o { int32 a = 2 [(gin_tonic.v1.field) = { deserialize_with: "not a path" }]; } }"#,
            "check.v1.M.a: `(gin_tonic.v1.field.deserialize_with)`",
        ),
        (
            r#"enum E { option (gin_tonic.v1.enum) = { strum_case: CASE_UNSPECIFIED }; E_A = 0; }"#,
            "`(gin_tonic.v1.enum.strum_case)` value `CASE_UNSPECIFIED` is not supported",
        ),
        (
            r#"message M { int32 a = 1; oneof o { option (gin_tonic.v1.oneof) = { serde_case: CASE_UNSPECIFIED }; int32 b = 2; } }"#,
            "`(gin_tonic.v1.oneof.serde_case)` value `CASE_UNSPECIFIED` is not supported",
        ),
        (
            r#"option (gin_tonic.v1.file) = { enum: { serde_case: CASE_UNSPECIFIED } }; enum E { E_A = 0; }"#,
            "`(gin_tonic.v1.file.enum.serde_case)` value `CASE_UNSPECIFIED` is not supported",
        ),
        (
            r#"message M { option (gin_tonic.v1.message) = { serde_default: true }; oneof o { int32 a = 1; } }"#,
            "`(gin_tonic.v1.message.serde_default)` does not apply to a message holding only a oneof",
        ),
    ];
    for (index, (body, expected)) in cases.into_iter().enumerate() {
        let (result, _) = compile_options(&format!("unsupported-{index}"), body);
        let error = result.expect_err(body).to_string();
        assert!(error.contains(expected), "{body}: {error}");
    }
}
