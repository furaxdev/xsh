use crate::ast::*;
use crate::lexer::Token;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

const KEYWORDS: &[&str] = &[
    "if", "then", "elif", "else", "fi", "for", "in", "do", "done", "while", "until", "function",
    "return", "break", "continue",
];

fn is_valid_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

fn plain_literal(w: &Word) -> Option<String> {
    if w.len() == 1 {
        if let WordPart::Literal(s) = &w[0] {
            return Some(s.clone());
        }
    }
    None
}

fn is_plain_ident(w: &Word) -> Option<String> {
    plain_literal(w).filter(|s| is_valid_ident(s))
}

fn try_split_assignment(w: &Word) -> Option<(String, Word)> {
    if let Some(WordPart::Literal(s)) = w.first() {
        if let Some(eq_idx) = s.find('=') {
            let name = &s[..eq_idx];
            if !name.is_empty() && is_valid_ident(name) {
                let rest_lit = s[eq_idx + 1..].to_string();
                let mut val = vec![WordPart::Literal(rest_lit)];
                val.extend(w[1..].iter().cloned());
                return Some((name.to_string(), val));
            }
        }
    }
    None
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset)
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), Token::Eof)
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), Token::Newline) {
            self.advance();
        }
    }

    fn kw(&self) -> Option<String> {
        if let Token::Word(w) = self.peek() {
            if let Some(s) = plain_literal(w) {
                if KEYWORDS.contains(&s.as_str()) {
                    return Some(s);
                }
            }
        }
        None
    }

    fn expect_kw(&mut self, kw: &str) -> Result<(), String> {
        if self.kw().as_deref() == Some(kw) {
            self.advance();
            Ok(())
        } else {
            Err(format!("xsh: syntax error: expected '{}'", kw))
        }
    }

    fn expect_word(&mut self) -> Result<Word, String> {
        match self.advance() {
            Token::Word(w) => Ok(w),
            _ => Err("xsh: syntax error: expected word".into()),
        }
    }

    pub fn parse_program(&mut self) -> Result<Vec<Node>, String> {
        let prog = self.parse_stmt_list(&[])?;
        Ok(prog)
    }

    fn parse_stmt_list(&mut self, stop: &[&str]) -> Result<Vec<Node>, String> {
        let mut nodes = Vec::new();
        loop {
            self.skip_newlines();
            while matches!(self.peek(), Token::Semi) {
                self.advance();
                self.skip_newlines();
            }
            if self.at_eof() {
                break;
            }
            if let Some(k) = self.kw() {
                if stop.contains(&k.as_str()) {
                    break;
                }
            }
            if stop.contains(&"}") && matches!(self.peek(), Token::RBrace) {
                break;
            }
            let mut node = self.parse_and_or()?;
            if matches!(self.peek(), Token::Amp) {
                self.advance();
                node = Node::Background(Box::new(node));
            }
            nodes.push(node);
            match self.peek() {
                Token::Semi | Token::Newline => {
                    self.advance();
                }
                _ => {}
            }
        }
        Ok(nodes)
    }

    fn parse_and_or(&mut self) -> Result<Node, String> {
        let mut current = self.parse_pipeline()?;
        let mut items: Vec<(Node, Sep)> = Vec::new();
        loop {
            let sep = match self.peek() {
                Token::AndIf => Sep::And,
                Token::OrIf => Sep::Or,
                _ => break,
            };
            self.advance();
            self.skip_newlines();
            let next = self.parse_pipeline()?;
            items.push((current, sep));
            current = next;
        }
        if items.is_empty() {
            Ok(current)
        } else {
            items.push((current, Sep::Seq));
            Ok(Node::List(items))
        }
    }

    fn parse_pipeline(&mut self) -> Result<Node, String> {
        let negate = if matches!(self.peek(), Token::Bang) {
            self.advance();
            true
        } else {
            false
        };
        let mut cmds = vec![self.parse_command()?];
        while matches!(self.peek(), Token::Pipe) {
            self.advance();
            self.skip_newlines();
            cmds.push(self.parse_command()?);
        }
        if cmds.len() == 1 && !negate {
            Ok(cmds.pop().unwrap())
        } else {
            Ok(Node::Pipeline(cmds, negate))
        }
    }

    fn parse_command(&mut self) -> Result<Node, String> {
        match self.kw().as_deref() {
            Some("if") => self.parse_if(),
            Some("for") => self.parse_for(),
            Some("while") => self.parse_while(false),
            Some("until") => self.parse_while(true),
            Some("function") => self.parse_function_kw(),
            Some("return") => {
                self.advance();
                if let Token::Word(_) = self.peek() {
                    let w = self.expect_word()?;
                    Ok(Node::Return(Some(w)))
                } else {
                    Ok(Node::Return(None))
                }
            }
            Some("break") => {
                self.advance();
                Ok(Node::Break)
            }
            Some("continue") => {
                self.advance();
                Ok(Node::Continue)
            }
            _ => {
                if matches!(self.peek(), Token::LBrace) {
                    self.advance();
                    let body = self.parse_stmt_list(&["}"])?;
                    if !matches!(self.peek(), Token::RBrace) {
                        return Err("xsh: syntax error: expected '}'".into());
                    }
                    self.advance();
                    return Ok(Node::Subshell(body));
                }
                self.parse_simple_or_funcdef()
            }
        }
    }

    fn parse_if(&mut self) -> Result<Node, String> {
        self.advance(); // if
        let cond = self.parse_stmt_list(&["then"])?;
        self.expect_kw("then")?;
        let body = self.parse_stmt_list(&["elif", "else", "fi"])?;
        let mut branches = vec![(cond, body)];
        let else_branch;
        loop {
            match self.kw().as_deref() {
                Some("elif") => {
                    self.advance();
                    let c = self.parse_stmt_list(&["then"])?;
                    self.expect_kw("then")?;
                    let b = self.parse_stmt_list(&["elif", "else", "fi"])?;
                    branches.push((c, b));
                }
                Some("else") => {
                    self.advance();
                    let b = self.parse_stmt_list(&["fi"])?;
                    self.expect_kw("fi")?;
                    else_branch = Some(b);
                    break;
                }
                Some("fi") => {
                    self.advance();
                    else_branch = None;
                    break;
                }
                _ => return Err("xsh: syntax error: expected 'fi'".into()),
            }
        }
        Ok(Node::If {
            branches,
            else_branch,
        })
    }

    fn parse_for(&mut self) -> Result<Node, String> {
        self.advance(); // for
        let name_word = self.expect_word()?;
        let var = is_plain_ident(&name_word)
            .ok_or_else(|| "xsh: syntax error: bad 'for' variable name".to_string())?;
        self.skip_newlines();
        let mut words = Vec::new();
        if self.kw().as_deref() == Some("in") {
            self.advance();
            loop {
                match self.peek().clone() {
                    Token::Word(w) => {
                        self.advance();
                        words.push(w);
                    }
                    _ => break,
                }
            }
            match self.peek() {
                Token::Semi | Token::Newline => {
                    self.advance();
                }
                _ => {}
            }
        }
        self.skip_newlines();
        self.expect_kw("do")?;
        let body = self.parse_stmt_list(&["done"])?;
        self.expect_kw("done")?;
        Ok(Node::For { var, words, body })
    }

    fn parse_while(&mut self, until: bool) -> Result<Node, String> {
        self.advance(); // while/until
        let cond = self.parse_stmt_list(&["do"])?;
        self.expect_kw("do")?;
        let body = self.parse_stmt_list(&["done"])?;
        self.expect_kw("done")?;
        Ok(Node::While { cond, body, until })
    }

    fn parse_function_kw(&mut self) -> Result<Node, String> {
        self.advance(); // function
        let name_word = self.expect_word()?;
        let name = is_plain_ident(&name_word)
            .ok_or_else(|| "xsh: syntax error: bad function name".to_string())?;
        if matches!(self.peek(), Token::LParen) {
            self.advance();
            if !matches!(self.peek(), Token::RParen) {
                return Err("xsh: syntax error: expected ')'".into());
            }
            self.advance();
        }
        self.skip_newlines();
        if !matches!(self.peek(), Token::LBrace) {
            return Err("xsh: syntax error: expected '{' after function name".into());
        }
        self.advance();
        let body = self.parse_stmt_list(&["}"])?;
        if !matches!(self.peek(), Token::RBrace) {
            return Err("xsh: syntax error: expected '}'".into());
        }
        self.advance();
        Ok(Node::FunctionDef { name, body })
    }

    fn parse_simple_or_funcdef(&mut self) -> Result<Node, String> {
        if let Token::Word(w) = self.peek().clone() {
            if let Some(name) = is_plain_ident(&w) {
                if matches!(self.peek_at(1), Some(Token::LParen))
                    && matches!(self.peek_at(2), Some(Token::RParen))
                {
                    self.advance();
                    self.advance();
                    self.advance();
                    self.skip_newlines();
                    if !matches!(self.peek(), Token::LBrace) {
                        return Err("xsh: syntax error: expected '{' after function name".into());
                    }
                    self.advance();
                    let body = self.parse_stmt_list(&["}"])?;
                    if !matches!(self.peek(), Token::RBrace) {
                        return Err("xsh: syntax error: expected '}'".into());
                    }
                    self.advance();
                    return Ok(Node::FunctionDef { name, body });
                }
            }
        }
        self.parse_simple_command()
    }

    fn parse_simple_command(&mut self) -> Result<Node, String> {
        let mut assignments = Vec::new();
        let mut words = Vec::new();
        let mut redirects = Vec::new();
        let mut seen_word = false;

        loop {
            match self.peek().clone() {
                Token::Word(w) => {
                    if !seen_word {
                        if let Some((name, val)) = try_split_assignment(&w) {
                            self.advance();
                            assignments.push((name, val));
                            continue;
                        }
                    }
                    seen_word = true;
                    self.advance();
                    words.push(w);
                }
                Token::Less => {
                    self.advance();
                    let t = self.expect_word()?;
                    redirects.push(Redirect {
                        kind: RedirectKind::In,
                        target: t,
                    });
                }
                Token::Great => {
                    self.advance();
                    let t = self.expect_word()?;
                    redirects.push(Redirect {
                        kind: RedirectKind::Out,
                        target: t,
                    });
                }
                Token::DGreat => {
                    self.advance();
                    let t = self.expect_word()?;
                    redirects.push(Redirect {
                        kind: RedirectKind::Append,
                        target: t,
                    });
                }
                Token::ErrGreat => {
                    self.advance();
                    let t = self.expect_word()?;
                    redirects.push(Redirect {
                        kind: RedirectKind::ErrOut,
                        target: t,
                    });
                }
                Token::ErrDGreat => {
                    self.advance();
                    let t = self.expect_word()?;
                    redirects.push(Redirect {
                        kind: RedirectKind::ErrAppend,
                        target: t,
                    });
                }
                Token::AndGreat => {
                    self.advance();
                    let t = self.expect_word()?;
                    redirects.push(Redirect {
                        kind: RedirectKind::Both,
                        target: t,
                    });
                }
                Token::DupErrToOut => {
                    self.advance();
                    redirects.push(Redirect {
                        kind: RedirectKind::DupErrToOut,
                        target: Vec::new(),
                    });
                }
                Token::DupOutToErr => {
                    self.advance();
                    redirects.push(Redirect {
                        kind: RedirectKind::DupOutToErr,
                        target: Vec::new(),
                    });
                }
                _ => break,
            }
        }

        if assignments.is_empty() && words.is_empty() && redirects.is_empty() {
            return Err(format!("xsh: syntax error near token '{:?}'", self.peek()));
        }

        Ok(Node::Simple(SimpleCommand {
            assignments,
            words,
            redirects,
        }))
    }
}

pub fn parse(tokens: Vec<Token>) -> Result<Vec<Node>, String> {
    let mut p = Parser::new(tokens);
    p.parse_program()
}
