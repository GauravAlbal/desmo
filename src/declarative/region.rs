//! Schema-2 authoring frontend. Only the emitted schema-1 Definition executes.
use super::compile::CompiledMachine;
use super::chart::{
    check_consistent, hierarchical_chart, Chart, ChartEdge, ChartState, EdgeProvenance, GroupChart,
    JointChart, RegionChart,
};
use super::def::{
    self, AuthoredCoordinate, CommandDef, Definition, EffectDef, FactDef, InvariantDef, OriginMap,
    OutcomeDef, StateDef, StaticErrors,
};
use super::explore::{Bounds, CaseCorpus, InvariantRefusalWitness, WitnessStep};
use super::expr::{self, Expr, Operand};
use crate::canonical::digest_json;
use crate::diagnostic::Diagnostic;
use crate::effect::EffectStatus;
use crate::machine::{Decision, Machine};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const PRODUCT_BOUND: usize = 64;
const RESERVED_PREFIX: &str = "region_";

/// Resource limits for eager region lowering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionLimits {
    /// Maximum number of Cartesian-product states to materialize.
    pub region_product_bound: usize,
}

impl Default for RegionLimits {
    fn default() -> Self {
        Self {
            region_product_bound: PRODUCT_BOUND,
        }
    }
}

#[derive(Clone, Debug)]
struct LoweredEdge {
    from: String,
    to: String,
}

#[derive(Clone, Debug)]
struct BlockedOutcome {
    from: String,
    to: String,
    reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutcomeDisposition {
    Emitted,
    PrunedFalseGuard,
}

#[derive(Clone, Debug)]
struct OriginCopy {
    command: String,
    from: String,
    to: String,
    authored: AuthoredCoordinate,
}

#[derive(Clone, Debug, Default)]
struct OriginCollector {
    authored: Vec<AuthoredCoordinate>,
    copies: Vec<OriginCopy>,
}

impl OriginCollector {
    fn from_source(source: &RegionDefinition) -> Self {
        Self {
            authored: collect_authored(source),
            copies: Vec::new(),
        }
    }

    fn note(&mut self, command: &str, from: String, to: String, authored: AuthoredCoordinate) {
        self.copies.push(OriginCopy {
            command: command.to_string(),
            from,
            to,
            authored,
        });
    }

