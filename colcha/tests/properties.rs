use colcha::{
    EmptyHost, Error, Fact, Materialize, NodeId, Path, Schema, SchemaId, Stamp, State, Value, fold,
    parse,
};
use proptest::prelude::*;

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Sample {
    a: i64,
    b: bool,
    note: Option<String>,
}

fn stamp(clock: u64) -> Stamp {
    Stamp::new(clock, NodeId::from_u128(1))
}

fn schema() -> SchemaId {
    SchemaId::new(Sample::NAME, Sample::VERSION)
}

fn fact_strategy() -> impl Strategy<Value = Fact> {
    (
        0usize..3,
        0u64..8,
        0u128..3,
        any::<i64>(),
        any::<bool>(),
        ".{0,8}",
    )
        .prop_map(|(field, clock, node, integer, boolean, text)| {
            let path = Path::parse(["a", "b", "note"][field]).expect("path");
            let value = match field {
                0 => Value::Int(integer),
                1 => Value::Bool(boolean),
                _ => Value::String(text),
            };
            Fact::candidate(
                path,
                Stamp::new(clock, NodeId::from_u128(node)),
                schema(),
                value,
            )
        })
}

fn parts_strategy() -> impl Strategy<Value = Vec<Vec<Fact>>> {
    prop::collection::vec(prop::collection::vec(fact_strategy(), 0..6), 0..4)
}

fn result_key(result: Result<Value, Error>) -> String {
    match result {
        Ok(value) => format!("ok:{value:?}"),
        Err(error) => format!("err:{error:?}"),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn union_is_commutative_and_associative(parts in parts_strategy()) {
        let states: Vec<State> = parts.into_iter().map(State::from_iter).collect();
        let mut forward = State::empty();
        for state in &states {
            forward = forward.union(state);
        }
        let mut backward = State::empty();
        for state in states.iter().rev() {
            backward = backward.union(state);
        }
        prop_assert_eq!(&forward, &backward);
        prop_assert_eq!(&forward.union(&forward), &forward);
        prop_assert_eq!(
            result_key(fold::<Sample>(&forward, &EmptyHost)),
            result_key(fold::<Sample>(&backward, &EmptyHost)),
        );

        if states.len() >= 3 {
            let left = states[0].union(&states[1]).union(&states[2]);
            let right = states[0].union(&states[1].union(&states[2]));
            prop_assert_eq!(&left, &right);
        }
    }

    #[test]
    fn compact_preserves_the_fold(parts in parts_strategy()) {
        let mut state = State::empty();
        for part in parts {
            state = state.union(&State::from_iter(part));
        }
        prop_assert_eq!(
            result_key(fold::<Sample>(&state, &EmptyHost)),
            result_key(fold::<Sample>(&state.compact::<Sample>(), &EmptyHost)),
        );
    }

    #[test]
    fn parser_never_panics(source in ".{0,200}") {
        let _ = parse::<Sample>(&source, stamp(1));
    }
}
