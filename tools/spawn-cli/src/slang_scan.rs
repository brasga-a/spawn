//! Bounded untrusted `.slang` entry-point attribute scanner (`SPN-INV-034`, `SPN-INV-081`).
//!
//! Discovers `[shader("<stage>")]` declarations and their associated entry-point
//! function names while ignoring comments and string literals, without invoking
//! external tools.

#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

use std::collections::BTreeSet;
use std::path::Path;

use spawn_render::ShaderStage;

use crate::error::{CliError, CliErrorKind};

/// Maximum `.slang` source file size accepted by `spawn-cli` (4 MiB, `SPN-INV-034`).
pub(crate) const MAX_SLANG_SOURCE_BYTES: u64 = 4 * 1024 * 1024;

/// Discovered entry point inside a `.slang` source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscoveredEntryPoint {
    pub(crate) stage: ShaderStage,
    pub(crate) name: String,
    pub(crate) line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    LBracket(usize),
    RBracket(usize),
    LParen(usize),
    RParen(usize),
    Ident(String, usize),
    StringLit(String, usize),
    Other(char, usize),
}

/// Scans `source` for `[shader("<stage>")] <ReturnType> <entry_name>(...)` declarations.
#[track_caller]
pub(crate) fn scan_slang_entry_points(
    file: &Path,
    source: &str,
) -> Result<Vec<DiscoveredEntryPoint>, CliError> {
    if source.len() as u64 > MAX_SLANG_SOURCE_BYTES {
        return Err(CliError::new(CliErrorKind::InvalidShaderSource)
            .with_file(file)
            .with_field("source_size")
            .with_detail(format!(
                "shader source size {} bytes exceeds maximum {} bytes",
                source.len(),
                MAX_SLANG_SOURCE_BYTES
            )));
    }

    let tokens = tokenize_slang(file, source)?;
    let mut entries = Vec::new();
    let mut seen_names = BTreeSet::new();
    let mut brace_depth = 0usize;
    let mut idx = 0usize;

    while let Some(tok) = tokens.get(idx) {
        match *tok {
            Token::Other('{', _) => {
                brace_depth = brace_depth.saturating_add(1);
                idx = idx.saturating_add(1);
                continue;
            }
            Token::Other('}', _) => {
                brace_depth = brace_depth.saturating_sub(1);
                idx = idx.saturating_add(1);
                continue;
            }
            Token::LBracket(open_line) if brace_depth == 0 => {
                let group_end = skip_bracket_group(file, &tokens, idx, open_line)?;
                let Some((stage, stage_str, attr_line)) =
                    extract_shader_attr_from_group(file, &tokens, idx, group_end)?
                else {
                    idx = group_end;
                    continue;
                };

                idx = group_end;

                // Skip any subsequent `[...]` or `[[...]]` attributes before the function signature
                // (for example `[numthreads(8, 8, 1)]`), rejecting duplicate `shader(...)` attributes
                // on the same declaration.
                while let Some(&Token::LBracket(next_open_line)) = tokens.get(idx) {
                    let next_end = skip_bracket_group(file, &tokens, idx, next_open_line)?;
                    if let Some((_, _, dup_line)) =
                        extract_shader_attr_from_group(file, &tokens, idx, next_end)?
                    {
                        return Err(CliError::new(CliErrorKind::InvalidShaderSource)
                            .with_file(file)
                            .with_field("shader_attribute")
                            .with_detail(format!(
                                "line {dup_line}: duplicate `shader(...)` attribute on a single declaration"
                            )));
                    }
                    idx = next_end;
                }

                // Read the function signature up to the parameter list's `(`; the identifier
                // immediately preceding `(` is the entry-point function name.
                let mut last_ident: Option<String> = None;
                let mut found_lparen = false;
                while let Some(sig_tok) = tokens.get(idx) {
                    match sig_tok {
                        Token::LParen(_) => {
                            found_lparen = true;
                            idx = idx.saturating_add(1);
                            break;
                        }
                        Token::Ident(ident, _) => {
                            last_ident = Some(ident.clone());
                            idx = idx.saturating_add(1);
                        }
                        Token::LBracket(sig_open_line) => {
                            idx = skip_bracket_group(file, &tokens, idx, *sig_open_line)?;
                        }
                        Token::Other(';' | '{' | '}', bad_line) => {
                            return Err(CliError::new(CliErrorKind::InvalidShaderSource)
                                .with_file(file)
                                .with_field("entry_point")
                                .with_detail(format!(
                                    "line {bad_line}: `[shader(\"{stage_str}\")]` at line {attr_line} is not followed by a function declaration"
                                )));
                        }
                        _ => {
                            idx = idx.saturating_add(1);
                        }
                    }
                }

                let Some(fn_name) = last_ident.filter(|_| found_lparen) else {
                    return Err(CliError::new(CliErrorKind::InvalidShaderSource)
                        .with_file(file)
                        .with_field("entry_point")
                        .with_detail(format!(
                            "line {attr_line}: missing entry-point function declaration after `[shader(\"{stage_str}\")]`"
                        )));
                };

                if !seen_names.insert(fn_name.clone()) {
                    return Err(CliError::new(CliErrorKind::InvalidShaderSource)
                        .with_file(file)
                        .with_field("entry_point")
                        .with_detail(format!(
                            "line {attr_line}: duplicate entry-point name `{fn_name}` in shader file"
                        )));
                }

                entries.push(DiscoveredEntryPoint {
                    stage,
                    name: fn_name,
                    line: attr_line,
                });
                continue;
            }
            _ => {
                idx = idx.saturating_add(1);
            }
        }
    }

    if entries.is_empty() {
        return Err(CliError::new(CliErrorKind::InvalidShaderSource)
            .with_file(file)
            .with_field("entry_points")
            .with_detail(
                "no `[shader(\"vertex\"|\"fragment\"|\"compute\")]` entry points found in source",
            ));
    }

    Ok(entries)
}

