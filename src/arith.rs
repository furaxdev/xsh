use std::collections::HashMap;

pub fn eval(src: &str, vars: &HashMap<String, String>) -> Result<i64, String> {
    let mut p = ArithParser {
        chars: src.chars().collect(),
        pos: 0,
        vars,
    };
    let v = p.parse_expr()?;
    p.skip_ws();
    if p.pos != p.chars.len() {
        return Err(format!("arith: unexpected trailing input in '{}'", src));
    }
    Ok(v)
}

struct ArithParser<'a> {
    chars: Vec<char>,
    pos: usize,
    vars: &'a HashMap<String, String>,
}

impl<'a> ArithParser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.skip_ws();
        self.chars.get(self.pos).copied()
    }

    fn parse_expr(&mut self) -> Result<i64, String> {
        self.parse_cmp()
    }

    fn parse_cmp(&mut self) -> Result<i64, String> {
        let mut v = self.parse_add()?;
        loop {
            self.skip_ws();
            let ops = ["==", "!=", "<=", ">=", "<", ">"];
            let mut matched = None;
            for op in ops {
                if self.chars[self.pos..].starts_with(&op.chars().collect::<Vec<_>>()[..]) {
                    matched = Some(op);
                    break;
                }
            }
            match matched {
                Some(op) => {
                    self.pos += op.len();
                    let rhs = self.parse_add()?;
                    v = match op {
                        "==" => (v == rhs) as i64,
                        "!=" => (v != rhs) as i64,
                        "<=" => (v <= rhs) as i64,
                        ">=" => (v >= rhs) as i64,
                        "<" => (v < rhs) as i64,
                        ">" => (v > rhs) as i64,
                        _ => unreachable!(),
                    };
                }
                None => break,
            }
        }
        Ok(v)
    }

    fn parse_add(&mut self) -> Result<i64, String> {
        let mut v = self.parse_mul()?;
        loop {
            match self.peek() {
                Some('+') => {
                    self.pos += 1;
                    v += self.parse_mul()?;
                }
                Some('-') => {
                    self.pos += 1;
                    v -= self.parse_mul()?;
                }
                _ => break,
            }
        }
        Ok(v)
    }

    fn parse_mul(&mut self) -> Result<i64, String> {
        let mut v = self.parse_unary()?;
        loop {
            match self.peek() {
                Some('*') => {
                    self.pos += 1;
                    v *= self.parse_unary()?;
                }
                Some('/') => {
                    self.pos += 1;
                    let rhs = self.parse_unary()?;
                    if rhs == 0 {
                        return Err("arith: division by zero".into());
                    }
                    v /= rhs;
                }
                Some('%') => {
                    self.pos += 1;
                    let rhs = self.parse_unary()?;
                    if rhs == 0 {
                        return Err("arith: division by zero".into());
                    }
                    v %= rhs;
                }
                _ => break,
            }
        }
        Ok(v)
    }

    fn parse_unary(&mut self) -> Result<i64, String> {
        match self.peek() {
            Some('-') => {
                self.pos += 1;
                Ok(-self.parse_unary()?)
            }
            Some('+') => {
                self.pos += 1;
                self.parse_unary()
            }
            Some('!') => {
                self.pos += 1;
                Ok((self.parse_unary()? == 0) as i64)
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<i64, String> {
        match self.peek() {
            Some('(') => {
                self.pos += 1;
                let v = self.parse_expr()?;
                self.skip_ws();
                if self.peek() != Some(')') {
                    return Err("arith: expected ')'".into());
                }
                self.pos += 1;
                Ok(v)
            }
            Some(c) if c.is_ascii_digit() => {
                let start = self.pos;
                while self.pos < self.chars.len() && self.chars[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
                let s: String = self.chars[start..self.pos].iter().collect();
                s.parse::<i64>().map_err(|e| e.to_string())
            }
            Some(c) if c.is_alphabetic() || c == '_' => {
                let start = self.pos;
                while self.pos < self.chars.len()
                    && (self.chars[self.pos].is_alphanumeric() || self.chars[self.pos] == '_')
                {
                    self.pos += 1;
                }
                let name: String = self.chars[start..self.pos].iter().collect();
                Ok(self
                    .vars
                    .get(&name)
                    .and_then(|v| v.trim().parse::<i64>().ok())
                    .unwrap_or(0))
            }
            other => Err(format!("arith: unexpected token {:?}", other)),
        }
    }
}
