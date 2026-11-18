//! Shared workspace dependency-graph audit for `FND-T09` and `F3D-T13`
//! (`SPN-INV-001`, `SPN-INV-002`, `FND-INV-01`, `F3D-INV-01`).
//!
//! Edges come from Cargo's own resolver (`cargo tree --invert`), never from
//! hand-parsing `Cargo.toml` dependency tables, so every valid manifest form
//! is covered: `[dependencies]`, `[dependencies.<name>]` tables, renamed
//! dependencies (`alias = { package = "<name>" }`), `[build-dependencies]`,
//! and `[target.'cfg(..)'.dependencies]` for every target (`--target all`).
//! Dev-dependencies are excluded (`-e normal,build`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Resolved `spawn-*` workspace graph: `edges[a]` holds the workspace packages
/// that `a` depends on directly (normal + build edges, all targets).
pub struct WorkspaceGraph {
    pub edges: BTreeMap<String, BTreeSet<String>>,
    pub engine_crates: BTreeSet<String>,
    pub tool_crates: BTreeSet<String>,
}

/// Discovers the root, `crates/*` and `tools/*` packages and resolves their
/// direct dependency edges through `cargo tree --invert --depth 1`.
pub fn resolved_workspace_graph(root: &Path) -> Result<WorkspaceGraph, Box<dyn Error>> {
    let mut packages = BTreeSet::new();
    let mut engine_crates = BTreeSet::new();
    let mut tool_crates = BTreeSet::new();

    packages.insert(package_name(&root.join("Cargo.toml"))?);
    for (group, members) in [("crates", &mut engine_crates), ("tools", &mut tool_crates)] {
        for entry in fs::read_dir(root.join(group))? {
            let manifest = entry?.path().join("Cargo.toml");
            if manifest.is_file() {
                let name = package_name(&manifest)?;
                members.insert(name.clone());
                packages.insert(name);
            }
        }
    }

    let mut edges: BTreeMap<String, BTreeSet<String>> = packages
        .iter()
        .map(|name| (name.clone(), BTreeSet::new()))
        .collect();
    for target in &packages {
        let inverted = inverted_direct_dependents(root, target)?;
        add_inverted_tree_edges(&mut edges, target, &inverted);
    }

    Ok(WorkspaceGraph {
        edges,
        engine_crates,
        tool_crates,
    })
}

/// Records `dependent -> target` for every package listed in an inverted
/// `cargo tree -i <target> --depth 1 --prefix none` output (the first line is
/// `target` itself). Only names already present in `edges` (workspace
/// packages) are recorded.
pub fn add_inverted_tree_edges(
    edges: &mut BTreeMap<String, BTreeSet<String>>,
    target: &str,
    inverted_tree: &str,
) {
    for line in inverted_tree.lines() {
        let Some(name) = line.split_whitespace().next() else {
            continue;
        };
        if name == target {
            continue;
        }
        if let Some(deps) = edges.get_mut(name) {
            deps.insert(target.to_owned());
        }
    }
}

/// Engine crates (`crates/*`) never depend on the root composition package
/// `spawn-engine` or on any `tools/*` package (`SPN-INV-001`).
pub fn assert_no_upward_edges(graph: &WorkspaceGraph) -> Result<(), String> {
    for crate_name in &graph.engine_crates {
        let Some(deps) = graph.edges.get(crate_name) else {
            continue;
        };
        if deps.contains("spawn-engine") {
            return Err(format!(
                "{crate_name} must not depend on composition root spawn-engine"
            ));
        }
        for tool in &graph.tool_crates {
            if deps.contains(tool) {
                return Err(format!("{crate_name} must not depend on tool crate {tool}"));
            }
        }
    }
    Ok(())
}

/// Kahn's algorithm: the workspace `spawn-*` graph is acyclic (`SPN-INV-002`).
pub fn assert_acyclic(edges: &BTreeMap<String, BTreeSet<String>>) {
    let mut in_degree: BTreeMap<&str, usize> =
        edges.keys().map(|node| (node.as_str(), 0)).collect();
    for targets in edges.values() {
        for target in targets {
            if let Some(deg) = in_degree.get_mut(target.as_str()) {
                *deg += 1;
            }
        }
    }
    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter_map(|(&node, &deg)| (deg == 0).then_some(node))
        .collect();
    let mut visited = 0;
    while let Some(node) = queue.pop_front() {
        visited += 1;
        if let Some(targets) = edges.get(node) {
            for target in targets {
                if let Some(deg) = in_degree.get_mut(target.as_str()) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(target.as_str());
                    }
                }
            }
        }
    }
    assert_eq!(
        visited,
        edges.len(),
        "cycle detected in workspace spawn-* dependency graph: {edges:?}"
    );
}

fn inverted_direct_dependents(root: &Path, target: &str) -> Result<String, Box<dyn Error>> {
    let output = Command::new(env!("CARGO"))
        .current_dir(root)
        .env("CARGO_TERM_COLOR", "never")
        .args([
            "tree",
            "--workspace",
            "--invert",
            target,
            "-e",
            "normal,build",
            "--target",
            "all",
            "--depth",
            "1",
            "--prefix",
            "none",
            "--color",
            "never",
        ])
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "cargo tree --invert {target} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let stdout = String::from_utf8(output.stdout)?;
    if stdout.contains('\x1b') {
        return Err(format!("cargo tree --invert {target} emitted ANSI escapes:\n{stdout}").into());
    }
    Ok(stdout)
}

/// Reads only the mandatory `[package] name` key; dependency tables are never
/// parsed by hand.
fn package_name(manifest: &Path) -> Result<String, Box<dyn Error>> {
    let content = fs::read_to_string(manifest)?;
    let mut in_package = false;
    for raw_line in content.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == "name"
        {
            return Ok(value.trim().trim_matches('"').to_owned());
        }
    }
    Err(format!("{} has no [package] name", manifest.display()).into())
}
