//! Secondary mermaid flowchart. Text tags carry the encoding if styling is ignored.

use super::{display_label, mermaid_id, Diagram, DiagramKind, Emphasis, Hop, Station, StrokeClass};

pub(super) fn render(diagram: &Diagram) -> String {
    if diagram.kind == DiagramKind::Recovery {
        return recovery_mermaid(diagram);
    }
    if let Some(regions) = &diagram.regions {
        return region_mermaid(regions);
    }
    let mut out = String::from("flowchart LR\n");
    out.push_str("    classDef here stroke-width:3px\n");
    out.push_str("    classDef stuck stroke-width:2px\n");
    out.push_str("    classDef terminal stroke-width:2px\n");
    out.push_str("    i(( ))\n");
    for station in &diagram.stations {
        out.push_str("    ");
        out.push_str(&node_line(diagram, station));
        out.push('\n');
    }
    if let Some(initial) = diagram.stations.iter().find(|st| st.initial) {
        out.push_str("    i --> ");
        out.push_str(&mermaid_id(&initial.id));
        out.push('\n');
    }
    if let Some(here) = &diagram.you_are_here {
        out.push_str("    class ");
        out.push_str(&mermaid_id(here));
        out.push_str(" here\n");
    }
    for hop in &diagram.hops {
        out.push_str("    ");
        out.push_str(&edge_line(hop));
        out.push('\n');
    }
    out
}

fn node_line(diagram: &Diagram, station: &Station) -> String {
    let id = mermaid_id(&station.id);
    let mut label = display_label(&station.id);
    if let Some(n) = station.distance {
        label.push_str(" ·");
        label.push_str(&n.to_string());
    }
    if station.is_new {
        label.push_str(" new");
    }
    if station.stuck || station.unreachable {
        label.push_str(" cannot reach an end");
    }
    if diagram.you_are_here.as_deref() == Some(station.id.as_str())
        && diagram.emphasis == Emphasis::YouAreHere
    {
        label.push_str(" ▼ HERE");
    }
    let quoted = quoted(&label);
    if station.terminal {
        format!("{id}((\"{quoted}\"))")
    } else if station.initial {
        format!("{id}([\"{quoted}\"])")
    } else {
        format!("{id}[\"{quoted}\"]")
    }
}

fn edge_line(hop: &Hop) -> String {
    if hop.stub {
        let id = format!("r_{}", mermaid_id(&hop.command));
        let reason = hop.refused_reason.as_deref().unwrap_or("refusal");
        let label = quoted(&format!("refused · {} ✕ {reason}", hop.command));
        return format!("{id}[\"{label}\"]\n    {} -.-> {id}", mermaid_id(&hop.from));
    }
    let arrow = match hop.stroke {
        StrokeClass::Observed => "==>",
        StrokeClass::Explored => "-.->",
        StrokeClass::Declared | StrokeClass::Refused => "-->",
    };
    format!(
        "{} {arrow} |{}| {}",
        mermaid_id(&hop.from),
        quoted(&edge_label(hop)),
        mermaid_id(&hop.to)
    )
}

fn edge_label(hop: &Hop) -> String {
    let mut label = String::new();
    if let Some(step) = hop.step {
        label.push_str(&step.to_string());
        label.push_str(" · ");
    } else {
        match hop.stroke {
            StrokeClass::Explored => label.push_str("explored · "),
            StrokeClass::Declared => label.push_str("declared · "),
            StrokeClass::Observed => {}
            StrokeClass::Refused => label.push_str("refused · "),
        }
    }
    if hop.is_new {
        label.push_str("new · ");
    }
    if hop.is_removed {
        label.push_str("removed · ");
    }
    label.push_str(&hop.command);
    if hop.guarded {
        label.push_str(" (guard)");
    }
    display_label(&label)
}

fn region_mermaid(view: &super::RegionView) -> String {
    let mut out = String::from("flowchart LR\n");
    for lane in &view.lanes {
        let sub_id = mermaid_id(&format!("region_{}", lane.name));
        out.push_str(&format!(
            "    subgraph {sub_id}[\"{}\"]\n",
            quoted(&lane.name)
        ));
        for state in &lane.states {
            let nid = mermaid_id(&format!("{}_{}", lane.name, state));
            let label = quoted(&display_label(state));
            if lane.terminals.contains(state) {
                out.push_str(&format!("        {nid}((\"{label}\"))\n"));
            } else {
                out.push_str(&format!("        {nid}[\"{label}\"]\n"));
            }
        }
        for hop in lane.hops.iter().chain(lane.parent_hops.iter()) {
            let from_id = mermaid_id(&format!("{}_{}", lane.name, hop.from));
            let to_id = mermaid_id(&format!("{}_{}", lane.name, hop.to));
            let mut label = hop.command.clone();
            if !hop.guard.is_empty() {
                label.push_str(&format!(" ({})", hop.guard));
            }
            out.push_str(&format!(
                "        {from_id} --> |{}| {to_id}\n",
                quoted(&display_label(&label))
            ));
        }
        out.push_str("    end\n");
    }
    for joint in &view.joints {
        let jid = mermaid_id(&format!("joint_{}", joint.command));
        let mut label = format!("joint: {}", joint.command);
        if !joint.guard.is_empty() {
            label.push_str(&format!(" ({})", joint.guard));
        }
        out.push_str(&format!("    {jid}{{\"{}\"}}\n", quoted(&display_label(&label))));
        for (region, _from, to) in &joint.legs {
            let to_id = mermaid_id(&format!("{region}_{to}"));
            out.push_str(&format!("    {jid} -.-> {to_id}\n"));
        }
    }
    out
}

