//! Shaders a plugin brings for the cursor's effects, and what they may do.
//!
//! A plugin's shader is one function in GLSL ES 3.00, `vec4 lume_effect(vec2 pixel)`,
//! with any helpers of its own. The window puts it inside a program of its own, which
//! declares everything the function can read: the canvas's size, how far the effect has
//! got, the cursor's old and new cells and its colour. The function can read nothing
//! else. There is no texture to sample, so it never sees what a terminal shows, and
//! what it draws goes only to the screen: nothing is read back.
//!
//! What is left is the cost of running it, which is held down here before the window
//! ever compiles it: no loop runs more than a fixed number of times, every loop is a
//! `for` over a counter with bounds written as numbers, and the shader is short. The
//! window then shades only the pixels near the effect.

/// The longest a shader may be, in bytes.
pub const SHADER_MAX_BYTES: usize = 16 * 1024;

/// How many times one loop may run, and loops inside each other together.
pub const LOOP_MAX: i64 = 64;
pub const NESTED_MAX: i64 = 1024;

/// The function a plugin's shader defines.
pub const ENTRY: &str = "lume_effect";

/// What the window declares for the shader to read.
pub const PROVIDED: [&str; 6] = [
    "lume_resolution",
    "lume_progress",
    "lume_time",
    "lume_from",
    "lume_to",
    "lume_color",
];

/// Words a plugin's shader may not use: what would declare inputs and outputs of its
/// own, sample a texture, loop without a bound, or change how the program is built.
const FORBIDDEN: [&str; 23] = [
    "uniform",
    "attribute",
    "varying",
    "layout",
    "buffer",
    "shared",
    "precision",
    "invariant",
    "centroid",
    "flat",
    "smooth",
    "while",
    "do",
    "goto",
    "texture",
    "texelFetch",
    "textureLod",
    "textureSize",
    "textureGrad",
    "textureProj",
    "textureOffset",
    "dFdx",
    "dFdy",
];

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Word(String),
    Number(String),
    Punct(char),
}

fn tokens(source: &str) -> Result<Vec<Token>, String> {
    let mut found = Vec::new();
    let bytes = source.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        let c = bytes[at] as char;
        if c.is_ascii_whitespace() {
            at += 1;
        } else if source[at..].starts_with("//") {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
        } else if source[at..].starts_with("/*") {
            let end = source[at + 2..]
                .find("*/")
                .ok_or("a comment is never closed")?;
            at += end + 4;
        } else if c == '#' {
            return Err("a plugin's shader cannot use the preprocessor (#)".to_owned());
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = at;
            while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                at += 1;
            }
            found.push(Token::Word(source[start..at].to_owned()));
        } else if c.is_ascii_digit()
            || (c == '.' && bytes.get(at + 1).is_some_and(u8::is_ascii_digit))
        {
            let start = at;
            while at < bytes.len()
                && (bytes[at].is_ascii_alphanumeric()
                    || bytes[at] == b'.'
                    || ((bytes[at] == b'+' || bytes[at] == b'-')
                        && matches!(bytes[at - 1], b'e' | b'E')))
            {
                at += 1;
            }
            found.push(Token::Number(source[start..at].to_owned()));
        } else if "{}()[];,.+-*/%<>=!&|^?:~".contains(c) {
            found.push(Token::Punct(c));
            at += 1;
        } else {
            return Err(format!("`{c}` has no place in a shader"));
        }
    }
    Ok(found)
}

fn integer(token: Option<&Token>) -> Option<i64> {
    match token {
        Some(Token::Number(text)) => text.parse().ok(),
        _ => None,
    }
}

fn punct(token: Option<&Token>, wanted: char) -> bool {
    matches!(token, Some(Token::Punct(c)) if *c == wanted)
}

fn word(token: Option<&Token>) -> Option<&str> {
    match token {
        Some(Token::Word(text)) => Some(text),
        _ => None,
    }
}

