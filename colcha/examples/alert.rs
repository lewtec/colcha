//! Three independent contributions compose into one keyed list of alerts.
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
    alert: Vec<Alert>,
}

#[derive(Debug, PartialEq, Schema, Materialize)]
struct Alert {
    #[colcha(key)]
    name: String,
    threshold: f64,
    #[colcha(default = "warn")]
    severity: Severity,
    #[colcha(native)]
    notify: Webhook,
    #[colcha(union)]
    tags: BTreeSet<String>,
    #[colcha(default = false)]
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

fn summarize(value: &Value) -> Vec<String> {
    let Some(Value::List(alerts)) = value.as_map().and_then(|map| map.get("alert")) else {
        return Vec::new();
    };
    alerts
        .iter()
        .filter_map(|alert| {
            let map = alert.as_map()?;
            let name = map.get("name")?.as_str()?;
            let threshold = map.get("threshold")?.as_f64()?;
            let tags = map
                .get("tags")
                .and_then(Value::as_list)
                .map(|tags| {
                    tags.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            Some(format!(
                "  {name:<12} threshold={threshold:<4} tags=[{tags}]"
            ))
        })
        .collect()
}

fn main() -> Result<(), Box<dyn StdError>> {
    let platform = state_of(
        r#"
        alert.db-alerts: {
          threshold: float & >=0 & <=100
          notify:    _core.Webhook
          tags:      ["prod"]
        }
        "#,
        1,
    )?;

    let service = state_of(
        r#"
        alert.db-alerts: {
          threshold: 80
          notify:    { url: "https://example.test/hook" }
          tags:      ["db"]
        }
        alert.cache-miss: {
          threshold: 60
          notify:    { url: "https://example.test/cache" }
          tags:      ["cache"]
        }
        "#,
        2,
    )?;

    let oncall = state_of(
        r#"
        alert.db-alerts: {
          threshold: 95
          tags:      ["oncall"]
        }
        alert.disk-full: {
          threshold: 99
          notify:    { url: "https://example.test/disk" }
          tags:      ["oncall"]
        }
        "#,
        3,
    )?;

    let merged = platform.union(&service).union(&oncall);
    let reordered = oncall.union(&platform).union(&service);
    println!("merge order independent: {}", merged == reordered);

    let host = DemoHost::new();
    let folded = colcha::fold::<Config>(&merged, &host)?;
    println!("\nfolded list (pure, no host effect):");
    for line in summarize(&folded) {
        println!("{line}");
    }

    let config = resolve::<Config>(&merged, &host)?;
    println!("\nresolved Config:\n{config:#?}");

    Ok(())
}
