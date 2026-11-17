//! Three independent contributions compose into one typed `Alert`.
//!
//! Run with:
//!
//! ```text
//! cargo run --example alert
//! ```

use std::collections::BTreeSet;
use std::error::Error as StdError;

use colcha::{
    Materialize, Native, NativeHandle, NativeHost, NativeRegistry, NativeType, NodeId, Schema,
    Stamp, State, Value, parse, resolve,
};

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Config {
    alert: Alert,
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Alert {
    name: String,
    threshold: f64,
    severity: Severity,
    #[colcha(native)]
    notify: Webhook,
    #[colcha(union)]
    tags: BTreeSet<String>,
    muted: bool,
}

#[derive(Debug, PartialEq, Schema, Materialize)]
enum Severity {
    #[colcha(rename = "info")]
    Info,
    #[colcha(rename = "warn")]
    Warn,
    #[colcha(rename = "page")]
    Page,
}

#[derive(Debug, PartialEq)]
struct Webhook {
    url: String,
}

#[derive(Schema)]
#[allow(dead_code)]
struct WebhookArgs {
    url: String,
}

impl NativeType for Webhook {
    const NAME: &'static str = "Webhook";

    fn from_native(handle: NativeHandle) -> Result<Self, String> {
        handle.downcast::<Webhook>()
    }
}

struct DemoHost {
    webhook: Native,
}

impl DemoHost {
    fn new() -> DemoHost {
        DemoHost {
            webhook: Native::new("Webhook", WebhookArgs::schema()),
        }
    }
}

impl NativeRegistry for DemoHost {
    fn spec(&self, name: &str) -> Option<&Native> {
        (name == "Webhook").then_some(&self.webhook)
    }
}

impl NativeHost for DemoHost {
    fn build(
        &self,
        name: &str,
        args: &Value,
    ) -> Result<NativeHandle, Box<dyn StdError + Send + Sync>> {
        match name {
            "Webhook" => {
                let url = args
                    .as_map()
                    .and_then(|map| map.get("url"))
                    .and_then(Value::as_str)
                    .ok_or("Webhook needs a string url")?;
                println!("[host] build Webhook {{ url: {url:?} }}");
                Ok(NativeHandle::new(Webhook {
                    url: url.to_string(),
                }))
            }
            other => Err(format!("unknown native '{other}'").into()),
        }
    }
}

fn state_of(source: &str, clock: u64) -> Result<State, Box<dyn StdError>> {
    let stamp = Stamp::new(clock, NodeId::from_u128(0xfeed));
    let mut state = State::empty();
    for fact in parse::<Config>(source, stamp)? {
        state.insert::<Config>(fact)?;
    }
    Ok(state)
}

fn main() -> Result<(), Box<dyn StdError>> {
    let platform = state_of(
        r#"
        alert: {
          name:      string & =~"^[a-z0-9-]+$"
          threshold: float & >=0 & <=100
          severity:  *"warn" | "page" | "info"
          notify:    _core.Webhook
          tags:      ["prod"]
        }
        "#,
        1,
    )?;

    let service = state_of(
        r#"
        alert: {
          name:      "db-alerts"
          threshold: 80
          notify:    { url: "https://example.test/hook" }
          tags:      ["db"]
        }
        "#,
        2,
    )?;

    let oncall = state_of(
        r#"
        alert: {
          threshold: 95
          tags:      ["oncall"]
          muted:     false
        }
        "#,
        3,
    )?;

    let merged = platform.union(&service).union(&oncall);
    let reordered = oncall.union(&platform).union(&service);
    println!("merge order independent: {}", merged == reordered);

    let host = DemoHost::new();
    let folded = colcha::fold::<Config>(&merged, &host)?;
    println!("\nfolded value:\n{folded:#?}");

    let config = resolve::<Config>(&merged, &host)?;
    println!("\nresolved Config:\n{config:#?}");

    Ok(())
}