/// Reads the head of a `for` loop that begins at `at` (the word `for`) and says how many
/// times it runs and where its body begins. The head must be
/// `for (int i = A; i < B; i++)`, with `<=`, `>`, `>=`, `++`, `--`, `+= N` or `-= N`.
fn loop_head(tokens: &[Token], at: usize) -> Result<(i64, usize), String> {
    let refused = || {
        "a loop is `for (int i = 0; i < 8; i++)`, its bounds and step written as numbers".to_owned()
    };
    let t = |offset: usize| tokens.get(at + offset);
    if !punct(t(1), '(') || word(t(2)) != Some("int") {
        return Err(refused());
    }
    let counter = word(t(3)).ok_or_else(refused)?;
    if !punct(t(4), '=') {
        return Err(refused());
    }
    let mut next = 5;
    let signed = |next: &mut usize| -> Option<i64> {
        let negative = punct(tokens.get(at + *next), '-');
        if negative {
            *next += 1;
        }
        let value = integer(tokens.get(at + *next))?;
        *next += 1;
        Some(if negative { -value } else { value })
    };
    let start = signed(&mut next).ok_or_else(refused)?;
    if !punct(t(next), ';') || word(t(next + 1)) != Some(counter) {
        return Err(refused());
    }
    next += 2;
    let less = punct(t(next), '<');
    let more = punct(t(next), '>');
    if !less && !more {
        return Err(refused());
    }
    next += 1;
    let inclusive = punct(t(next), '=');
    if inclusive {
        next += 1;
    }
    let end = signed(&mut next).ok_or_else(refused)?;
    if !punct(t(next), ';') || word(t(next + 1)) != Some(counter) {
        return Err(refused());
    }
    next += 2;
    let step = if punct(t(next), '+') && punct(t(next + 1), '+') {
        next += 2;
        1
    } else if punct(t(next), '-') && punct(t(next + 1), '-') {
        next += 2;
        -1
    } else if (punct(t(next), '+') || punct(t(next), '-')) && punct(t(next + 1), '=') {
        let sign = if punct(t(next), '+') { 1 } else { -1 };
        next += 2;
        let amount = integer(t(next)).filter(|n| *n > 0).ok_or_else(refused)?;
        next += 1;
        sign * amount
    } else {
        return Err(refused());
    };
    if !punct(t(next), ')') {
        return Err(refused());
    }
    // The counter must go towards the bound.
    if (less && step < 0) || (more && step > 0) {
        return Err(format!("the loop over `{counter}` never ends"));
    }
    let span = if less {
        end - start + i64::from(inclusive)
    } else {
        start - end + i64::from(inclusive)
    };
    let runs = if span <= 0 {
        0
    } else {
        (span + step.abs() - 1) / step.abs()
    };
    Ok((runs, at + next + 1))
}

