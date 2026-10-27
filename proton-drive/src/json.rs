//! Small dependency-free JSON representation and strict parser.

use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

impl JsonValue {
    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(values) => values.get(key),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Number(value) => value.parse().ok(),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(values) => Some(values),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JsonError {
    pub offset: usize,
    pub message: &'static str,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for JsonError {}

pub fn parse(input: &str) -> Result<JsonValue, JsonError> {
    let mut parser = Parser {
        input: input.as_bytes(),
        offset: 0,
    };
    let value = parser.value()?;
    parser.whitespace();
    if parser.offset != parser.input.len() {
        return parser.error("trailing data");
    }
    Ok(value)
}

struct Parser<'a> {
    input: &'a [u8],
    offset: usize,
}

impl Parser<'_> {
    fn value(&mut self) -> Result<JsonValue, JsonError> {
        self.whitespace();
        match self.peek() {
            Some(b'n') => self.literal(b"null", JsonValue::Null),
            Some(b't') => self.literal(b"true", JsonValue::Bool(true)),
            Some(b'f') => self.literal(b"false", JsonValue::Bool(false)),
            Some(b'"') => self.string().map(JsonValue::String),
            Some(b'[') => self.array(),
            Some(b'{') => self.object(),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => self.error("expected JSON value"),
        }
    }

    fn literal(&mut self, literal: &[u8], value: JsonValue) -> Result<JsonValue, JsonError> {
        if self.input.get(self.offset..self.offset + literal.len()) == Some(literal) {
            self.offset += literal.len();
            Ok(value)
        } else {
            self.error("invalid literal")
        }
    }

    fn array(&mut self) -> Result<JsonValue, JsonError> {
        self.offset += 1;
        let mut values = Vec::new();
        self.whitespace();
        if self.consume(b']') {
            return Ok(JsonValue::Array(values));
        }
        loop {
            values.push(self.value()?);
            self.whitespace();
            if self.consume(b']') {
                return Ok(JsonValue::Array(values));
            }
            if !self.consume(b',') {
                return self.error("expected ',' or ']'");
            }
        }
    }

    fn object(&mut self) -> Result<JsonValue, JsonError> {
        self.offset += 1;
        let mut values = BTreeMap::new();
        self.whitespace();
        if self.consume(b'}') {
            return Ok(JsonValue::Object(values));
        }
        loop {
            self.whitespace();
            if self.peek() != Some(b'"') {
                return self.error("expected object key");
            }
            let key = self.string()?;
            self.whitespace();
            if !self.consume(b':') {
                return self.error("expected ':'");
            }
            values.insert(key, self.value()?);
            self.whitespace();
            if self.consume(b'}') {
                return Ok(JsonValue::Object(values));
            }
            if !self.consume(b',') {
                return self.error("expected ',' or '}'");
            }
        }
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.offset += 1;
        let mut value = String::new();
        loop {
            let byte = self.peek().ok_or(JsonError {
                offset: self.offset,
                message: "unterminated string",
            })?;
            match byte {
                b'"' => {
                    self.offset += 1;
                    return Ok(value);
                }
                b'\\' => {
                    self.offset += 1;
                    let escaped = self.next().ok_or(JsonError {
                        offset: self.offset,
                        message: "unterminated escape",
                    })?;
                    match escaped {
                        b'"' => value.push('"'),
                        b'\\' => value.push('\\'),
                        b'/' => value.push('/'),
                        b'b' => value.push('\u{0008}'),
                        b'f' => value.push('\u{000c}'),
                        b'n' => value.push('\n'),
                        b'r' => value.push('\r'),
                        b't' => value.push('\t'),
                        b'u' => self.unicode_escape(&mut value)?,
                        _ => return self.error("invalid escape"),
                    }
                }
                0x00..=0x1f => return self.error("control character in string"),
                0x20..=0x7f => {
                    self.offset += 1;
                    value.push(char::from(byte));
                }
                _ => {
                    let remaining =
                        std::str::from_utf8(&self.input[self.offset..]).map_err(|_| JsonError {
                            offset: self.offset,
                            message: "invalid UTF-8",
                        })?;
                    let character = remaining.chars().next().expect("non-empty UTF-8 slice");
                    self.offset += character.len_utf8();
                    value.push(character);
                }
            }
        }
    }

    fn unicode_escape(&mut self, value: &mut String) -> Result<(), JsonError> {
        let first = self.hex4()?;
        let codepoint = if (0xd800..=0xdbff).contains(&first) {
            if self.next() != Some(b'\\') || self.next() != Some(b'u') {
                return self.error("missing low surrogate");
            }
            let second = self.hex4()?;
            if !(0xdc00..=0xdfff).contains(&second) {
                return self.error("invalid low surrogate");
            }
            0x10000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
        } else if (0xdc00..=0xdfff).contains(&first) {
            return self.error("unexpected low surrogate");
        } else {
            u32::from(first)
        };
        value.push(char::from_u32(codepoint).ok_or(JsonError {
            offset: self.offset,
            message: "invalid Unicode scalar",
        })?);
        Ok(())
    }

    fn hex4(&mut self) -> Result<u16, JsonError> {
        let mut value = 0_u16;
        for _ in 0..4 {
            let digit = self.next().and_then(|byte| (byte as char).to_digit(16));
            let Some(digit) = digit else {
                return self.error("invalid Unicode escape");
            };
            value = value * 16 + digit as u16;
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<JsonValue, JsonError> {
        let start = self.offset;
        self.consume(b'-');
        match self.peek() {
            Some(b'0') => self.offset += 1,
            Some(b'1'..=b'9') => {
                self.offset += 1;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.offset += 1;
                }
            }
            _ => return self.error("invalid number"),
        }
        if self.consume(b'.') {
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return self.error("invalid fraction");
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.offset += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.offset += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.offset += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return self.error("invalid exponent");
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.offset += 1;
            }
        }
        let number = std::str::from_utf8(&self.input[start..self.offset])
            .expect("JSON number is ASCII")
            .to_owned();
        Ok(JsonValue::Number(number))
    }

    fn whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.offset += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.offset).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let value = self.peek()?;
        self.offset += 1;
        Some(value)
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.offset += 1;
            true
        } else {
            false
        }
    }

    fn error<T>(&self, message: &'static str) -> Result<T, JsonError> {
        Err(JsonError {
            offset: self.offset,
            message,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_json_and_surrogates() {
        let value =
            parse(r#"{"text":"caf\u00e9 \ud83d\ude80","n":-1.25e2,"a":[true,null]}"#).unwrap();
        assert_eq!(
            value.get("text").and_then(JsonValue::as_str),
            Some("café 🚀")
        );
        assert_eq!(value.get("n"), Some(&JsonValue::Number("-1.25e2".into())));
    }

    #[test]
    fn rejects_trailing_and_invalid_json() {
        assert!(parse("{} x").is_err());
        assert!(parse("[1,]").is_err());
        assert!(parse(r#""\ud800""#).is_err());
    }
}
