//! One-command verification check over a declarative machine.
//!
//! Composes plan, coordinate-stable diff, explore, discharge, change-local
//! adequacy, report fold, and judge. Does not decide transition legality.

use super::compile::compile;
use super::def::Definition;
use super::diff::diff_machines;
use super::evidence::discharge_exploration;
use super::explore::{explore, Bounds, CaseCorpus, Exploration};
use super::local::{change_local_guards, LocalAdequacy};
use super::plan::{plan_diff, verification_plan, VerificationObligationV1, VerificationPlanV1};
use super::release::{release_evidence, ReleaseEvidence, ReleaseInputs};
use super::report::{judge_acceptance, report_from_evidence, AcceptJudgment, VerificationReportV1};
use serde::{Deserialize, Serialize};

/// Standard `ik check` lane. Other kinds need a specialized verifier.
pub fn standard_lane(kind: &str) -> bool {
    matches!(
        kind,
        "OBL-STATE-001" | "OBL-CMD-001" | "OBL-OUT-001" | "OBL-TERM-001"
    )
}

/// Result of `ik check`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    pub verdict: String,
    pub text: String,
    pub report: VerificationReportV1,
    pub judgment: AcceptJudgment,
    /// Strict release policy, status, and provenance. A developer ACCEPT does not imply
    /// `release.release.verdict == "release_pass"`.
    #[serde(default)]
    pub release: ReleaseEvidence,
    /// Prior-vs-new structural delta. Absent unless `--prior` was supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<DefinitionDelta>,
}

/// Declared shape change. Not an execution or readiness result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionDelta {
    pub schema: String,
    pub prior_version: u64,
    pub new_version: u64,
    pub commands_added: Vec<String>,
    pub commands_removed: Vec<String>,
    pub states_added: Vec<String>,
    pub states_removed: Vec<String>,
    pub statement: String,
}

/// Run the full check pipeline.
pub fn check_machine(
    new: &Definition,
    prior: Option<&Definition>,
    corpus: &CaseCorpus,
    baseline: &[String],
    bounds: Bounds,
) -> Result<CheckResult, String> {
    check_with_run(new, prior, corpus, baseline, bounds, false).map(|(result, _, _, _, _)| result)
}

pub(crate) fn check_with_run(
    new: &Definition,
    prior: Option<&Definition>,
    corpus: &CaseCorpus,
    baseline: &[String],
    bounds: Bounds,
    capture_invariant_refusals: bool,
) -> Result<
    (
        CheckResult,
        Exploration,
        Vec<super::explore::InvariantRefusalWitness>,
        super::compile::CompiledMachine,
        Option<Exploration>,
    ),
    String,
> {
    let new_machine = compile(new.clone()).map_err(|e| e.join("; "))?;
    let mut new_plan = verification_plan(new);
    if crate::smt::adapter_configured() {
        for problem in crate::smt::adapter_problems() {
            eprintln!("ik: feasibility adapter configured but unusable: {problem}; nothing is discharged for it");
        }
        crate::smt::merge_certificates(new, &new_machine, &mut new_plan);
    }
    let (new_run, invariant_refusals) = if capture_invariant_refusals {
        super::explore::explore_with_invariant_witnesses(&new_machine, corpus, bounds)
    } else {
        (explore(&new_machine, corpus, bounds), Vec::new())
    };
    let discharges = discharge_exploration(&new_plan, &new_run);
    let local = if let Some(old) = prior {
        change_local_guards(old, new, corpus, bounds)?
    } else {
        Vec::new()
    };
    let report = report_from_evidence(&new_plan, &discharges, &local);
    let diff = match prior {
        Some(old) => plan_diff(&verification_plan(old), &new_plan),
        None => Default::default(),
    };
    let mut judge_report = report.clone();
    judge_report.obligations.uncovered_ids.retain(|id| {
        new_plan
            .obligations
            .iter()
            .any(|o| o.obligation_id == *id && standard_lane(&o.kind))
    });
    judge_report.obligations.uncovered = judge_report.obligations.uncovered_ids.len();
    judge_report.obligations.satisfied = judge_report.obligations.total
        - judge_report.obligations.uncovered
        - judge_report.obligations.falsified
        - judge_report.obligations.incomplete;
    let mut judgment = judge_acceptance(&new_plan, &judge_report, &diff, baseline);
    if !invariant_refusals.is_empty() {
        judgment.verdict = "refused".into();
        judgment.status = "witnessed_violation".into();
        for witness in &invariant_refusals {
            judgment.reasons.push(format!(
                "model transition violates invariant {}; runtime blocked the proposal",
                witness.diagnostic.rule.as_deref().unwrap_or("unknown")
            ));
        }
    }
    let release = release_evidence(&ReleaseInputs {
        plan: &new_plan,
        run: &new_run,
        discharges: &discharges,
        report: &report,
        judgment: &judgment,
        baseline,
        corpus,
        bounds,
    });
    let old_run = prior
        .map(|old| compile(old.clone()).map(|m| explore(&m, corpus, bounds)))
        .transpose()
        .map_err(|e| e.join("; "))?;
    let machine_diff = match prior {
        Some(old) => Some(diff_machines(old, new, corpus, bounds)?),
        None => None,
    };
    let mut text = render_packet(RenderArgs {
        new,
        new_plan: &new_plan,
        prior,
        diff: &diff,
        new_run: &new_run,
        old_run: old_run.as_ref(),
        local: &local,
        judgment: &judgment,
        machine_diff: machine_diff.as_ref(),
        witness: machine_diff
            .as_ref()
            .and_then(|d| d.executable.witness.as_ref()),
    });
    text.push_str(&release.render());
    Ok((
        CheckResult {
            verdict: if judgment.verdict == "accept" {
                "ACCEPT".into()
            } else {
                "REFUSED".into()
            },
            text,
            report,
            judgment,
            release,
            definition: machine_diff.as_ref().map(definition_delta),
        },
        new_run,
        invariant_refusals,
        new_machine,
        old_run,
    ))
}

