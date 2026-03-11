/// Workspace-wide reconciliation of cross-file checkbox states.
///
/// Walks all notes, builds a dependency graph (A depends on B if A has
/// `- [ ] @B` on a todo line), computes SCCs via Tarjan's algorithm, resolves
/// a fixed-point in topological order, then rewrites changed files.
use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;

use crate::config::WikiConfig;
use crate::handlers::formatting::{is_note_done, normalize_note};
use crate::parser;

struct NoteRec {
    path: PathBuf,
    content: String,
}

pub struct ReconcileStats {
    pub rounds: usize,
    pub files_changed: usize,
    pub converged: bool,
}

// ---------------------------------------------------------------------------
// Scan
// ---------------------------------------------------------------------------

async fn scan_notes(note_dir: &std::path::Path) -> Result<HashMap<String, NoteRec>> {
    let mut map = HashMap::new();
    let mut rd = tokio::fs::read_dir(note_dir).await?;
    while let Some(entry) = rd.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("typ") {
            continue;
        }
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        // Only 10-digit IDs
        if stem.len() != 10 || !stem.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => {
                map.insert(stem, NoteRec { path, content });
            }
            Err(_) => continue,
        }
    }
    Ok(map)
}

// ---------------------------------------------------------------------------
// Dependency extraction
// ---------------------------------------------------------------------------

fn extract_todo_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in content.lines() {
        let t = line.trim_start();
        if !(t.starts_with("- [") && t.len() >= 5) {
            continue;
        }
        for cap in parser::RE_ID_REF.captures_iter(line) {
            if let Some(m) = cap.get(1) {
                let id = m.as_str().to_string();
                if !deps.contains(&id) {
                    deps.push(id);
                }
            }
        }
    }
    deps
}

fn build_dep_graph(notes: &HashMap<String, NoteRec>) -> HashMap<String, Vec<String>> {
    notes
        .iter()
        .map(|(id, rec)| (id.clone(), extract_todo_deps(&rec.content)))
        .collect()
}

// ---------------------------------------------------------------------------
// Tarjan SCC
// ---------------------------------------------------------------------------

struct TarjanState<'a> {
    graph: &'a HashMap<String, Vec<String>>,
    index_counter: usize,
    stack: Vec<String>,
    on_stack: HashMap<String, bool>,
    index: HashMap<String, usize>,
    lowlink: HashMap<String, usize>,
    sccs: Vec<Vec<String>>,
}

impl<'a> TarjanState<'a> {
    fn strongconnect(&mut self, v: &str) {
        self.index.insert(v.to_string(), self.index_counter);
        self.lowlink.insert(v.to_string(), self.index_counter);
        self.index_counter += 1;
        self.stack.push(v.to_string());
        self.on_stack.insert(v.to_string(), true);

        if let Some(neighbors) = self.graph.get(v) {
            let neighbors: Vec<String> = neighbors.clone();
            for w in neighbors {
                if !self.index.contains_key(&w) {
                    self.strongconnect(&w);
                    let lv = *self.lowlink.get(v).unwrap();
                    let lw = *self.lowlink.get(&w).unwrap_or(&usize::MAX);
                    self.lowlink.insert(v.to_string(), lv.min(lw));
                } else if *self.on_stack.get(&w).unwrap_or(&false) {
                    let lv = *self.lowlink.get(v).unwrap();
                    let iw = *self.index.get(&w).unwrap();
                    self.lowlink.insert(v.to_string(), lv.min(iw));
                }
            }
        }

        if self.lowlink.get(v) == self.index.get(v) {
            let mut scc = Vec::new();
            loop {
                let w = self.stack.pop().unwrap();
                self.on_stack.insert(w.clone(), false);
                scc.push(w.clone());
                if w == v {
                    break;
                }
            }
            self.sccs.push(scc);
        }
    }
}

fn tarjan_sccs(
    graph: &HashMap<String, Vec<String>>,
    all_nodes: &[String],
) -> Vec<Vec<String>> {
    let mut state = TarjanState {
        graph,
        index_counter: 0,
        stack: Vec::new(),
        on_stack: HashMap::new(),
        index: HashMap::new(),
        lowlink: HashMap::new(),
        sccs: Vec::new(),
    };
    for v in all_nodes {
        if !state.index.contains_key(v.as_str()) {
            state.strongconnect(v);
        }
    }
    state.sccs
}

// ---------------------------------------------------------------------------
// SCC DAG + topological order
// ---------------------------------------------------------------------------

