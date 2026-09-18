//! Lightweight symbol extraction (heuristic; Tree-sitter can replace later).

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolHit {
    pub path: PathBuf,
    pub name: String,
    pub kind: String,
    pub line: u32,
}

/// Extract symbols from source text for the given language id.
pub fn extract_symbols(text: &str, language: &str) -> Vec<SymbolHit> {
    let mut out = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let trimmed = line.trim();
        if let Some((kind, name)) = match language {
            "rust" => parse_rust(trimmed),
            "typescript" | "javascript" => parse_js(trimmed),
            "python" => parse_python(trimmed),
            "go" => parse_go(trimmed),
            "java" => parse_java(trimmed),
            _ => None,
        } {
            out.push(SymbolHit {
                path: PathBuf::new(),
                name,
                kind: kind.into(),
                line: line_no,
            });
        }
    }
    out
}

fn parse_rust(line: &str) -> Option<(&str, String)> {
    let line = line.trim_start_matches("pub ").trim_start_matches("async ");
    if let Some(rest) = line.strip_prefix("fn ") {
        return Some(("function", take_ident(rest)?));
    }
    if let Some(rest) = line.strip_prefix("struct ") {
        return Some(("struct", take_ident(rest)?));
    }
    if let Some(rest) = line.strip_prefix("enum ") {
        return Some(("enum", take_ident(rest)?));
    }
    if let Some(rest) = line.strip_prefix("trait ") {
        return Some(("trait", take_ident(rest)?));
    }
    if let Some(rest) = line.strip_prefix("mod ") {
        return Some(("module", take_ident(rest)?));
    }
    if let Some(rest) = line.strip_prefix("type ") {
        return Some(("type", take_ident(rest)?));
    }
    None
}

fn parse_js(line: &str) -> Option<(&str, String)> {
    let line = line
        .trim_start_matches("export ")
        .trim_start_matches("async ");
    if let Some(rest) = line.strip_prefix("function ") {
        return Some(("function", take_ident(rest)?));
    }
    if let Some(rest) = line.strip_prefix("class ") {
        return Some(("class", take_ident(rest)?));
    }
    if let Some(rest) = line.strip_prefix("const ") {
        let name = take_ident(rest)?;
        if rest.contains("=>") || rest.contains("function") {
            return Some(("function", name));
        }
    }
    None
}

fn parse_python(line: &str) -> Option<(&str, String)> {
    if let Some(rest) = line.strip_prefix("def ") {
        return Some(("function", take_ident(rest)?));
    }
    if let Some(rest) = line.strip_prefix("class ") {
        return Some(("class", take_ident(rest)?));
    }
    None
}

fn parse_go(line: &str) -> Option<(&str, String)> {
    if let Some(rest) = line.strip_prefix("func ") {
        let rest = rest.trim_start_matches('(');
        // skip receiver
        let rest = if rest.starts_with('*') || rest.contains(')') {
            rest.split(')').nth(1)?.trim()
        } else {
            rest
        };
        return Some(("function", take_ident(rest)?));
    }
    if let Some(rest) = line.strip_prefix("type ") {
        return Some(("type", take_ident(rest)?));
    }
    None
}

fn parse_java(line: &str) -> Option<(&str, String)> {
    let line = line
        .trim_start_matches("public ")
        .trim_start_matches("private ")
        .trim_start_matches("protected ")
        .trim_start_matches("static ");
    if let Some(rest) = line.strip_prefix("class ") {
        return Some(("class", take_ident(rest)?));
    }
    if let Some(rest) = line.strip_prefix("interface ") {
        return Some(("interface", take_ident(rest)?));
    }
    None
}

fn take_ident(s: &str) -> Option<String> {
    let s = s.trim_start();
    let mut name = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            name.push(c);
        } else {
            break;
        }
    }
    if name.is_empty() { None } else { Some(name) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_fn() {
        let s = extract_symbols("pub fn hello_raya() {}\n", "rust");
        assert_eq!(s[0].name, "hello_raya");
        assert_eq!(s[0].kind, "function");
    }
}
