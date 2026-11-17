# colcha

Typed configuration composed from replicable facts.

`colcha` derives the schema from a Rust type, accepts independent contributions as facts, merges them by set union and materializes one deterministic value. Merge is commutative, associative and idempotent. The only host effect happens in `resolve`.

Status: v1 preview. The contract is in [`SPEC.md`](SPEC.md); the rejected alternatives are in [`docs/adr/`](docs/adr/).

## Crates

| Crate | Role |
|-------|------|
| `colcha` | Algebra, parser, fold and materialization |
| `colcha-derive` | `#[derive(Schema)]` and `#[derive(Materialize)]` |

Edition 2024. MSRV 1.93.

## Quick start

```toml
[dependencies]
colcha = { path = "colcha" }
```

```rust
use std::collections::BTreeSet;
use colcha::{Materialize, NodeId, Schema, Stamp, State, parse, resolve};

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Alert {
    name: String,
    threshold: f64,
    tags: BTreeSet<String>,
    muted: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stamp = Stamp::new(1, NodeId::from_u128(7));
    let source = r#"
        name:      "db-alerts"
        threshold: 42.0 & <=100
    "#;

    let state = State::try_from_iter::<Alert>(parse::<Alert>(source, stamp)?)?;

    let alert = resolve::<Alert>(&state, &colcha::EmptyHost)?;
    assert_eq!(alert.threshold, 42.0);
    assert_eq!(alert.muted, false);
    assert!(alert.tags.is_empty());
    Ok(())
}
```

`fold::<Alert>(&state, &registry)` returns the concrete [`Value`] without effects. `resolve::<Alert>` calls `fold` and then materializes the Rust type.

## The alert example

The full example is in [`crates/colcha/examples/alert.rs`](crates/colcha/examples/alert.rs). The root holds `alert: Vec<Alert>`, keyed by `name`. Three contributions write different keys and merge field by field:

```text
alert.db-alerts: {
  threshold: float & >=0 & <=100
  notify:    _core.Webhook
  tags:      ["prod"]
}
```

`db-alerts` becomes the element key, so `name` needs no write. `severity` defaults to `warn` from the enum's `#[default]`; `tags` and `muted` use their natural defaults. A native `Webhook` runs per element only during `resolve`.

```text
cargo run -p colcha --example alert
```

Expected output, abridged:

```text
merge order independent: true

folded list (pure, no host effect):
  cache-miss   threshold=60   tags=[cache]
  db-alerts    threshold=95   tags=[db, oncall, prod]
  disk-full    threshold=99   tags=[oncall]
[host] build Webhook { url: "https://example.test/cache" }
[host] build Webhook { url: "https://example.test/hook" }
[host] build Webhook { url: "https://example.test/disk" }

resolved Config:
Config {
    alert: [
        Alert {
            name: "cache-miss",
            threshold: 60.0,
            severity: Warn,
            notify: Webhook { url: "https://example.test/cache" },
            tags: { "cache" },
            muted: false,
        },
        Alert {
            name: "db-alerts",
            threshold: 95.0,
            severity: Warn,
            notify: Webhook { url: "https://example.test/hook" },
            tags: { "db", "oncall", "prod" },
            muted: false,
        },
        Alert {
            name: "disk-full",
            threshold: 99.0,
            severity: Warn,
            notify: Webhook { url: "https://example.test/disk" },
            tags: { "oncall" },
            muted: false,
        },
    ],
}
```

## Model

| Concept | Meaning |
|---------|---------|
| `State` | `BTreeSet<Fact>`; merge uses set union |
| `Fact` | path, stamp, schema identity and kind |
| `Node` | schema derived from the Rust type |
| `fold` | pure selection of one concrete `Value` |
| `resolve` | `fold` plus `Materialize`; runs `NativeHost::build` |
| Tombstone | fact that absorbs older facts in its path subtree |

Defaults, constraints and candidates stay in the state. A candidate that violates a constraint loses without causing a conflict. Without a valid candidate, the fold uses the config default, then the schema default, then `Error::Fold`.

## Resolution

References, conditionals, comprehensions, interpolation and non-literal defaults are in scope, but they do not live in the fact algebra. Resolution turns authored config into concrete facts before the state:

```text
authoring -> resolution -> facts -> State -> fold -> Materialize
```

`evaluate::<T>(source, &context, stamp)` performs that resolution. It emits only `Fact`s with concrete `Value`s, so `fold` and `Materialize` never see authored syntax. `parse::<T>` and `evaluate::<T>` share one parser; `parse` runs it in concrete mode and rejects references, interpolation, conditionals, comprehensions and templated keys with `Error::Parse`.

```rust
use colcha::{Context, EmptyHost, NodeId, Stamp, State, evaluate, resolve};

let source = r#"
    name: "service"
    host: runtime.hostname
    label: "\(name)@\(host)"
    theme: *"dark" | string
    if theme == "dark" {
        mode: "night"
    } else {
        mode: "day"
    }
    aliases: {
        for key, value in known {
            "\(key)-alias": value
        }
    }
"#;

let context = Context::new()
    .with("runtime.hostname", colcha::Value::String("box".into()))?
    .with(
        "known",
        colcha::Value::Map([("a".into(), colcha::Value::String("one".into()))].into()),
    )?;

let state = State::try_from_iter::<Config>(evaluate::<Config>(
    source,
    &context,
    Stamp::new(1, NodeId::from_u128(1)),
)?)?;
let config = resolve::<Config>(&state, &EmptyHost)?;
```

