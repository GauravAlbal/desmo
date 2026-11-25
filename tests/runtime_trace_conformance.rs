use ik::canonical::digest_json;
use ik::declarative::explore::{explore_graph, GraphEdge, RuntimeGraph};
use ik::declarative::{Bounds, CaseCorpus};
use ik::identity::AggregateId;
use ik::machine::Machine;
use ik::store::{CommandRequest, Store};
use serde_json::{json, Value};

#[derive(Debug)]
struct Mismatch<'a> {
    step: usize,
    observed: &'a Observation,
    expected: Vec<&'a GraphEdge>,
}

#[derive(Clone, Debug)]
struct Observation {
    command: String,
    args: Value,
    outcome: String,
    before: Value,
    after: Value,
}

fn state_id(state: &Value) -> String {
    format!("n{}", digest_json(state))
}

fn mismatch<'a>(
    graph: &'a RuntimeGraph,
    trace: &'a [Observation],
) -> Option<Mismatch<'a>> {
    let mut current = graph.initial.clone();
    for (index, observed) in trace.iter().enumerate() {
        let from = state_id(&observed.before);
        let to = state_id(&observed.after);
        if graph.edges.iter().any(|edge| {
            edge.from == current && edge.from == from && edge.command == observed.command
                && edge.args == observed.args && edge.outcome == observed.outcome && edge.to == to
        }) {
            current = to;
        } else {
            let expected = graph.edges.iter().filter(|edge| edge.from == current).collect();
            return Some(Mismatch { step: index + 1, observed, expected });
        }
    }
    None
}

#[test]
fn store_runtime_trace_matches_full_state_graph_and_rejects_mutations() {
    let machine = ik::declarative::load_def(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/profile/linear.ik.toml"),
    ).unwrap();
    let identity = machine.identity().clone();
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(&dir.path().join("trace.db")).unwrap();
    let aggregate = AggregateId("trace".into());
    store.create_aggregate(&machine, &aggregate, &identity).unwrap();
    let (_run, graph) = explore_graph(&machine, &CaseCorpus::new(), Bounds {
        max_nodes: 32, max_depth: 8,
    });
    let mut trace = Vec::new();
    for command in ["finish", "step", "finish"] {
        let projection = store.show(&aggregate).unwrap();
        let before = projection.state;
        let request = CommandRequest {
            aggregate_id: aggregate.clone(), machine: identity.clone(),
            capability: projection.capability,
            command: json!({"name": command}), observation: None,
        };
        let result = store.command(&machine, &request);
        let (outcome, after) = match result {
            Ok(result) => {
                assert_eq!(result.state, store.show(&aggregate).unwrap().state);
                (if result.mutation { "committed" } else { "noop" }.to_string(), result.state)
            }
            Err(error) => {
                assert_eq!(error.code.as_str(), "IK_ILLEGAL_STATE");
                (format!("refused:{}", error.code.as_str()), store.show(&aggregate).unwrap().state)
            }
        };
        trace.push(Observation { command: command.into(), args: json!({}), outcome, before, after });
    }
    assert_eq!(trace.iter().map(|step| step.outcome.as_str()).collect::<Vec<_>>(),
        ["refused:IK_ILLEGAL_STATE", "committed", "committed"]);
    assert_eq!(trace[0].before, trace[0].after);
    eprintln!("runtime observations: {trace:#?}");
    assert_eq!(trace[2].after["state"], "done");
    assert!(mismatch(&graph, &trace).is_none(), "real trace rejected");

    let mut wrong_outcome = trace.clone();
    wrong_outcome[0].outcome = "committed".into();
    let rejected = mismatch(&graph, &wrong_outcome).expect("refusal mutation accepted");
    assert_eq!(rejected.step, 1);
    assert_eq!(rejected.observed.outcome, "committed");
    assert!(rejected.expected.iter().any(|edge| edge.outcome == "refused:IK_ILLEGAL_STATE"));
    eprintln!("outcome control rejected: {rejected:#?}");

    let mut wrong_fact = trace.clone();
    wrong_fact[1].after["unexpected_fact"] = json!("different");
    assert_eq!(wrong_fact[1].after["state"], trace[1].after["state"]);
    let rejected = mismatch(&graph, &wrong_fact).expect("fact mutation accepted");
    assert_eq!(rejected.step, 2);
    assert_eq!(rejected.observed.after["unexpected_fact"], "different");
    eprintln!("same-label fact control rejected: {rejected:#?}");
}