struct RenderArgs<'a> {
    new: &'a Definition,
    new_plan: &'a VerificationPlanV1,
    prior: Option<&'a Definition>,
    diff: &'a super::plan::VerificationPlanDiff,
    new_run: &'a Exploration,
    old_run: Option<&'a Exploration>,
    local: &'a [LocalAdequacy],
    judgment: &'a AcceptJudgment,
    machine_diff: Option<&'a super::diff::MachineDiff>,
    witness: Option<&'a Vec<super::explore::WitnessStep>>,
}

fn render_packet(args: RenderArgs<'_>) -> String {
    let RenderArgs {
        new,
        new_plan,
        prior,
        diff,
        new_run,
        old_run,
        local,
        judgment,
        machine_diff,
        witness,
    } = args;
    let mut out = String::new();
    let debt: Vec<&VerificationObligationV1> = new_plan
        .obligations
        .iter()
        .filter(|o| standard_lane(&o.kind))
        .filter(|o| {
            matches!(
                diff.status(&o.obligation_id),
                Some("ADDED") | Some("MODIFIED")
            )
        })
        .collect();
    if !debt.is_empty() {
        out.push_str("NEW SEMANTIC DEBT\n\n");
        for obligation in debt {
            out.push_str(&format_debt_item(obligation, new, new_run, local, witness));
            out.push('\n');
        }
    }
    let lost = lost_behavior(old_run, new_run);
    if !lost.is_empty() {
        out.push_str("DOWNSTREAM BEHAVIOR LOST\n");
        for item in &lost {
            out.push_str(&format!("  {item:<22} UNREACHABLE\n"));
        }
        out.push('\n');
    }
    if let Some(why) = first_match_why(new, prior) {
        out.push_str("WHY\n");
        out.push_str(&why);
        out.push('\n');
    }
    if !lost.is_empty() {
        out.push_str("NEXT\n");
        out.push_str("  provide a domain where the lost outcomes can execute,\n");
        out.push_str("  or remove the now-dead branches.\n\n");
    }
    if let Some(machine_diff) = machine_diff {
        out.push_str(&change_impact(machine_diff));
    }
    out.push_str("VERDICT\n");
    out.push_str(&format!(
        "  {}\n",
        if judgment.verdict == "accept" {
            "ACCEPT"
        } else {
            "REFUSED"
        }
    ));
    out.push_str("ASSURANCE\n");
    out.push_str("  class bounded_checked\n");
    out.push_str(&format!(
        "  exploration {}\n",
        if new_run.complete {
            "complete_within_corpus"
        } else {
            "incomplete"
        }
    ));
    out.push_str("  ACCEPT is corpus-relative, not a proof\n");
    if judgment.verdict != "accept" && !judgment.reasons.is_empty() {
        out.push_str("\nREASONS\n");
        for reason in &judgment.reasons {
            out.push_str(&format!("  - {reason}\n"));
        }
    }
    push_impossibilities(&mut out, &new_plan.impossibilities);
    push_uncovered_next_step(&mut out, &judgment.verdict, &judgment.reasons);
    out
}

fn named_delta(kind: &str, names: &[String]) -> String {
    match names {
        [] => "none".into(),
        [one] => format!("{kind} {one}"),
        many => format!("{kind}s {}", many.join(", ")),
    }
}