    fn finalize(self, flat: &mut Definition, map: &RegionMap, chart: Chart) {
        let mut generated: BTreeMap<String, Vec<AuthoredCoordinate>> = BTreeMap::new();
        bind_product_origins(&mut generated, flat, map, &self.authored);
        bind_outcome_origins(&mut generated, flat, &self.copies);
        flat.origin = OriginMap {
            authored: self.authored,
            generated,
            products: map.states.clone(),
        };
        flat.chart = chart;
    }
}

fn collect_authored(source: &RegionDefinition) -> Vec<AuthoredCoordinate> {
    let mut out = Vec::new();
    for (region_name, region) in &source.regions {
        for (state_name, state) in &region.states {
            out.push(AuthoredCoordinate::state(region_name, state_name));
            if state.terminal {
                out.push(AuthoredCoordinate::terminal(region_name, state_name));
            }
        }
        for (cmd_name, cmd) in &region.commands {
            push_command_authored(&mut out, cmd_name, "child", cmd);
        }
        if let Some(parent) = &region.parent {
            for (cmd_name, cmd) in &parent.commands {
                push_command_authored(&mut out, cmd_name, "parent", cmd);
            }
        }
    }
    for (cmd_name, cmd) in &source.commands {
        let count = match &cmd.outcomes {
            Some(outcomes) => outcomes.len(),
            None if cmd.to.is_some() => 1,
            None => 0,
        };
        for ordinal in 0..count {
            out.push(AuthoredCoordinate::outcome(cmd_name, "joint", ordinal));
        }
    }
    out.sort();
    out.dedup();
    out
}

fn chart_from_source(source: &RegionDefinition) -> Chart {
    let mut regions = BTreeMap::new();
    for (region_name, region) in &source.regions {
        regions.insert(region_name.clone(), region_chart(region_name, region));
    }
    let mut joints = Vec::new();
    for (name, cmd) in &source.commands {
        joints.push(joint_chart(name, cmd));
    }
    hierarchical_chart(
        source.machine_id.clone(),
        source.machine_version,
        source.facts.clone(),
        regions,
        joints,
    )
}

fn region_chart(region_name: &str, region: &Region) -> RegionChart {
    let mut states = BTreeMap::new();
    for (name, state) in &region.states {
        states.insert(
            name.clone(),
            ChartState {
                initial: state.initial,
                terminal: state.terminal,
            },
        );
    }
    let mut edges = Vec::new();
    for (cmd_name, cmd) in &region.commands {
        edges.extend(chart_edges_from_cmd(
            cmd_name,
            cmd,
            EdgeProvenance::Child {
                region: region_name.to_string(),
            },
        ));
    }
    let mut groups = BTreeMap::new();
    if let Some(parent) = &region.parent {
        let mut parent_edges = Vec::new();
        for (cmd_name, cmd) in &parent.commands {
            parent_edges.extend(chart_edges_from_cmd(
                cmd_name,
                cmd,
                EdgeProvenance::Parent {
                    region: region_name.to_string(),
                    group: parent.name.clone(),
                },
            ));
        }
        groups.insert(
            parent.name.clone(),
            GroupChart {
                parent: parent.name.clone(),
                children: parent.children.clone(),
                parent_edges,
            },
        );
    }
    RegionChart {
        states,
        groups,
        edges,
    }
}

fn chart_edges_from_cmd(
    command: &str,
    cmd: &CommandDef,
    provenance: EdgeProvenance,
) -> Vec<ChartEdge> {
    let dests: Vec<(Option<String>, String)> = if let Some(outcomes) = &cmd.outcomes {
        outcomes
            .iter()
            .map(|o| (o.when.clone(), o.to.clone()))
            .collect()
    } else if let Some(to) = &cmd.to {
        vec![(cmd.when.clone(), to.clone())]
    } else {
        Vec::new()
    };
    let froms = if let EdgeProvenance::Parent { group, .. } = &provenance {
        vec![group.clone()]
    } else {
        cmd.from.clone()
    };
    let mut out = Vec::new();
    for from in froms {
        for (rule, to) in &dests {
            out.push(ChartEdge {
                command: command.to_string(),
                from: from.clone(),
                to: to.clone(),
                rule: rule.clone(),
                provenance: provenance.clone(),
            });
        }
    }
    out
}

fn joint_chart(name: &str, cmd: &JointCommand) -> JointChart {
    let mut legs = Vec::new();
    if let Some(to_map) = &cmd.to {
        for (region, to_leaf) in to_map {
            let froms = cmd.from.get(region).cloned().unwrap_or_default();
            for from in froms {
                legs.push((region.clone(), from, to_leaf.clone()));
            }
        }
    }
    JointChart {
        command: name.to_string(),
        rule: cmd.when.clone(),
        legs,
    }
}

fn push_command_authored(
    out: &mut Vec<AuthoredCoordinate>,
    name: &str,
    layer: &str,
    cmd: &CommandDef,
) {
    let count = cmd.outcomes.as_ref().map(Vec::len).unwrap_or(1);
    for ordinal in 0..count {
        out.push(AuthoredCoordinate::outcome(name, layer, ordinal));
    }
}

fn bind_product_origins(
    generated: &mut BTreeMap<String, Vec<AuthoredCoordinate>>,
    flat: &Definition,
    map: &RegionMap,
    authored: &[AuthoredCoordinate],
) {
    for (label, product) in &map.states {
        let mut states = Vec::new();
        let mut terminals = Vec::new();
        for (region, leaf) in product {
            states.push(AuthoredCoordinate::state(region, leaf));
            let terminal = AuthoredCoordinate::terminal(region, leaf);
            if authored.contains(&terminal) {
                terminals.push(terminal);
            }
        }
        let mut product_coords = states.clone();
        product_coords.extend(terminals.clone());
        generated.insert(format!("product/{label}"), product_coords);
        if flat.states.get(label).is_some_and(|state| !state.initial) {
            generated.insert(format!("state/{label}/executable"), states);
        }
        if flat.states.get(label).is_some_and(|state| state.terminal) {
            generated.insert(format!("terminal/{label}/reachable"), terminals);
        }
    }
}

fn bind_outcome_origins(
    generated: &mut BTreeMap<String, Vec<AuthoredCoordinate>>,
    flat: &Definition,
    copies: &[OriginCopy],
) {
    for (command, cmd) in &flat.commands {
        let mut remaining: Vec<&OriginCopy> = copies
            .iter()
            .filter(|copy| copy.command == *command)
            .collect();
        let Some(outcomes) = &cmd.outcomes else {
            continue;
        };
        for (index, outcome) in outcomes.iter().enumerate() {
            let from = outcome_source(outcome);
            let Some(pos) = remaining
                .iter()
                .position(|copy| from.as_ref() == Some(&copy.from) && copy.to == outcome.to)
            else {
                continue;
            };
            let copy = remaining.remove(pos);
            generated.insert(
                format!("outcome/{command}/{index}/executable"),
                vec![copy.authored.clone()],
            );
        }
    }
}

#[derive(Clone, Debug)]
pub struct SourceDefinition {
    pub definition: Definition,
    pub regions: Option<RegionMap>,
}

/// Derived metadata, never a second machine or a persisted identity.
#[derive(Clone, Debug)]
pub struct RegionMap {
    pub authoring_hash: String,
    /// Static, corpus-independent facts about lowering and product pruning.
    pub diagnostics: Vec<String>,
    states: BTreeMap<String, BTreeMap<String, String>>,
    spec_hash: String,
    parents: BTreeMap<String, (String, Vec<String>)>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegionDefinition {
    schema_version: u64,
    machine_id: String,
    machine_version: u64,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    facts: BTreeMap<String, FactDef>,
    regions: BTreeMap<String, Region>,
    #[serde(default)]
    commands: BTreeMap<String, JointCommand>,
    #[serde(default)]
    invariants: BTreeMap<String, InvariantDef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Region {
    states: BTreeMap<String, StateDef>,
    #[serde(default)]
    commands: BTreeMap<String, CommandDef>,
    #[serde(default)]
    parent: Option<Parent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Parent {
    name: String,
    children: Vec<String>,
    #[serde(default)]
    commands: BTreeMap<String, CommandDef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JointCommand {
    from: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    to: Option<BTreeMap<String, String>>,
    #[serde(default)]
    when: Option<String>,
    #[serde(default)]
    outcomes: Option<Vec<JointOutcome>>,
    #[serde(default)]
    args: Option<BTreeMap<String, String>>,
    #[serde(default)]
    then: Option<BTreeMap<String, String>>,
    #[serde(default)]
    effect: Option<EffectDef>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct JointOutcome {
    #[serde(default)]
    when: Option<String>,
    to: BTreeMap<String, String>,
    #[serde(default)]
    then: Option<BTreeMap<String, String>>,
}

/// All text loaders dispatch here; schema 1 bypasses region lowering entirely.
pub fn parse_source(text: &str) -> Result<SourceDefinition, StaticErrors> {
    parse_source_with_limits(text, RegionLimits::default())
}

/// Parse with a caller-selected eager product resource bound.
pub fn parse_source_with_limits(
    text: &str,
    limits: RegionLimits,
) -> Result<SourceDefinition, StaticErrors> {
    let value: toml::Value =
        toml::from_str(text).map_err(|e| vec![format!("definition parse error: {e}")])?;
    let version = value
        .get("schema_version")
        .and_then(toml::Value::as_integer);
    let source = if version == Some(2) {
        let authored: RegionDefinition =
            toml::from_str(text).map_err(|e| vec![format!("region parse error: {e}")])?;
        lower(authored, digest_json(&value), limits)?
    } else {
        SourceDefinition {
            definition: {
                let mut definition: Definition = toml::from_str(text)
                    .map_err(|e| vec![format!("definition parse error: {e}")])?;
                definition.chart = Chart::from_flat(&definition);
                check_consistent(&definition.chart, &definition)
                    .map_err(|err| vec![err.to_string()])?;
                definition
            },
            regions: None,
        }
    };
    let errors = def::check(&source.definition);
    if errors.is_empty() {
        Ok(source)
    } else {
        Err(errors)
    }
}

fn refuse(message: impl Into<String>) -> StaticErrors {
    vec![format!("region lowering refused: {}", message.into())]
}

fn check_name(name: &str, kind: &str) -> Result<(), StaticErrors> {
    let mut chars = name.chars();
    if !chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        || name.contains("__")
    {
        return Err(refuse(format!(
            "{kind} `{name}` must be an ASCII identifier without `__`"
        )));
    }
    Ok(())
}

fn lower(
    source: RegionDefinition,
    authoring_hash: String,
    limits: RegionLimits,
) -> Result<SourceDefinition, StaticErrors> {
    debug_assert_eq!(source.schema_version, 2);
    check_product_bound(&source.regions, limits.region_product_bound)?;
    let mut facts = source.facts.clone();
    if facts
        .keys()
        .chain(source.invariants.keys())
        .any(|n| n.starts_with(RESERVED_PREFIX))
    {
        return Err(refuse(
            "fact and invariant names beginning `region_` are reserved",
        ));
    }
    let parents = lower_region_facts(&source.regions, &mut facts)?;
    for invariant in source.invariants.values() {
        check_expression(&invariant.assert, &source.regions)?;
        if let Some(when) = &invariant.when {
            check_expression(when, &source.regions)?;
        }
    }
    let products = expand_products(&source.regions);
    let mut flat = Definition {
        schema_version: 1,
        machine_id: source.machine_id.clone(),
        machine_version: source.machine_version,
        name: source.name.clone(),
        facts,
        states: BTreeMap::new(),
        commands: BTreeMap::new(),
        invariants: source.invariants.clone(),
        origin: OriginMap::default(),
        chart: Chart::default(),
    };
    let mut map = RegionMap {
        authoring_hash,
        diagnostics: Vec::new(),
        states: BTreeMap::new(),
        parents,
        spec_hash: String::new(),
    };
    let mut origin = OriginCollector::from_source(&source);
    lower_product_states(&source.regions, &products, &mut flat, &mut map)?;
    let mut edges = Vec::new();
    let mut blocked = Vec::new();
    lower_local_commands(
        &source,
        &products,
        &mut flat,
        &mut map.diagnostics,
        &mut edges,
        &mut origin,
    )?;
    for (name, cmd) in &source.commands {
        lower_joint(
            &source,
            &mut flat,
            &products,
            name,
            cmd,
            &mut map.diagnostics,
            &mut blocked,
            &mut edges,
            &mut origin,
        )?;
    }
    prune_unreachable_products(&mut flat, &mut map, &edges, &blocked);
    origin.finalize(&mut flat, &map, chart_from_source(&source));
    check_consistent(&flat.chart, &flat).map_err(|err| vec![err.to_string()])?;
    validate_initial(&flat)?;
    map.spec_hash = digest_json(&flat);
    Ok(SourceDefinition {
        definition: flat,
        regions: Some(map),
    })
}

fn check_product_bound(
    regions: &BTreeMap<String, Region>,
    bound: usize,
) -> Result<(), StaticErrors> {
    if regions.is_empty() {
        return Err(refuse("at least one region is required"));
    }
    let sizes = regions
        .iter()
        .map(|(n, r)| format!("{n}={}", r.states.len()))
        .collect::<Vec<_>>()
        .join(", ");
    let mut size = 1usize;
    for region in regions.values() {
        size = size.checked_mul(region.states.len()).ok_or_else(|| {
            refuse(format!(
                "product size overflow ({sizes}); configured bound {bound}"
            ))
        })?;
        if size > bound {
            return Err(refuse(format!(
                "product exceeds {bound} states ({sizes}); raise region_product_bound to allow this product"
            )));
        }
    }
    Ok(())
}

fn lower_region_facts(
    regions: &BTreeMap<String, Region>,
    facts: &mut BTreeMap<String, FactDef>,
) -> Result<BTreeMap<String, (String, Vec<String>)>, StaticErrors> {
    let mut parents = BTreeMap::new();
    for (name, region) in regions {
        let initial = check_region_initial(name, region)?;
        facts.insert(
            format!("region_{name}"),
            FactDef {
                ty: "string".into(),
                policy: "replace".into(),
                initial: Some(Value::String(initial.clone())),
            },
        );
        if let Some(parent) = &region.parent {
            check_region_parent(name, region, parent)?;
            parents.insert(name.clone(), (parent.name.clone(), parent.children.clone()));
        }
    }
    Ok(parents)
}

fn check_region_initial<'a>(name: &str, region: &'a Region) -> Result<&'a String, StaticErrors> {
    check_name(name, "region")?;
    for state in region.states.keys() {
        check_name(state, "state")?;
    }
    let initials = region
        .states
        .iter()
        .filter(|(_, s)| s.initial)
        .collect::<Vec<_>>();
    if initials.len() != 1 {
        return Err(refuse(format!(
            "region `{name}` requires exactly one initial state (found {})",
            initials.len()
        )));
    }
    if initials[0].1.terminal {
        return Err(refuse(format!(
            "region `{name}` initial state must not be terminal"
        )));
    }
    Ok(initials[0].0)
}

fn check_region_parent(name: &str, region: &Region, parent: &Parent) -> Result<(), StaticErrors> {
    check_name(&parent.name, "parent")?;
    if region.states.contains_key(&parent.name) || parent.children.is_empty() {
        return Err(refuse(format!(
            "region `{name}` parent must be distinct and have children"
        )));
    }
    let unique: BTreeSet<_> = parent.children.iter().collect();
    if unique.len() != parent.children.len()
        || parent
            .children
            .iter()
            .any(|s| !region.states.contains_key(s))
    {
        return Err(refuse(format!(
            "region `{name}` parent has duplicate or unknown children"
        )));
    }
    Ok(())
}

fn expand_products(regions: &BTreeMap<String, Region>) -> Vec<BTreeMap<String, String>> {
    let mut products = vec![BTreeMap::new()];
    for (name, region) in regions {
        let mut next = Vec::with_capacity(products.len() * region.states.len());
        for product in &products {
            for state in region.states.keys() {
                let mut entry = product.clone();
                entry.insert(name.clone(), state.clone());
                next.push(entry);
            }
        }
        products = next;
    }
    products
}

fn lower_product_states(
    regions: &BTreeMap<String, Region>,
    products: &[BTreeMap<String, String>],
    flat: &mut Definition,
    map: &mut RegionMap,
) -> Result<(), StaticErrors> {
    for product in products {
        let label = flat_name(product);
        flat.states.insert(
            label.clone(),
            StateDef {
                initial: product.iter().all(|(r, s)| regions[r].states[s].initial),
                terminal: product.iter().all(|(r, s)| regions[r].states[s].terminal),
            },
        );
        // Hard agreement check: generated facts can never silently disagree with projection.
        flat.invariants.insert(
            format!("region_projection_{label}"),
            InvariantDef {
                when: Some(format!("state == {}", quoted(&label)?)),
                assert: product_agreement(product)?,
            },
        );
        map.states.insert(label, product.clone());
    }
    Ok(())
}

fn lower_local_commands(
    source: &RegionDefinition,
    products: &[BTreeMap<String, String>],
    flat: &mut Definition,
    diagnostics: &mut Vec<String>,
    edges: &mut Vec<LoweredEdge>,
    origin: &mut OriginCollector,
) -> Result<(), StaticErrors> {
    let mut owners = BTreeSet::new();
    for (region_name, region) in &source.regions {
        let names = region
            .commands
            .keys()
            .chain(region.parent.iter().flat_map(|p| p.commands.keys()))
            .cloned()
            .collect::<BTreeSet<_>>();
        for name in names {
            if !owners.insert(name.clone()) || source.commands.contains_key(&name) {
                return Err(refuse(format!("command `{name}` has multiple region/joint owners; use one explicit joint command")));
            }
            let lowered = lower_local_command(
                source,
                flat,
                products,
                region_name,
                region,
                &name,
                diagnostics,
                edges,
                origin,
            )?;
            flat.commands.insert(name, lowered);
        }
    }
    Ok(())
}

fn lower_local_command(
    source: &RegionDefinition,
    flat: &Definition,
    products: &[BTreeMap<String, String>],
    region_name: &str,
    region: &Region,
    name: &str,
    diagnostics: &mut Vec<String>,
    edges: &mut Vec<LoweredEdge>,
    origin: &mut OriginCollector,
) -> Result<CommandDef, StaticErrors> {
    let child = region.commands.get(name);
    let parent = region.parent.as_ref().and_then(|p| p.commands.get(name));
    for cmd in [child, parent].into_iter().flatten() {
        check_authored_command(cmd, &source.regions)?;
    }
    if let (Some(c), Some(p)) = (child, parent) {
        if digest_json(&c.args) != digest_json(&p.args)
            || digest_json(&c.effect) != digest_json(&p.effect)
        {
            return Err(refuse(format!("command `{name}` child and parent must have exactly identical args and effect declarations")));
        }
    }
    if let Some(cmd) = child {
        validate_local(flat, region, name, cmd, false)?;
    }
    if let Some(cmd) = parent {
        validate_local(flat, region, name, cmd, true)?;
    }
    let child_outcomes = child.map(command_outcomes);
    let parent_outcomes = parent.map(command_outcomes);
    let template = declared_command(name, child, parent)?;
    let mut lowered = command_shell(template.args.clone(), template.effect.clone());
    lower_local_products(
        &mut lowered,
        products,
        region_name,
        region,
        name,
        child,
        child_outcomes.as_deref(),
        parent_outcomes.as_deref(),
        flat,
        diagnostics,
        edges,
        origin,
    )?;
    Ok(lowered)
}

fn lower_local_products(
    lowered: &mut CommandDef,
    products: &[BTreeMap<String, String>],
    region_name: &str,
    region: &Region,
    name: &str,
    child: Option<&CommandDef>,
    child_outcomes: Option<&[OutcomeDef]>,
    parent_outcomes: Option<&[OutcomeDef]>,
    flat: &Definition,
    diagnostics: &mut Vec<String>,
    edges: &mut Vec<LoweredEdge>,
    origin: &mut OriginCollector,
) -> Result<(), StaticErrors> {
    for product in products {
        let state = &product[region_name];
        let c = child
            .filter(|cmd| cmd.from.contains(state))
            .and(child_outcomes);
        let child_always_handles =
            c.is_some_and(|outcomes| outcomes.last().is_some_and(|o| o.when.is_none()));
        let p = parent_outcomes.filter(|_| {
            !child_always_handles
                && region
                    .parent
                    .as_ref()
                    .is_some_and(|parent| parent.children.contains(state))
        });
        if c.is_none() && p.is_none() {
            continue;
        }
        let label = flat_name(product);
        if flat.states[&label].terminal {
            continue;
        }
        lowered.from.push(label);
        let before = lowered.outcomes.as_ref().map_or(0, Vec::len);
        let guard_pruned = push_local_outcomes(
            lowered,
            product,
            region_name,
            name,
            c,
            p,
            diagnostics,
            edges,
            origin,
        )?;
        if lowered.outcomes.as_ref().map_or(0, Vec::len) == before && !guard_pruned {
            lowered.from.pop();
        }
    }
    Ok(())
}

fn push_local_outcomes(
    lowered: &mut CommandDef,
    product: &BTreeMap<String, String>,
    region_name: &str,
    command: &str,
    child: Option<&[OutcomeDef]>,
    parent: Option<&[OutcomeDef]>,
    diagnostics: &mut Vec<String>,
    edges: &mut Vec<LoweredEdge>,
    origin: &mut OriginCollector,
) -> Result<bool, StaticErrors> {
    let mut guard_pruned = false;
    guard_pruned |= push_layer_outcomes(
        lowered,
        product,
        region_name,
        command,
        "child",
        child,
        diagnostics,
        edges,
        origin,
    )?;
    guard_pruned |= push_layer_outcomes(
        lowered,
        product,
        region_name,
        command,
        "parent",
        parent,
        diagnostics,
        edges,
        origin,
    )?;
    Ok(guard_pruned)
}

fn push_layer_outcomes(
    lowered: &mut CommandDef,
    product: &BTreeMap<String, String>,
    region_name: &str,
    command: &str,
    layer: &str,
    outcomes: Option<&[OutcomeDef]>,
    diagnostics: &mut Vec<String>,
    edges: &mut Vec<LoweredEdge>,
    origin: &mut OriginCollector,
) -> Result<bool, StaticErrors> {
    let mut guard_pruned = false;
    let Some(outcomes) = outcomes else {
        return Ok(false);
    };
    for (ordinal, outcome) in outcomes.iter().enumerate() {
        let mut target = product.clone();
        target.insert(region_name.to_string(), outcome.to.clone());
        guard_pruned |= push_outcome(
            lowered,
            product,
            &target,
            command,
            outcome.when.as_deref(),
            outcome.then.as_ref(),
            diagnostics,
            edges,
            origin,
            AuthoredCoordinate::outcome(command, layer, ordinal),
        )? == OutcomeDisposition::PrunedFalseGuard;
    }
    Ok(guard_pruned)
}

fn declared_command<'a>(
    name: &str,
    child: Option<&'a CommandDef>,
    parent: Option<&'a CommandDef>,
) -> Result<&'a CommandDef, StaticErrors> {
    match (child, parent) {
        (Some(command), _) | (None, Some(command)) => Ok(command),
        (None, None) => Err(refuse(format!(
            "command `{name}` is not declared in the child or parent region"
        ))),
    }
}

fn check_authored_command(
    cmd: &CommandDef,
    regions: &BTreeMap<String, Region>,
) -> Result<(), StaticErrors> {
    if cmd.outcomes.is_some() && cmd.then.is_some() {
        return Err(refuse(
            "`then` belongs on each outcome, not beside `outcomes`",
        ));
    }
    if let Some(when) = &cmd.when {
        check_expression(when, regions)?;
    }
    check_assignments(cmd.then.as_ref())?;
    for outcome in cmd.outcomes.iter().flatten() {
        if let Some(when) = &outcome.when {
            check_expression(when, regions)?;
        }
        check_assignments(outcome.then.as_ref())?;
    }
    Ok(())
}

fn validate_local(
    flat: &Definition,
    region: &Region,
    name: &str,
    cmd: &CommandDef,
    is_parent: bool,
) -> Result<(), StaticErrors> {
    let mut cmd = cmd.clone();
    if is_parent {
        let parent = region.parent.as_ref().ok_or_else(|| {
            refuse(format!(
                "parent command `{name}` is missing its parent declaration"
            ))
        })?;
        if cmd.from != vec![parent.name.clone()] {
            return Err(refuse(format!(
                "parent command `{name}` must use from = [\"{}\"]",
                parent.name
            )));
        }
        cmd.from = parent.children.clone();
    }
    let local = Definition {
        schema_version: 1,
        machine_id: flat.machine_id.clone(),
        machine_version: flat.machine_version,
        name: flat.name.clone(),
        facts: flat.facts.clone(),
        states: region.states.clone(),
        commands: BTreeMap::from([(name.to_string(), cmd)]),
        invariants: BTreeMap::new(),
        origin: OriginMap::default(),
        chart: Chart::default(),
    };
    let errors = def::check(&local);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors
            .into_iter()
            .map(|e| format!("region command `{name}`: {e}"))
            .collect())
    }
}

