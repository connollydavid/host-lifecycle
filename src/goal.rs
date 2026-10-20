//! The goal subsystem (plan/0101, call/0061). A goal IS a plan/NNNN milestone;
//! state splits by volatility: durable facts are tracked surfaces the tool already
//! authors (`[receipt "goal"]` stanzas in the operational ledger, the milestone
//! Status line, OWED rows in PLAN.md's follow-up table), and machine state is the
//! gitignored `.host-goal` sidecar holding only this worktree's claim, re-derivable
//! by `goal repair`. No authority tokens: the audited commit of a declaration is
//! the act. Parking is re-queuing. Completion composes the existing gates as
//! subprocesses and prints every hazard, each with its remedy.

use std::path::{Path, PathBuf};
use std::process;

use crate::{
    append_receipt, latest_task_receipt, parse_receipts, parse_tasks, read_task_receipts, today,
    tracked_markdown, Receipt,
};

const SIDECAR: &str = ".host-goal";
const LEDGER: &str = ".host-lifecycle-receipts";

/// The goal state as a fold of the receipt log (append-only, last transition wins).
#[derive(Debug, PartialEq, Clone)]
enum GoalState {
    Active(String),
    Parked(String, String),
    None,
}

fn goal_state(receipts: &[Receipt]) -> GoalState {
    let mut state = GoalState::None;
    for r in receipts.iter().filter(|r| r.phase == "goal") {
        let target = r.target.clone().unwrap_or_default();
        match r.component.as_deref() {
            Some("set") if !target.is_empty() => state = GoalState::Active(target),
            Some("park") if !target.is_empty() => {
                state = GoalState::Parked(target, r.reason.clone().unwrap_or_default())
            }
            Some("unpark") if !target.is_empty() => state = GoalState::Active(target),
            Some("abandon") | Some("complete") => state = GoalState::None,
            _ => {}
        }
    }
    state
}

fn read_goal_receipts(root: &Path) -> Vec<Receipt> {
    let text = std::fs::read_to_string(root.join(LEDGER)).unwrap_or_default();
    parse_receipts(&text)
}

fn goal_receipt(component: &str, target: &str, disposition: &str, extra: &[(&str, String)]) -> Receipt {
    let mut r = Receipt {
        phase: "goal".to_string(),
        component: Some(component.to_string()),
        disposition: disposition.to_string(),
        evidence: None,
        reason: None,
        authorization: None,
        tool: Some(format!("host-lifecycle@{}", env!("CARGO_PKG_VERSION"))),
        recorded: Some(today()),
        revision: None,
        conclusion: None,
        target: Some(target.to_string()),
    };
    for (k, v) in extra {
        match *k {
            "reason" => r.reason = Some(v.clone()),
            "conclusion" => r.conclusion = Some(v.clone()),
            _ => {}
        }
    }
    r
}

/// The sidecar: `[claim]` + `target = plan/NNNN-slug`. Tool-written, gitignored,
/// machine state only. Absent means no claim (the tracked record still stands).
fn read_claim(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join(SIDECAR)).ok()?;
    for line in text.lines() {
        if let Some(v) = line.trim().strip_prefix("target = ") {
            return Some(v.trim().to_string());
        }
    }
    None
}

