//! Minimal, self-contained JSON parser for local configuration manifests.

#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<JsonValue>),
    Obj(Vec<(String, JsonValue)>),
}

impl JsonValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JsonValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            JsonValue::Num(n) => Some(*n as i64),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[JsonValue]> {
        match self {
            JsonValue::Arr(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            JsonValue::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(|v| v.as_str())
    }
}

pub fn parse(input: &str) -> Result<JsonValue, String> {
    let trimmed = input.strip_prefix('\u{FEFF}').unwrap_or(input);
    let mut p = Parser {
        bytes: trimmed.as_bytes(),
        pos: 0,
    };
    let val = p.parse_value()?;
    p.skip_ws();
    if p.pos != p.bytes.len() {
        return Err(format!("trailing JSON bytes at offset {}", p.pos));
    }
    Ok(val)
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() && matches!(self.bytes[self.pos], b' ' | b'\t' | b'\r' | b'\n') {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.bytes.get(self.pos).copied()
    }

    fn expect(&mut self, expected: u8) -> Result<(), String> {
        self.skip_ws();
        if self.bytes.get(self.pos).copied() == Some(expected) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at offset {}", expected as char, self.pos))
        }
    }

    fn parse_value(&mut self) -> Result<JsonValue, String> {
        match self.peek() {
            Some(b'n') => self.parse_lit(b"null", JsonValue::Null),
            Some(b't') => self.parse_lit(b"true", JsonValue::Bool(true)),
            Some(b'f') => self.parse_lit(b"false", JsonValue::Bool(false)),
            Some(b'"') => self.parse_string().map(JsonValue::Str),
            Some(b'[') => self.parse_array(),
            Some(b'{') => self.parse_object(),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(c) => Err(format!("unexpected byte '{}' at offset {}", c as char, self.pos)),
            None => Err("unexpected EOF".to_string()),
        }
    }

    fn parse_lit(&mut self, lit: &[u8], val: JsonValue) -> Result<JsonValue, String> {
        if self.bytes[self.pos..].starts_with(lit) {
            self.pos += lit.len();
            Ok(val)
        } else {
            Err(format!("invalid literal at offset {}", self.pos))
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            self.pos += 1;
            match b {
                b'"' => return Ok(out),
                b'\\' => {
                    if self.pos >= self.bytes.len() {
                        return Err("unterminated escape".to_string());
                    }
                    let esc = self.bytes[self.pos];
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let code = self.read_hex4()?;
                            if (0xD800..=0xDBFF).contains(&code) {
                                if self.bytes.get(self.pos..self.pos + 2) == Some(b"\\u") {
                                    self.pos += 2;
                                    let low = self.read_hex4()?;
                                    let cp = 0x10000 + (((code as u32 - 0xD800) << 10) | (low as u32 - 0xDC00));
                                    out.push(char::from_u32(cp).ok_or("invalid unicode surrogate")?);
                                } else {
                                    return Err("unpaired high surrogate".to_string());
                                }
                            } else {
                                out.push(char::from_u32(code as u32).ok_or("invalid unicode codepoint")?);
                            }
                        }
                        other => return Err(format!("unknown escape \\{}", other as char)),
                    }
                }
                c if c < 0x80 => out.push(c as char),
                _ => {
                    self.pos -= 1;
                    let s = std::str::from_utf8(&self.bytes[self.pos..])
                        .map_err(|_| "invalid UTF-8 in JSON string".to_string())?;
                    let ch = s.chars().next().ok_or("EOF in UTF-8 char")?;
                    self.pos += ch.len_utf8();
                    out.push(ch);
                }
            }
        }
        Err("unterminated JSON string".to_string())
    }

    fn read_hex4(&mut self) -> Result<u16, String> {
        if self.pos + 4 > self.bytes.len() {
            return Err("short \\uXXXX escape".to_string());
        }
        let s = std::str::from_utf8(&self.bytes[self.pos..self.pos + 4]).map_err(|_| "invalid hex")?;
        self.pos += 4;
        u16::from_str_radix(s, 16).map_err(|_| format!("invalid hex '{}'", s))
    }

    fn parse_number(&mut self) -> Result<JsonValue, String> {
        let start = self.pos;
        if self.bytes.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        if self.bytes.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        if matches!(self.bytes.get(self.pos), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.bytes.get(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        let s = std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|_| "invalid number")?;
        let num = s.parse::<f64>().map_err(|_| format!("invalid float '{}'", s))?;
        Ok(JsonValue::Num(num))
    }

    fn parse_array(&mut self) -> Result<JsonValue, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Arr(items));
        }
        loop {
            items.push(self.parse_value()?);
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Arr(items));
                }
                _ => return Err(format!("expected ',' or ']' at offset {}", self.pos)),
            }
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, String> {
        self.expect(b'{')?;
        let mut pairs = Vec::new();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Obj(pairs));
        }
        loop {
            let key = self.parse_string()?;
            self.expect(b':')?;
            let val = self.parse_value()?;
            pairs.push((key, val));
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Obj(pairs));
                }
                _ => return Err(format!("expected ',' or '}}' at offset {}", self.pos)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cyrillic_and_escapes() {
        let raw = r#"{"title":"Герои Меча","num":9260904,"flag":true,"arr":["a","b\"c","\u0041"]}"#;
        let v = parse(raw).unwrap();
        assert_eq!(v.get_str("title"), Some("Герои Меча"));
        assert_eq!(v.get("num").unwrap().as_i64(), Some(9260904));
        assert_eq!(v.get("flag").unwrap().as_bool(), Some(true));
        let arr = v.get("arr").unwrap().as_arr().unwrap();
        assert_eq!(arr[1].as_str(), Some("b\"c"));
        assert_eq!(arr[2].as_str(), Some("A"));
    }
}