fn command_outcomes(cmd: &CommandDef) -> Vec<OutcomeDef> {
    cmd.outcomes.clone().unwrap_or_else(|| {
        vec![OutcomeDef {
            when: cmd.when.clone(),
            to: cmd.to.clone().unwrap_or_default(),
            then: cmd.then.clone(),
        }]
    })
}

fn command_shell(args: Option<BTreeMap<String, String>>, effect: Option<EffectDef>) -> CommandDef {
    CommandDef {
        from: Vec::new(),
        to: None,
        when: None,
        outcomes: Some(Vec::new()),
        args,
        then: None,
        effect,
    }
}

fn flat_name(product: &BTreeMap<String, String>) -> String {
    product.values().cloned().collect::<Vec<_>>().join("__")
}
fn quoted(value: &str) -> Result<String, StaticErrors> {
    serde_json::to_string(value).map_err(|error| {
        refuse(format!(
            "cannot serialize `{value}` as a JSON string: {error}"
        ))
    })
}

fn product_agreement(product: &BTreeMap<String, String>) -> Result<String, StaticErrors> {
    let mut parts = Vec::with_capacity(product.len());
    for (region, state) in product {
        parts.push(format!("region_{region} == {}", quoted(state)?));
    }
    Ok(parts.join(" && "))
}