fn scc_dag(
    sccs: &[Vec<String>],
    graph: &HashMap<String, Vec<String>>,
) -> HashMap<usize, Vec<usize>> {
    // Map each node to its SCC index
    let mut node_to_scc: HashMap<&str, usize> = HashMap::new();
    for (i, scc) in sccs.iter().enumerate() {
        for node in scc {
            node_to_scc.insert(node.as_str(), i);
        }
    }

    let mut dag: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..sccs.len() {
        dag.entry(i).or_default();
    }

    for (i, scc) in sccs.iter().enumerate() {
        for node in scc {
            if let Some(neighbors) = graph.get(node) {
                for nb in neighbors {
                    if let Some(&j) = node_to_scc.get(nb.as_str()) {
                        if j != i {
                            let edges = dag.entry(i).or_default();
                            if !edges.contains(&j) {
                                edges.push(j);
                            }
                        }
                    }
                }
            }
        }
    }
    dag
}

fn topo_order(dag: &HashMap<usize, Vec<usize>>, n: usize) -> Vec<usize> {
    let mut in_degree = vec![0usize; n];
    for neighbors in dag.values() {
        for &j in neighbors {
            in_degree[j] += 1;
        }
    }
    let mut queue: std::collections::VecDeque<usize> = (0..n)
        .filter(|&i| in_degree[i] == 0)
        .collect();
    let mut order = Vec::new();
    while let Some(i) = queue.pop_front() {
        order.push(i);
        if let Some(neighbors) = dag.get(&i) {
            for &j in neighbors {
                in_degree[j] -= 1;
                if in_degree[j] == 0 {
                    queue.push_back(j);
                }
            }
        }
    }
    // If there are cycles in the DAG (shouldn't happen after SCC), append remaining
    for i in 0..n {
        if !order.contains(&i) {
            order.push(i);
        }
    }
    order
}

// ---------------------------------------------------------------------------
// Fixed-point solver
// ---------------------------------------------------------------------------

fn compute_node_done(content: &str, dep_states: &HashMap<String, bool>) -> bool {
    let updated = normalize_note(content, dep_states);
    is_note_done(&updated)
}

fn solve_scc(
    scc: &[String],
    external: &HashMap<String, bool>,
    notes: &HashMap<String, NoteRec>,
) -> HashMap<String, bool> {
    let mut internal: HashMap<String, bool> = scc
        .iter()
        .map(|id| (id.clone(), false))
        .collect();

    // Fixed-point iteration (max 100 rounds to guard against bugs)
    for _ in 0..100 {
        let mut new_states: HashMap<String, bool> = HashMap::new();
        for id in scc {
            let content = match notes.get(id) {
                Some(r) => r.content.as_str(),
                None => {
                    new_states.insert(id.clone(), false);
                    continue;
                }
            };
            let mut combined = external.clone();
            combined.extend(internal.iter().map(|(k, v)| (k.clone(), *v)));
            new_states.insert(id.clone(), compute_node_done(content, &combined));
        }
        if new_states == internal {
            break;
        }
        internal = new_states;
    }
    internal
}

fn solve_global(notes: &HashMap<String, NoteRec>) -> HashMap<String, bool> {
    let graph = build_dep_graph(notes);
    let all_nodes: Vec<String> = notes.keys().cloned().collect();
    let sccs = tarjan_sccs(&graph, &all_nodes);

    // Build DAG and process in reverse topological order (dependencies first)
    let dag = scc_dag(&sccs, &graph);
    let order = topo_order(&dag, sccs.len());

    let mut global: HashMap<String, bool> = HashMap::new();
    // Process in reverse order so that dependencies come first
    for &scc_idx in order.iter().rev() {
        let scc = &sccs[scc_idx];
        let external: HashMap<String, bool> = global
            .iter()
            .filter(|(id, _)| !scc.contains(id))
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        let result = solve_scc(scc, &external, notes);
        global.extend(result);
    }
    global
}

// ---------------------------------------------------------------------------
// Rewrite + convergence loop
// ---------------------------------------------------------------------------