References resolve in this order: comprehension binding, document field (including forward references), then `Context`. A missing reference, a cycle, an active duplicate or an ambiguous reference returns `Error::Eval`. Interpolation accepts scalars only. Truthiness is false for `null`, `false`, `0`, `0.0`, empty string, empty list and empty map.

`Context` is immutable for the caller. `with` and `merge` return a new context; `merge` is commutative and idempotent when paths do not disagree. Different values at the same path return `Error::Eval`.

`State` is immutable too. `with`, `union` and `compact` return a new state; no call changes an existing one in place.

## Defaults

Each type answers one question: does it have a natural default? If yes, the field opens as `*default | type` and absence stays valid. If no, the field must be present.

| Type | Natural default |
|------|-----------------|
| `bool` | `false` |
| `Option<T>` | `null` |
| `Vec<T>` | `[]` |
| `BTreeSet<T>`, `HashSet<T>` | `[]` |
| `BTreeMap<String, V>`, `HashMap<String, V>` | `{}` |
| unit enum with `#[default]` or `#[colcha(default)]` | the variant segment |
| tagged enum with a default variant | object with the tag and field defaults |
| struct whose fields all have defaults | composition of those defaults |

`String`, integers, `f32`, `f64` and native types have no natural default. A field of one of those types must be present. To accept absence, wrap the field in `Option<T>`. There is no `Required<T>`.

`Option<T>` treats absence and an explicit `null` as the same value. CUE distinguishes the two; colcha does not, and v1 has no `Nullable<T>`.

`#[colcha(default = <literal>)]` overrides the natural default. On an enum, `#[default]` comes from `#[derive(Default)]`; `#[colcha(default)]` marks the variant without deriving `Default`.

## Keyed collections

By default, `Vec<T>` behaves as a map keyed by one field. The element needs exactly one `#[colcha(key)]` `String` field. The key becomes a path segment: `alert.foo` addresses element `foo`, and `alert."web.1"` addresses element `web.1`. Keys are case-sensitive; quote a segment that contains a dot, slash, or other punctuation. The fold derives the key field from that segment. The fold orders output by key.

Use `#[colcha(atomic)]` for a whole-list value. `BTreeSet<T>` uses union.

Add `#[colcha(min_len = N)]` to a `Vec<T>` field to require at least `N` items. The constraint applies to the folded list, so the empty natural default fails when `N > 0`. An explicit empty keyed list, written as `tools: []`, is a valid fact and also fails the constraint.

## Native types

A native field uses `#[colcha(native)]` and implements `NativeType`. The host registers `Native` specs and returns a `NativeHandle` from `build`. The fold applies inline arguments against `Native::args` and rejects unknown arguments.

## Raw values

A `Raw` field accepts any concrete value. Use it for heterogeneous maps and for data whose shape the Rust type does not model.

```rust
use colcha::Raw;
use std::collections::BTreeMap;

#[derive(colcha::Schema, colcha::Materialize)]
struct File {
    values: BTreeMap<String, Raw>,
}
```

The DSL accepts an object literal after `values:`. `Raw` has no child paths. The parser rejects `values.foo.bar`; `values.foo` addresses one map entry. `Raw` materialization rejects native values.

## Tagged unions

An enum without a tag is a flat string. Add `#[colcha(tag = "kind")]` to carry fields per variant.

```rust
#[derive(colcha::Schema, colcha::Materialize)]
#[colcha(tag = "kind")]
enum Action {
    Restart,
    Start {
        service: String,
        #[colcha(default = 3)]
        retries: u32,
    },
}
```

The DSL writes the value as an object: `step: { kind: "start", service: "api" }`. The fold reads the tag, selects the variant, fills field defaults, and rejects unknown fields. The folded value keeps the tag. `Option<Action>` accepts `null`; `Vec<Action>` needs `#[colcha(atomic)]`.

## Open structs

A `#[colcha(rest)]` map collects the keys that the struct does not declare. The map value type decides how each extra key folds.

```rust
#[derive(colcha::Schema, colcha::Materialize)]
struct Place {
    name: String,
    #[colcha(rest)]
    extra: BTreeMap<String, Raw>,
}
```

With that schema, `owner: "team"` is a valid field and the folded value is `{ name: "...", owner: "team" }`. The materialized `extra` map holds `owner`. The rest segment itself is not writable: `extra: {...}` is `Error::Path`. A tagged union variant cannot declare a rest field.

## Tests

```text
cargo test --workspace
```

On NixOS without `cc` on `PATH`, run the same command under a compiler:

```text
nix shell nixpkgs#gcc --command cargo test --workspace
```

The suite covers the merge laws, compact equivalence, parser robustness, LWW, conflicts, tombstones, keyed lists, schema identity and native materialization. It also covers authored resolution: references, interpolation, conditionals, comprehensions and non-literal defaults.

## CUE parity corpus

`fixtures/cue-parity/` records real `modot home config dump` output for CUE features that colcha models in the fact algebra or resolves in `evaluate`. Its [README](fixtures/cue-parity/README.md) holds the feature checklist, the refresh command, and the open gaps.

## Limits

- No module system in the DSL.
- No network, storage or CLI in v1.
- No JSON, TOML or YAML input in v1.
- No true sequences (RGA, LSEQ).
- Truthful sequence editing, schema migration and `no_std` are later work.
