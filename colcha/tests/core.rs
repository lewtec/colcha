use std::collections::{BTreeMap, BTreeSet};

use colcha::{
    EmptyHost, Error, FactKind, Materialize, Native, NativeHandle, NativeHost, NativeRegistry,
    NativeType, NodeId, Raw, Schema, Stamp, State, Value, fold, parse, resolve,
};

fn stamp(clock: u64) -> Stamp {
    Stamp::new(clock, NodeId::from_u128(7))
}

fn state_of<T: Schema>(source: &str, clock: u64) -> State {
    let mut state = State::empty();
    for fact in parse::<T>(source, stamp(clock)).expect("parse") {
        state.insert::<T>(fact).expect("insert");
    }
    state
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Service {
    name: String,
    #[colcha(default = 0.0)]
    threshold: f64,
    #[colcha(default = true)]
    enabled: bool,
    tags: Option<BTreeSet<String>>,
    #[colcha(atomic)]
    order: Option<Vec<String>>,
}

#[test]
fn parse_fold_and_resolve_a_struct() {
    let base = state_of::<Service>(
        r#"
        name: "checkout"
        threshold: float & >=0 & <=100
        tags: ["prod"]
        order: ["a", "b"]
        "#,
        1,
    );
    let extra = state_of::<Service>(
        r#"
        threshold: 12.5
        tags: ["db", "prod"]
        "#,
        2,
    );
    let state = extra.union(&base);
    let value = fold::<Service>(&state, &EmptyHost).expect("fold");
    let map = value.as_map().expect("map");
    assert_eq!(map.get("name"), Some(&Value::String("checkout".into())));
    assert_eq!(
        map.get("threshold"),
        Some(&Value::Float(colcha::Float::new(12.5).unwrap()))
    );
    assert_eq!(map.get("enabled"), Some(&Value::Bool(true)));
    assert_eq!(
        map.get("tags"),
        Some(&Value::List(vec![
            Value::String("db".into()),
            Value::String("prod".into()),
        ]))
    );
    assert_eq!(
        map.get("order"),
        Some(&Value::List(vec![
            Value::String("a".into()),
            Value::String("b".into()),
        ]))
    );

    let service = resolve::<Service>(&state, &EmptyHost).expect("resolve");
    assert_eq!(
        service,
        Service {
            name: "checkout".into(),
            threshold: 12.5,
            enabled: true,
            tags: Some(BTreeSet::from(["db".into(), "prod".into()])),
            order: Some(vec!["a".into(), "b".into()]),
        }
    );
}

#[test]
fn lww_uses_the_highest_stamp() {
    let first = state_of::<Service>(r#"name: "first""#, 1);
    let second = state_of::<Service>(r#"name: "second""#, 2);
    let value = fold::<Service>(&first.union(&second), &EmptyHost).expect("fold");
    assert_eq!(
        value.as_map().unwrap().get("name"),
        Some(&Value::String("second".into()))
    );
}

#[test]
fn same_stamp_different_values_conflict() {
    let first = state_of::<Service>(r#"name: "first""#, 1);
    let second = state_of::<Service>(r#"name: "second""#, 1);
    let error = fold::<Service>(&first.union(&second), &EmptyHost).expect_err("conflict");
    assert!(matches!(error, Error::Conflict { .. }), "{error:?}");
}

#[test]
fn an_invalid_candidate_loses_to_a_valid_one() {
    let constrained = state_of::<Service>(
        r#"name: "svc"
threshold: float & <=10"#,
        1,
    );
    let invalid = state_of::<Service>(r#"threshold: 50"#, 2);
    let valid = state_of::<Service>(r#"threshold: 4"#, 1);
    let value =
        fold::<Service>(&constrained.union(&invalid).union(&valid), &EmptyHost).expect("fold");
    assert_eq!(
        value.as_map().unwrap().get("threshold"),
        Some(&Value::Int(4))
    );
}

#[test]
fn a_tombstone_absorbs_older_facts() {
    let mut state = state_of::<Service>(r#"name: "old""#, 1);
    let path = colcha::Path::parse("name").unwrap();
    state
        .insert::<Service>(colcha::Fact::tombstone(
            path,
            stamp(2),
            colcha::SchemaId::new(Service::NAME, Service::VERSION),
        ))
        .expect("tombstone");
    let error = fold::<Service>(&state, &EmptyHost).expect_err("required field is gone");
    assert!(matches!(error, Error::Fold { .. }), "{error:?}");
}

#[test]
fn a_newer_fact_revives_a_tombstone() {
    let mut state = state_of::<Service>(r#"name: "old""#, 1);
    let path = colcha::Path::parse("name").unwrap();
    state
        .insert::<Service>(colcha::Fact::tombstone(
            path,
            stamp(2),
            colcha::SchemaId::new(Service::NAME, Service::VERSION),
        ))
        .expect("tombstone");
    let newer = state_of::<Service>(r#"name: "new""#, 3);
    let value = fold::<Service>(&state.union(&newer), &EmptyHost).expect("fold");
    assert_eq!(
        value.as_map().unwrap().get("name"),
        Some(&Value::String("new".into()))
    );
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Server {
    #[colcha(key)]
    name: String,
    port: u16,
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Fleet {
    servers: Vec<Server>,
}

#[test]
fn a_keyed_vec_materializes_in_key_order() {
    let state = state_of::<Fleet>(
        r#"servers: [{ name: "beta", port: 2 }, { name: "alpha", port: 1 }]"#,
        1,
    );
    let fleet = resolve::<Fleet>(&state, &EmptyHost).expect("resolve");
    assert_eq!(
        fleet.servers,
        vec![
            Server {
                name: "alpha".into(),
                port: 1,
            },
            Server {
                name: "beta".into(),
                port: 2,
            },
        ]
    );
}

#[test]
fn writing_a_key_field_is_rejected() {
    let error = parse::<Fleet>(r#"servers.alpha.name: "other""#, stamp(1)).expect_err("key write");
    assert!(matches!(error, Error::Path { .. }), "{error:?}");
}

#[test]
fn unknown_fields_are_rejected() {
    let error = parse::<Service>(r#"unknown: 1"#, stamp(1)).expect_err("unknown field");
    assert!(matches!(error, Error::Path { .. }), "{error:?}");
}

#[test]
fn fold_rejects_a_foreign_schema() {
    let state = state_of::<Service>(r#"name: "x""#, 1);
    let fact = colcha::Fact::candidate(
        colcha::Path::parse("name").unwrap(),
        stamp(2),
        colcha::SchemaId::new("other::Type", 1),
        Value::String("y".into()),
    );
    let foreign = State::from_iter([fact]);
    let error = fold::<Service>(&state.union(&foreign), &EmptyHost).expect_err("schema");
    assert!(matches!(error, Error::Schema { .. }), "{error:?}");
}

#[test]
fn parse_reports_bad_syntax() {
    let error = parse::<Service>(r#"name: "#, stamp(1)).expect_err("syntax");
    assert!(matches!(error, Error::Parse { .. }), "{error:?}");
}

#[test]
fn compact_preserves_the_current_fold() {
    let one = state_of::<Service>(r#"name: "first""#, 1);
    let two = state_of::<Service>(r#"name: "second""#, 2);
    let state = one.union(&two);
    let before = fold::<Service>(&state, &EmptyHost).expect("fold");
    let after = fold::<Service>(&state.compact::<Service>(), &EmptyHost).expect("fold");
    assert_eq!(before, after);
    assert!(state.compact::<Service>().len() < state.len());
}

#[test]
fn a_union_fact_is_not_dropped_by_compact() {
    let one = state_of::<Service>(
        r#"name: "svc"
tags: ["a"]"#,
        1,
    );
    let two = state_of::<Service>(r#"tags: ["b"]"#, 2);
    let state = one.union(&two);
    let compacted = state.compact::<Service>();
    let value = fold::<Service>(&compacted, &EmptyHost).expect("fold");
    assert_eq!(
        value.as_map().unwrap().get("tags"),
        Some(&Value::List(vec![
            Value::String("a".into()),
            Value::String("b".into()),
        ]))
    );
}

#[test]
fn facts_are_immutable_values() {
    let facts = parse::<Service>(r#"name: "x""#, stamp(1)).unwrap();
    assert!(matches!(facts[0].kind, FactKind::Candidate(_)));
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct NativeConfig {
    name: String,
    #[colcha(native)]
    hook: Hook,
}

#[derive(Debug, PartialEq)]
struct Hook {
    url: String,
}

#[derive(Schema)]
#[allow(dead_code)]
struct HookArgs {
    url: String,
}

impl NativeType for Hook {
    const NAME: &'static str = "Hook";

    fn from_native(handle: NativeHandle) -> Result<Self, String> {
        handle.downcast::<Hook>()
    }
}

struct TestHost {
    hook: Native,
    builds: std::cell::Cell<usize>,
}

impl TestHost {
    fn new() -> TestHost {
        TestHost {
            hook: Native::new("Hook", HookArgs::schema()),
            builds: std::cell::Cell::new(0),
        }
    }
}

impl NativeRegistry for TestHost {
    fn spec(&self, name: &str) -> Option<&Native> {
        (name == "Hook").then_some(&self.hook)
    }
}

impl NativeHost for TestHost {
    fn build(
        &self,
        name: &str,
        args: &Value,
    ) -> Result<NativeHandle, Box<dyn std::error::Error + Send + Sync>> {
        self.builds.set(self.builds.get() + 1);
        match name {
            "Hook" => {
                let url = args
                    .as_map()
                    .and_then(|map| map.get("url"))
                    .and_then(Value::as_str)
                    .ok_or("Hook needs url")?;
                Ok(NativeHandle::new(Hook {
                    url: url.to_string(),
                }))
            }
            other => Err(format!("unknown native '{other}'").into()),
        }
    }
}

#[test]
fn resolve_builds_each_native_once() {
    let state = state_of::<NativeConfig>(
        r#"name: "x"
hook: _core.Hook { url: "u" }"#,
        1,
    );
    let host = TestHost::new();
    fold::<NativeConfig>(&state, &host).expect("fold");
    assert_eq!(host.builds.get(), 0);
    let config = resolve::<NativeConfig>(&state, &host).expect("resolve");
    assert_eq!(host.builds.get(), 1);
    assert_eq!(config.hook.url, "u");
}

#[test]
fn resolve_does_not_build_when_fold_fails() {
    let state = state_of::<NativeConfig>(r#"hook: _core.Hook { url: "u" }"#, 1);
    let host = TestHost::new();
    let error = resolve::<NativeConfig>(&state, &host).expect_err("missing name");
    assert!(matches!(error, Error::Fold { .. }), "{error:?}");
    assert_eq!(host.builds.get(), 0);
}

#[test]
fn unknown_native_argument_is_rejected() {
    let state = state_of::<NativeConfig>(
        r#"name: "x"
hook: _core.Hook { urll: "u" }"#,
        1,
    );
    let host = TestHost::new();
    let error = fold::<NativeConfig>(&state, &host).expect_err("unknown argument");
    assert!(matches!(error, Error::Fold { .. }), "{error:?}");
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct FlatConfig {
    #[colcha(flat)]
    mode: String,
}

#[test]
fn flat_policy_rejects_disagreement() {
    let first = state_of::<FlatConfig>(r#"mode: "a""#, 1);
    let second = state_of::<FlatConfig>(r#"mode: "b""#, 2);
    let error = fold::<FlatConfig>(&first.union(&second), &EmptyHost).expect_err("conflict");
    assert!(matches!(error, Error::Conflict { .. }), "{error:?}");
}

#[test]
fn a_large_input_is_rejected_before_parsing() {
    let source = "x".repeat(1024 * 1024 + 1);
    let error = parse::<Service>(&source, stamp(1)).expect_err("too large");
    assert!(matches!(error, Error::Parse { .. }), "{error:?}");
}

#[test]
fn contributions_add_elements_to_a_keyed_list() {
    let one = state_of::<Fleet>(r#"servers.alpha: { port: 1 }"#, 1);
    let two = state_of::<Fleet>(r#"servers.beta: { port: 2 }"#, 2);
    let fleet = resolve::<Fleet>(&one.union(&two), &EmptyHost).expect("resolve");
    assert_eq!(
        fleet.servers,
        vec![
            Server {
                name: "alpha".into(),
                port: 1,
            },
            Server {
                name: "beta".into(),
                port: 2,
            },
        ]
    );
}

#[test]
fn writing_a_key_field_inside_a_block_is_rejected() {
    let error = parse::<Fleet>(r#"servers.alpha: { name: "other", port: 1 }"#, stamp(1))
        .expect_err("key write");
    assert!(matches!(error, Error::Path { .. }), "{error:?}");
}

#[test]
fn keyed_lists_accept_quoted_keys() {
    let state = state_of::<Fleet>(
        r#"servers."web.1": { port: 1 }
servers."org/edge": { port: 2 }"#,
        1,
    );
    let fleet = resolve::<Fleet>(&state, &EmptyHost).expect("resolve");
    assert_eq!(
        fleet.servers,
        vec![
            Server {
                name: "org/edge".into(),
                port: 2,
            },
            Server {
                name: "web.1".into(),
                port: 1,
            },
        ]
    );
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Labels {
    values: BTreeMap<String, String>,
}

#[test]
fn map_keys_are_arbitrary_and_case_sensitive() {
    let state = state_of::<Labels>(
        r#"
        values."base0A": "yellow"
        values."base0a": "lower"
        values."org/gnome/desktop/interface": "dark"
        values."a.b": "dot"
        "#,
        1,
    );
    let labels = resolve::<Labels>(&state, &EmptyHost).expect("resolve");
    assert_eq!(
        labels.values.get("base0A").map(String::as_str),
        Some("yellow")
    );
    assert_eq!(
        labels.values.get("base0a").map(String::as_str),
        Some("lower")
    );
    assert_eq!(
        labels
            .values
            .get("org/gnome/desktop/interface")
            .map(String::as_str),
        Some("dark")
    );
    assert_eq!(labels.values.get("a.b").map(String::as_str), Some("dot"));
}

#[test]
fn map_blocks_preserve_case_and_punctuation() {
    let state = state_of::<Labels>(
        r#"
        values: {
            "base0A": "yellow"
            "org/gnome/desktop/interface": "dark"
        }
        "#,
        1,
    );
    let labels = resolve::<Labels>(&state, &EmptyHost).expect("resolve");
    assert_eq!(
        labels.values.get("base0A").map(String::as_str),
        Some("yellow")
    );
    assert_eq!(
        labels
            .values
            .get("org/gnome/desktop/interface")
            .map(String::as_str),
        Some("dark")
    );
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct LabelLists {
    #[colcha(atomic)]
    entries: Vec<BTreeMap<String, String>>,
}

#[test]
fn object_literals_preserve_case_and_punctuation() {
    let state = state_of::<LabelLists>(
        r#"entries: [{ "base0A": "yellow", "org/gnome/desktop/interface": "dark" }]"#,
        1,
    );
    let lists = resolve::<LabelLists>(&state, &EmptyHost).expect("resolve");
    let entry = &lists.entries[0];
    assert_eq!(entry.get("base0A").map(String::as_str), Some("yellow"));
    assert_eq!(
        entry.get("org/gnome/desktop/interface").map(String::as_str),
        Some("dark")
    );
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Palette {
    #[colcha(rename = "base0A")]
    base_0a: String,
}

#[test]
fn renamed_fields_stay_case_sensitive() {
    let state = state_of::<Palette>(r#"base0A: "yellow""#, 1);
    let palette = resolve::<Palette>(&state, &EmptyHost).expect("resolve");
    assert_eq!(palette.base_0a, "yellow");

    let error = parse::<Palette>(r#"base0a: "lower""#, stamp(1)).expect_err("unknown field");
    assert!(matches!(error, Error::Path { .. }), "{error:?}");
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct RawConfig {
    metadata: BTreeMap<String, Raw>,
    blob: Option<Raw>,
}

#[test]
fn raw_fields_accept_any_concrete_value() {
    let state = state_of::<RawConfig>(
        r#"
        metadata: {
            "base0A": { hex: "yellow", rank: 1 }
            "org/gnome/desktop/interface": { "gtk-theme": "adw", flags: [1, true, null] }
        }
        blob: { enabled: true, items: [{ name: "x" }, 3] }
        "#,
        1,
    );
    let config = resolve::<RawConfig>(&state, &EmptyHost).expect("resolve");

    let yellow = config.metadata.get("base0A").expect("yellow").as_value();
    let yellow = yellow.as_map().expect("map");
    assert_eq!(yellow.get("hex"), Some(&Value::String("yellow".into())));
    assert_eq!(yellow.get("rank"), Some(&Value::Int(1)));

    let dconf = config
        .metadata
        .get("org/gnome/desktop/interface")
        .expect("dconf")
        .as_value();
    assert_eq!(
        dconf.as_map().and_then(|map| map.get("gtk-theme")),
        Some(&Value::String("adw".into()))
    );

    let blob = config.blob.expect("blob").into_inner();
    assert_eq!(
        blob.as_map().and_then(|map| map.get("enabled")),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        blob.as_map().and_then(|map| map.get("items")),
        Some(&Value::List(vec![
            Value::Map(BTreeMap::from([
                ("name".into(), Value::String("x".into()),)
            ])),
            Value::Int(3),
        ]))
    );
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct RawBlock {
    blob: Raw,
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct RawItem {
    #[colcha(key)]
    name: String,
    payload: Raw,
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct RawItems {
    items: Vec<RawItem>,
}

#[test]
fn raw_fields_work_inside_keyed_lists() {
    let state = state_of::<RawItems>(r#"items: [{ name: "a", payload: { x: 1, y: [true] } }]"#, 1);
    let items = resolve::<RawItems>(&state, &EmptyHost).expect("resolve");
    assert_eq!(items.items.len(), 1);
    assert_eq!(items.items[0].name, "a");
    let payload = items.items[0].payload.as_value().as_map().expect("map");
    assert_eq!(payload.get("x"), Some(&Value::Int(1)));
}

#[test]
fn raw_fields_accept_block_shorthand() {
    let state = state_of::<RawBlock>(r#"blob { a: 1 b: [true] }"#, 1);
    let config = resolve::<RawBlock>(&state, &EmptyHost).expect("resolve");
    let blob = config.blob.as_value().as_map().expect("map");
    assert_eq!(blob.get("a"), Some(&Value::Int(1)));
    assert_eq!(blob.get("b"), Some(&Value::List(vec![Value::Bool(true)])));
}

#[test]
fn raw_fields_have_no_child_paths() {
    let error = parse::<RawConfig>(r#"blob.enabled: true"#, stamp(1)).expect_err("raw child");
    assert!(matches!(error, Error::Path { .. }), "{error:?}");
}

#[test]
fn path_display_round_trips_arbitrary_segments() {
    let path = colcha::Path::from_segments(vec![
        "base0A".into(),
        "org/gnome/desktop/interface".into(),
        ".config/legacy".into(),
        "quote\"slash\\".into(),
        "line\nbreak".into(),
    ])
    .expect("path");
    assert_eq!(
        path.to_string(),
        r#"base0A."org/gnome/desktop/interface".".config/legacy"."quote\"slash\\"."line\nbreak""#
    );
    assert_eq!(
        colcha::Path::parse(&path.to_string()).expect("round trip"),
        path
    );
}

#[derive(Debug, Default, PartialEq, Schema, Materialize)]
enum Mode {
    #[default]
    #[colcha(rename = "off")]
    Off,
    #[colcha(rename = "on")]
    On,
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Natural {
    name: String,
    enabled: bool,
    tags: BTreeSet<String>,
    mode: Mode,
    note: Option<String>,
}

#[test]
fn natural_defaults_fill_a_partial_struct() {
    let state = state_of::<Natural>(r#"name: "svc""#, 1);
    let value = fold::<Natural>(&state, &EmptyHost).expect("fold");
    let map = value.as_map().expect("map");
    assert_eq!(map.get("enabled"), Some(&Value::Bool(false)));
    assert_eq!(map.get("tags"), Some(&Value::List(Vec::new())));
    assert_eq!(map.get("mode"), Some(&Value::String("off".into())));
    assert_eq!(map.get("note"), Some(&Value::Null));

    let natural = resolve::<Natural>(&state, &EmptyHost).expect("resolve");
    assert_eq!(
        natural,
        Natural {
            name: "svc".into(),
            enabled: false,
            tags: BTreeSet::new(),
            mode: Mode::Off,
            note: None,
        }
    );
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct AllDefaults {
    enabled: bool,
    tags: BTreeSet<String>,
    mode: Mode,
    note: Option<String>,
}

#[test]
fn a_struct_composes_the_defaults_of_its_fields() {
    let default = resolve::<AllDefaults>(&State::empty(), &EmptyHost).expect("resolve");
    assert_eq!(
        default,
        AllDefaults {
            enabled: false,
            tags: BTreeSet::new(),
            mode: Mode::Off,
            note: None,
        }
    );
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Required {
    name: String,
}

#[test]
fn a_type_without_a_natural_default_stays_required() {
    let error = fold::<Required>(&State::empty(), &EmptyHost).expect_err("required");
    assert!(matches!(error, Error::Fold { .. }), "{error:?}");
}

#[derive(Debug, PartialEq, Schema, Materialize)]
enum Fallback {
    #[colcha(default)]
    Off,
    On,
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct UsesFallback {
    mode: Fallback,
}

#[test]
fn a_colcha_default_variant_works_without_deriving_default() {
    let value = resolve::<UsesFallback>(&State::empty(), &EmptyHost).expect("resolve");
    assert_eq!(
        value,
        UsesFallback {
            mode: Fallback::Off,
        }
    );
}

#[test]
fn a_keyed_list_defaults_to_empty() {
    let fleet = resolve::<Fleet>(&State::empty(), &EmptyHost).expect("resolve");
    assert!(fleet.servers.is_empty());
}
