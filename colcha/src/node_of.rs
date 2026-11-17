use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::schema::{Constraint, FoldPolicy, Node};

/// Derive-time bridge from a Rust field type to its schema node.
#[doc(hidden)]
pub trait NodeOf {
    const KEY_FIELD: Option<&'static str> = None;
    fn node() -> Node;
}

impl NodeOf for String {
    fn node() -> Node {
        Node::leaf(FoldPolicy::Lww).with_constraint(Constraint::type_name("string"))
    }
}

impl NodeOf for bool {
    fn node() -> Node {
        Node::leaf(FoldPolicy::Lww).with_constraint(Constraint::type_name("bool"))
    }
}

macro_rules! integer_node {
    ($($ty:ty),* $(,)?) => {
        $(
            impl NodeOf for $ty {
                fn node() -> Node {
                    Node::leaf(FoldPolicy::Lww)
                        .with_constraint(Constraint::type_name("int"))
                }
            }
        )*
    };
}

integer_node!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

macro_rules! float_node {
    ($($ty:ty),* $(,)?) => {
        $(
            impl NodeOf for $ty {
                fn node() -> Node {
                    Node::leaf(FoldPolicy::Lww)
                        .with_constraint(Constraint::type_name("float"))
                }
            }
        )*
    };
}

float_node!(f32, f64);

impl<T: NodeOf> NodeOf for Option<T> {
    fn node() -> Node {
        T::node().optional()
    }
}

impl<T: NodeOf> NodeOf for Vec<T> {
    fn node() -> Node {
        Node::list(T::node(), T::KEY_FIELD.map(str::to_string))
    }
}

impl<T: NodeOf> NodeOf for BTreeSet<T> {
    fn node() -> Node {
        Node {
            element: Some(Box::new(T::node())),
            ..Node::leaf(FoldPolicy::Union)
        }
    }
}

impl<T: NodeOf> NodeOf for HashSet<T> {
    fn node() -> Node {
        Node {
            element: Some(Box::new(T::node())),
            ..Node::leaf(FoldPolicy::Union)
        }
    }
}

impl<V: NodeOf> NodeOf for BTreeMap<String, V> {
    fn node() -> Node {
        Node::map(V::node())
    }
}

impl<V: NodeOf> NodeOf for HashMap<String, V> {
    fn node() -> Node {
        Node::map(V::node())
    }
}