#[track_caller]
fn extract_shader_attr_from_group(
    file: &Path,
    tokens: &[Token],
    group_start: usize,
    group_end: usize,
) -> Result<Option<(ShaderStage, String, usize)>, CliError> {
    let inner_start = group_start.saturating_add(1);
    let inner_end = group_end.saturating_sub(1);
    let Some(mut inner) = tokens.get(inner_start..inner_end) else {
        return Ok(None);
    };

    // Unwrap C++/Slang double-bracket `[[ ... ]]` attributes when the entire inner slice
    // is enclosed by a single matching `[` ... `]` pair.
    if let (Some(Token::LBracket(open_line)), Some(Token::RBracket(_))) =
        (inner.first(), inner.last())
        && skip_bracket_group(file, inner, 0, *open_line)? == inner.len()
        && let Some(unwrapped) = inner.get(1..inner.len().saturating_sub(1))
    {
        inner = unwrapped;
    }

    let mut found: Option<(ShaderStage, String, usize)> = None;
    let mut item_start = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;

    for (rel_idx, tok) in inner.iter().enumerate() {
        match tok {
            Token::LParen(_) => paren_depth = paren_depth.saturating_add(1),
            Token::RParen(_) => paren_depth = paren_depth.saturating_sub(1),
            Token::LBracket(_) => bracket_depth = bracket_depth.saturating_add(1),
            Token::RBracket(_) => bracket_depth = bracket_depth.saturating_sub(1),
            Token::Other(',', _) if paren_depth == 0 && bracket_depth == 0 => {
                if let Some(item) = inner.get(item_start..rel_idx)
                    && let Some(parsed) = parse_shader_attr_item(file, item)?
                {
                    if found.is_some() {
                        return Err(CliError::new(CliErrorKind::InvalidShaderSource)
                            .with_file(file)
                            .with_field("shader_attribute")
                            .with_detail(format!(
                                "line {}: duplicate `shader(...)` in attribute list",
                                parsed.2
                            )));
                    }
                    found = Some(parsed);
                }
                item_start = rel_idx.saturating_add(1);
            }
            _ => {}
        }
    }

    if let Some(tail) = inner.get(item_start..)
        && let Some(parsed) = parse_shader_attr_item(file, tail)?
    {
        if found.is_some() {
            return Err(CliError::new(CliErrorKind::InvalidShaderSource)
                .with_file(file)
                .with_field("shader_attribute")
                .with_detail(format!(
                    "line {}: duplicate `shader(...)` in attribute list",
                    parsed.2
                )));
        }
        found = Some(parsed);
    }

    Ok(found)
}

