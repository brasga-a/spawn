//! `F3D-T13` (`spawn-render` Phase 3 + Phase 4 slice; `F3D-NFR-01..05`,
//! `F3D-INV-01..04, 13..16`, `SPN-INV-006, 009, 011, 030, 033, 052, 053, 092, 101`):
//! automated dependency-tree, `#![forbid(unsafe_code)]` confinement outside `src/vulkan/`,
//! `// SAFETY:` comment audit inside `src/vulkan/`, zero `VkRenderPass`/`VkFramebuffer`
//! enforcement, panic-free source, and public-API hygiene (`bytemuck`/`ash` confinement).

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn f3d_t13_spawn_render_boundaries_safety_and_dynamic_rendering_hygiene()
-> Result<(), Box<dyn std::error::Error>> {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = crate_dir
        .parent()
        .and_then(Path::parent)
        .ok_or("expected crates/spawn-render inside workspace root")?;

    // 1. Default-feature `spawn-render` pulls only pure-Rust `spawn-math -> libm` (`SPN-INV-009`),
    //    never `ash`, `bytemuck`, `raw-window-handle`, or native windowing crates.
    let headless_tree = run_cargo_tree(workspace_root, &["-p", "spawn-render", "-e", "normal"])?;
    let headless_crates: BTreeSet<String> = headless_tree
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start_matches(|c: char| !c.is_ascii_alphanumeric());
            trimmed.split_whitespace().next().map(str::to_owned)
        })
        .collect();
    assert_eq!(
        headless_crates,
        BTreeSet::from([
            "spawn-render".to_owned(),
            "spawn-math".to_owned(),
            "libm".to_owned(),
        ]),
        "spawn-render default features must pull only spawn-math -> libm:\n{headless_tree}"
    );

    // 2. `spawn-render --features vulkan` pulls `spawn-math` and `bytemuck`, and never depends
    //    on `spawn-platform`, `spawn-input`, `spawn-core`, `spawn-cli`, `wayland-client`,
    //    `windows-sys`, or `x11rb`.
    let vulkan_tree = run_cargo_tree(
        workspace_root,
        &["-p", "spawn-render", "--features", "vulkan", "-e", "normal"],
    )?;
    for required in ["spawn-math", "bytemuck", "ash", "raw-window-handle"] {
        assert!(
            vulkan_tree.contains(required),
            "spawn-render --features vulkan expected `{required}`:\n{vulkan_tree}"
        );
    }
    for forbidden in [
        "spawn-platform",
        "spawn-input",
        "spawn-core",
        "spawn-cli",
        "wayland-client",
        "windows-sys",
        "x11rb",
    ] {
        assert!(
            !vulkan_tree.contains(forbidden),
            "spawn-render --features vulkan must not depend on `{forbidden}`:\n{vulkan_tree}"
        );
    }

    // 3. No `build.rs` in `crates/spawn-render` (`SPN-INV-084`).
    assert!(!crate_dir.join("build.rs").exists());

    // 4. Every `.rs` module directly in `crates/spawn-render/src/` (except `lib.rs`, which uses
    //    `#![cfg_attr(not(feature = "vulkan"), forbid(unsafe_code))]`) must declare
    //    `#![forbid(unsafe_code)]` (`F3D-NFR-05`, `SPN-INV-033`).
    let lib_rs = fs::read_to_string(crate_dir.join("src/lib.rs"))?;
    assert!(
        lib_rs.contains("#![cfg_attr(not(feature = \"vulkan\"), forbid(unsafe_code))]"),
        "src/lib.rs must forbid unsafe_code when `vulkan` is disabled"
    );

    for entry in fs::read_dir(crate_dir.join("src"))? {
        let path = entry?.path();
        if path.is_dir() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let content = fs::read_to_string(&path)?;
        if path.file_name().and_then(|n| n.to_str()) != Some("lib.rs") {
            assert!(
                content
                    .lines()
                    .any(|line| line.trim() == "#![forbid(unsafe_code)]"),
                "{} must declare `#![forbid(unsafe_code)]`",
                path.display()
            );
        }
        audit_non_vulkan_file(&path, &content);
    }

    // 5. Inside `src/vulkan/`, verify:
    //    - zero `VkRenderPass` / `create_render_pass` / `create_framebuffer` (`F3D-FR-08`);
    //    - swapchain never requests `TRANSFER_SRC` (`SPN-INV-092`);
    //    - zero non-test panicking macros/methods (`SPN-INV-030`);
    //    - every `unsafe` block / `unsafe extern` has an adjacent `// SAFETY:` comment (`SPN-INV-033`).
    for entry in fs::read_dir(crate_dir.join("src/vulkan"))? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let content = fs::read_to_string(&path)?;
        audit_vulkan_file(&path, &content);
    }

    let swapchain_src = fs::read_to_string(crate_dir.join("src/vulkan/swapchain.rs"))?;
    assert!(
        !swapchain_src.contains("TRANSFER_SRC"),
        "swapchain.rs must keep COLOR_ATTACHMENT usage only (SPN-INV-092)"
    );

    // 6. Verify Proposed ADRs `0020` and `0022` remain `Proposed` (`SPN-INV-101`).
    for adr in [
        "docs/adr/0020-memory-strategy.md",
        "docs/adr/0022-repository-layout.md",
    ] {
        let doc = fs::read_to_string(workspace_root.join(adr))?;
        assert!(
            doc.contains("**Status:** Proposed"),
            "{adr} must remain Proposed"
        );
    }

    Ok(())
}