fn added_delta(diff: &super::diff::MachineDiff) -> String {
    let commands = named_delta("command", &diff.structural.commands_added);
    let states = named_delta("state", &diff.structural.states_added);
    if commands == "none" && states == "none" {
        return "none".into();
    }
    if states == "none" {
        return commands;
    }
    if commands == "none" {
        return states;
    }
    format!("{commands}; {states}")
}

fn removed_delta(diff: &super::diff::MachineDiff) -> String {
    let commands = named_delta("command", &diff.structural.commands_removed);
    let states = named_delta("state", &diff.structural.states_removed);
    if commands == "none" && states == "none" {
        return "none".into();
    }
    if states == "none" {
        return commands;
    }
    if commands == "none" {
        return states;
    }
    format!("{commands}; {states}")
}

fn definition_delta(diff: &super::diff::MachineDiff) -> DefinitionDelta {
    DefinitionDelta {
        schema: "ik.definition.delta.v1".into(),
        prior_version: diff.old_machine.version,
        new_version: diff.new_machine.version,
        commands_added: diff.structural.commands_added.clone(),
        commands_removed: diff.structural.commands_removed.clone(),
        states_added: diff.structural.states_added.clone(),
        states_removed: diff.structural.states_removed.clone(),
        statement: format!(
            "new in version {}: {}; removed: {}",
            diff.new_machine.version,
            added_delta(diff),
            removed_delta(diff)
        ),
    }
}

fn change_impact(diff: &super::diff::MachineDiff) -> String {
    let mut out = String::from("CHANGE IMPACT\n");
    out.push_str(&format!(
        "  new machine version {} (prior version {})\n",
        diff.new_machine.version, diff.old_machine.version
    ));
    if diff.new_machine.version != diff.old_machine.version {
        out.push_str("  existing instances keep the version they were created under\n");
    }
    push_name_list(&mut out, "commands added", &diff.structural.commands_added);
    push_name_list(
        &mut out,
        "commands removed",
        &diff.structural.commands_removed,
    );
    push_name_list(&mut out, "states added", &diff.structural.states_added);
    push_name_list(&mut out, "states removed", &diff.structural.states_removed);
    if diff.new_machine.version != diff.old_machine.version {
        out.push_str(&format!(
            "  existing instances stay on their version; moving one to version {} in place is not supported\n",
            diff.new_machine.version
        ));
    }
    out.push('\n');
    out
}

fn push_name_list(out: &mut String, label: &str, names: &[String]) {
    out.push_str("  ");
    out.push_str(label);
    out.push_str(": ");
    if names.is_empty() {
        out.push_str("none\n");
        return;
    }
    for (index, name) in names.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(name);
    }
    out.push('\n');
}

fn push_impossibilities(out: &mut String, rows: &[super::plan::ImpossibilityRecordV1]) {
    if rows.is_empty() {
        return;
    }
    out.push_str("\nIMPOSSIBLE SUCCESS\n");
    for row in rows {
        out.push_str(&format!("  {}\n", row.obligation_id));
        out.push_str(&format!("    class {}\n", row.class));
        out.push_str(&format!(
            "    discharged {}\n",
            if row.discharged {
                "true"
            } else {
                "false (obligation retained)"
            }
        ));
        out.push_str(&format!("    reason {}\n", row.reason));
        out.push_str(&format!("    proving_source {}\n", row.proving_source));
        push_optional(out, "basis", &row.basis);
        push_optional(out, "scope", &row.scope);
        push_optional(out, "trust", &row.trust);
        push_optional(out, "certificate", &row.certificate_digest);
        if !row.solvers.is_empty() {
            out.push_str(&format!("    solvers {}\n", row.solvers.join(", ")));
        }
    }
}

fn push_optional(out: &mut String, label: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    out.push_str(&format!("    {label} {value}\n"));
}

fn push_uncovered_next_step(out: &mut String, verdict: &str, reasons: &[String]) {
    let uncovered = reasons
        .iter()
        .any(|r| r == "uncovered newly introduced mandatory obligation");
    if verdict == "accept" || !uncovered {
        return;
    }
    out.push_str("\nNEXT\n");
    out.push_str("  obligations are uncovered: no case has witnessed them.\n");
    out.push_str("  write cases that reach each guarded outcome (start with\n");
    out.push_str("  `ik corpus init <machine> --out <cases.json>`), then run\n");
    out.push_str("  `ik check <machine> --corpus <cases.json>`.\n");
}

fn format_debt_item(
    obligation: &VerificationObligationV1,
    def: &Definition,
    run: &Exploration,
    local: &[LocalAdequacy],
    fallback: Option<&Vec<super::explore::WitnessStep>>,
) -> String {
    let mut lines = vec![format!("{}", obligation.obligation_id)];
    if obligation.kind == "OBL-OUT-001" {
        push_outcome_debt(&mut lines, obligation, def, run, local, fallback);
    } else {
        push_reachability_debt(&mut lines, obligation, run);
    }
    lines.join("\n") + "\n"
}

