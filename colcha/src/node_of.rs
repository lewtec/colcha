use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::schema::{Constraint, FoldPolicy, Node};
use crate::value::{Raw, Value};

/// Derive-time bridge from a Rust field type to its schema node.
#[doc(hidden)]
pub trait NodeOf {
    const KEY_FIELD: Option<&'static str> = None;

    /// Natural default of the type. `None` makes the field required.
    fn default_value() -> Option<Value> {
        None
    }

    fn node() -> Node;
}

impl NodeOf for String {
    fn node() -> Node {
        Node::leaf(FoldPolicy::Lww).with_constraint(Constraint::type_name("string"))
    }
}

impl NodeOf for Raw {
    fn node() -> Node {
        Node::raw()
    }
}

impl NodeOf for bool {
    fn default_value() -> Option<Value> {
        Some(Value::Bool(false))
    }

    fn node() -> Node {
        let mut node = Node::leaf(FoldPolicy::Lww).with_constraint(Constraint::type_name("bool"));
        node.default = Self::default_value();
        node
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
    fn default_value() -> Option<Value> {
        Some(Value::Null)
    }

    fn node() -> Node {
        let mut node = T::node().optional();
        node.default = Self::default_value();
        node
    }
}

impl<T: NodeOf> NodeOf for Vec<T> {
    fn default_value() -> Option<Value> {
        Some(Value::List(Vec::new()))
    }

    fn node() -> Node {
        let mut node = Node::list(T::node(), T::KEY_FIELD.map(str::to_string));
        node.default = Self::default_value();
        node
    }
}

impl<T: NodeOf> NodeOf for BTreeSet<T> {
    fn default_value() -> Option<Value> {
        Some(Value::List(Vec::new()))
    }

    fn node() -> Node {
        let mut node = Node {
            element: Some(Box::new(T::node())),
            ..Node::leaf(FoldPolicy::Union)
        };
        node.default = Self::default_value();
        node
    }
}

impl<T: NodeOf> NodeOf for HashSet<T> {
    fn default_value() -> Option<Value> {
        Some(Value::List(Vec::new()))
    }

    fn node() -> Node {
        let mut node = Node {
            element: Some(Box::new(T::node())),
            ..Node::leaf(FoldPolicy::Union)
        };
        node.default = Self::default_value();
        node
    }
}

impl<V: NodeOf> NodeOf for BTreeMap<String, V> {
    fn default_value() -> Option<Value> {
        Some(Value::Map(BTreeMap::new()))
    }

    fn node() -> Node {
        let mut node = Node::map(V::node());
        node.default = Self::default_value();
        node
    }
}

impl<V: NodeOf> NodeOf for HashMap<String, V> {
    fn default_value() -> Option<Value> {
        Some(Value::Map(BTreeMap::new()))
    }

    fn node() -> Node {
        let mut node = Node::map(V::node());
        node.default = Self::default_value();
        node
    }
}
