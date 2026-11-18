//! `F3D-T13` (Phase 1 `spawn-math` slice; `F3D-NFR-01..05, 07`, `F3D-INV-01..04, 15, 16`,
//! `SPN-INV-003, 004, 009, 011, 023, 030, 100`): automated dependency-tree, `#![forbid(unsafe_code)]`,
//! public-API third-party/genre-term hygiene, panic-free source, and documentation sync audit.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn f3d_t13_spawn_math_dependency_tree_public_api_safety_and_doc_sync()
-> Result<(), Box<dyn std::error::Error>> {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = crate_dir
        .parent()
        .and_then(Path::parent)
        .ok_or("expected crates/spawn-math inside workspace root")?;

    // 1. `cargo tree -p spawn-math -e normal` must contain ONLY `spawn-math` and `libm`
    //    (zero `spawn-*` deps per SPN-INV-004, zero OS/GPU/test deps per SPN-INV-009/023).
    let tree_out = Command::new(env!("CARGO"))
        .current_dir(workspace_root)
        .args(["tree", "-p", "spawn-math", "-e", "normal"])
        .output()?;
    assert!(
        tree_out.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&tree_out.stderr)
    );
    let tree = String::from_utf8(tree_out.stdout)?;
    let lines: Vec<&str> = tree.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "spawn-math normal dependency tree must contain only spawn-math -> libm:\n{tree}"
    );
    assert!(lines[0].starts_with("spawn-math v"));
    assert!(
        lines[1].contains("libm v"),
        "expected libm as sole normal dependency, got:\n{tree}"
    );
    for forbidden in [
        "spawn-core",
        "spawn-platform",
        "spawn-input",
        "spawn-render",
        "spawn-cli",
        "ash",
        "wayland",
        "raw-window-handle",
        "glam",
        "proptest",
        "insta",
        "bytemuck",
    ] {
        assert!(
            !tree.contains(forbidden),
            "spawn-math normal tree must not contain `{forbidden}`:\n{tree}"
        );
    }

    // 2. No `build.rs` in `crates/spawn-math` (`SPN-INV-084`).
    assert!(!crate_dir.join("build.rs").exists());

    // 3. `#![forbid(unsafe_code)]` in `src/lib.rs`, zero `unsafe`, zero non-test panicking macros/methods,
    //    zero leaked 3rd-party types (`SPN-INV-011`), and zero genre terms (`SPN-INV-003`).
    let lib_rs = fs::read_to_string(crate_dir.join("src/lib.rs"))?;
    assert!(
        lib_rs
            .lines()
            .any(|line| line.trim() == "#![forbid(unsafe_code)]"),
        "crates/spawn-math/src/lib.rs must declare `#![forbid(unsafe_code)]`"
    );
    audit_source_dir(&crate_dir.join("src"))?;

    // 4. Governance & status synchronization (`SPN-INV-100`, `SPN-INV-023`, Gate G2).
    let local_agents = fs::read_to_string(crate_dir.join("AGENTS.md"))?;
    assert!(
        local_agents.contains("`EXISTS`") && !local_agents.contains("`SCAFFOLD`"),
        "crates/spawn-math/AGENTS.md must reflect `EXISTS` status"
    );

    let root_agents = fs::read_to_string(workspace_root.join("AGENTS.md"))?;
    let math_row = root_agents
        .lines()
        .find(|line| line.contains("[spawn-math](crates/spawn-math/AGENTS.md)"))
        .ok_or("missing spawn-math row in root AGENTS.md")?;
    assert!(
        math_row.contains("EXISTS") && !math_row.contains("SCAFFOLD"),
        "root AGENTS.md spawn-math row must be EXISTS, got: {math_row}"
    );

    let deps_doc = fs::read_to_string(workspace_root.join("docs/DEPENDENCIES.md"))?;
    for crate_token in ["| `libm` |", "| `glam` |", "| `proptest` |", "| `insta` |"] {
        let row = deps_doc
            .lines()
            .find(|line| line.starts_with(crate_token))
            .ok_or_else(|| format!("missing `{crate_token}` row in docs/DEPENDENCIES.md"))?;
        assert!(
            row.contains("| yes"),
            "docs/DEPENDENCIES.md row `{row}` must mark In use = yes"
        );
    }

    Ok(())
}

fn audit_source_dir(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    const FORBIDDEN_GENRE_TOKENS: &[&str] = &[
        "weapon", "health", "team", "player", "vehicle", "match", "ammo", "reload", "kill",
        "score", "damage",
    ];
    const FORBIDDEN_THIRD_PARTY_PREFIXES: &[&str] = &[
        "libm::",
        "glam::",
        "bytemuck::",
        "proptest::",
        "insta::",
        "ash::",
        "vk::",
    ];
    const FORBIDDEN_PANIC_CALLS: &[&str] = &[
        ".unwrap(",
        ".expect(",
        "panic!(",
        "todo!(",
        "unimplemented!(",
    ];

    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            audit_source_dir(&path)?;
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let content = fs::read_to_string(&path)?;
        for (idx, raw_line) in content.lines().enumerate() {
            let code = raw_line.split("//").next().unwrap_or("").trim();
            if code.starts_with("#[cfg(test)]") {
                break;
            }
            for bad in FORBIDDEN_PANIC_CALLS {
                assert!(
                    !code.contains(bad),
                    "{}:{} contains forbidden non-test `{bad}`: `{code}`",
                    path.display(),
                    idx + 1
                );
            }
            assert!(
                !code.contains("unsafe "),
                "{}:{} contains forbidden `unsafe`: `{code}`",
                path.display(),
                idx + 1
            );

            let is_public_item = code.starts_with("pub ") && !code.starts_with("pub mod ");
            if !is_public_item {
                continue;
            }
            for third_party in FORBIDDEN_THIRD_PARTY_PREFIXES {
                assert!(
                    !code.contains(third_party),
                    "{}:{} leaks third-party token `{third_party}` in public API: `{code}`",
                    path.display(),
                    idx + 1
                );
            }
            for token in code
                .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                .filter(|tok| !tok.is_empty())
            {
                let lower = token.to_ascii_lowercase();
                assert!(
                    !FORBIDDEN_GENRE_TOKENS.contains(&lower.as_str()),
                    "{}:{} contains genre token `{token}` in public API: `{code}`",
                    path.display(),
                    idx + 1
                );
            }
        }
    }
    Ok(())
}
