//! `F3D-T13` (Phase 5 `spawn-engine` slice; `F3D-NFR-01..05, 07`,
//! `F3D-INV-01..04, 11, 15, 16`, `SPN-INV-001..006, 009, 011, 023, 030, 084, 100, 101`):
//! automated dependency-graph, `#![forbid(unsafe_code)]`, panic-free source,
//! genre-free/3rd-party-free public API, uncommitted-shader-binary, and
//! documentation-synchronization audit for the `spawn-engine` composition root.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn f3d_t13_spawn_engine_boundaries_safety_genre_neutrality_and_doc_sync()
-> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // 1. Parse every workspace manifest and verify Phase 5 edges + acyclicity.
    let mut manifests = vec![root.join("Cargo.toml")];
    for group in ["crates", "tools"] {
        for entry in fs::read_dir(root.join(group))? {
            let path = entry?.path().join("Cargo.toml");
            if path.is_file() {
                manifests.push(path);
            }
        }
    }

    let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for manifest in &manifests {
        let content = fs::read_to_string(manifest)?;
        let (pkg_name, deps) = parse_manifest_spawn_deps(&content);
        edges.insert(pkg_name, deps);
    }

    assert_eq!(
        edges["spawn-engine"],
        BTreeSet::from([
            "spawn-core".to_owned(),
            "spawn-input".to_owned(),
            "spawn-math".to_owned(),
            "spawn-platform".to_owned(),
            "spawn-render".to_owned(),
        ]),
        "spawn-engine must depend only on spawn-core, spawn-input, spawn-math, spawn-platform, spawn-render"
    );
    assert!(edges["spawn-math"].is_empty());
    assert!(edges["spawn-core"].is_empty());
    assert_eq!(
        edges["spawn-render"],
        BTreeSet::from(["spawn-math".to_owned()])
    );
    assert_eq!(
        edges["spawn-cli"],
        BTreeSet::from(["spawn-render".to_owned()])
    );
    assert_acyclic(&edges);

    // 2. `cargo metadata --no-deps` and `cargo tree -p spawn-engine -e normal`
    //    must contain zero tools, test-only crates, or rejected/unapproved crates.
    let metadata_out = Command::new(env!("CARGO"))
        .current_dir(&root)
        .args(["metadata", "--format-version=1", "--no-deps"])
        .output()?;
    assert!(
        metadata_out.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&metadata_out.stderr)
    );

    let engine_tree = run_cargo_tree(&root, &["-p", "spawn-engine", "-e", "normal"])?;
    for required in [
        "spawn-core",
        "spawn-input",
        "spawn-math",
        "spawn-platform",
        "spawn-render",
        "libm",
    ] {
        assert!(
            engine_tree.contains(required),
            "spawn-engine normal tree missing `{required}`:\n{engine_tree}"
        );
    }
    for forbidden in [
        "spawn-cli",
        "spawn-editor",
        "spawn-mcp",
        "glam",
        "proptest",
        "insta",
        "serde_json",
        "tempfile",
        "thiserror",
        "anyhow",
        "wgpu",
        "gpu-allocator",
        "shaderc",
        "naga",
        "spirv-reflect",
    ] {
        assert!(
            !engine_tree.contains(forbidden),
            "spawn-engine normal tree must not contain `{forbidden}`:\n{engine_tree}"
        );
    }
    if cfg!(target_os = "linux") {
        for linux_backend in ["ash", "wayland-client", "wayland-protocols"] {
            assert!(
                engine_tree.contains(linux_backend),
                "spawn-engine on Linux must enable `{linux_backend}`:\n{engine_tree}"
            );
        }
    } else {
        for linux_backend in ["ash", "wayland-client", "wayland-protocols"] {
            assert!(
                !engine_tree.contains(linux_backend),
                "spawn-engine on non-Linux targets must not pull Linux backend `{linux_backend}`:\n{engine_tree}"
            );
        }
    }

    // 3. Zero `build.rs` scripts across root and Milestone 2 packages (`SPN-INV-084`),
    //    and zero tracked `.spv` or `.spnshdr` shader binaries in git.
    for rel in [
        "",
        "crates/spawn-core",
        "crates/spawn-math",
        "crates/spawn-platform",
        "crates/spawn-input",
        "crates/spawn-render",
        "tools/spawn-cli",
    ] {
        let build_rs = if rel.is_empty() {
            root.join("build.rs")
        } else {
            root.join(rel).join("build.rs")
        };
        assert!(
            !build_rs.exists(),
            "unexpected build.rs at {}",
            build_rs.display()
        );
    }

    let ls_files = Command::new("git")
        .current_dir(&root)
        .args(["ls-files"])
        .output()?;
    if ls_files.status.success() {
        let tracked = String::from_utf8_lossy(&ls_files.stdout);
        for file in tracked.lines() {
            assert!(
                !file.ends_with(".spv") && !file.ends_with(".spnshdr"),
                "compiled shader binary `{file}` must never be tracked in git (ADR-0014)"
            );
        }
    }

    // 4. `#![forbid(unsafe_code)]`, zero `unsafe`, zero non-test panic macros/methods,
    //    and zero genre terms or leaked 3rd-party/backend types in `spawn-engine/src/`.
    for entry in fs::read_dir(root.join("src"))? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let content = fs::read_to_string(&path)?;
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if file_name != "main.rs" {
            assert!(
                content
                    .lines()
                    .any(|line| line.trim() == "#![forbid(unsafe_code)]"),
                "{} must declare `#![forbid(unsafe_code)]`",
                path.display()
            );
        }
        audit_engine_source(&path, &content);
    }

    // 5. Governance & documentation sync (`SPN-INV-023`, `SPN-INV-100`, `SPN-INV-101`).
    let root_agents = fs::read_to_string(root.join("AGENTS.md"))?;
    assert!(
        root_agents.contains(
            "`spawn-core`, `spawn-input`, `spawn-math`, `spawn-platform` and `spawn-render`"
        ),
        "root AGENTS.md must record spawn-engine -> spawn-math dependency edge"
    );
    assert!(
        root_agents.contains("`FreeCamera`") && root_agents.contains("`ProceduralScene`"),
        "root AGENTS.md verified state must describe Phase 5 FreeCamera and ProceduralScene"
    );

    let deps_doc = fs::read_to_string(root.join("docs/DEPENDENCIES.md"))?;
    for tool_dep in ["| `proptest` |", "| `insta` |"] {
        let row = deps_doc
            .lines()
            .find(|line| line.starts_with(tool_dep))
            .ok_or_else(|| format!("missing `{tool_dep}` row in docs/DEPENDENCIES.md"))?;
        assert!(
            row.contains("spawn-engine"),
            "docs/DEPENDENCIES.md row `{row}` must record `spawn-engine` usage"
        );
    }

    for adr in [
        "docs/adr/0020-memory-strategy.md",
        "docs/adr/0022-repository-layout.md",
    ] {
        let doc = fs::read_to_string(root.join(adr))?;
        assert!(
            doc.contains("**Status:** Proposed"),
            "{adr} must remain Proposed"
        );
    }

    for milestone_doc in [
        "docs/first-3d/VERIFICATION.md",
        "docs/first-3d/PLAN.md",
        "docs/first-3d/README.md",
        "docs/first-3d/SPEC.md",
        "docs/first-3d/DESIGN.md",
        "docs/first-3d/INVARIANTS.md",
        "docs/README.md",
    ] {
        let doc = fs::read_to_string(root.join(milestone_doc))?;
        assert!(
            doc.contains("IN PROGRESS — PENDING OWNER VERIFICATION & SIGN-OFF"),
            "{milestone_doc} must keep Phase 6 / Gate G3 open pending owner verification & sign-off"
        );
        assert!(
            !doc.contains("satisfied for Linux/Wayland") && !doc.contains("Gate G3 satisfied"),
            "{milestone_doc} must not prematurely declare Gate G3 satisfied"
        );
    }
    assert!(
        root_agents.contains("IN PROGRESS — PENDING OWNER VERIFICATION & SIGN-OFF"),
        "root AGENTS.md must record Phase 6 / Gate G3 as pending owner verification & sign-off"
    );

    Ok(())
}