fn push_outcome(
    cmd: &mut CommandDef,
    source: &BTreeMap<String, String>,
    target: &BTreeMap<String, String>,
    command: &str,
    guard: Option<&str>,
    assignments: Option<&BTreeMap<String, String>>,
    diagnostics: &mut Vec<String>,
    edges: &mut Vec<LoweredEdge>,
    origin: &mut OriginCollector,
    authored: AuthoredCoordinate,
) -> Result<OutcomeDisposition, StaticErrors> {
    let known_facts = source
        .iter()
        .map(|(region, state)| (format!("region_{region}"), Value::String(state.clone())))
        .collect::<BTreeMap<_, _>>();
    if let Some(guard_text) = guard {
        if let Ok(expression) = expr::parse(guard_text) {
            if expr::eval_with_assignments(&expression, &known_facts) == Some(false) {
                diagnostics.push(format!(
                    "command `{command}` at product `{}` dropped outcome toward product `{}`: region guard `{guard_text}` is contradicted by the product assignment",
                    flat_name(source),
                    flat_name(target)
                ));
                return Ok(OutcomeDisposition::PrunedFalseGuard);
            }
        }
    }
    let selector = format!("state == {}", quoted(&flat_name(source))?);
    let agreement = product_agreement(source)?;
    let when = Some(match guard {
        Some(g) => format!("{selector} && ({agreement}) && ({g})"),
        None => format!("{selector} && ({agreement})"),
    });
    let mut then = assignments.cloned().unwrap_or_default();
    for (region, state) in target {
        then.insert(format!("region_{region}"), quoted(state)?);
    }
    cmd.outcomes.get_or_insert_with(Vec::new).push(OutcomeDef {
        when,
        to: flat_name(target),
        then: Some(then),
    });
    let from = flat_name(source);
    let to = flat_name(target);
    origin.note(command, from.clone(), to.clone(), authored);
    edges.push(LoweredEdge { from, to });
    Ok(OutcomeDisposition::Emitted)
}

fn check_assignments(assignments: Option<&BTreeMap<String, String>>) -> Result<(), StaticErrors> {
    if assignments.is_some_and(|a| a.keys().any(|n| n.starts_with(RESERVED_PREFIX))) {
        return Err(refuse("authors cannot assign generated `region_` facts"));
    }
    Ok(())
}