pub async fn run_reconcile(
    config: &WikiConfig,
    dry_run: bool,
    max_rounds: usize,
) -> Result<ReconcileStats> {
    let mut total_changed = 0usize;

    for round in 0..max_rounds {
        let notes = scan_notes(&config.note_dir).await?;
        let global = solve_global(&notes);

        let mut round_changed = 0usize;
        for (_id, rec) in &notes {
            let dep_states: HashMap<String, bool> = extract_todo_deps(&rec.content)
                .into_iter()
                .map(|dep_id| {
                    let done = global.get(&dep_id).copied().unwrap_or(false);
                    (dep_id, done)
                })
                .collect();

            let new_content = normalize_note(&rec.content, &dep_states);
            if new_content != rec.content {
                round_changed += 1;
                if !dry_run {
                    tokio::fs::write(&rec.path, new_content.as_bytes()).await?;
                } else {
                    eprintln!("  would update: {}", rec.path.display());
                }
            }
        }

        total_changed += round_changed;

        if round_changed == 0 {
            return Ok(ReconcileStats {
                rounds: round + 1,
                files_changed: total_changed,
                converged: true,
            });
        }
    }

    Ok(ReconcileStats {
        rounds: max_rounds,
        files_changed: total_changed,
        converged: false,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::formatting::normalize_note;

    fn make_toml_note(title: &str, id: &str, status: &str, body: &str) -> String {
        format!(
            "#import \"../include.typ\": *\n\
             #let zk-metadata = toml(bytes(\n\
             \x20 ```toml\n\
             \x20 schema-version = 1\n\
             \x20 title = \"{title}\"\n\
             \x20 tags = []\n\
             \x20 checklist-status = \"{status}\"\n\
             \x20 generated = false\n\
             \x20 ```.text,\n\
             ))\n\
             #show: zettel.with(metadata: zk-metadata)\n\
             \n\
             = {title} <{id}>\n\
             {body}"
        )
    }

    #[test]
    fn normalize_note_is_pure() {
        let content = "- [ ] @1234567890 do thing\n";
        let mut dep_states = HashMap::new();
        dep_states.insert("1234567890".to_string(), true);
        let result = normalize_note(content, &dep_states);
        assert!(result.contains("- [x]"), "checkbox should be checked");
    }

    #[test]
    fn scc_cycle_no_spurious_done() {
        // A depends on B, B depends on A; neither has local tasks done
        let content_a = "- [ ] @2222222222\n";
        let content_b = "- [ ] @1111111111\n";
        let mut notes = HashMap::new();
        notes.insert(
            "1111111111".to_string(),
            NoteRec { path: PathBuf::from("1111111111.typ"), content: content_a.to_string() },
        );
        notes.insert(
            "2222222222".to_string(),
            NoteRec { path: PathBuf::from("2222222222.typ"), content: content_b.to_string() },
        );
        let global = solve_global(&notes);
        assert!(!global.get("1111111111").copied().unwrap_or(true));
        assert!(!global.get("2222222222").copied().unwrap_or(true));
    }

    #[test]
    fn chain_propagation() {
        // A: done (TOML status = "done"); B references A; C references B
        let content_a = make_toml_note("A", "1010101010", "done", "");
        let content_b = make_toml_note("B", "2020202020", "none", "- [ ] @1010101010\n");
        let content_c = make_toml_note("C", "3030303030", "none", "- [ ] @2020202020\n");

        let mut dep_a: HashMap<String, bool> = HashMap::new();
        let normalized_a = normalize_note(&content_a, &dep_a);
        assert!(is_note_done(&normalized_a), "A should be done");

        dep_a.insert("1010101010".to_string(), true);
        let normalized_b = normalize_note(&content_b, &dep_a);
        assert!(
            normalized_b.contains("- [x]"),
            "B's ref to A should be checked"
        );
        let b_done = is_note_done(&normalized_b);

        let mut dep_b: HashMap<String, bool> = HashMap::new();
        dep_b.insert("2020202020".to_string(), b_done);
        let normalized_c = normalize_note(&content_c, &dep_b);
        if b_done {
            assert!(
                normalized_c.contains("- [x]"),
                "C's ref to B should be checked when B is done"
            );
        }
    }

    #[test]
    fn reconcile_idempotent_in_memory() {
        // Simulate two rounds of solve_global: second should yield no changes
        let content_a = make_toml_note("A", "4040404040", "done", "");
        let content_b = make_toml_note("B", "5050505050", "none", "- [ ] @4040404040\n");

        let mut notes = HashMap::new();
        notes.insert(
            "4040404040".to_string(),
            NoteRec { path: PathBuf::from("4040404040.typ"), content: content_a.clone() },
        );
        notes.insert(
            "5050505050".to_string(),
            NoteRec { path: PathBuf::from("5050505050.typ"), content: content_b.clone() },
        );

        let global = solve_global(&notes);

        // Apply first round of rewrites in memory
        let mut updated: HashMap<String, String> = HashMap::new();
        for (id, rec) in &notes {
            let deps: HashMap<String, bool> = extract_todo_deps(&rec.content)
                .into_iter()
                .map(|dep_id| (dep_id.clone(), global.get(&dep_id).copied().unwrap_or(false)))
                .collect();
            updated.insert(id.clone(), normalize_note(&rec.content, &deps));
        }

        // Build second notes map from updated content
        let mut notes2: HashMap<String, NoteRec> = HashMap::new();
        for (id, content) in &updated {
            notes2.insert(
                id.clone(),
                NoteRec {
                    path: notes[id].path.clone(),
                    content: content.clone(),
                },
            );
        }

        let global2 = solve_global(&notes2);
        let mut changed = 0usize;
        for (id, rec) in &notes2 {
            let deps: HashMap<String, bool> = extract_todo_deps(&rec.content)
                .into_iter()
                .map(|dep_id| (dep_id.clone(), global2.get(&dep_id).copied().unwrap_or(false)))
                .collect();
            let new_content = normalize_note(&rec.content, &deps);
            if new_content != rec.content {
                changed += 1;
                eprintln!("id={id} changed:\n---\n{}\n---\n{}\n---", rec.content, new_content);
            }
        }
        assert_eq!(changed, 0, "second round should produce no changes");
    }
}