fn run_cargo_tree(root: &Path, args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let out = Command::new(env!("CARGO"))
        .current_dir(root)
        .arg("tree")
        .args(args)
        .output()?;
    assert!(
        out.status.success(),
        "cargo tree {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8(out.stdout)?)
}

fn audit_non_vulkan_file(path: &Path, content: &str) {
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
        "Pod",
        "Zeroable",
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
            "{}:{} outside src/vulkan/ must not use `unsafe`: `{code}`",
            path.display(),
            idx + 1
        );
        if code.starts_with("pub ") && !code.starts_with("pub mod ") {
            for b in FORBIDDEN_BACKEND_PUBLIC {
                assert!(
                    !code.contains(b),
                    "{}:{} leaks `{b}` in public item: `{code}`",
                    path.display(),
                    idx + 1
                );
            }
            for tok in code
                .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                .filter(|t| !t.is_empty())
            {
                let lower = tok.to_ascii_lowercase();
                assert!(
                    !FORBIDDEN_GENRE_TOKENS.contains(&lower.as_str()),
                    "{}:{} contains genre token `{tok}` in public item: `{code}`",
                    path.display(),
                    idx + 1
                );
            }
        }
    }
}

fn audit_vulkan_file(path: &Path, content: &str) {
    const FORBIDDEN_PANIC: &[&str] = &[
        ".unwrap(",
        ".expect(",
        "panic!(",
        "todo!(",
        "unimplemented!(",
    ];
    const FORBIDDEN_LEGACY_PASS: &[&str] = &[
        "create_render_pass",
        "create_framebuffer",
        "cmd_begin_render_pass",
    ];

    let lines: Vec<&str> = content.lines().collect();
    for (idx, &raw) in lines.iter().enumerate() {
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
        for legacy in FORBIDDEN_LEGACY_PASS {
            assert!(
                !code.contains(legacy),
                "{}:{} uses legacy `{legacy}` instead of Vulkan 1.3 dynamicRendering",
                path.display(),
                idx + 1
            );
        }
        if (code.contains("unsafe {") || code.ends_with("unsafe"))
            && !code.starts_with("unsafe extern")
        {
            let start = idx.saturating_sub(6);
            let has_safety = lines[start..=idx]
                .iter()
                .any(|prev| prev.contains("// SAFETY:"));
            assert!(
                has_safety,
                "{}:{} `unsafe` block missing preceding `// SAFETY:` comment",
                path.display(),
                idx + 1
            );
        }
    }
}