fn push_outcome_debt(
    lines: &mut Vec<String>,
    obligation: &VerificationObligationV1,
    def: &Definition,
    run: &Exploration,
    local: &[LocalAdequacy],
    fallback: Option<&Vec<super::explore::WitnessStep>>,
) {
    let (Some(command), Some(index)) = (
        obligation.coordinate.command.as_deref(),
        obligation.coordinate.outcome_index,
    ) else {
        return;
    };
    if let Some(outcome) = def
        .commands
        .get(command)
        .and_then(|c| c.outcomes.as_ref())
        .and_then(|o| o.get(index))
    {
        let when = outcome.when.as_deref().unwrap_or("unconditional");
        lines.push(format!("  {when} → {}", outcome.to));
    }
    let executed = run
        .coverage
        .executed_outcomes
        .iter()
        .any(|o| o.command == command && o.outcome_index == index);
    let witness = run
        .witnesses
        .by_outcome
        .get(&format!("{command}.{index}"))
        .or_else(|| {
            fallback.filter(|steps| {
                steps
                    .iter()
                    .any(|s| s.command == command && s.outcome_index == Some(index))
            })
        });
    if executed || witness.is_some() {
        let path = witness
            .map(|steps| {
                steps
                    .iter()
                    .map(|s| s.command.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_else(|| "explore".into());
        lines.push(format!("  executable witness     FOUND   {path}"));
    } else {
        lines.push("  executable witness     MISSING".into());
    }
    if let Some(row) = local.iter().find(|row| {
        row.coordinate.command.as_deref() == Some(command)
            && row.coordinate.outcome_index == Some(index)
    }) {
        let weak = row.neighbors.iter().any(|n| n.status == "survived");
        lines.push(format!(
            "  semantic adequacy      {}",
            if weak { "WEAK" } else { "KILLED" }
        ));
    }
}

fn push_reachability_debt(
    lines: &mut Vec<String>,
    obligation: &VerificationObligationV1,
    run: &Exploration,
) {
    let reached = match obligation.kind.as_str() {
        "OBL-STATE-001" | "OBL-TERM-001" => {
            obligation.coordinate.state.as_deref().is_some_and(|s| {
                run.coverage
                    .executably_reached_states
                    .iter()
                    .any(|r| r == s)
            })
        }
        "OBL-CMD-001" => obligation.coordinate.command.as_deref().is_some_and(|c| {
            obligation
                .coordinate
                .source
                .as_deref()
                .is_some_and(|source| {
                    run.coverage
                        .executed_command_sources
                        .iter()
                        .any(|row| row.command == c && row.source == source)
                })
        }),
        _ => false,
    };
    lines.push(format!(
        "  executable witness     {}",
        if reached { "FOUND" } else { "MISSING" }
    ));
}

fn lost_behavior(old: Option<&Exploration>, new: &Exploration) -> Vec<String> {
    let Some(old) = old else {
        return Vec::new();
    };
    let mut lost = Vec::new();
    for state in &old.coverage.executably_reached_states {
        if !new
            .coverage
            .executably_reached_states
            .iter()
            .any(|s| s == state)
        {
            lost.push(state.clone());
        }
    }
    for row in &old.coverage.executed_command_sources {
        if !new
            .coverage
            .executed_command_sources
            .iter()
            .any(|other| other == row)
        {
            lost.push(format!("{}/from/{}", row.command, row.source));
        }
    }
    lost.sort();
    lost.dedup();
    lost
}

fn first_match_why(new: &Definition, prior: Option<&Definition>) -> Option<String> {
    let old = prior?;
    for (name, cmd) in &new.commands {
        let Some(new_outcomes) = cmd.outcomes.as_ref() else {
            continue;
        };
        if new_outcomes.len() < 2 {
            continue;
        }
        let old_len = old
            .commands
            .get(name)
            .and_then(|c| c.outcomes.as_ref())
            .map(|o| o.len())
            .unwrap_or(0);
        if new_outcomes.len() <= old_len
            && new_outcomes[0].when
                == old
                    .commands
                    .get(name)
                    .and_then(|c| c.outcomes.as_ref())
                    .and_then(|o| o.first())
                    .and_then(|o| o.when.clone())
        {
            continue;
        }
        if let Some(when) = &new_outcomes[0].when {
            return Some(format!(
                "  {name}[0] is first-match and fires when {when},\n  preventing later {name} outcomes from executing.\n"
            ));
        }
    }
    None
}