#[track_caller]
fn parse_shader_attr_item(
    file: &Path,
    item: &[Token],
) -> Result<Option<(ShaderStage, String, usize)>, CliError> {
    let Some(Token::Ident(name, attr_line)) = item.first() else {
        return Ok(None);
    };
    if name != "shader" || matches!(item.get(1), Some(Token::Other(':', _))) {
        return Ok(None);
    }

    if !matches!(item.get(1), Some(Token::LParen(_))) {
        return Err(CliError::new(CliErrorKind::InvalidShaderSource)
            .with_file(file)
            .with_field("shader_attribute")
            .with_detail(format!(
                "line {attr_line}: malformed `[shader(\"...\")]` attribute syntax"
            )));
    }
    let Some(Token::StringLit(stage_str, stage_line)) = item.get(2) else {
        return Err(CliError::new(CliErrorKind::InvalidShaderSource)
            .with_file(file)
            .with_field("shader_attribute")
            .with_detail(format!(
                "line {attr_line}: expected stage string literal inside `[shader(\"...\")]`"
            )));
    };
    if item.len() != 4 || !matches!(item.get(3), Some(Token::RParen(_))) {
        return Err(CliError::new(CliErrorKind::InvalidShaderSource)
            .with_file(file)
            .with_field("shader_attribute")
            .with_detail(format!(
                "line {attr_line}: malformed `[shader(\"...\")]` attribute syntax"
            )));
    }

    let stage = match stage_str.as_str() {
        "vertex" => ShaderStage::Vertex,
        "fragment" | "pixel" => ShaderStage::Fragment,
        "compute" => ShaderStage::Compute,
        other => {
            return Err(CliError::new(CliErrorKind::InvalidShaderSource)
                .with_file(file)
                .with_field("shader_stage")
                .with_detail(format!(
                    "line {stage_line}: unsupported shader stage `{other}` (expected `vertex`, `fragment`, or `compute`)"
                )));
        }
    };

    Ok(Some((stage, stage_str.clone(), *attr_line)))
}

#[track_caller]
fn skip_bracket_group(
    file: &Path,
    tokens: &[Token],
    start: usize,
    open_line: usize,
) -> Result<usize, CliError> {
    let mut depth = 0usize;
    let mut i = start;
    while let Some(tok) = tokens.get(i) {
        match tok {
            Token::LBracket(_) => depth = depth.saturating_add(1),
            Token::RBracket(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Ok(i.saturating_add(1));
                }
            }
            _ => {}
        }
        i = i.saturating_add(1);
    }
    Err(CliError::new(CliErrorKind::InvalidShaderSource)
        .with_file(file)
        .with_field("attribute")
        .with_detail(format!("line {open_line}: unclosed `[` in attribute")))
}