/// Checks a plugin's shader. The source must be the function and its helpers alone.
pub fn check(source: &str) -> Result<(), String> {
    if source.len() > SHADER_MAX_BYTES {
        return Err(format!(
            "a shader may be {} KiB at most",
            SHADER_MAX_BYTES / 1024
        ));
    }
    if !source
        .chars()
        .all(|c| c.is_ascii_graphic() || c == ' ' || c == '\n' || c == '\r' || c == '\t')
    {
        return Err("a shader is plain ASCII text".to_owned());
    }
    let tokens = tokens(source)?;
    // Each loop open now, by the depth of braces its body is at and how many times it
    // runs.
    let mut open: Vec<(usize, i64)> = Vec::new();
    let mut depth = 0usize;
    let mut entry = false;
    let mut index = 0;
    while index < tokens.len() {
        match &tokens[index] {
            Token::Word(name) => {
                if FORBIDDEN.contains(&name.as_str()) {
                    return Err(format!("a plugin's shader cannot use `{name}`"));
                }
                if name.starts_with("gl_") || name.starts_with("__") {
                    return Err(format!("`{name}` is the shader language's own"));
                }
                if name.starts_with("sampler") {
                    return Err(format!("a plugin's shader cannot use `{name}`"));
                }
                if name.starts_with("lume_") && name != ENTRY && !PROVIDED.contains(&name.as_str())
                {
                    return Err(format!(
                        "`{name}`: names beginning with lume_ are the application's"
                    ));
                }
                if (name == "in" || name == "out" || name == "inout") && depth == 0 {
                    // At the top they would declare inputs and outputs; inside a list
                    // of parameters, which is not at the top of braces but inside
                    // brackets, they qualify a parameter.
                    let in_parameters = tokens[..index]
                        .iter()
                        .rev()
                        .find_map(|token| match token {
                            Token::Punct('(') => Some(true),
                            Token::Punct(')' | ';' | '{' | '}') => Some(false),
                            _ => None,
                        })
                        .unwrap_or(false);
                    if !in_parameters {
                        return Err(format!(
                            "a plugin's shader cannot declare `{name}` values of its own"
                        ));
                    }
                }
                if name == ENTRY
                    && word(index.checked_sub(1).and_then(|at| tokens.get(at))) == Some("vec4")
                    && punct(tokens.get(index + 1), '(')
                    && word(tokens.get(index + 2)) == Some("vec2")
                {
                    entry = true;
                }
                if name == "for" {
                    let (runs, body) = loop_head(&tokens, index)?;
                    if runs > LOOP_MAX {
                        return Err(format!("a loop runs {runs} times; {LOOP_MAX} at most"));
                    }
                    if !punct(tokens.get(body), '{') {
                        return Err("a loop's body is written in braces".to_owned());
                    }
                    let together =
                        open.iter().map(|(_, runs)| *runs).product::<i64>() * runs.max(1);
                    if together > NESTED_MAX {
                        return Err(format!(
                            "loops inside each other run {together} times together; \
                             {NESTED_MAX} at most"
                        ));
                    }
                    depth += 1;
                    open.push((depth, runs.max(1)));
                    index = body + 1;
                    continue;
                }
            }
            Token::Punct('{') => depth += 1,
            Token::Punct('}') => {
                if depth == 0 {
                    return Err("a brace is closed that was never opened".to_owned());
                }
                if open.last().is_some_and(|(at, _)| *at == depth) {
                    open.pop();
                }
                depth -= 1;
            }
            _ => {}
        }
        index += 1;
    }
    if depth != 0 {
        return Err("a brace is never closed".to_owned());
    }
    if !entry {
        return Err(format!(
            "the shader defines no `vec4 {ENTRY}(vec2 pixel)`, which draws the effect"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GLOW: &str = r"
// A glow round the cell the cursor lands in.
float box(vec2 p, vec4 r) {
  vec2 d = abs(p - (r.xy + r.zw * 0.5)) - r.zw * 0.5;
  return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
}

vec4 lume_effect(vec2 pixel) {
  float d = box(pixel, lume_to);
  float glow = 0.0;
  for (int i = 0; i < 4; i++) {
    glow += exp(-max(d, 0.0) / (4.0 + float(i) * 3.0)) * 0.25;
  }
  return vec4(lume_color, glow * (1.0 - lume_progress));
}
";

    #[test]
    fn a_shader_that_keeps_to_the_rules_is_taken() {
        assert_eq!(check(GLOW), Ok(()));
        let nested = "vec4 lume_effect(vec2 p) { float a = 0.0;\n\
                      for (int i = 0; i < 32; i++) { for (int j = 32; j > 0; j -= 1) { a += 1.0; } }\n\
                      return vec4(a); }";
        assert_eq!(check(nested), Ok(()));
        let parameters = "void f(in vec2 a, out float b) { b = a.x; }\n\
                          vec4 lume_effect(vec2 p) { float b; f(p, b); return vec4(b); }";
        assert_eq!(check(parameters), Ok(()));
    }

    #[test]
    fn what_could_run_long_or_reach_further_is_refused() {
        let wrap = |body: &str| format!("vec4 lume_effect(vec2 p) {{ {body} return vec4(0.0); }}");
        let says = |source: &str, what: &str| {
            let found = check(source).unwrap_err();
            assert!(found.contains(what), "{source}: {found}");
        };
        says(&wrap("while (true) {}"), "while");
        says(&wrap("for (int i = 0; i < 1000; i++) {}"), "1000 times");
        says(
            &wrap("int n = 4; for (int i = 0; i < n; i++) {}"),
            "written as numbers",
        );
        says(&wrap("for (int i = 0; i < 8; i--) {}"), "never ends");
        says(&wrap("for (int i = 0; i < 8; i++) p.x += 1.0;"), "braces");
        says(
            &wrap("for (int i = 0; i < 64; i++) { for (int j = 0; j < 64; j++) {} }"),
            "4096 times",
        );
        says(&format!("uniform sampler2D t;\n{}", wrap("")), "uniform");
        says(&wrap("vec4 c = texture(t, p);"), "texture");
        says(&format!("#define X 1\n{}", wrap("")), "preprocessor");
        says(&wrap("float x = gl_FragCoord.x;"), "language's own");
        says(&wrap("float x = lume_secret;"), "application's");
        says(&format!("out vec4 colour;\n{}", wrap("")), "declare");
        says("vec4 effect(vec2 p) { return vec4(0.0); }", "defines no");
        says(&wrap("{"), "never closed");
        says(&"x".repeat(SHADER_MAX_BYTES + 1), "KiB");
        says(&wrap("float é = 1.0;"), "ASCII");
    }
}
