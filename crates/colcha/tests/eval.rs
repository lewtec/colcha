use std::collections::{BTreeMap, BTreeSet};

use colcha::{
    Context, EmptyHost, Error, Materialize, NodeId, Schema, Stamp, State, Value, evaluate, fold,
    resolve,
};

fn stamp(clock: u64) -> Stamp {
    Stamp::new(clock, NodeId::from_u128(11))
}

fn state_of<T: Schema>(source: &str, clock: u64) -> State {
    state_with_context::<T>(source, &Context::new(), clock)
}

fn state_with_context<T: Schema>(source: &str, context: &Context, clock: u64) -> State {
    let facts = evaluate::<T>(source, context, stamp(clock)).expect("evaluate");
    State::try_from_iter::<T>(facts).expect("state")
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Interpolated {
    label: String,
    first: String,
    second: String,
}

#[test]
fn forward_references_and_interpolation_resolve() {
    let state = state_of::<Interpolated>(
        r#"
        label: "\(first)-\(second)"
        first: "alpha"
        second: "beta"
        "#,
        1,
    );
    let value = resolve::<Interpolated>(&state, &EmptyHost).expect("resolve");
    assert_eq!(
        value,
        Interpolated {
            label: "alpha-beta".into(),
            first: "alpha".into(),
            second: "beta".into(),
        }
    );
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Defaulted {
    theme: String,
    active: String,
}

#[test]
fn a_non_literal_default_can_reference_another_default() {
    let state = state_of::<Defaulted>(
        r#"
        theme: *"dark" | string
        active: *theme
        "#,
        1,
    );
    let value = resolve::<Defaulted>(&state, &EmptyHost).expect("resolve");
    assert_eq!(value.theme, "dark");
    assert_eq!(value.active, "dark");
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Conditional {
    mode: Option<String>,
    count: Option<f64>,
    label: Option<String>,
    alternate: Option<String>,
}

#[test]
fn conditionals_select_only_the_matching_branch() {
    let state = state_of::<Conditional>(
        r#"
        mode: "dark"
        if mode == "dark" {
            label: "night"
        } else {
            alternate: "day"
        }
        "#,
        1,
    );
    let value = resolve::<Conditional>(&state, &EmptyHost).expect("resolve");
    assert_eq!(value.label.as_deref(), Some("night"));
    assert_eq!(value.alternate, None);
}

#[test]
fn else_if_selects_the_first_matching_branch() {
    let state = state_of::<Conditional>(
        r#"
        mode: "light"
        if mode == "dark" {
            label: "night"
        } else if mode == "light" {
            alternate: "day"
        } else {
            label: "other"
        }
        "#,
        1,
    );
    let value = resolve::<Conditional>(&state, &EmptyHost).expect("resolve");
    assert_eq!(value.label, None);
    assert_eq!(value.alternate.as_deref(), Some("day"));
}

#[test]
fn equality_compares_numbers_across_int_and_float() {
    let state = state_of::<Conditional>(
        r#"
        mode: "1"
        count: 1
        if count == 1.0 {
            label: "one"
        }
        "#,
        1,
    );
    let value = resolve::<Conditional>(&state, &EmptyHost).expect("resolve");
    assert_eq!(value.label.as_deref(), Some("one"));
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct App {
    browser: String,
    url: String,
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Apps {
    sources: BTreeMap<String, App>,
    apps: BTreeMap<String, App>,
    default: String,
}

#[test]
fn comprehensions_expand_map_entries_and_object_references() {
    let state = state_of::<Apps>(
        r#"
        default: "firefox"
        sources: {
            media: { browser: "firefox", url: "https://media.test" }
            code:  { browser: "chromium", url: "https://code.test" }
        }
        apps: {
            for name, app in sources {
                "\(name)": {
                    browser: app.browser
                    url: app.url
                }
            }
        }
        "#,
        1,
    );
    let value = resolve::<Apps>(&state, &EmptyHost).expect("resolve");
    assert_eq!(value.apps.len(), 2);
    assert_eq!(value.apps["media"].browser, "firefox");
    assert_eq!(value.apps["code"].url, "https://code.test");
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct AppPair {
    base: App,
    other: App,
}

#[test]
fn a_struct_reference_becomes_a_nested_value() {
    let state = state_of::<AppPair>(
        r#"
        base: { browser: "firefox", url: "https://base.test" }
        other: base
        "#,
        1,
    );
    let value = resolve::<AppPair>(&state, &EmptyHost).expect("resolve");
    assert_eq!(value.other, value.base);
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Numbered {
    #[colcha(atomic)]
    source: Vec<String>,
    lines: BTreeMap<String, String>,
}

#[test]
fn comprehensions_over_lists_expose_the_index() {
    let state = state_of::<Numbered>(
        r#"
        source: ["alpha", "beta"]
        lines: {
            for i, item in source {
                "line-\(i)": item
            }
        }
        "#,
        1,
    );
    let value = resolve::<Numbered>(&state, &EmptyHost).expect("resolve");
    assert_eq!(value.lines["line-0"], "alpha");
    assert_eq!(value.lines["line-1"], "beta");
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Hosted {
    hostname: String,
    timezone: String,
}

#[test]
fn context_values_fill_references_that_are_not_in_the_document() {
    let context = Context::new()
        .with(
            "runtime",
            Value::Map(BTreeMap::from([
                ("hostname".into(), Value::String("box".into())),
                ("timezone".into(), Value::String("UTC".into())),
            ])),
        )
        .unwrap();
    let state = state_with_context::<Hosted>(
        r#"
        hostname: runtime.hostname
        timezone: runtime.timezone
        "#,
        &context,
        1,
    );
    let value = resolve::<Hosted>(&state, &EmptyHost).expect("resolve");
    assert_eq!(value.hostname, "box");
    assert_eq!(value.timezone, "UTC");
}

#[test]
fn contexts_combine_into_new_values() {
    let base = Context::new()
        .with("runtime.hostname", Value::String("box".into()))
        .unwrap();
    let extra = Context::new()
        .with("runtime.timezone", Value::String("UTC".into()))
        .unwrap();

    let merged = base.merge(&extra).expect("merge");
    assert_eq!(
        merged
            .get(&colcha::Path::parse("runtime.hostname").unwrap())
            .and_then(Value::as_str),
        Some("box")
    );
    assert_eq!(
        merged
            .get(&colcha::Path::parse("runtime.timezone").unwrap())
            .and_then(Value::as_str),
        Some("UTC")
    );

    assert_eq!(
        base.get(&colcha::Path::parse("runtime.timezone").unwrap()),
        None
    );
    assert_eq!(base.merge(&extra).unwrap(), merged);
    assert_eq!(extra.merge(&base).unwrap(), merged);
}

#[test]
fn context_merges_reject_conflicting_values() {
    let first = Context::new()
        .with("runtime.hostname", Value::String("box".into()))
        .unwrap();
    let second = Context::new()
        .with("runtime.hostname", Value::String("other".into()))
        .unwrap();
    let error = first.merge(&second).expect_err("conflict");
    assert!(matches!(error, Error::Eval { .. }), "{error:?}");

    let repeated = first
        .with("runtime.hostname", Value::String("box".into()))
        .expect("equal value is idempotent");
    assert_eq!(repeated, first);

    let error = first
        .with("runtime.hostname", Value::String("other".into()))
        .expect_err("conflict");
    assert!(matches!(error, Error::Eval { .. }), "{error:?}");
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Points {
    mode: Option<String>,
    values: Option<BTreeSet<String>>,
}

#[test]
fn conditional_branches_can_be_empty() {
    let state = state_of::<Points>(
        r#"
        mode: "off"
        if mode == "on" {
            values: ["a"]
        }
        "#,
        1,
    );
    let value = resolve::<Points>(&state, &EmptyHost).expect("resolve");
    assert_eq!(value.values, None);
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Files {
    home: BTreeMap<String, String>,
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct FileConfig {
    file: Files,
}

#[test]
fn chained_field_blocks_are_flattened() {
    let state = state_of::<FileConfig>(
        r#"
        file: home: {
            ".config/theme.toml": "accent = \"blue\""
        }
        "#,
        1,
    );
    let value = resolve::<FileConfig>(&state, &EmptyHost).expect("resolve");
    assert_eq!(
        value
            .file
            .home
            .get(".config/theme.toml")
            .map(String::as_str),
        Some("accent = \"blue\"")
    );
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Nested {
    seeds: BTreeMap<String, String>,
    first: BTreeMap<String, String>,
    second: BTreeMap<String, String>,
}

#[test]
fn a_comprehension_can_read_another_comprehensions_output() {
    let state = state_of::<Nested>(
        r#"
        seeds: { a: "one", b: "two" }
        first: {
            for key, value in seeds {
                "nested-\(key)": value
            }
        }
        second: {
            for key, value in first {
                "copy-\(key)": value
            }
        }
        "#,
        1,
    );
    let value = resolve::<Nested>(&state, &EmptyHost).expect("resolve");
    assert_eq!(value.second["copy-nested-a"], "one");
    assert_eq!(value.second["copy-nested-b"], "two");
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct RawHolder {
    source: BTreeMap<String, String>,
    data: colcha::Raw,
}

#[test]
fn raw_blocks_can_reference_document_fields() {
    let state = state_of::<RawHolder>(
        r#"
        source: { a: "x" }
        data: { value: source.a }
        "#,
        1,
    );
    let value = resolve::<RawHolder>(&state, &EmptyHost).expect("resolve");
    assert_eq!(
        value.data.as_value().as_map().unwrap().get("value"),
        Some(&Value::String("x".into()))
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
    copy: Vec<Server>,
}

#[test]
fn keyed_lists_fold_from_blocks_and_references() {
    let state = state_of::<Fleet>(
        r#"
        servers: {
            beta: { port: 2 }
            alpha: { port: 1 }
        }
        copy: servers
        "#,
        1,
    );
    let value = resolve::<Fleet>(&state, &EmptyHost).expect("resolve");
    assert_eq!(
        value.servers,
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
    assert_eq!(value.copy, value.servers);
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct GuardedComp {
    enabled: bool,
    values: BTreeMap<String, String>,
}

#[test]
fn a_comprehension_in_an_inactive_branch_is_not_evaluated() {
    let state = state_of::<GuardedComp>(
        r#"
        enabled: false
        if enabled {
            values: {
                for key, value in missing {
                    "\(key)": value
                }
            }
        }
        "#,
        1,
    );
    let value = resolve::<GuardedComp>(&state, &EmptyHost).expect("resolve");
    assert!(value.values.is_empty());
}

#[test]
fn an_unresolved_reference_is_an_evaluation_error() {
    let error = evaluate::<Interpolated>(
        r#"label: "x"
first: missing"#,
        &Context::new(),
        stamp(1),
    )
    .expect_err("unresolved reference");
    assert!(matches!(error, Error::Eval { .. }), "{error:?}");
    let message = error.to_string();
    assert!(message.contains("reference 'missing'"), "{message}");
    assert!(message.contains("does not resolve"), "{message}");
    assert!(message.contains("(at 2:1)"), "{message}");
}

#[test]
fn a_reference_cycle_is_an_evaluation_error() {
    let error = evaluate::<Interpolated>(
        r#"
        label: first
        first: second
        second: first
        "#,
        &Context::new(),
        stamp(1),
    )
    .expect_err("cycle");
    assert!(matches!(error, Error::Eval { .. }), "{error:?}");
}

#[test]
fn a_guard_that_reads_its_own_field_is_an_evaluation_error() {
    let error = evaluate::<Interpolated>(
        r#"
        label: "x"
        if label == "x" {
            label: "x"
        }
        "#,
        &Context::new(),
        stamp(1),
    )
    .expect_err("self-referential guard");
    assert!(matches!(error, Error::Eval { .. }), "{error:?}");
}

#[test]
fn duplicate_active_fields_are_an_evaluation_error() {
    let error = evaluate::<Interpolated>(
        r#"
        label: "a"
        label: "b"
        "#,
        &Context::new(),
        stamp(1),
    )
    .expect_err("duplicate");
    assert!(matches!(error, Error::Eval { .. }), "{error:?}");
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct InterpObject {
    label: String,
    data: BTreeMap<String, String>,
    second: String,
}

#[test]
fn interpolation_rejects_non_scalars() {
    let error = evaluate::<InterpObject>(
        r#"
        label: "\(data)"
        data: { nested: "x" }
        second: "y"
        "#,
        &Context::new(),
        stamp(1),
    )
    .expect_err("object interpolation");
    assert!(matches!(error, Error::Eval { .. }), "{error:?}");
}

#[test]
fn authored_facts_are_concrete() {
    let facts = evaluate::<Interpolated>(
        r#"
        label: "\(first)"
        first: "a"
        second: "b"
        "#,
        &Context::new(),
        stamp(1),
    )
    .expect("evaluate");
    assert!(!facts.is_empty());
    for fact in &facts {
        let value = match &fact.kind {
            colcha::FactKind::Candidate(value) | colcha::FactKind::Default(value) => value,
            _ => continue,
        };
        assert!(!value.contains_native());
    }
    let state = State::try_from_iter::<Interpolated>(facts).expect("state");
    let value = fold::<Interpolated>(&state, &EmptyHost).expect("fold");
    assert_eq!(
        value.as_map().unwrap().get("label"),
        Some(&Value::String("a".into()))
    );
}