fn check_expression(text: &str, regions: &BTreeMap<String, Region>) -> Result<(), StaticErrors> {
    let expression = expr::parse(text).map_err(|e| refuse(format!("expression `{text}`: {e}")))?;
    fn operand(op: &Operand) -> bool {
        match op {
            Operand::State => true,
            Operand::Arith { base, .. } | Operand::Mul { base, .. } | Operand::Div { base, .. } => {
                operand(base)
            }
            Operand::BinArith { left, right, .. } => operand(left) || operand(right),
            _ => false,
        }
    }
    fn walk(expression: &Expr, regions: &BTreeMap<String, Region>) -> Result<(), StaticErrors> {
        match expression {
            Expr::And(parts) | Expr::Or(parts) => {
                for part in parts {
                    walk(part, regions)?;
                }
            }
            Expr::Cmp(c) => {
                if operand(&c.left) || operand(&c.right) {
                    return Err(refuse(
                        "schema 2 expressions use `region_<name>`, not ambiguous flat `state`",
                    ));
                }
                check_region_literal(&c.left, &c.right, regions)?;
                check_region_literal(&c.right, &c.left, regions)?;
            }
            Expr::TextIn(pred) => {
                if operand(&pred.value) {
                    return Err(refuse(
                        "schema 2 expressions use `region_<name>`, not ambiguous flat `state`",
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    }
    walk(&expression, regions)
}

fn check_region_literal(
    left: &Operand,
    right: &Operand,
    regions: &BTreeMap<String, Region>,
) -> Result<(), StaticErrors> {
    let (Operand::Fact(f), Operand::Literal(Value::String(s))) = (left, right) else {
        return Ok(());
    };
    let Some(region) = f.strip_prefix(RESERVED_PREFIX).and_then(|r| regions.get(r)) else {
        return Ok(());
    };
    if !region.states.contains_key(s) {
        return Err(refuse(format!("`{f}` references unknown state `{s}`")));
    }
    Ok(())
}

fn lower_joint(
    source: &RegionDefinition,
    flat: &mut Definition,
    products: &[BTreeMap<String, String>],
    name: &str,
    cmd: &JointCommand,
    diagnostics: &mut Vec<String>,
    blocked: &mut Vec<BlockedOutcome>,
    edges: &mut Vec<LoweredEdge>,
    origin: &mut OriginCollector,
) -> Result<(), StaticErrors> {
    check_joint_sources(&source.regions, name, cmd)?;
    let outcomes = joint_outcomes(name, cmd)?;
    check_joint_outcomes(&source.regions, name, &outcomes)?;
    let mut lowered = command_shell(cmd.args.clone(), cmd.effect.clone());
    for product in products {
        let label = flat_name(product);
        if flat.states[&label].terminal {
            continue;
        }
        if let Some(reason) = joint_source_mismatch(name, cmd, product, &label) {
            record_blocked_joint_outcomes(product, &label, &outcomes, &reason, blocked);
            continue;
        }
        push_joint_product(
            &mut lowered,
            product,
            &label,
            name,
            source,
            &outcomes,
            diagnostics,
            edges,
            origin,
        )?;
    }
    flat.commands.insert(name.to_string(), lowered);
    Ok(())
}

fn push_joint_product(
    lowered: &mut CommandDef,
    product: &BTreeMap<String, String>,
    label: &str,
    name: &str,
    source: &RegionDefinition,
    outcomes: &[JointOutcome],
    diagnostics: &mut Vec<String>,
    edges: &mut Vec<LoweredEdge>,
    origin: &mut OriginCollector,
) -> Result<(), StaticErrors> {
    let mut has_outcome = false;
    let mut guard_pruned = false;
    for (ordinal, outcome) in outcomes.iter().enumerate() {
        if outcome
            .to
            .keys()
            .any(|r| source.regions[r].states[&product[r]].terminal)
        {
            continue;
        }
        let mut target = product.clone();
        for (r, s) in &outcome.to {
            target.insert(r.clone(), s.clone());
        }
        match push_outcome(
            lowered,
            product,
            &target,
            name,
            outcome.when.as_deref(),
            outcome.then.as_ref(),
            diagnostics,
            edges,
            origin,
            AuthoredCoordinate::outcome(name, "joint", ordinal),
        )? {
            OutcomeDisposition::Emitted => has_outcome = true,
            OutcomeDisposition::PrunedFalseGuard => guard_pruned = true,
        }
    }
    // A statically false guard removes its edge, not the command's legal source.
    if has_outcome || guard_pruned {
        lowered.from.push(label.to_string());
    }
    Ok(())
}

fn joint_source_mismatch(
    name: &str,
    cmd: &JointCommand,
    product: &BTreeMap<String, String>,
    label: &str,
) -> Option<String> {
    cmd.from.iter().find_map(|(region, allowed)| {
        let actual = &product[region];
        (!allowed.contains(actual)).then(|| {
            format!(
                "joint command `{name}` requires {region} in [{}], but source product `{label}` has {region}=`{actual}`",
                allowed.join(", ")
            )
        })
    })
}

fn record_blocked_joint_outcomes(
    product: &BTreeMap<String, String>,
    label: &str,
    outcomes: &[JointOutcome],
    reason: &str,
    blocked: &mut Vec<BlockedOutcome>,
) {
    for outcome in outcomes {
        let mut target = product.clone();
        for (region, state) in &outcome.to {
            target.insert(region.clone(), state.clone());
        }
        if &target != product {
            blocked.push(BlockedOutcome {
                from: label.to_string(),
                to: flat_name(&target),
                reason: reason.to_string(),
            });
        }
    }
}

fn check_joint_sources(
    regions: &BTreeMap<String, Region>,
    name: &str,
    cmd: &JointCommand,
) -> Result<(), StaticErrors> {
    if cmd.from.is_empty() {
        return Err(refuse(format!(
            "joint command `{name}` requires a nonempty region source map"
        )));
    }
    for (region, states) in &cmd.from {
        let r = regions.get(region).ok_or_else(|| {
            refuse(format!(
                "joint command `{name}` references unknown region `{region}`"
            ))
        })?;
        if states.is_empty() || states.iter().any(|s| !r.states.contains_key(s)) {
            return Err(refuse(format!(
                "joint command `{name}` has empty or unknown sources in `{region}`"
            )));
        }
        if states.iter().any(|s| r.states[s].terminal) {
            return Err(refuse(format!(
                "joint command `{name}` lists completed region `{region}` as a source"
            )));
        }
    }
    Ok(())
}

fn joint_outcomes(name: &str, cmd: &JointCommand) -> Result<Vec<JointOutcome>, StaticErrors> {
    let outcomes = match (&cmd.to, &cmd.outcomes) {
        (Some(to), None) => vec![JointOutcome {
            when: cmd.when.clone(),
            to: to.clone(),
            then: cmd.then.clone(),
        }],
        (None, Some(outcomes))
            if cmd.when.is_none() && cmd.then.is_none() && !outcomes.is_empty() =>
        {
            outcomes.clone()
        }
        _ => {
            return Err(refuse(format!(
                "joint command `{name}` must declare either to/when/then or nonempty outcomes"
            )))
        }
    };
    Ok(outcomes)
}

fn check_joint_outcomes(
    regions: &BTreeMap<String, Region>,
    name: &str,
    outcomes: &[JointOutcome],
) -> Result<(), StaticErrors> {
    for (i, outcome) in outcomes.iter().enumerate() {
        if outcome.when.is_none() && i + 1 < outcomes.len() {
            return Err(refuse(format!(
                "joint command `{name}` has unreachable outcomes after unconditional outcome {i}"
            )));
        }
        if outcome.to.is_empty() {
            return Err(refuse(format!(
                "joint command `{name}` requires a nonempty destination map"
            )));
        }
        for (region, state) in &outcome.to {
            let r = regions.get(region).ok_or_else(|| {
                refuse(format!(
                    "joint command `{name}` targets unknown region `{region}`"
                ))
            })?;
            if !r.states.contains_key(state) {
                return Err(refuse(format!(
                    "joint command `{name}` targets unknown state `{region}.{state}`"
                )));
            }
        }
        if let Some(when) = &outcome.when {
            check_expression(when, regions)?;
        }
        check_assignments(outcome.then.as_ref())?;
    }
    Ok(())
}

fn outcome_source(outcome: &OutcomeDef) -> Option<String> {
    let expression = expr::parse(outcome.when.as_deref()?).ok()?;
    let selector = match expression {
        Expr::And(parts) => parts.into_iter().next()?,
        other => other,
    };
    match selector {
        Expr::Cmp(cmp) if cmp.left == Operand::State && cmp.op == expr::CmpOp::Eq => {
            match cmp.right {
                Operand::Literal(Value::String(label)) => Some(label),
                _ => None,
            }
        }
        _ => None,
    }
}

fn prune_unreachable_products(
    flat: &mut Definition,
    map: &mut RegionMap,
    edges: &[LoweredEdge],
    blocked: &[BlockedOutcome],
) {
    let Some(initial) = flat
        .states
        .iter()
        .find_map(|(label, state)| state.initial.then_some(label.clone()))
    else {
        return;
    };
    let mut reached = BTreeSet::from([initial.clone()]);
    let mut frontier = VecDeque::from([initial]);
    while let Some(from) = frontier.pop_front() {
        for edge in edges.iter().filter(|edge| edge.from == from) {
            if flat.states.contains_key(&edge.to) && reached.insert(edge.to.clone()) {
                frontier.push_back(edge.to.clone());
            }
        }
    }
    let removed = map
        .states
        .keys()
        .filter(|label| !reached.contains(*label))
        .cloned()
        .collect::<BTreeSet<_>>();
    for label in &removed {
        let assignment = map.states[label]
            .iter()
            .map(|(region, state)| format!("{region}={state}"))
            .collect::<Vec<_>>()
            .join(", ");
        let dropped_guards = map
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.contains(&format!("toward product `{label}`")))
            .cloned()
            .collect::<Vec<_>>();
        let blocked_couplings = blocked
            .iter()
            .filter(|candidate| candidate.to == *label && reached.contains(&candidate.from))
            .map(|candidate| candidate.reason.clone())
            .collect::<BTreeSet<_>>();
        let mut evidence = Vec::new();
        if !dropped_guards.is_empty() {
            evidence.push(format!(
                "pruned guard evidence: {}",
                dropped_guards.join("; ")
            ));
        }
        if !blocked_couplings.is_empty() {
            evidence.push(format!(
                "blocked coupling evidence: {}",
                blocked_couplings.into_iter().collect::<Vec<_>>().join("; ")
            ));
        }
        let detail = if evidence.is_empty() {
            "no feasible lowered outcome targets it".to_string()
        } else {
            evidence.join("; ")
        };
        map.diagnostics.push(format!(
            "product `{label}` ({assignment}) is unreachable by corpus-independent lowering graph: {detail}"
        ));
    }
    flat.states.retain(|label, _| reached.contains(label));
    flat.invariants.retain(|name, _| {
        !name.starts_with("region_projection_")
            || !removed
                .iter()
                .any(|label| name == &format!("region_projection_{label}"))
    });
    map.states.retain(|label, _| reached.contains(label));
    for cmd in flat.commands.values_mut() {
        cmd.from.retain(|label| reached.contains(label));
        if let Some(outcomes) = &mut cmd.outcomes {
            outcomes.retain(|outcome| {
                outcome_source(outcome).is_some_and(|from| reached.contains(&from))
                    && reached.contains(&outcome.to)
            });
        }
        if cmd.outcomes.as_ref().is_some_and(Vec::is_empty) {
            cmd.from.clear();
        }
    }
    flat.commands.retain(|_, command| !command.from.is_empty());
}

impl RegionMap {
    pub fn decode(&self, state: &Value) -> Result<&BTreeMap<String, String>, String> {
        let label = state
            .get("state")
            .and_then(Value::as_str)
            .ok_or("projection has no state label")?;
        let decoded = self
            .states
            .get(label)
            .ok_or_else(|| format!("unknown flat state `{label}`"))?;
        for (region, expected) in decoded {
            if state
                .get(format!("region_{region}"))
                .and_then(Value::as_str)
                != Some(expected.as_str())
            {
                return Err(format!(
                    "flat state `{label}` disagrees with region_{region}"
                ));
            }
        }
        Ok(decoded)
    }

    /// Mermaid is a lossy derived view of actual lowered transitions, not authority.
    pub fn mermaid(&self, definition: &Definition) -> String {
        let mut out = String::from("```mermaid\nstateDiagram-v2\n");
        let ids = self.mermaid_ids(&mut out);
        let edges = self.mermaid_edges(definition, &ids);
        for (from, to, command) in edges {
            out.push_str(&format!(
                "  s{from} --> s{to}: {}\n",
                mermaid_label(&command)
            ));
        }
        out.push_str("```\n");
        out
    }

    fn mermaid_ids(&self, out: &mut String) -> BTreeMap<(String, String), usize> {
        let mut ids = BTreeMap::new();
        let mut count = 0;
        let Some(product) = self.states.values().next() else {
            return ids;
        };
        for region in product.keys() {
            out.push_str(&format!(
                "  state \"{}\" as r{count} {{\n",
                mermaid_label(region)
            ));
            let region_id = count;
            count += 1;
            let states = self
                .states
                .values()
                .map(|p| p[region].clone())
                .collect::<BTreeSet<_>>();
            if let Some((parent, children)) = self.parents.get(region) {
                out.push_str(&format!(
                    "    state \"{}\" as p{region_id} {{\n",
                    mermaid_label(parent)
                ));
                for state in &states {
                    if children.contains(state) {
                        ids.insert((region.clone(), state.clone()), count);
                        out.push_str(&format!(
                            "      state \"{}\" as s{count}\n",
                            mermaid_label(state)
                        ));
                        count += 1;
                    }
                }
                out.push_str("    }\n");
            }
            for state in &states {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    ids.entry((region.clone(), state.clone()))
                {
                    entry.insert(count);
                    out.push_str(&format!(
                        "    state \"{}\" as s{count}\n",
                        mermaid_label(state)
                    ));
                    count += 1;
                }
            }
            out.push_str("  }\n");
        }
        ids
    }

    fn mermaid_edges(
        &self,
        definition: &Definition,
        ids: &BTreeMap<(String, String), usize>,
    ) -> BTreeSet<(usize, usize, String)> {
        let mut edges = BTreeSet::new();
        for (command, cmd) in &definition.commands {
            for from in &cmd.from {
                for outcome in cmd.outcomes.iter().flatten() {
                    self.collect_mermaid_outcome(from, outcome, command, ids, &mut edges);
                }
            }
        }
        edges
    }

    fn collect_mermaid_outcome(
        &self,
        from: &str,
        outcome: &OutcomeDef,
        command: &str,
        ids: &BTreeMap<(String, String), usize>,
        edges: &mut BTreeSet<(usize, usize, String)>,
    ) {
        // Lowering emits one selector per source; parse it, never guess by text substitution.
        let Some(when) = outcome.when.as_deref() else {
            return;
        };
        let Ok(expression) = expr::parse(when) else {
            return;
        };
        let selector = match &expression {
            Expr::And(parts) => parts.first(),
            e => Some(e),
        };
        let matches = matches!(selector, Some(Expr::Cmp(c)) if c.left == Operand::State && c.right == Operand::Literal(Value::String(from.to_string())));
        if !matches {
            return;
        }
        let (Some(before), Some(after)) = (self.states.get(from), self.states.get(&outcome.to))
        else {
            return;
        };
        for (region, state) in before {
            if state != &after[region] {
                edges.insert((
                    ids[&(region.clone(), state.clone())],
                    ids[&(region.clone(), after[region].clone())],
                    command.to_string(),
                ));
            }
        }
    }
}

fn mermaid_label(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace(['\n', '\r'], " ")
}

/// A target is a proposal only when `committed` is false. Such a proposal was
/// blocked by the authoritative machine and is never a runtime after-image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionStep {
    pub command: String,
    pub case: Option<String>,
    pub before: BTreeMap<String, String>,
    pub target: BTreeMap<String, String>,
    pub committed: bool,
    pub diagnostic: Option<Diagnostic>,
}

pub struct RegionCheckResult {
    pub verdict: String,
    pub text: String,
    pub flat: super::check::CheckResult,
    pub invariant_refusals: Vec<InvariantRefusalWitness>,
    pub exploration_complete: bool,
    pub exploration: super::explore::Exploration,
    /// Exploration of `--prior`, when one was supplied. Same corpus and bounds as the candidate.
    pub prior_exploration: Option<super::explore::Exploration>,
}

fn append_unresolved_region_obligations(
    text: &mut String,
    definition: &Definition,
    regions: &RegionMap,
    obligations: &super::report::ObligationCounts,
) {
    if obligations.falsified_ids.is_empty() && obligations.uncovered_ids.is_empty() {
        return;
    }
    let plan = super::plan::verification_plan(definition);
    let unresolved = grouped_region_obligations(obligations, &plan, regions);
    if unresolved.is_empty() {
        return;
    }
    text.push_str("UNRESOLVED REGION OBLIGATIONS\n");
    for (group, obligations) in unresolved {
        text.push_str(&format!("  {group}\n"));
        append_command_source_products(text, &group, definition, regions, &plan);
        for (id, status) in obligations {
            text.push_str(&format!("    {id} {status}\n"));
        }
    }
}

fn grouped_region_obligations<'a>(
    obligations: &'a super::report::ObligationCounts,
    plan: &super::plan::VerificationPlanV1,
    regions: &RegionMap,
) -> BTreeMap<String, Vec<(&'a str, &'static str)>> {
    let mut groups = BTreeMap::new();
    for (ids, status) in [
        (obligations.falsified_ids.as_slice(), "falsified"),
        (obligations.uncovered_ids.as_slice(), "uncovered"),
    ] {
        add_region_obligations(&mut groups, plan, ids, status, regions);
    }
    groups
}

fn add_region_obligations<'a>(
    groups: &mut BTreeMap<String, Vec<(&'a str, &'static str)>>,
    plan: &super::plan::VerificationPlanV1,
    ids: &'a [String],
    status: &'static str,
    regions: &RegionMap,
) {
    for id in ids {
        let Some(obligation) = plan
            .obligations
            .iter()
            .find(|obligation| obligation.obligation_id == *id)
        else {
            continue;
        };
        let group = region_obligation_group(&obligation.coordinate, regions);
        groups.entry(group).or_default().push((id.as_str(), status));
    }
}

fn region_obligation_group(
    coordinate: &super::plan::SemanticCoordinate,
    regions: &RegionMap,
) -> String {
    if let Some(command) = &coordinate.command {
        format!("command {command}")
    } else if let Some(state) = &coordinate.state {
        let assignment = regions
            .states
            .get(state)
            .map(readable)
            .unwrap_or_else(|| format!("unresolved product {state}"));
        format!("product {state} ({assignment})")
    } else if let Some(invariant) = &coordinate.invariant {
        format!("invariant {invariant}")
    } else {
        "other obligations".to_string()
    }
}

fn append_command_source_products(
    text: &mut String,
    group: &str,
    definition: &Definition,
    regions: &RegionMap,
    plan: &super::plan::VerificationPlanV1,
) {
    let Some(command) = group.strip_prefix("command ") else {
        return;
    };
    let mut products = BTreeSet::new();
    if let Some(command_definition) = definition.commands.get(command) {
        add_region_products(
            &mut products,
            command_definition.from.iter().map(String::as_str),
            regions,
        );
    }
    add_region_products(
        &mut products,
        plan.obligations
            .iter()
            .filter(|obligation| obligation.coordinate.command.as_deref() == Some(command))
            .filter_map(|obligation| obligation.coordinate.source.as_deref()),
        regions,
    );
    if !products.is_empty() {
        text.push_str(&format!(
            "    source products: {}\n",
            products.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
}

fn add_region_products<'a>(
    products: &mut BTreeSet<String>,
    labels: impl Iterator<Item = &'a str>,
    regions: &RegionMap,
) {
    products.extend(labels.filter_map(|label| {
        regions
            .states
            .get(label)
            .map(|states| format!("{label} ({})", readable(states)))
    }));
}

/// Reusable analysis, not CLI-only safety logic. Schema-1 checks stay unchanged.
pub fn check_source(
    source: &SourceDefinition,
    prior: Option<&Definition>,
    corpus: &CaseCorpus,
    baseline: &[String],
    bounds: Bounds,
) -> Result<RegionCheckResult, String> {
    let (flat, run, invariant_refusals, machine, prior_exploration) =
        super::check::check_with_run(&source.definition, prior, corpus, baseline, bounds, true)?;
    let mut text = flat.text.clone();
    if let Some(regions) = &source.regions {
        append_unresolved_region_obligations(
            &mut text,
            &source.definition,
            regions,
            &flat.report.obligations,
        );
        if !regions.diagnostics.is_empty() {
            text.push_str("REGION LOWERING DIAGNOSTICS (corpus-independent)\n");
            for diagnostic in &regions.diagnostics {
                text.push_str(&format!("  {diagnostic}\n"));
            }
        }
        text.push_str(&format!(
            "\nREGION VIEW (derived, not a receipt)\n  authoring_hash {}\n  spec_hash {}\n  exploration {} (max_nodes={}, max_depth={}, corpus={})\n",
            regions.authoring_hash, run.spec_hash,
            if run.complete { "complete within supplied finite corpus" } else { "INCOMPLETE" },
            bounds.max_nodes, bounds.max_depth, run.case_corpus_digest,
        ));
        if let Some(reason) = &run.incomplete_reason {
            text.push_str(&format!("  incomplete_reason {reason}\n"));
        }
        text.push_str(
            "  ACCEPT, when present, is bounded evidence, not exhaustive proof.\nREGION PATHS\n",
        );
        for (label, witness) in &run.witnesses.by_state_label {
            text.push_str(&format!("  reached {label}\n"));
            render_reached_path(&mut text, regions, &machine, witness, corpus);
        }
        for witness in &invariant_refusals {
            text.push_str(&format!(
                "  invariant-refusal witness {} rule={}\n",
                witness.diagnostic.code.as_str(),
                witness.diagnostic.rule.as_deref().unwrap_or("unknown")
            ));
            match regions.project_invariant_witness(&machine, witness, corpus) {
                Ok(path) => render_path(&mut text, &path),
                Err(error) => text.push_str(&format!("    projection unavailable: {error}\n")),
            }
        }
        text.push_str("CHART (lossy structural view; guards, invariants, facts and joint atomicity are not shown)\n");
        text.push_str(&regions.mermaid(&source.definition));
    }
    if source.regions.is_none() {
        render_flat_invariant_witnesses(&mut text, &invariant_refusals);
    }
    Ok(RegionCheckResult {
        verdict: flat.verdict.clone(),
        text,
        flat,
        invariant_refusals,
        exploration_complete: run.complete,
        exploration: run,
        prior_exploration,
    })
}

/// A flat (region-less) definition has no chart to project onto, so name the bounded
/// counterexample path directly. The path is whatever the exploration actually found.
fn render_flat_invariant_witnesses(text: &mut String, witnesses: &[InvariantRefusalWitness]) {
    for witness in witnesses {
        text.push_str(&format!(
            "\nINVARIANT-REFUSAL WITNESS (bounded counterexample within the supplied corpus)\n  {} rule={}\n  steps:",
            witness.diagnostic.code.as_str(),
            witness.diagnostic.rule.as_deref().unwrap_or("unknown")
        ));
        for step in &witness.steps {
            let case = step
                .case
                .as_ref()
                .map(|c| format!("[{c}]"))
                .unwrap_or_default();
            text.push_str(&format!(" {}{case}", step.command));
        }
        text.push('\n');
    }
}

/// Machine-readable invariant-refusal witnesses for `ik check --json`.
pub fn invariant_witness_json(witnesses: &[InvariantRefusalWitness]) -> serde_json::Value {
    witnesses
        .iter()
        .map(|w| serde_json::json!({"diagnostic": w.diagnostic, "steps": w.steps}))
        .collect()
}

fn render_reached_path(
    text: &mut String,
    regions: &RegionMap,
    machine: &CompiledMachine,
    witness: &[WitnessStep],
    corpus: &CaseCorpus,
) {
    match regions.project_path(machine, witness, corpus) {
        Ok(path) if path.is_empty() => match regions.decode(&machine.initial_state()) {
            Ok(state) => text.push_str(&format!("    initial {}\n", readable(state))),
            Err(error) => text.push_str(&format!("    projection unavailable: {error}\n")),
        },
        Ok(path) => render_path(text, &path),
        Err(error) => text.push_str(&format!("    projection unavailable: {error}\n")),
    }
}

fn readable(states: &BTreeMap<String, String>) -> String {
    states
        .iter()
        .map(|(r, s)| format!("{r}={s}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_path(out: &mut String, path: &[RegionStep]) {
    for step in path {
        out.push_str(&format!(
            "    {}{}: [{}] -> [{}] {}\n",
            step.command,
            step.case
                .as_ref()
                .map(|c| format!(" (case={c})"))
                .unwrap_or_default(),
            readable(&step.before),
            readable(&step.target),
            if step.committed {
                "COMMITTED"
            } else {
                "REFUSED PROPOSAL (not committed)"
            },
        ));
    }
}

impl RegionMap {
    /// Replay exact input cases through decide/apply. No destinations are inferred
    /// from a command name or invented from the chart.
    pub fn project_path(
        &self,
        machine: &CompiledMachine,
        steps: &[WitnessStep],
        corpus: &CaseCorpus,
    ) -> Result<Vec<RegionStep>, String> {
        if self.spec_hash != machine.identity().spec_hash.0 {
            return Err("region map belongs to a different lowered machine".into());
        }
        let mut state = machine.initial_state();
        let mut path = Vec::with_capacity(steps.len());
        for (position, step) in steps.iter().enumerate() {
            let before = self.decode(&state)?.clone();
            let command = witness_command(step, corpus)?;
            let decision = machine
                .decide(&state, None, &EffectStatus::None, &command)
                .map_err(|d| format!("witness decide refused: {d}"))?;
            let Decision::Emit { event, .. } = decision else {
                return Err("witness unexpectedly produced a noop".into());
            };
            let actual_index = event
                .payload
                .get("outcome")
                .and_then(Value::as_u64)
                .map(|i| i as usize);
            if step.outcome_index.is_some() && step.outcome_index != actual_index {
                return Err("witness outcome index disagrees with authoritative selection".into());
            }
            match machine.apply(&state, &event) {
                Ok(applied) => {
                    let target = self.decode(&applied.state)?.clone();
                    path.push(RegionStep {
                        command: step.command.clone(),
                        case: step.case.clone(),
                        before,
                        target,
                        committed: true,
                        diagnostic: None,
                    });
                    state = applied.state;
                }
                Err(diagnostic) => {
                    if position + 1 != steps.len() {
                        return Err("witness continues after a refused transition".into());
                    }
                    let target =
                        self.project_refused_target(machine, &state, &event, &diagnostic)?;
                    path.push(RegionStep {
                        command: step.command.clone(),
                        case: step.case.clone(),
                        before,
                        target,
                        committed: false,
                        diagnostic: Some(diagnostic),
                    });
                }
            }
        }
        Ok(path)
    }

    fn project_refused_target(
        &self,
        machine: &CompiledMachine,
        state: &Value,
        event: &crate::machine::DomainEvent,
        diagnostic: &Diagnostic,
    ) -> Result<BTreeMap<String, String>, String> {
        // The shared reducer exposes the proposal which apply refused;
        // it is explicitly NOT installed as the replay state.
        let reduced = machine
            .reduce_event(state, event)
            .map_err(|d| format!("cannot project refused proposal: {d}"))?;
        let target = self.decode(&reduced.state)?.clone();
        let same = reduced
            .invariants
            .error
            .as_ref()
            .is_some_and(|d| d.code == diagnostic.code && d.rule == diagnostic.rule);
        if !same {
            return Err("refusal is not a reproducible model invariant failure".into());
        }
        Ok(target)
    }

    pub fn project_invariant_witness(
        &self,
        machine: &CompiledMachine,
        witness: &InvariantRefusalWitness,
        corpus: &CaseCorpus,
    ) -> Result<Vec<RegionStep>, String> {
        let path = self.project_path(machine, &witness.steps, corpus)?;
        let observed = path.last().and_then(|s| s.diagnostic.as_ref());
        if !observed
            .is_some_and(|d| d.code == witness.diagnostic.code && d.rule == witness.diagnostic.rule)
        {
            return Err("invariant witness replay changed diagnostic code or rule".into());
        }
        Ok(path)
    }
}

fn witness_command(step: &WitnessStep, corpus: &CaseCorpus) -> Result<Value, String> {
    let Some(case_name) = &step.case else {
        return Ok(serde_json::json!({"name": step.command}));
    };
    let mut cases = corpus
        .commands
        .get(&step.command)
        .into_iter()
        .flatten()
        .filter(|c| &c.name == case_name);
    let case = cases
        .next()
        .ok_or_else(|| format!("missing input case {}.{case_name}", step.command))?;
    if cases.next().is_some() {
        return Err(format!("ambiguous input case {}.{case_name}", step.command));
    }
    Ok(serde_json::json!({"name": step.command, "args": case.args}))
}

fn validate_initial(definition: &Definition) -> Result<(), StaticErrors> {
    use super::evaluate::{evaluate_invariants, InvariantSpec};
    let mut parsed = Vec::with_capacity(definition.invariants.len());
    for (name, invariant) in &definition.invariants {
        let when = invariant
            .when
            .as_deref()
            .map(expr::parse)
            .transpose()
            .map_err(refuse)?;
        let assertion = expr::parse(&invariant.assert).map_err(refuse)?;
        parsed.push((
            name,
            invariant,
            when,
            assertion,
            format!("invariants.{name}.when"),
            format!("invariants.{name}.assert"),
        ));
    }
    let specs = parsed
        .iter()
        .map(
            |(name, invariant, when, assertion, when_rule, assert_rule)| InvariantSpec {
                name,
                when: when.as_ref(),
                when_src: invariant.when.as_deref(),
                when_rule,
                assert: assertion,
                assert_rule,
            },
        )
        .collect::<Vec<_>>();
    let initial = definition
        .states
        .iter()
        .find(|(_, s)| s.initial)
        .ok_or_else(|| refuse("lowered region definition is missing an initial product"))?;
    let facts = definition
        .facts
        .iter()
        .filter_map(|(n, f)| f.initial.as_ref().map(|v| (n.clone(), v.clone())))
        .collect();
    let args = BTreeMap::new();
    let context = expr::EvalCtx {
        facts: &facts,
        args: &args,
        state: initial.0,
        effect: None,
    };
    if let Some(error) = evaluate_invariants(&specs, &context).error {
        return Err(refuse(format!(
            "initial region configuration violates invariant: {error}"
        )));
    }
    Ok(())
}