#[track_caller]
fn tokenize_slang(file: &Path, source: &str) -> Result<Vec<Token>, CliError> {
    let mut tokens = Vec::new();
    let mut chars = source.chars().peekable();
    let mut line = 1usize;

    while let Some(ch) = chars.next() {
        if ch == '\n' {
            line = line.saturating_add(1);
            continue;
        }
        if ch.is_whitespace() {
            continue;
        }

        if ch == '/' {
            match chars.peek().copied() {
                Some('/') => {
                    let _ = chars.next();
                    for c in chars.by_ref() {
                        if c == '\n' {
                            line = line.saturating_add(1);
                            break;
                        }
                    }
                    continue;
                }
                Some('*') => {
                    let open_line = line;
                    let _ = chars.next();
                    let mut closed = false;
                    while let Some(c) = chars.next() {
                        if c == '\n' {
                            line = line.saturating_add(1);
                        } else if c == '*' && chars.peek().copied() == Some('/') {
                            let _ = chars.next();
                            closed = true;
                            break;
                        }
                    }
                    if !closed {
                        return Err(CliError::new(CliErrorKind::InvalidShaderSource)
                            .with_file(file)
                            .with_field("comment")
                            .with_detail(format!(
                                "line {open_line}: unterminated `/*` block comment"
                            )));
                    }
                    continue;
                }
                _ => {
                    tokens.push(Token::Other('/', line));
                    continue;
                }
            }
        }

        if ch == '"' || ch == '\'' {
            let quote = ch;
            let open_line = line;
            let mut value = String::new();
            let mut closed = false;
            while let Some(c) = chars.next() {
                if c == '\n' {
                    return Err(CliError::new(CliErrorKind::InvalidShaderSource)
                        .with_file(file)
                        .with_field("string_literal")
                        .with_detail(format!(
                            "line {open_line}: unterminated string literal before newline"
                        )));
                }
                if c == '\\' {
                    if let Some(escaped) = chars.next() {
                        if escaped == '\n' {
                            line = line.saturating_add(1);
                        } else {
                            value.push(escaped);
                        }
                    }
                    continue;
                }
                if c == quote {
                    closed = true;
                    break;
                }
                value.push(c);
            }
            if !closed {
                return Err(CliError::new(CliErrorKind::InvalidShaderSource)
                    .with_file(file)
                    .with_field("string_literal")
                    .with_detail(format!("line {open_line}: unterminated string literal")));
            }
            if quote == '"' {
                tokens.push(Token::StringLit(value, open_line));
            }
            continue;
        }

        if ch == '_' || ch.is_ascii_alphabetic() {
            let start_line = line;
            let mut ident = String::from(ch);
            while let Some(&next) = chars.peek() {
                if next == '_' || next.is_ascii_alphanumeric() {
                    ident.push(next);
                    let _ = chars.next();
                } else {
                    break;
                }
            }
            tokens.push(Token::Ident(ident, start_line));
            continue;
        }

        match ch {
            '[' => tokens.push(Token::LBracket(line)),
            ']' => tokens.push(Token::RBracket(line)),
            '(' => tokens.push(Token::LParen(line)),
            ')' => tokens.push(Token::RParen(line)),
            other => tokens.push(Token::Other(other, line)),
        }
    }

    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_vertex_fragment_and_compute_ignoring_comments() {
        let src = r#"
            // [shader("geometry")] void fake_comment() {}
            /* [shader("vertex")] void fake_block() {} */
            [shader("vertex")]
            VertexOutput vs_main(VertexInput input) { return (VertexOutput)0; }

            [shader("fragment")]
            float4 fs_main(VertexOutput input) : SV_Target0 { return float4(1, 1, 1, 1); }

            [shader("compute")]
            [numthreads(8, 8, 1)]
            void cs_main(uint3 tid : SV_DispatchThreadID) {}
        "#;
        let found = scan_slang_entry_points(Path::new("test.slang"), src).unwrap();
        assert_eq!(found.len(), 3);
        assert_eq!(
            found.first().map(|e| (e.stage, e.name.as_str())),
            Some((ShaderStage::Vertex, "vs_main"))
        );
        assert_eq!(
            found.get(1).map(|e| (e.stage, e.name.as_str())),
            Some((ShaderStage::Fragment, "fs_main"))
        );
        assert_eq!(
            found.get(2).map(|e| (e.stage, e.name.as_str())),
            Some((ShaderStage::Compute, "cs_main"))
        );
    }

    #[test]
    fn rejects_unsupported_stage_and_empty_entry_points() {
        let err = scan_slang_entry_points(
            Path::new("geom.slang"),
            r#"[shader("geometry")] void gs_main() {}"#,
        )
        .unwrap_err();
        assert_eq!(err.kind(), CliErrorKind::InvalidShaderSource);
        assert_eq!(err.field(), Some("shader_stage"));

        let err2 =
            scan_slang_entry_points(Path::new("empty.slang"), "struct OnlyTypes { float x; };")
                .unwrap_err();
        assert_eq!(err2.kind(), CliErrorKind::InvalidShaderSource);
        assert_eq!(err2.field(), Some("entry_points"));
    }

    #[test]
    fn scans_comma_separated_double_bracket_and_brace_scoped_attributes() {
        let src = r#"
            struct Helper {
                float values[4];
            };

            [[vk::binding(0, 0)]]
            SamplerState g_samplers[4];

            [[shader("vertex")]]
            float4 vs_double(uint vid : SV_VulkanVertexID) : SV_Position {
                [unroll]
                for (int i = 0; i < 3; ++i) {
                    int shader[2] = { i, i + 1 };
                    if (shader[0] > 10) { return float4(0, 0, 0, 1); }
                }
                return float4(0.0, 0.0, 0.0, 1.0);
            }

            [shader("compute"), numthreads(8, 8, 1)]
            void cs_first(uint3 tid : SV_DispatchThreadID) {}

            [numthreads(4, 4, 1), shader("compute")]
            void cs_second(uint3 tid : SV_DispatchThreadID) {}
        "#;

        let found = scan_slang_entry_points(Path::new("attrs.slang"), src).unwrap();
        assert_eq!(found.len(), 3);
        assert_eq!(
            found.first().map(|e| (e.stage, e.name.as_str())),
            Some((ShaderStage::Vertex, "vs_double"))
        );
        assert_eq!(
            found.get(1).map(|e| (e.stage, e.name.as_str())),
            Some((ShaderStage::Compute, "cs_first"))
        );
        assert_eq!(
            found.get(2).map(|e| (e.stage, e.name.as_str())),
            Some((ShaderStage::Compute, "cs_second"))
        );

        // Duplicate `shader(...)` in one attribute list or across stacked lists is rejected.
        for bad in [
            r#"[shader("vertex"), shader("fragment")] void bad1() {}"#,
            r#"[shader("vertex")] [shader("vertex")] void bad2() {}"#,
            r#"[shader(123)] void bad3() {}"#,
        ] {
            let err = scan_slang_entry_points(Path::new("bad_attr.slang"), bad).unwrap_err();
            assert_eq!(err.kind(), CliErrorKind::InvalidShaderSource);
        }
    }
}
