//! A tiny JSON reader/writer.
//!
//! Settings and high scores are the only things that need serialising in this game, and pulling an
//! HTML-sized serde graph into a dependency-light workspace to write ten integers would be silly.
//! The parser is strict about what it accepts and forgiving about what it finds on disk: a corrupt
//! file yields an error the caller turns into defaults, never a panic.

use std::fmt::{self, Write as _};

/// Any JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    /// Insertion-ordered object: settings files stay readable in a diff.
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn object() -> Self {
        Value::Object(Vec::new())
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        self.as_f64().and_then(|n| {
            if n.is_finite() && n >= 0.0 && n <= u64::MAX as f64 {
                Some(n as u64)
            } else {
                None
            }
        })
    }

    pub fn as_u32(&self) -> Option<u32> {
        self.as_u64().and_then(|n| u32::try_from(n).ok())
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    /// `key` as a string, falling back to `default`.
    pub fn str_or(&self, key: &str, default: &str) -> String {
        self.get(key)
            .and_then(Value::as_str)
            .unwrap_or(default)
            .to_string()
    }

    /// `key` as `u64`, falling back to `default`.
    pub fn u64_or(&self, key: &str, default: u64) -> u64 {
        self.get(key).and_then(Value::as_u64).unwrap_or(default)
    }

    /// `key` as `u32`, falling back to `default`.
    pub fn u32_or(&self, key: &str, default: u32) -> u32 {
        self.get(key).and_then(Value::as_u32).unwrap_or(default)
    }

    /// `key` as `f64`, falling back to `default`.
    pub fn f64_or(&self, key: &str, default: f64) -> f64 {
        self.get(key)
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
            .unwrap_or(default)
    }

    /// `key` as `bool`, falling back to `default`.
    pub fn bool_or(&self, key: &str, default: bool) -> bool {
        self.get(key).and_then(Value::as_bool).unwrap_or(default)
    }

    /// Append a key/value pair to an object (no-op if `self` is not an object).
    pub fn set(&mut self, key: &str, value: Value) {
        if let Value::Object(entries) = self {
            match entries.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = value,
                None => entries.push((key.to_string(), value)),
            }
        }
    }

    /// Serialise with two-space indentation, one key per line.
    pub fn to_pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0)
            .expect("writing to a String cannot fail");
        out
    }

    fn write(&self, out: &mut String, depth: usize) -> fmt::Result {
        let pad = "  ".repeat(depth);
        let pad_inner = "  ".repeat(depth + 1);
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(true) => out.push_str("true"),
            Value::Bool(false) => out.push_str("false"),
            Value::Number(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15_f64 {
                    write!(out, "{}", *n as i64)?;
                } else {
                    // Five decimals with trailing zeros trimmed: an f32 volume of 0.85 must not be
                    // written into a human-readable file as 0.8500000238418579.
                    let text = format!("{n:.5}");
                    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
                    out.push_str(if trimmed.is_empty() { "0" } else { trimmed });
                }
            }
            Value::String(s) => write_string(out, s)?,
            Value::Array(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return Ok(());
                }
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    out.push_str(&pad_inner);
                    item.write(out, depth + 1)?;
                    if i + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&pad);
                out.push(']');
            }
            Value::Object(entries) => {
                if entries.is_empty() {
                    out.push_str("{}");
                    return Ok(());
                }
                out.push_str("{\n");
                for (i, (key, value)) in entries.iter().enumerate() {
                    out.push_str(&pad_inner);
                    write_string(out, key)?;
                    out.push_str(": ");
                    value.write(out, depth + 1)?;
                    if i + 1 < entries.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&pad);
                out.push('}');
            }
        }
        Ok(())
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let pretty = self.to_pretty();
        f.write_str(&pretty)
    }
}

fn write_string(out: &mut String, s: &str) -> fmt::Result {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                write!(out, "\\u{:04x}", c as u32)?;
            }
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(())
}

/// Parse a JSON document.
pub fn parse(input: &str) -> Result<Value, String> {
    let bytes: Vec<char> = input.chars().collect();
    let mut parser = Parser {
        chars: &bytes,
        pos: 0,
    };
    parser.skip_ws();
    let value = parser.value()?;
    parser.skip_ws();
    if parser.pos != parser.chars.len() {
        return Err(format!("trailing characters at byte {}", parser.pos));
    }
    Ok(value)
}

