use std::fs;

pub fn run(args: &[String]) -> i32 {
    match eval(args) {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(e) => {
            eprintln!("xsh: test: {}", e);
            2
        }
    }
}

fn eval(args: &[String]) -> Result<bool, String> {
    if args.is_empty() {
        return Ok(false);
    }
    let mut p = P { a: args, pos: 0 };
    let v = p.or_expr()?;
    if p.pos != p.a.len() {
        return Err(format!("unexpected argument '{}'", p.a[p.pos]));
    }
    Ok(v)
}

struct P<'a> {
    a: &'a [String],
    pos: usize,
}

impl<'a> P<'a> {
    fn or_expr(&mut self) -> Result<bool, String> {
        let mut v = self.and_expr()?;
        while self.peek() == Some("-o") {
            self.pos += 1;
            let rhs = self.and_expr()?;
            v = v || rhs;
        }
        Ok(v)
    }

    fn and_expr(&mut self) -> Result<bool, String> {
        let mut v = self.not_expr()?;
        while self.peek() == Some("-a") {
            self.pos += 1;
            let rhs = self.not_expr()?;
            v = v && rhs;
        }
        Ok(v)
    }

    fn not_expr(&mut self) -> Result<bool, String> {
        if self.peek() == Some("!") {
            self.pos += 1;
            return Ok(!self.not_expr()?);
        }
        self.primary()
    }

    fn peek(&self) -> Option<&str> {
        self.a.get(self.pos).map(|s| s.as_str())
    }

    fn next(&mut self) -> Result<String, String> {
        let v = self
            .a
            .get(self.pos)
            .cloned()
            .ok_or_else(|| "unexpected end of expression".to_string())?;
        self.pos += 1;
        Ok(v)
    }

    fn primary(&mut self) -> Result<bool, String> {
        match self.peek() {
            Some("-z") => {
                self.pos += 1;
                Ok(self.next()?.is_empty())
            }
            Some("-n") => {
                self.pos += 1;
                Ok(!self.next()?.is_empty())
            }
            Some(op @ ("-e" | "-f" | "-d" | "-r" | "-w" | "-x" | "-s" | "-L")) => {
                let op = op.to_string();
                self.pos += 1;
                let path = self.next()?;
                Ok(file_test(&op, &path))
            }
            Some("(") => {
                self.pos += 1;
                let v = self.or_expr()?;
                if self.peek() != Some(")") {
                    return Err("expected ')'".into());
                }
                self.pos += 1;
                Ok(v)
            }
            _ => {
                let lhs = self.next()?;
                match self.peek() {
                    Some(op @ ("=" | "!=")) => {
                        let op = op.to_string();
                        self.pos += 1;
                        let rhs = self.next()?;
                        Ok(if op == "=" { lhs == rhs } else { lhs != rhs })
                    }
                    Some(
                        op @ ("-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge"),
                    ) => {
                        let op = op.to_string();
                        self.pos += 1;
                        let rhs = self.next()?;
                        let a: i64 = lhs
                            .trim()
                            .parse()
                            .map_err(|_| format!("integer expected: '{}'", lhs))?;
                        let b: i64 = rhs
                            .trim()
                            .parse()
                            .map_err(|_| format!("integer expected: '{}'", rhs))?;
                        Ok(match op.as_str() {
                            "-eq" => a == b,
                            "-ne" => a != b,
                            "-lt" => a < b,
                            "-le" => a <= b,
                            "-gt" => a > b,
                            "-ge" => a >= b,
                            _ => unreachable!(),
                        })
                    }
                    _ => Ok(!lhs.is_empty()),
                }
            }
        }
    }
}

fn file_test(op: &str, path: &str) -> bool {
    match op {
        "-e" => fs::metadata(path).is_ok(),
        "-f" => fs::metadata(path).map(|m| m.is_file()).unwrap_or(false),
        "-d" => fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false),
        "-s" => fs::metadata(path).map(|m| m.len() > 0).unwrap_or(false),
        "-L" => fs::symlink_metadata(path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false),
        "-r" | "-w" | "-x" => fs::metadata(path).is_ok(),
        _ => false,
    }
}
