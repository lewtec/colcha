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
    #[colcha(union)]
    tags: BTreeSet<String>,
    muted: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stamp = Stamp::new(1, NodeId::from_u128(7));
    let source = r#"
        name:      "db-alerts"
        threshold: 42.0 & <=100
        tags:      ["prod"]
        muted:     false
    "#;

    let mut state = State::empty();
    for fact in parse::<Alert>(source, stamp)? {
        state.insert::<Alert>(fact)?;
    }

    let alert = resolve::<Alert>(&state, &colcha::EmptyHost)?;
    assert_eq!(alert.threshold, 42.0);
    Ok(())
}
```

`fold::<Alert>(&state, &registry)` returns the concrete [`Value`] without effects. `resolve::<Alert>` calls `fold` and then materializes the Rust type.

## The alert example

The full example is in [`colcha/examples/alert.rs`](colcha/examples/alert.rs). It merges three contributions with distinct stamps, folds them and resolves a typed value. A native `Webhook` runs only during `resolve`.

```text
cargo run -p colcha --example alert
```

Expected output, abridged:

```text
merge order independent: true
[host] build Webhook { url: "https://example.test/hook" }

resolved Config:
Config {
    alert: Alert {
        name: "db-alerts",
        threshold: 95.0,
        severity: Warn,
        notify: Webhook {
            url: "https://example.test/hook",
        },
        tags: {
            "db",
            "oncall",
            "prod",
        },
        muted: false,
    },
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

## Keyed collections

`Vec<T>` is a map by key. The element needs exactly one `#[colcha(key)]` `String` field. The key becomes a path segment. The fold derives the field value from the key. Output is ordered by key.

Use `#[colcha(atomic)]` for a whole-list value. `BTreeSet<T>` uses union.

## Native types

A native field uses `#[colcha(native)]` and implements `NativeType`. The host registers `Native` specs and returns a `NativeHandle` from `build`. The fold applies inline arguments against `Native::args` and rejects unknown arguments.

## Tests

```text
cargo test --workspace
```

On NixOS without `cc` on `PATH`, run the same command under a compiler:

```text
nix shell nixpkgs#gcc --command cargo test --workspace
```

The suite covers the merge laws, compact equivalence, parser robustness, LWW, conflicts, tombstones, keyed lists, schema identity and native materialization.

## Limits

- No module system in the DSL.
- No network, storage or CLI in v1.
- No JSON, TOML or YAML input in v1.
- No true sequences (RGA, LSEQ).
- Truthful sequence editing, schema migration and `no_std` are later work.