fn run_cargo_tree(root: &Path, args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let out = Command::new(env!("CARGO"))
        .current_dir(root)
        .env("CARGO_TERM_COLOR", "never")
        .arg("tree")
        .args(["--color", "never"])
        .args(args)
        .output()?;
    assert!(
        out.status.success(),
        "cargo tree {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8(out.stdout)?)
}

fn parse_manifest_spawn_deps(content: &str) -> (String, BTreeSet<String>) {
    let mut section = "";
    let mut name = String::new();
    let mut deps = BTreeSet::new();

    for raw_line in content.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = &line[1..line.len() - 1];
            continue;
        }
        if section == "package"
            && let Some(rest) = line.strip_prefix("name")
            && let Some((_, rhs)) = rest.split_once('=')
        {
            name = rhs.trim().trim_matches('"').to_owned();
        } else if (section == "dependencies" || section.ends_with(".dependencies"))
            && !section.contains("dev-dependencies")
            && let Some((dep_key, _)) = line.split_once('=')
        {
            let dep = dep_key.trim().trim_matches('"');
            if dep.starts_with("spawn-") {
                deps.insert(dep.to_owned());
            }
        }
    }
    assert!(!name.is_empty(), "manifest missing [package] name");
    (name, deps)
}

fn assert_acyclic(edges: &BTreeMap<String, BTreeSet<String>>) {
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

fn audit_engine_source(path: &Path, content: &str) {
    const FORBIDDEN_PANIC: &[&str] = &[
        ".unwrap(",
        ".expect(",
        "panic!(",
        "todo!(",
        "unimplemented!(",
    ];
    const FORBIDDEN_BACKEND_PUBLIC: &[&str] = &[
        "ash::",
        "vk::",
        "bytemuck::",
        "libm::",
        "glam::",
        "wayland_client::",
        "wayland_protocols::",
        "wl_",
        "xdg_",
        "zwp_",
    ];
    const FORBIDDEN_GENRE_TOKENS: &[&str] = &[
        "weapon", "health", "team", "player", "vehicle", "match", "ammo", "reload", "kill",
        "score", "damage",
    ];

    for (idx, raw) in content.lines().enumerate() {
        let code = raw.split("//").next().unwrap_or("").trim();
        if code.starts_with("#[cfg(test)]") {
            break;
        }
        for bad in FORBIDDEN_PANIC {
            assert!(
                !code.contains(bad),
                "{}:{} contains non-test `{bad}`: `{code}`",
                path.display(),
                idx + 1
            );
        }
        assert!(
            !code.contains("unsafe "),
            "{}:{} must not contain `unsafe`: `{code}`",
            path.display(),
            idx + 1
        );
        if code.starts_with("pub ") && !code.starts_with("pub mod ") {
            for backend in FORBIDDEN_BACKEND_PUBLIC {
                assert!(
                    !code.contains(backend),
                    "{}:{} leaks `{backend}` in public item: `{code}`",
                    path.display(),
                    idx + 1
                );
            }
            for token in code
                .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                .filter(|t| !t.is_empty())
            {
                let lower = token.to_ascii_lowercase();
                assert!(
                    !FORBIDDEN_GENRE_TOKENS.contains(&lower.as_str()),
                    "{}:{} contains genre token `{token}` in public item: `{code}`",
                    path.display(),
                    idx + 1
                );
            }
        }
    }
}