fn write_claim(root: &Path, target: Option<&str>) {
    let path = root.join(SIDECAR);
    match target {
        Some(t) => {
            let body = format!(
                "# Tool-written (plan/0101): this worktree's claim on the repo's active goal.\n# Re-derivable: host-lifecycle goal repair <dir>. Never hand-edit.\n[claim]\n    target = {t}\n"
            );
            if let Err(e) = std::fs::write(&path, body) {
                eprintln!("host-lifecycle: goal: cannot write {SIDECAR}: {e}");
                process::exit(2);
            }
        }
        None => {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Maintain the milestone README's Status line: strip any goal suffix the tool
/// previously appended, then append the new one (None leaves the line bare).
/// The operator's own status prose is never touched: only the ` — ` marker
/// suffixes this subsystem owns are rewritten.
const SUFFIX_TOKENS: [&str; 4] = [
    "active goal since ",
    "parked (",
    "complete 2",
    "abandoned 2",
];

fn strip_goal_suffix(status: &str) -> String {
    let mut cut = None;
    for (idx, _) in status.match_indices(" — ") {
        let rest = &status[idx + 5..];
        if SUFFIX_TOKENS.iter().any(|t| rest.starts_with(t)) {
            cut = Some(idx);
        }
    }
    match cut {
        Some(i) => status[..i].trim_end().to_string(),
        None => status.trim_end().to_string(),
    }
}

fn set_status_suffix(readme: &str, suffix: Option<&str>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut replaced = false;
    for line in readme.lines() {
        if line.trim_start().starts_with("**Status:") {
            let base = strip_goal_suffix(line);
            out.push(match suffix {
                Some(s) => format!("{base} — {s}"),
                None => base,
            });
            replaced = true;
        } else {
            out.push(line.to_string());
        }
    }
    if !replaced {
        // Insert a Status line under the title heading so the declaration is
        // always machine-findable; the suffix alone carries the goal state.
        if let Some(first) = readme.lines().next() {
            out.insert(
                1,
                match suffix {
                    Some(s) => format!("**Status:** — {s}"),
                    None => "**Status:**".to_string(),
                },
            );
            let _ = first;
        }
    }
    let mut text = out.join("\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

fn readme_path(root: &Path, target: &str) -> PathBuf {
    root.join(target).join("README.md")
}

fn valid_target(root: &Path, target: &str) -> bool {
    match target.strip_prefix("plan/") {
        Some(rest) => {
            !rest.is_empty()
                && rest.chars().next().is_some_and(|c| c.is_ascii_digit())
                && root.join(target).is_dir()
        }
        None => false,
    }
}

/// The first sentence of the milestone's Why section: the purpose line, human
/// and short. Hard-wrapped paragraphs are joined before the sentence is cut.
/// Falls back to the title when no Why section exists.
fn purpose_of(readme: &str) -> String {
    let mut in_why = false;
    let mut para: Vec<String> = Vec::new();
    for line in readme.lines() {
        let t = line.trim();
        if t.starts_with("## ") {
            if in_why && !para.is_empty() {
                break;
            }
            in_why = t == "## Why";
            continue;
        }
        if !in_why {
            continue;
        }
        if t.is_empty() {
            if !para.is_empty() {
                break;
            }
        } else {
            para.push(t.to_string());
        }
    }
    if para.is_empty() {
        return readme
            .lines()
            .next()
            .map(|l| truncate(l.trim_start_matches("# ").trim(), 120))
            .unwrap_or_else(|| "(none stated)".to_string());
    }
    let joined = para.join(" ");
    let sentence: String = joined.chars().take_while(|&c| c != '.').collect();
    let clean = sentence
        .trim_start_matches("- ")
        .replace(['`', '*', '[', ']'], "");
    let s = clean.trim();
    if s.is_empty() {
        "(none stated)".to_string()
    } else {
        truncate(s, 120)
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let cut: String = s.chars().take(n).collect();
        format!("{cut} …")
    }
}

struct MilestoneStanding {
    total: usize,
    discharged: usize,
    frontier: Vec<String>,
    open: Vec<String>,
}

fn milestone_standing(root: &Path, target: &str) -> Result<MilestoneStanding, String> {
    let docs = tracked_markdown(root)?;
    let (tasks, _) = parse_tasks(&docs);
    let receipts = read_task_receipts(root);
    let mine: Vec<_> = tasks.into_iter().filter(|t| t.rel.starts_with(&format!("{target}/"))).collect();
    let discharged_set: std::collections::HashSet<String> = mine
        .iter()
        .filter(|t| {
            latest_task_receipt(&receipts, &t.key)
                .is_some_and(|r| r.disposition == "done" || r.disposition == "skip")
        })
        .map(|t| t.key.clone())
        .collect();
    let frontier = mine
        .iter()
        .filter(|t| {
            !discharged_set.contains(&t.key)
                && t.depends.iter().all(|d| discharged_set.contains(d.as_str()))
        })
        .map(|t| t.key.clone())
        .collect();
    let open = mine
        .iter()
        .filter(|t| !discharged_set.contains(&t.key))
        .map(|t| t.key.clone())
        .collect();
    Ok(MilestoneStanding {
        total: mine.len(),
        discharged: discharged_set.len(),
        frontier,
        open,
    })
}

fn plan_row_present(root: &Path, target: &str) -> bool {
    std::fs::read_to_string(root.join("PLAN.md"))
        .map(|t| t.contains(target))
        .unwrap_or(false)
}

struct CiStanding {
    done: usize,
    open: Vec<String>,
}

fn ci_standing(root: &Path) -> CiStanding {
    let receipts = read_goal_receipts(root);
    let mut standing = CiStanding { done: 0, open: Vec::new() };
    for r in receipts.iter().filter(|r| r.phase == "ci") {
        let name = r.component.clone().unwrap_or_else(|| "(unnamed)".to_string());
        if r.disposition == "done" {
            standing.done += 1;
        } else {
            standing
                .open
                .push(format!("{name} (recorded {})", r.recorded.as_deref().unwrap_or("?")));
        }
    }
    standing
}

fn git_out(root: &Path, args: &[&str]) -> Option<String> {
    std::process::Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn tree_line(root: &Path) -> String {
    let head = git_out(root, &["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "(no git)".into());
    let dirty = std::process::Command::new("git")
        .current_dir(root)
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    let unpushed = git_out(root, &["rev-list", "--count", "@{u}..HEAD"])
        .and_then(|s| s.parse::<usize>().ok());
    match (dirty, unpushed) {
        (0, Some(0)) => format!("clean at {head}, nothing unpushed"),
        (0, Some(n)) => format!("clean at {head}, {n} commit(s) unpushed"),
        (0, None) => format!("clean at {head}, no upstream tracked"),
        (n, u) => format!("dirty: {n} uncommitted change(s) at {head}, {} unpushed", u.unwrap_or(0)),
    }
}

/// The orientation emission (plan/0101): labeled lines in a fixed order, house
/// shapes where a house shape exists, the next move on the last line. A pure
/// function of the working tree: no clock, no network. Determinism is
/// same-tree-same-bytes.
fn emission(root: &Path) -> Result<String, String> {
    let receipts = read_goal_receipts(root);
    if let Some((target, kind)) = unresolved_closure(root, &receipts) {
        return Ok(format!(
            "GOAL: {target}\nNEEDS YOU: the {kind} closure is incomplete (the receipt landed, the Status line did not); reconcile it\nNEXT: host-lifecycle goal repair .\n"
        ));
    }
    let state = goal_state(&receipts);
    let mut out = Vec::new();
    match &state {
        GoalState::None => {
            out.push("GOAL: (none)".to_string());
            out.push("NEEDS YOU: no goal is set; cut a milestone, then propose it".to_string());
            out.push("NEXT: host-lifecycle goal propose plan/NNNN-slug .".to_string());
        }
        GoalState::Parked(target, reason) => {
            out.push(purpose_line(root, target)?);
            out.push(format!("GOAL: {target}"));
            out.push(position_line(root, target));
            out.push(format!(
                "NEEDS YOU: the goal is parked ({reason}); resume with unpark, or exit with abandon"
            ));
            out.push(ci_line(root));
            out.push("FRONTIER: (held while parked)".to_string());
            out.push(tree_line(root));
            out.push("NEXT: host-lifecycle goal unpark .".to_string());
        }
        GoalState::Active(target) => {
            out.push(purpose_line(root, target)?);
            out.push(format!("GOAL: {target}"));
            out.push(position_line(root, target));
            out.push("NEEDS YOU: none".to_string());
            let ci = ci_line(root);
            out.push(ci.clone());
            let standing = milestone_standing(root, target);
            match &standing {
                Ok(s) => out.push(frontier_line(s)),
                Err(_) => out.push("FRONTIER: (unreadable: the plan room did not read)".to_string()),
            }
            out.push(tree_line(root));
            let next = if read_goal_receipts(root)
                .iter()
                .any(|r| r.phase == "ci" && r.disposition != "done")
            {
                "host-lifecycle ci .".to_string()
            } else if standing.is_err() {
                "host-lifecycle tasks .".to_string()
            } else if standing.as_ref().is_ok_and(|s| s.frontier.is_empty()) {
                format!("host-lifecycle goal complete {target} .")
            } else {
                format!("work {}", standing.as_ref().unwrap().frontier[0])
            };
            out.push(format!("NEXT: {next}"));
        }
    }
    Ok(out.join("\n") + "\n")
}

fn purpose_line(root: &Path, target: &str) -> Result<String, String> {
    let readme = std::fs::read_to_string(readme_path(root, target))
        .map_err(|e| format!("the goal's README is unreadable ({target}/README.md: {e})"))?;
    Ok(format!("PURPOSE: {}", purpose_of(&readme)))
}

fn position_line(root: &Path, target: &str) -> String {
    match milestone_standing(root, target) {
        Ok(s) => format!(
            "POSITION: {} of {} milestone tasks receipted; PLAN.md row {}",
            s.discharged,
            s.total,
            if plan_row_present(root, target) { "present" } else { "MISSING" }
        ),
        Err(_) => "POSITION: (unreadable: the plan room did not read)".to_string(),
    }
}

fn frontier_line(standing: &MilestoneStanding) -> String {
    if standing.frontier.is_empty() {
        "FRONTIER: (none — every task discharged, or blocked on an undischarged dep)".to_string()
    } else {
        format!(
            "FRONTIER: ready (deps satisfied, may run in parallel): {}",
            standing.frontier.join(", ")
        )
    }
}

fn ci_line(root: &Path) -> String {
    let standing = ci_standing(root);
    if standing.done == 0 && standing.open.is_empty() {
        "CI: no receipts (run: host-lifecycle ci .)".to_string()
    } else if standing.open.is_empty() {
        format!("CI: {} lane(s) receipted done", standing.done)
    } else {
        format!(
            "CI: {} of {} lane receipt(s) open: {}",
            standing.open.len(),
            standing.done + standing.open.len(),
            standing.open.join("; ")
        )
    }
}

/// The composed gate set equals the verify sweep (the fold-in ruling): the
/// software gate, the ci judge, book --check, the refs gate, and obligations
/// per discovered spec, each invoked as a subprocess of this same binary.
fn gate_verbs(root: &Path) -> Vec<(&'static str, Vec<String>)> {
    let mut gates: Vec<(&'static str, Vec<String>)> = vec![
        ("software-check", vec!["software".into(), "--check".into()]),
        ("ci", vec!["ci".into()]),
        ("book", vec!["book".into(), "--check".into()]),
        ("refs", vec!["refs".into(), "--gate".into()]),
    ];
    if let Some(out) = git_out(root, &["ls-files", "--", "*.allium"]) {
        for spec in out.lines().filter(|l| !l.trim().is_empty()) {
            gates.push(("obligations", vec!["obligations".into(), spec.to_string()]));
        }
    }
    gates
}

/// A gate hazard: label, note, remedy command. The verify-setup shape.
struct Hazard {
    label: String,
    note: String,
    remedy: Option<String>,
}

/// The composed closure gate (plan/0101): the tool invokes its own verbs as
/// subprocesses and reads their exits; their HAZARD lines pass through intact.
/// Nothing is written unless every gate passes.
fn complete_gate(root: &Path, target: &str) -> Vec<Hazard> {
    let mut hazards = Vec::new();
    // A present-but-divergent sidecar must not sail through (the cold-read
    // ruling): absent is fine (no claim), a claim on another goal is a lie
    // that looks healthy. The tracked record is the authority.
    if let Some(claim) = read_claim(root) {
        if claim != target {
            hazards.push(Hazard {
                label: "sidecar".into(),
                note: format!("this worktree claims {claim} while the closure names {target}"),
                remedy: Some("host-lifecycle goal repair .".into()),
            });
        }
    }
    if !valid_target(root, target) {
        hazards.push(Hazard {
            label: "goal-plan".into(),
            note: format!("the goal's plan directory is missing ({target})"),
            remedy: Some("host-lifecycle goal repair .".into()),
        });
        return hazards;
    }
    if !plan_row_present(root, target) {
        hazards.push(Hazard {
            label: "plan-row".into(),
            note: format!("PLAN.md does not name {target}"),
            remedy: Some(format!("edit PLAN.md: add the {target} milestone row")),
        });
    }
    if let Ok(standing) = milestone_standing(root, target) {
        if standing.discharged < standing.total {
            let open: Vec<String> = standing.open.iter().take(3).cloned().collect();
            let more = if standing.open.len() > 3 { format!(", +{} more", standing.open.len() - 3) } else { String::new() };
            hazards.push(Hazard {
                label: "tasks".into(),
                note: format!(
                    "{} of {} milestone tasks carry no done receipt (open: {}{})",
                    standing.total - standing.discharged,
                    standing.total,
                    open.join(", "),
                    more
                ),
                remedy: Some(
                    "host-lifecycle tasks --record <plan/NNNN#anchor> --disposition done --evidence <e> (or re-cut the milestone)"
                        .to_string(),
                ),
            });
        }
    } else {
        hazards.push(Hazard {
            label: "tasks".into(),
            note: "the plan room did not read".into(),
            remedy: Some("host-lifecycle tasks .".into()),
        });
    }
    for (label, verb) in gate_verbs(root) {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("host-lifecycle"));
        let mut cmd = process::Command::new(&exe);
        cmd.args(&verb).arg(root);
        if let Ok(o) = cmd.output() {
            if !o.status.success() {
                let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
                // Pass through the child's own hazard lines only: a failing
                // sweep still prints ok lines, and an ok line is never a
                // hazard whatever wrapped it.
                for line in text.lines().filter(|l| l.contains("HAZARD")) {
                    hazards.push(Hazard {
                        label: label.into(),
                        note: line.trim().to_string(),
                        remedy: None,
                    });
                }
            }
        } else {
            hazards.push(Hazard {
                label: label.into(),
                note: "the gate could not run".into(),
                remedy: Some(format!("host-lifecycle {} .", verb.join(" "))),
            });
        }
    }
    let dirty = std::process::Command::new("git")
        .current_dir(root)
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    if dirty > 0 {
        hazards.push(Hazard {
            label: "tree".into(),
            note: format!("the working tree carries {dirty} uncommitted change(s)"),
            remedy: Some("git status --short (commit and push them first)".into()),
        });
    }
    if let Some(n) = git_out(root, &["rev-list", "--count", "@{u}..HEAD"]).and_then(|s| s.parse::<usize>().ok()) {
        if n > 0 {
            hazards.push(Hazard {
                label: "tree".into(),
                note: format!("{n} commit(s) are not pushed"),
                remedy: Some("git push".into()),
            });
        }
    }
    hazards
}

fn print_hazards(hazards: &[Hazard]) {
    println!("REFUSED. Nothing was banked and nothing was cleared.");
    for h in hazards {
        println!("HAZARD   {} — {}", h.label, h.note);
        if let Some(rem) = &h.remedy {
            println!("           remedy: {rem}");
        }
    }
}

/// Append an OWED row to PLAN.md's follow-up table: debt lives where a cold
/// reader looks first (the cast's second convening).
fn append_owed_row(root: &Path, target: &str, reason: &str) -> Result<(), String> {
    let path = root.join("PLAN.md");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("PLAN.md unreadable: {e}"))?;
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let header = lines.iter().position(|l| l.trim() == "| Follow-up | Named in | Disposition |");
    let Some(h) = header else {
        return Err("PLAN.md carries no follow-up table".into());
    };
    let mut last_row = h;
    for (i, l) in lines.iter().enumerate().skip(h + 1) {
        if l.trim_start().starts_with('|') {
            last_row = i;
        } else {
            break;
        }
    }
    lines.insert(
        last_row + 1,
        format!(
            "| The {target} work owed: {reason} | plan goal abandon | Owed since {}; discharge by re-cut or explicit closure |",
            today()
        ),
    );
    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(&path, out).map_err(|e| format!("PLAN.md unwritable: {e}"))
}

/// One front-door definition. The door family is generated, never hand-written
/// (the host#26 fold-in): one source in this binary, per-harness renderings,
/// both flat and hyphenated, `/host-<verb>` fronting the verb it names.
struct Door {
    name: &'static str,
    description: &'static str,
    injection: &'static str,
    doctrine: &'static str,
}

const DOORS: &[Door] = &[
    Door {
        name: "host-goal",
        description: "Orient on the active goal (plan/0101)",
        injection: "host-lifecycle goal show .",
        doctrine: "The block above is the orientation emission. Work the NEXT line. If NEEDS YOU is not none, ask the operator first; nothing else runs. The verbs own the state: never hand-edit a state file.",
    },
    Door {
        name: "host-tasks",
        description: "The receipted task graph and its ready frontier",
        injection: "host-lifecycle tasks .",
        doctrine: "The block above is the task graph. Claim from the ready frontier. Record each done task with the tasks verb; a done receipt needs the task's declared verify.",
    },
    Door {
        name: "host-ci",
        description: "The offline CI judge with remedies",
        injection: "host-lifecycle ci .",
        doctrine: "The block above is the CI verdict. Fix the remedies it names, in order; nothing else clears a red lane.",
    },
];

/// The door body is the prompt: the injected emission, the doctrine, and the
/// re-orient link that closes the loop (door to emission to verb to door).
fn door_body(d: &Door) -> String {
    format!(
        "---\ndescription: {}\n---\n!`{}`\n\n{}\n\nThen re-orient: /host-goal\n",
        d.description, d.injection, d.doctrine
    )
}

fn door_path(root: &Path, harness: &str, name: &str) -> PathBuf {
    let dir = match harness {
        "opencode" => ".opencode/commands",
        _ => ".claude/commands",
    };
    root.join(dir).join(format!("{name}.md"))
}

/// `host-lifecycle install <dir> [--check]` — install (or verify) the door family's
/// per-harness renderings from the one source above. The concept: the tool's
/// interface, installed into the harness's own command surface. Generated artifacts: a
/// mismatch is a HAZARD naming the regenerate command, the `book --check` shape.
pub fn install(args: &[String]) {
    let check = args.iter().any(|a| a == "--check");
    let root = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .cloned()
        .unwrap_or_else(|| ".".to_string());
    let root = PathBuf::from(root);
    let mut bad = 0;
    for harness in ["opencode", "claude"] {
        for d in DOORS {
            let path = door_path(&root, harness, d.name);
            let want = door_body(d);
            if check {
                let have = std::fs::read_to_string(&path).unwrap_or_default();
                if have != want {
                    println!("HAZARD   {harness}/{} — the door is absent or hand-edited", d.name);
                    println!("           remedy: host-lifecycle install .");
                    bad += 1;
                }
            } else {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = std::fs::write(&path, &want) {
                    eprintln!("host-lifecycle: install: cannot write {}: {e}", path.display());
                    process::exit(2);
                }
                println!("door     {harness}/{} (injects: {})", d.name, d.injection);
            }
        }
    }
    if check && bad > 0 {
        process::exit(1);
    }
    if !check {
        println!("install: {} door(s) x 2 harnesses emitted; verify with: host-lifecycle install --check .", DOORS.len());
    }
}

fn usage() -> ! {
    eprintln!("usage: host-lifecycle goal <set|show|propose|park|unpark|abandon|repair|complete> ...");
    eprintln!("  goal set <plan/NNNN-slug> [<dir>]         — declare the active goal (writes the receipt, the Status line, the sidecar)");
    eprintln!("  goal show [<dir>]                          — the orientation emission");
    eprintln!("  goal propose <plan/NNNN-slug> [<dir>]      — print the set command for the operator; writes nothing");
    eprintln!("  goal park --reason <r> [<dir>]             — re-queue (receipt + Status line)");
    eprintln!("  goal unpark [<dir>]                        — resume");
    eprintln!("  goal abandon --reason <r> [<dir>]          — exit honestly (receipt + OWED row in PLAN.md)");
    eprintln!("  goal repair [<dir>]                        — reconcile the sidecar to the tracked record");
    eprintln!("  goal complete [<plan/NNNN-slug>] [<dir>]   — the composed closure gate");
    process::exit(2)
}

/// `goal <verb> ...` — see the verb list in `usage`.
pub fn goal(args: &[String]) {
    let mut positional: Vec<String> = Vec::new();
    let mut reason: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--reason" => {
                reason.clone_from(&args.get(i + 1).cloned());
                i += 2;
            }
            s if !s.starts_with('-') => {
                positional.push(s.to_string());
                i += 1;
            }
            other => {
                eprintln!("host-lifecycle: unknown goal argument `{other}`");
                usage();
            }
        }
    }
    let Some(verb) = positional.first().cloned() else { usage() };
    let rest = &positional[1..];
    let root = rest
        .iter()
        .rev()
        .find(|a| a.contains('/') || Path::new(a).is_dir() || a.as_str() == ".")
        .cloned()
        .unwrap_or_else(|| ".".to_string());
    let root = PathBuf::from(&root);
    let target_arg = rest.iter().find(|a| a.starts_with("plan/"));

    match verb.as_str() {
        "show" => show(&root),
        "set" => {
            let Some(target) = target_arg.cloned() else { usage() };
            set(&root, &target);
        }
        "propose" => {
            let Some(target) = target_arg.cloned() else { usage() };
            propose(&root, &target);
        }
        "park" => park(&root, reason.as_deref()),
        "unpark" => unpark(&root),
        "abandon" => abandon(&root, reason.as_deref()),
        "repair" => repair(&root),
        "complete" => {
            let target = match target_arg.cloned() {
                Some(t) => t,
                None => match goal_state(&read_goal_receipts(&root)) {
                    GoalState::Active(t) | GoalState::Parked(t, _) => t,
                    GoalState::None => {
                        eprintln!("host-lifecycle: goal: no active goal to complete");
                        usage();
                    }
                },
            };
            complete(&root, &target);
        }
        _ => usage(),
    }
}

/// The closure pair check (the freeze bench's crash window): a complete or
/// abandon receipt whose Status-line transition never landed is an UNRESOLVED
/// closure, never a false green. The reader surface reports it and routes to
/// repair; repair finishes the interrupted write.
fn unresolved_closure(root: &Path, receipts: &[Receipt]) -> Option<(String, &'static str)> {
    let last = receipts
        .iter()
        .rev()
        .find(|r| r.phase == "goal" && matches!(r.component.as_deref(), Some("complete") | Some("abandon")));
    let r = last?;
    let target = r.target.clone()?;
    let wanted = match r.component.as_deref() {
        Some("complete") => " — complete 2",
        _ => " — abandoned 2",
    };
    let readme = std::fs::read_to_string(readme_path(root, &target)).ok()?;
    let status_landed = readme
        .lines()
        .any(|l| l.trim_start().starts_with("**Status:") && l.contains(wanted));
    if status_landed {
        None
    } else {
        Some((target, match r.component.as_deref() {
            Some("complete") => "complete",
            _ => "abandoned",
        }))
    }
}

fn show(root: &Path) {
    match emission(root) {
        Ok(text) => print!("{text}"),
        Err(e) => {
            eprintln!("host-lifecycle: goal: REFUSING TO EMIT. {e}");
            // The refusal shares the emission's shape (the first convening's
            // ruling): the move rides the last line, NEXT-labeled, so the
            // weak model reads one form everywhere. Measured: the bare
            // unlabeled command was ignored 4 of 4 times.
            eprintln!("NEXT: host-lifecycle goal repair .");
            process::exit(2);
        }
    }
}

fn set(root: &Path, target: &str) {
    if !valid_target(root, target) {
        eprintln!("host-lifecycle: goal: `{target}` is not a plan/NNNN directory under this tree");
        usage();
    }
    match goal_state(&read_goal_receipts(root)) {
        GoalState::None => {}
        GoalState::Active(t) => {
            eprintln!("host-lifecycle: goal: HAZARD   already-active — {t} is the active goal");
            eprintln!("           remedy: host-lifecycle goal park --reason <r> . (or abandon)");
            process::exit(1);
        }
        GoalState::Parked(t, _) => {
            eprintln!("host-lifecycle: goal: HAZARD   parked — {t} is parked; unpark or abandon it first");
            process::exit(1);
        }
    }
    let receipt = goal_receipt("set", target, "done", &[]);
    if let Err(e) = append_receipt(root, &receipt) {
        eprintln!("host-lifecycle: goal: cannot append receipt: {e}");
        process::exit(2);
    }
    write_readme_status(root, target, Some(&format!("active goal since {}", today())));
    write_claim(root, Some(target));
    println!("goal set: {target} (receipt appended; Status line and sidecar written)");
    println!("NEXT: host-lifecycle goal show .");
}

fn propose(root: &Path, target: &str) {
    if !valid_target(root, target) {
        eprintln!("host-lifecycle: goal: `{target}` is not a plan/NNNN directory under this tree");
        usage();
    }
    println!("host-lifecycle goal set {target} .");
    println!("(propose writes nothing; the operator runs the command above)");
}

fn park(root: &Path, reason: Option<&str>) {
    match goal_state(&read_goal_receipts(root)) {
        GoalState::Active(t) => {
            let reason = reason.unwrap_or("");
            if reason.is_empty() {
                eprintln!("usage: host-lifecycle goal park --reason <r> [<dir>]");
                process::exit(2);
            }
            let receipt = goal_receipt("park", &t, "done", &[("reason", reason.to_string())]);
            if let Err(e) = append_receipt(root, &receipt) {
                eprintln!("host-lifecycle: goal: cannot append receipt: {e}");
                process::exit(2);
            }
            write_readme_status(root, &t, Some(&format!("parked ({reason}) since {}", today())));
            println!("goal park: {t} re-queued ({reason}); the receipt and Status line carry it");
        }
        GoalState::None => {
            eprintln!("host-lifecycle: goal: no active goal to park");
            process::exit(1);
        }
        GoalState::Parked(t, _) => {
            eprintln!("host-lifecycle: goal: {t} is already parked");
            process::exit(1);
        }
    }
}

fn unpark(root: &Path) {
    match goal_state(&read_goal_receipts(root)) {
        GoalState::Parked(t, _) | GoalState::Active(t) => {
            let receipt = goal_receipt("unpark", &t, "done", &[]);
            if let Err(e) = append_receipt(root, &receipt) {
                eprintln!("host-lifecycle: goal: cannot append receipt: {e}");
                process::exit(2);
            }
            write_readme_status(root, &t, Some(&format!("active goal since {}", today())));
            write_claim(root, Some(&t));
            println!("goal unpark: {t} resumed");
        }
        GoalState::None => {
            eprintln!("host-lifecycle: goal: no goal to unpark");
            process::exit(1);
        }
    }
}

fn abandon(root: &Path, reason: Option<&str>) {
    let reason = reason.unwrap_or("");
    if reason.is_empty() {
        eprintln!("usage: host-lifecycle goal abandon --reason <r> [<dir>]");
        process::exit(2);
    }
    match goal_state(&read_goal_receipts(root)) {
        GoalState::Active(t) | GoalState::Parked(t, _) => {
            let receipt = goal_receipt("abandon", &t, "done", &[("reason", reason.to_string())]);
            if let Err(e) = append_receipt(root, &receipt) {
                eprintln!("host-lifecycle: goal: cannot append receipt: {e}");
                process::exit(2);
            }
            if let Err(e) = append_owed_row(root, &t, reason) {
                eprintln!("host-lifecycle: goal: HAZARD   owed-row — {e}");
                eprintln!("           remedy: add the OWED row to PLAN.md's follow-up table by hand, then re-run");
                process::exit(1);
            }
            write_readme_status(root, &t, Some(&format!("abandoned {} (owed; see PLAN.md)", today())));
            write_claim(root, None);
            println!("goal abandon: {t} ({reason}); the OWED row in PLAN.md carries the debt");
        }
        GoalState::None => {
            eprintln!("host-lifecycle: goal: no goal to abandon");
            process::exit(1);
        }
    }
}

fn repair(root: &Path) {
    let receipts = read_goal_receipts(root);
    if let Some((target, kind)) = unresolved_closure(root, &receipts) {
        let today = today();
        let suffix = match kind {
            "complete" => format!("complete {today}"),
            _ => format!("abandoned {today} (owed; see PLAN.md)"),
        };
        write_readme_status(root, &target, Some(&suffix));
        write_claim(root, None);
        println!("goal repair: the interrupted {kind} closure was finished ({target}); the pair now agrees");
        return;
    }
    match goal_state(&receipts) {
        GoalState::Active(t) | GoalState::Parked(t, _) => {
            if !valid_target(root, &t) {
                eprintln!("host-lifecycle: goal: HAZARD   moved — the tracked goal names {t}, which is not a directory here");
                eprintln!("           remedy: if it was renamed, abandon and set anew (both leave receipts); if merged, abandon with the reason");
                process::exit(1);
            }
            let prior = read_claim(root);
            write_claim(root, Some(&t));
            match prior {
                Some(p) if p != t => {
                    println!("goal repair: drifted claim ({p}) reconciled to the tracked record ({t})")
                }
                _ => println!("goal repair: sidecar reconciled to the tracked record ({t})"),
            }
        }
        GoalState::None => {
            write_claim(root, None);
            println!("goal repair: no tracked goal; any stale sidecar claim was cleared");
        }
    }
}

fn write_readme_status(root: &Path, target: &str, suffix: Option<&str>) {
    let path = readme_path(root, target);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let updated = set_status_suffix(&text, suffix);
    if updated != text {
        if let Err(e) = std::fs::write(&path, &updated) {
            eprintln!("host-lifecycle: goal: cannot rewrite {target}/README.md: {e}");
            process::exit(2);
        }
    }
}

fn complete(root: &Path, target: &str) {
    let hazards = complete_gate(root, target);
    if !hazards.is_empty() {
        print_hazards(&hazards);
        process::exit(1);
    }
    let receipt = goal_receipt(
        "complete",
        target,
        "done",
        &[(
            "conclusion",
            "closed by the composed gates: plan-row, tasks, software --check, ci, tree".to_string(),
        )],
    );
    if let Err(e) = append_receipt(root, &receipt) {
        eprintln!("host-lifecycle: goal: cannot append receipt: {e}");
        process::exit(2);
    }
    write_readme_status(root, target, Some(&format!("complete {}", today())));
    write_claim(root, None);
    println!("goal complete: {target} (closure receipt appended; Status line updated)");
    println!("changed: {LEDGER}, {target}/README.md — commit and push the closure, then run: host-lifecycle goal show .");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hl-goal-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fixture(tag: &str) -> PathBuf {
        let root = temp_root(tag);
        let plan = root.join("plan/0100-fixture");
        std::fs::create_dir_all(&plan).unwrap();
        std::fs::write(
            plan.join("README.md"),
            "# plan/0100: fixture\n\n**Status: cut**\n\n## Why\n\nLand the thing without breaking the box. More context follows.\n\n## Build sequence\n\n### First task {#first}\n\n- verify: it works\n\n### Second task {#second}\n\n- depends: plan/0100#first\n- verify: it also works\n",
        )
        .unwrap();
        std::fs::write(root.join("PLAN.md"), "# PLAN\n\n| Follow-up | Named in | Disposition |\n|---|---|---|\n").unwrap();
        // The plan room reads through `git ls-files`, so the fixture is a repo.
        std::process::Command::new("git")
            .args(["-c", "user.email=test@localhost", "-c", "user.name=test", "init", "-q"])
            .current_dir(&root)
            .output()
            .unwrap();
        for args in [["add", "-A"], ["commit", "-qm=fixture"]] {
            std::process::Command::new("git")
                .args(["-c", "user.email=test@localhost", "-c", "user.name=test"])
                .args(args)
                .current_dir(&root)
                .output()
                .unwrap();
        }
        root
    }

    #[test]
    fn goal_state_folds_the_receipt_log() {
        let receipts = parse_receipts(
            "[receipt \"goal\" \"set\"]\n    disposition = done\n    target = plan/0100-fixture\n\n[receipt \"goal\" \"park\"]\n    disposition = done\n    target = plan/0100-fixture\n    reason = upstream block\n",
        );
        assert_eq!(goal_state(&receipts), GoalState::Parked("plan/0100-fixture".into(), "upstream block".into()));
        let receipts = parse_receipts(
            "[receipt \"goal\" \"set\"]\n    disposition = done\n    target = plan/0100-fixture\n\n[receipt \"goal\" \"abandon\"]\n    disposition = done\n    target = plan/0100-fixture\n",
        );
        assert_eq!(goal_state(&receipts), GoalState::None);
    }

    #[test]
    fn status_suffix_round_trips() {
        let base = "**Status: cut and in build** (operator ruling)";
        let once = set_status_suffix(base, Some("active goal since 2026-10-19"));
        assert_eq!(once, format!("{base} — active goal since 2026-10-19\n"));
        let twice = set_status_suffix(&once, Some("parked (waiting) since 2026-10-20"));
        assert_eq!(twice, format!("{base} — parked (waiting) since 2026-10-20\n"));
        let cleared = set_status_suffix(&twice, None);
        assert_eq!(cleared, format!("{base}\n"));
    }

    #[test]
    fn purpose_joins_wrapped_lines_before_the_sentence_cut() {
        let readme = "# plan/0100: x\n\n## Why\n\nLand the thing\nwithout breaking the box. More context follows.\n";
        assert_eq!(purpose_of(readme), "Land the thing without breaking the box");
    }

    #[test]
    fn sidecar_round_trip() {
        let root = temp_root("sidecar");
        write_claim(&root, Some("plan/0100-fixture"));
        assert_eq!(read_claim(&root).as_deref(), Some("plan/0100-fixture"));
        write_claim(&root, None);
        assert_eq!(read_claim(&root), None);
    }

    #[test]
    fn emission_is_deterministic_and_house_shaped() {
        let root = fixture("emission");
        append_receipt(&root, &goal_receipt("set", "plan/0100-fixture", "done", &[])).unwrap();
        let a = emission(&root).unwrap();
        let b = emission(&root).unwrap();
        assert_eq!(a, b);
        assert!(a.starts_with("PURPOSE: Land the thing"));
        assert!(a.contains("GOAL: plan/0100-fixture"));
        assert!(a.contains("POSITION: 0 of 2 milestone tasks receipted; PLAN.md row MISSING"));
        assert!(a.contains("FRONTIER: ready (deps satisfied, may run in parallel): plan/0100#first"));
        assert!(a.ends_with("NEXT: work plan/0100#first\n"));
    }

    #[test]
    fn an_interrupted_closure_reads_unresolved_and_repair_finishes_it() {
        let root = fixture("pair");
        append_receipt(&root, &goal_receipt("set", "plan/0100-fixture", "done", &[])).unwrap();
        // The crash window: the complete receipt landed, the Status rewrite did not.
        append_receipt(&root, &goal_receipt("complete", "plan/0100-fixture", "done", &[])).unwrap();
        let text = emission(&root).unwrap();
        assert!(text.contains("NEEDS YOU: the complete closure is incomplete"), "{text}");
        assert!(text.ends_with("NEXT: host-lifecycle goal repair .\n"));
        repair(&root);
        let after = emission(&root).unwrap();
        assert!(!after.contains("closure pair is incomplete"), "{after}");
    }

    #[test]
    fn complete_refuses_as_a_noop_until_every_gate_passes() {
        let root = fixture("refuse");
        append_receipt(&root, &goal_receipt("set", "plan/0100-fixture", "done", &[])).unwrap();
        let hazards = complete_gate(&root, "plan/0100-fixture");
        assert!(hazards.iter().any(|h| h.label == "plan-row"));
        assert!(hazards.iter().any(|h| h.label == "tasks"));
        assert!(!hazards.is_empty());
        // The refusal banked nothing: no goal-complete receipt exists.
        assert!(read_goal_receipts(&root).iter().all(|r| r.component.as_deref() != Some("complete")));
    }

    #[test]
    fn install_emits_idempotently_and_check_guards_hand_edits() {
        let root = temp_root("install");
        let dir = root.to_string_lossy().to_string();
        install(std::slice::from_ref(&dir));
        let first = std::fs::read_to_string(door_path(&root, "opencode", "host-goal")).unwrap();
        install(&[dir]);
        let second = std::fs::read_to_string(door_path(&root, "opencode", "host-goal")).unwrap();
        assert_eq!(first, second);
        assert!(first.contains("!`host-lifecycle goal show .`"));
        assert!(std::fs::read_to_string(door_path(&root, "claude", "host-ci")).unwrap().contains("Then re-orient: /host-goal"));
        std::fs::write(door_path(&root, "claude", "host-ci"), "hand edit\n").unwrap();
        // A hand edit no longer matches the one source: --check's comparison basis.
        let path = door_path(&root, "claude", "host-ci");
        let want = door_body(&DOORS[2]);
        let mismatch = std::fs::read_to_string(&path).unwrap() != want;
        assert!(mismatch, "the hand edit must not match the source");
    }
}
