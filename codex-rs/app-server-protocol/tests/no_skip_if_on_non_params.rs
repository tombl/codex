use std::fs;

// Ensures no field in non-`*Params` types (structs or enums) in src/protocol.rs uses
// `#[serde(skip_serializing_if = "Option::is_none")]`.
//
// Intent: Responses, Notifications, and other objects must always serialize
// `null` for optional fields; only `*Params` structs may use this attribute.
//
// This pattern aligns with OpenAI public APIs such as Responses, where instead of
// omitting optional fields, they serialize them as `null`.
//
// This is a hacky test but it's better than nothing.
#[test]
fn no_skip_serializing_if_none_on_non_params() {
    let path = format!("{}/src/protocol.rs", env!("CARGO_MANIFEST_DIR"));
    let src = fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));

    // Collect struct/enum bodies with their names and byte ranges.
    #[derive(Debug)]
    struct Block {
        name: String,
        start: usize,
        end: usize,
    }

    let mut i = 0usize;
    let bytes = src.as_bytes();
    let mut blocks: Vec<Block> = Vec::new();

    // Find `struct` blocks.
    while let Some(pos) = find_token(bytes, i, b"struct") {
        // Ensure it's a standalone token (preceded/followed by non-ident chars)
        if !is_boundary(bytes, pos.saturating_sub(1)) || !is_boundary(bytes, pos + 6) {
            i = pos + 6;
            continue;
        }

        let mut j = skip_ws(bytes, pos + 6);
        let (name, next) = if let Some(t) = parse_ident(bytes, j) {
            t
        } else {
            i = j; // advance and continue searching
            continue;
        };
        if name.is_empty() {
            i = j;
            continue;
        }

        j = skip_ws(bytes, next);
        // Skip generics if present: <...>
        if j < bytes.len() && bytes[j] == b'<' {
            j = match skip_angle_block(bytes, j) {
                Some(n) => n,
                None => break,
            };
            j = skip_ws(bytes, j);
        }

        if j >= bytes.len() {
            break;
        }

        let (open, close) = match bytes[j] {
            b'{' => (b'{', b'}'),
            b'(' => (b'(', b')'),
            _ => {
                i = j + 1;
                continue;
            }
        };

        if let Some(end) = find_matching(bytes, j, open, close) {
            blocks.push(Block {
                name,
                start: j,
                end,
            });
            i = end + 1;
        } else {
            break;
        }
    }

    // Find `enum` blocks.
    i = 0;
    while let Some(pos) = find_token(bytes, i, b"enum") {
        if !is_boundary(bytes, pos.saturating_sub(1)) || !is_boundary(bytes, pos + 4) {
            i = pos + 4;
            continue;
        }

        let mut j = skip_ws(bytes, pos + 4);
        let (name, next) = if let Some(t) = parse_ident(bytes, j) {
            t
        } else {
            i = j;
            continue;
        };
        if name.is_empty() {
            i = j;
            continue;
        }

        j = skip_ws(bytes, next);
        // Skip generics if present: <...>
        if j < bytes.len() && bytes[j] == b'<' {
            j = match skip_angle_block(bytes, j) {
                Some(n) => n,
                None => break,
            };
            j = skip_ws(bytes, j);
        }
        if j >= bytes.len() {
            break;
        }

        // Enums should have a body `{ ... }`.
        if bytes[j] != b'{' {
            i = j + 1;
            continue;
        }
        if let Some(end) = find_matching(bytes, j, b'{', b'}') {
            blocks.push(Block {
                name,
                start: j,
                end,
            });
            i = end + 1;
        } else {
            break;
        }
    }

    let mut violations: Vec<String> = Vec::new();
    for blk in blocks {
        if blk.name.ends_with("Params") {
            continue; // Allowed to use skip_serializing_if
        }

        let body = &src[blk.start..=blk.end];

        // Fast-path check for the attribute pattern within this struct body.
        let mut search_from = 0usize;
        while let Some(rel) = body[search_from..].find("skip_serializing_if") {
            let abs = blk.start + search_from + rel;
            // Check the line that contains this occurrence.
            let line_start = src[..abs].rfind('\n').map(|p| p + 1).unwrap_or(0);
            let line_end = src[abs..].find('\n').map(|p| abs + p).unwrap_or(src.len());
            let line = &src[line_start..line_end];

            // If `//` appears before `#[`, treat as commented-out and ignore.
            let idx_hash = line.find("#[");
            let idx_comment = line.find("//");
            let looks_like_attr =
                idx_hash.is_some() && (idx_comment.is_none() || idx_hash < idx_comment);

            if looks_like_attr && line.contains("serde") && line.contains("Option::is_none") {
                // Record a helpful message with the struct and line.
                violations.push(format!(
                    "{}: disallowed #[serde(skip_serializing_if = \"Option::is_none\")] in non-Params type `{}`\n> {}",
                    path, blk.name, line.trim()
                ));
                break; // one hit is enough per struct
            }

            search_from = search_from + rel + "skip_serializing_if".len();
        }
    }

    if !violations.is_empty() {
        panic!(
            "Found disallowed serde skip_serializing_if on non-Params structs:\n{}",
            violations.join("\n\n")
        );
    }
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_boundary(bytes: &[u8], idx: usize) -> bool {
    if idx >= bytes.len() {
        return true;
    }
    !is_ident_char(bytes[idx])
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' => {
                i += 1;
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                // line comment
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                // block comment (non-nested for simplicity)
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
            }
            _ => break,
        }
    }
    i
}

fn parse_ident(bytes: &[u8], mut i: usize) -> Option<(String, usize)> {
    let start = i;
    while i < bytes.len() && is_ident_char(bytes[i]) {
        i += 1;
    }
    if i == start {
        return Some((String::new(), i));
    }
    let name = String::from_utf8(bytes[start..i].to_vec()).ok()?;
    Some((name, i))
}

fn skip_angle_block(bytes: &[u8], mut i: usize) -> Option<usize> {
    // assumes bytes[i] == b'<'
    let mut depth = 0i32;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => {
                depth += 1;
                i += 1;
            }
            b'>' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            b'\"' => {
                i = skip_string(bytes, i)?;
            }
            b'\'' => {
                i = skip_char(bytes, i)?;
            }
            _ => {
                i += 1;
            }
        }
    }
    None
}

fn skip_string(bytes: &[u8], mut i: usize) -> Option<usize> {
    // assumes bytes[i] == '"'
    i += 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                i += 2;
            }
            b'\"' => {
                i += 1;
                return Some(i);
            }
            _ => {
                i += 1;
            }
        }
    }
    None
}

fn skip_char(bytes: &[u8], mut i: usize) -> Option<usize> {
    // assumes bytes[i] == '\''
    i += 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                i += 2;
            }
            b'\'' => {
                i += 1;
                return Some(i);
            }
            _ => {
                i += 1;
            }
        }
    }
    None
}

fn find_matching(bytes: &[u8], mut i: usize, open: u8, close: u8) -> Option<usize> {
    // assumes bytes[i] == open
    let mut depth = 0i32;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\"' {
            i = skip_string(bytes, i)?;
            continue;
        }
        if b == b'\'' {
            i = skip_char(bytes, i)?;
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            // line comment
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            // block comment (non-nested)
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        if b == open {
            depth += 1;
        }
        if b == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn find_token(bytes: &[u8], from: usize, token: &[u8]) -> Option<usize> {
    let mut i = from;
    while i + token.len() <= bytes.len() {
        if &bytes[i..i + token.len()] == token {
            return Some(i);
        }
        i += 1;
    }
    None
}