struct Parser<'a> {
    chars: &'a [char],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, c: char) -> Result<(), String> {
        match self.peek() {
            Some(got) if got == c => {
                self.pos += 1;
                Ok(())
            }
            other => Err(format!("expected '{c}' at {}, found {other:?}", self.pos)),
        }
    }

    fn literal(&mut self, word: &str) -> Result<(), String> {
        for expected in word.chars() {
            match self.peek() {
                Some(c) if c == expected => self.pos += 1,
                _ => return Err(format!("expected '{word}' at {}", self.pos)),
            }
        }
        Ok(())
    }

    fn value(&mut self) -> Result<Value, String> {
        match self.peek() {
            Some('{') => self.object(),
            Some('[') => self.array(),
            Some('"') => Ok(Value::String(self.string()?)),
            Some('t') => {
                self.literal("true")?;
                Ok(Value::Bool(true))
            }
            Some('f') => {
                self.literal("false")?;
                Ok(Value::Bool(false))
            }
            Some('n') => {
                self.literal("null")?;
                Ok(Value::Null)
            }
            Some(c) if c == '-' || c.is_ascii_digit() => self.number(),
            other => Err(format!("unexpected character {other:?} at {}", self.pos)),
        }
    }

    fn object(&mut self) -> Result<Value, String> {
        self.expect('{')?;
        let mut entries = Vec::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(Value::Object(entries));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.expect(':')?;
            self.skip_ws();
            let value = self.value()?;
            entries.push((key, value));
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                }
                Some('}') => {
                    self.pos += 1;
                    return Ok(Value::Object(entries));
                }
                other => {
                    return Err(format!(
                        "expected ',' or '}}' at {}, found {other:?}",
                        self.pos
                    ))
                }
            }
        }
    }

    fn array(&mut self) -> Result<Value, String> {
        self.expect('[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                }
                Some(']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                other => {
                    return Err(format!(
                        "expected ',' or ']' at {}, found {other:?}",
                        self.pos
                    ))
                }
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err("unterminated string".to_string()),
                Some('"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some('\\') => {
                    self.pos += 1;
                    // The `\u` arm advances past its own digits; the simple escapes step over the
                    // character they consumed.
                    let mut step = 1;
                    match self.peek() {
                        Some('n') => out.push('\n'),
                        Some('t') => out.push('\t'),
                        Some('r') => out.push('\r'),
                        Some('b') => out.push('\u{8}'),
                        Some('f') => out.push('\u{c}'),
                        Some('"') => out.push('"'),
                        Some('\\') => out.push('\\'),
                        Some('/') => out.push('/'),
                        Some('u') => {
                            self.pos += 1;
                            let code = self.hex4()?;
                            out.push(
                                char::from_u32(code).ok_or_else(|| format!("bad \\u{code:04x}"))?,
                            );
                            // hex4 already left `pos` on the character after the digits.
                            step = 0;
                        }
                        other => return Err(format!("bad escape '\\{other:?}'")),
                    }
                    self.pos += step;
                }
                Some(c) => {
                    out.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let mut value = 0u32;
        for _ in 0..4 {
            let c = self.peek().ok_or("truncated \\u escape")?;
            let digit = c
                .to_digit(16)
                .ok_or_else(|| format!("bad hex digit '{c}'"))?;
            value = value * 16 + digit;
            self.pos += 1;
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        let mut digits = 0;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.pos += 1;
            digits += 1;
        }
        if digits == 0 {
            return Err(format!("bad number at {start}"));
        }
        if self.peek() == Some('.') {
            self.pos += 1;
            let mut frac = 0;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
                frac += 1;
            }
            if frac == 0 {
                return Err(format!("bad number at {start}"));
            }
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            self.pos += 1;
            if matches!(self.peek(), Some('+') | Some('-')) {
                self.pos += 1;
            }
            let mut exp_digits = 0;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
                exp_digits += 1;
            }
            if exp_digits == 0 {
                return Err(format!("bad number at {start}"));
            }
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        text.parse::<f64>()
            .map(Value::Number)
            .map_err(|e| format!("bad number '{text}': {e}"))
    }
}