fn recovery_mermaid(diagram: &Diagram) -> String {
    let mut out = String::from("flowchart LR\n");
    let Some(view) = &diagram.recovery else {
        out.push_str("    empty[\"no evidenced steps\"]\n");
        return out;
    };
    for (index, step) in view.timeline.iter().enumerate() {
        let id = format!("t{index}");
        let mut label = step.label.clone();
        if step.id == "lost" {
            label.push_str(" ✂");
        }
        if view.you_are_here.as_deref() == Some(step.id) {
            label.push_str(" ▼ HERE");
        }
        out.push_str(&format!(
            "    {id}[\"{}\"]\n",
            quoted(&format!("{label} · {}", step.how))
        ));
    }
    for index in 0..view.timeline.len().saturating_sub(1) {
        let both_observed = view.timeline[index].how == "observed"
            && view.timeline[index + 1].how == "observed";
        let arrow = if both_observed { "==>" } else { "-.->" };
        out.push_str(&format!("    t{index} {arrow} t{}\n", index + 1));
    }
    out
}

fn quoted(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("#quot;"),
            '[' => out.push('('),
            ']' => out.push(')'),
            '|' => out.push('/'),
            c => out.push(c),
        }
    }
    out
}

/// Structural check used by tests: balanced quotes/brackets, no unsupported directives.
pub fn structurally_ok(source: &str) -> bool {
    flowchart_header(source) && !forbidden_directive(source) && brackets_balanced(source)
}

fn flowchart_header(source: &str) -> bool {
    const HEADS: [&str; 2] = ["flowchart LR", "flowchart TB"];
    HEADS.iter().any(|head| source.starts_with(head))
}

fn forbidden_directive(source: &str) -> bool {
    const FORBIDDEN: [&str; 3] = ["%%{", "click ", "<script"];
    FORBIDDEN.iter().any(|token| source.contains(token))
}

enum Scan {
    ToggleQuote,
    Skip,
    Open(usize),
    Close(usize),
}

struct BracketDepth {
    counts: [i32; 3],
}

impl BracketDepth {
    fn open(&mut self, kind: usize) {
        self.counts[kind] += 1;
    }

    fn close(&mut self, kind: usize) -> bool {
        self.counts[kind] -= 1;
        self.counts[kind] >= 0
    }

    fn zero(&self) -> bool {
        self.counts.iter().all(|n| *n == 0)
    }
}

fn brackets_balanced(source: &str) -> bool {
    let mut depth = BracketDepth { counts: [0; 3] };
    let mut quote = false;
    for ch in source.chars() {
        if !apply_scan(&mut depth, &mut quote, ch) {
            return false;
        }
    }
    !quote && depth.zero()
}

fn apply_scan(depth: &mut BracketDepth, quote: &mut bool, ch: char) -> bool {
    match quote_or_bracket(ch, *quote) {
        Scan::ToggleQuote => {
            *quote = !*quote;
            true
        }
        Scan::Skip => true,
        Scan::Open(kind) => {
            depth.open(kind);
            true
        }
        Scan::Close(kind) => depth.close(kind),
    }
}

fn quote_or_bracket(ch: char, quote: bool) -> Scan {
    if quote {
        quoted_scan(ch)
    } else {
        unquoted_scan(ch)
    }
}

fn quoted_scan(ch: char) -> Scan {
    match ch {
        '"' => Scan::ToggleQuote,
        _ => Scan::Skip,
    }
}

fn unquoted_scan(ch: char) -> Scan {
    match ch {
        '"' => Scan::ToggleQuote,
        '[' => Scan::Open(0),
        ']' => Scan::Close(0),
        '(' => Scan::Open(1),
        ')' => Scan::Close(1),
        '{' => Scan::Open(2),
        '}' => Scan::Close(2),
        _ => Scan::Skip,
    }
}
