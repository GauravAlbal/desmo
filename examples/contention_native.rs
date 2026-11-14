//! IK-07: native-machine write contention on one store (several SQLite connections).
//!
//! Each writer thread opens its own `Store`, which gives the same locking behaviour as separate
//! processes sharing the file. Every request is recorded, failures included; latency statistics
//! cover all requests. Run: `cargo run --release --example contention_native -- <out.json>`.

use ik::diagnostic::Diagnostic;
use ik::effect::EffectStatus;
use ik::identity::{AggregateId, Capability, MachineIdentity};
use ik::machine::{
    DomainEvent, EventCodec, Machine, NativeAdapter, NativeApply, NativeDecision, NativeMachine,
};
use ik::store::{CommandRequest, Store};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, Barrier};
use std::time::Instant;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct Count {
    n: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "name", rename_all = "lowercase")]
enum Cmd {
    Tick,
}

#[derive(Clone, Debug, Deserialize)]
struct NoObservation;

#[derive(Clone, Debug)]
struct Ticked;

impl EventCodec for Ticked {
    fn event_type(&self) -> &'static str {
        "ticked"
    }
    fn event_version(&self) -> u64 {
        1
    }
    fn payload(&self) -> Value {
        json!({})
    }
    fn decode(event: &DomainEvent) -> Result<Option<Self>, Diagnostic> {
        Ok((event.event_type == "ticked" && event.event_version == 1).then_some(Ticked))
    }
}

enum Never {}

impl From<Never> for Diagnostic {
    fn from(never: Never) -> Self {
        match never {}
    }
}

struct Counter {
    identity: MachineIdentity,
}

impl NativeMachine for Counter {
    type State = Count;
    type Command = Cmd;
    type Observation = NoObservation;
    type Event = Ticked;
    type Diagnostic = Never;

    fn identity(&self) -> &MachineIdentity {
        &self.identity
    }
    fn initial_state(&self) -> Self::State {
        Count { n: 0 }
    }
    fn decide(
        &self,
        _state: &Self::State,
        _observation: Option<&Self::Observation>,
        _effect: &EffectStatus,
        _command: &Self::Command,
    ) -> Result<NativeDecision<Self::Event>, Self::Diagnostic> {
        Ok(NativeDecision::emit(Ticked))
    }
    fn apply(
        &self,
        state: &Self::State,
        _event: &Self::Event,
    ) -> Result<NativeApply<Self::State>, Self::Diagnostic> {
        Ok(NativeApply::new(Count { n: state.n + 1 }))
    }
}

fn machine() -> Arc<dyn Machine> {
    Arc::new(NativeAdapter::new(Counter {
        identity: ik::machine::native_identity("ik07_native_counter", 1, "ik07-native-counter-v1"),
    }))
}

struct Row {
    ms: f64,
    outcome: &'static str,
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((pct / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    (sorted[index.min(sorted.len() - 1)] * 100.0).round() / 100.0
}

fn summarize(name: &str, writers: usize, same_aggregate: bool, rows: Vec<Row>, wall: f64) -> Value {
    let mut latencies: Vec<f64> = rows.iter().map(|r| r.ms).collect();
    latencies.sort_by(|a, b| a.total_cmp(b));
    let mut outcomes = std::collections::BTreeMap::new();
    for row in &rows {
        *outcomes.entry(row.outcome).or_insert(0usize) += 1;
    }
    let completed = outcomes.get("ok").copied().unwrap_or(0);
    json!({
        "experiment": name,
        "mode": "native",
        "client": "in_process_connections",
        "writers": writers,
        "same_aggregate": same_aggregate,
        "requests": rows.len(),
        "completed": completed,
        "outcomes": outcomes,
        "latency_ms_all_requests": {
            "p50": percentile(&latencies, 50.0),
            "p95": percentile(&latencies, 95.0),
            "p99": percentile(&latencies, 99.0),
            "max": latencies.last().copied().unwrap_or(0.0),
        },
        "wall_s": (wall * 1000.0).round() / 1000.0,
        "completed_per_s": (completed as f64 / wall * 100.0).round() / 100.0,
    })
}

fn run(name: &str, writers: usize, count: usize, same_aggregate: bool) -> Value {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("store.db");
    let m = machine();
    let identity = m.identity().clone();
    {
        let mut store = Store::create(&path).expect("create store");
        let ids: Vec<String> = if same_aggregate {
            vec!["shared".into()]
        } else {
            (0..writers).map(|i| format!("agg-{i}")).collect()
        };
        for id in &ids {
            store
                .create_aggregate(m.as_ref(), &AggregateId(id.clone()), &identity)
                .expect("create aggregate");
        }
    }
    let barrier = Arc::new(Barrier::new(writers + 1));
    let handles: Vec<_> = (0..writers)
        .map(|index| {
            let path = path.clone();
            let barrier = barrier.clone();
            let identity = identity.clone();
            let aggregate = AggregateId(if same_aggregate {
                "shared".to_string()
            } else {
                format!("agg-{index}")
            });
            std::thread::spawn(move || {
                let m = machine();
                let mut store = Store::open(&path).expect("open");
                let mut capability: Capability = store.show(&aggregate).expect("show").capability;
                let mut rows = Vec::with_capacity(count);
                barrier.wait();
                for _ in 0..count {
                    let began = Instant::now();
                    let result = store.command(
                        m.as_ref(),
                        &CommandRequest {
                            aggregate_id: aggregate.clone(),
                            machine: identity.clone(),
                            capability: capability.clone(),
                            command: json!({"name": "tick"}),
                            observation: None,
                        },
                    );
                    let ms = began.elapsed().as_secs_f64() * 1000.0;
                    match result {
                        Ok(done) => {
                            capability = done.capability;
                            rows.push(Row { ms, outcome: "ok" });
                        }
                        Err(err) => {
                            let outcome = match err.code {
                                ik::diagnostic::DiagnosticCode::IkCapabilityStale => {
                                    "stale_refusal"
                                }
                                _ => "other",
                            };
                            rows.push(Row { ms, outcome });
                            capability = store.show(&aggregate).expect("show").capability;
                        }
                    }
                }
                rows
            })
        })
        .collect();
    barrier.wait();
    let began = Instant::now();
    let rows: Vec<Row> = handles
        .into_iter()
        .flat_map(|h| h.join().expect("writer"))
        .collect();
    summarize(
        name,
        writers,
        same_aggregate,
        rows,
        began.elapsed().as_secs_f64(),
    )
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .expect("usage: contention_native <out.json>");
    let results = vec![
        run("uncontended", 1, 300, false),
        run("same_aggregate_race", 4, 150, true),
        run("unrelated_aggregates_shared_store", 4, 150, false),
    ];
    std::fs::write(&out, serde_json::to_string_pretty(&results).unwrap() + "\n").unwrap();
    println!("wrote {out}");
}
