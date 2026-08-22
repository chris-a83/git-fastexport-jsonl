// A small JSON implementation. We only need enough of the spec to round-trip
// the shapes produced by model::Event::to_json, so this isn't a general
// purpose library: numbers are i64 only, and there's no pretty-printing.

#[derive(Debug)]
pub enum Value {
    Null,
    Bool(bool),
    Number(i64),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn to_compact_string(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Number(n) => out.push_str(&n.to_string()),
            Value::String(s) => write_json_string(s, out),
            Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Value::Object(entries) => {
                out.push('{');
                for (i, (key, value)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_json_string(key, out);
                    out.push(':');
                    value.write(out);
                }
                out.push('}');
            }
        }
    }
}

fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

pub fn parse(input: &str) -> Result<Value, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut pos = 0;
    skip_ws(&chars, &mut pos);
    let value = parse_value(&chars, &mut pos)?;
    skip_ws(&chars, &mut pos);
    if pos != chars.len() {
        return Err("trailing characters after JSON value".to_string());
    }
    Ok(value)
}

fn skip_ws(chars: &[char], pos: &mut usize) {
    while chars.get(*pos).map(|c| c.is_whitespace()).unwrap_or(false) {
        *pos += 1;
    }
}

fn parse_value(chars: &[char], pos: &mut usize) -> Result<Value, String> {
    skip_ws(chars, pos);
    match chars.get(*pos) {
        Some('{') => parse_object(chars, pos),
        Some('[') => parse_array(chars, pos),
        Some('"') => parse_string(chars, pos).map(Value::String),
        Some('t') => parse_literal(chars, pos, "true", Value::Bool(true)),
        Some('f') => parse_literal(chars, pos, "false", Value::Bool(false)),
        Some('n') => parse_literal(chars, pos, "null", Value::Null),
        Some(c) if *c == '-' || c.is_ascii_digit() => parse_number(chars, pos),
        Some(c) => Err(format!("unexpected character: {}", c)),
        None => Err("unexpected end of input".to_string()),
    }
}

fn parse_literal(chars: &[char], pos: &mut usize, lit: &str, value: Value) -> Result<Value, String> {
    let lit_chars: Vec<char> = lit.chars().collect();
    if *pos + lit_chars.len() > chars.len() || chars[*pos..*pos + lit_chars.len()] != lit_chars[..] {
        return Err(format!("expected literal '{}'", lit));
    }
    *pos += lit_chars.len();
    Ok(value)
}

fn parse_object(chars: &[char], pos: &mut usize) -> Result<Value, String> {
    *pos += 1; // consume '{'
    let mut entries = Vec::new();
    skip_ws(chars, pos);
    if chars.get(*pos) == Some(&'}') {
        *pos += 1;
        return Ok(Value::Object(entries));
    }
    loop {
        skip_ws(chars, pos);
        if chars.get(*pos) != Some(&'"') {
            return Err("expected string key in object".to_string());
        }
        let key = parse_string(chars, pos)?;
        skip_ws(chars, pos);
        if chars.get(*pos) != Some(&':') {
            return Err("expected ':' after object key".to_string());
        }
        *pos += 1;
        let value = parse_value(chars, pos)?;
        entries.push((key, value));
        skip_ws(chars, pos);
        match chars.get(*pos) {
            Some(',') => {
                *pos += 1;
            }
            Some('}') => {
                *pos += 1;
                break;
            }
            _ => return Err("expected ',' or '}' in object".to_string()),
        }
    }
    Ok(Value::Object(entries))
}

fn parse_array(chars: &[char], pos: &mut usize) -> Result<Value, String> {
    *pos += 1; // consume '['
    let mut items = Vec::new();
    skip_ws(chars, pos);
    if chars.get(*pos) == Some(&']') {
        *pos += 1;
        return Ok(Value::Array(items));
    }
    loop {
        let value = parse_value(chars, pos)?;
        items.push(value);
        skip_ws(chars, pos);
        match chars.get(*pos) {
            Some(',') => {
                *pos += 1;
            }
            Some(']') => {
                *pos += 1;
                break;
            }
            _ => return Err("expected ',' or ']' in array".to_string()),
        }
    }
    Ok(Value::Array(items))
}

fn parse_string(chars: &[char], pos: &mut usize) -> Result<String, String> {
    *pos += 1; // consume opening quote
    let mut out = String::new();
    loop {
        match chars.get(*pos) {
            None => return Err("unterminated string".to_string()),
            Some('"') => {
                *pos += 1;
                break;
            }
            Some('\\') => {
                *pos += 1;
                match chars.get(*pos) {
                    Some('"') => {
                        out.push('"');
                        *pos += 1;
                    }
                    Some('\\') => {
                        out.push('\\');
                        *pos += 1;
                    }
                    Some('/') => {
                        out.push('/');
                        *pos += 1;
                    }
                    Some('b') => {
                        out.push('\u{8}');
                        *pos += 1;
                    }
                    Some('f') => {
                        out.push('\u{c}');
                        *pos += 1;
                    }
                    Some('n') => {
                        out.push('\n');
                        *pos += 1;
                    }
                    Some('r') => {
                        out.push('\r');
                        *pos += 1;
                    }
                    Some('t') => {
                        out.push('\t');
                        *pos += 1;
                    }
                    Some('u') => {
                        *pos += 1;
                        let cp = parse_hex4(chars, pos)?;
                        if (0xD800..=0xDBFF).contains(&cp) {
                            if chars.get(*pos) == Some(&'\\') && chars.get(*pos + 1) == Some(&'u') {
                                *pos += 2;
                                let low = parse_hex4(chars, pos)?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err("invalid low surrogate in \\u escape".to_string());
                                }
                                let combined = 0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                                let ch = char::from_u32(combined)
                                    .ok_or_else(|| "invalid unicode escape".to_string())?;
                                out.push(ch);
                            } else {
                                return Err("unpaired high surrogate in \\u escape".to_string());
                            }
                        } else {
                            let ch = char::from_u32(cp).ok_or_else(|| "invalid unicode escape".to_string())?;
                            out.push(ch);
                        }
                    }
                    _ => return Err("invalid escape sequence".to_string()),
                }
            }
            Some(&c) => {
                out.push(c);
                *pos += 1;
            }
        }
    }
    Ok(out)
}

fn parse_hex4(chars: &[char], pos: &mut usize) -> Result<u32, String> {
    if *pos + 4 > chars.len() {
        return Err("truncated \\u escape".to_string());
    }
    let hex: String = chars[*pos..*pos + 4].iter().collect();
    *pos += 4;
    u32::from_str_radix(&hex, 16).map_err(|_| "invalid hex digits in \\u escape".to_string())
}

fn parse_number(chars: &[char], pos: &mut usize) -> Result<Value, String> {
    let start = *pos;
    if chars.get(*pos) == Some(&'-') {
        *pos += 1;
    }
    while chars.get(*pos).map(|c| c.is_ascii_digit()).unwrap_or(false) {
        *pos += 1;
    }
    let text: String = chars[start..*pos].iter().collect();
    text.parse::<i64>().map(Value::Number).map_err(|_| format!("invalid number: {}", text))
}
