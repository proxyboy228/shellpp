//! A small JSON reader for `server.json` of the OpenSSH test server
//! (tests/openssh/README.md). shellpp-ssh has no JSON dependency and its
//! dependencies are frozen in stage 2, so the tests carry this reader.

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Json::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn number(&self) -> Option<f64> {
        match self {
            Json::Number(value) => Some(*value),
            _ => None,
        }
    }

    pub fn entries(&self) -> &[(String, Json)] {
        match self {
            Json::Object(entries) => entries,
            _ => &[],
        }
    }

    /// Follows a path of object keys.
    pub fn path(&self, keys: &[&str]) -> Option<&Json> {
        keys.iter().try_fold(self, |value, key| value.get(key))
    }
}

pub fn parse(text: &str) -> Result<Json, String> {
    let mut parser = Parser {
        chars: text.chars().collect(),
        pos: 0,
    };
    let value = parser.value()?;
    parser.whitespace();
    if parser.pos != parser.chars.len() {
        return Err("trailing characters".to_owned());
    }
    Ok(value)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }

    fn whitespace(&mut self) {
        while matches!(self.peek(), Some(' ' | '\n' | '\r' | '\t')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, expected: char) -> Result<(), String> {
        match self.bump() {
            Some(c) if c == expected => Ok(()),
            other => Err(format!("expected {expected:?}, found {other:?}")),
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, String> {
        for expected in word.chars() {
            self.expect(expected)?;
        }
        Ok(value)
    }

    fn value(&mut self) -> Result<Json, String> {
        self.whitespace();
        match self.peek() {
            Some('{') => self.object(),
            Some('[') => self.array(),
            Some('"') => self.string().map(Json::String),
            Some('t') => self.literal("true", Json::Bool(true)),
            Some('f') => self.literal("false", Json::Bool(false)),
            Some('n') => self.literal("null", Json::Null),
            Some(c) if c == '-' || c.is_ascii_digit() => self.number(),
            other => Err(format!("unexpected {other:?}")),
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.expect('{')?;
        let mut entries = Vec::new();
        self.whitespace();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(Json::Object(entries));
        }
        loop {
            self.whitespace();
            let key = self.string()?;
            self.whitespace();
            self.expect(':')?;
            let value = self.value()?;
            entries.push((key, value));
            self.whitespace();
            match self.bump() {
                Some(',') => continue,
                Some('}') => return Ok(Json::Object(entries)),
                other => return Err(format!("expected ',' or '}}', found {other:?}")),
            }
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.expect('[')?;
        let mut items = Vec::new();
        self.whitespace();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.whitespace();
            match self.bump() {
                Some(',') => continue,
                Some(']') => return Ok(Json::Array(items)),
                other => return Err(format!("expected ',' or ']', found {other:?}")),
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            match self.bump().ok_or("unterminated string")? {
                '"' => return Ok(out),
                '\\' => match self.bump().ok_or("bad escape")? {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'u' => {
                        let hex: String = (0..4).filter_map(|_| self.bump()).collect();
                        let code = u32::from_str_radix(&hex, 16).map_err(|e| e.to_string())?;
                        out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                    }
                    other => return Err(format!("bad escape {other:?}")),
                },
                c => out.push(c),
            }
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c == '-' || c == '+' || c == '.' || c == 'e' || c == 'E' || c.is_ascii_digit())
        {
            self.pos += 1;
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        text.parse::<f64>()
            .map(Json::Number)
            .map_err(|e| e.to_string())
    }
}
