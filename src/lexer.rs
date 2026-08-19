use crate::ast::{Word, WordPart};

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Word(Word),
    Pipe,       // |
    AndIf,      // &&
    OrIf,       // ||
    Semi,       // ;
    DSemi,      // ;;
    Amp,        // &
    Newline,
    LBrace, // {
    RBrace, // }
    LParen, // (
    RParen, // )
    Bang,   // !
    Less,          // <
    Great,         // >
    DGreat,        // >>
    ErrGreat,      // 2>
    ErrDGreat,     // 2>>
    AndGreat,      // &> or >&
    DupErrToOut,   // 2>&1
    DupOutToErr,   // 1>&2
    Eof,
}

pub struct Lexer<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Lexer {
            chars: input.chars().peekable(),
        }
    }

    pub fn tokenize(mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        loop {
            self.skip_spaces_and_comments();
            match self.chars.peek() {
                None => {
                    tokens.push(Token::Eof);
                    break;
                }
                Some('\n') => {
                    self.chars.next();
                    tokens.push(Token::Newline);
                }
                Some('|') => {
                    self.chars.next();
                    if self.chars.peek() == Some(&'|') {
                        self.chars.next();
                        tokens.push(Token::OrIf);
                    } else {
                        tokens.push(Token::Pipe);
                    }
                }
                Some('&') => {
                    self.chars.next();
                    if self.chars.peek() == Some(&'&') {
                        self.chars.next();
                        tokens.push(Token::AndIf);
                    } else if self.chars.peek() == Some(&'>') {
                        self.chars.next();
                        tokens.push(Token::AndGreat);
                    } else {
                        tokens.push(Token::Amp);
                    }
                }
                Some(';') => {
                    self.chars.next();
                    if self.chars.peek() == Some(&';') {
                        self.chars.next();
                        tokens.push(Token::DSemi);
                    } else {
                        tokens.push(Token::Semi);
                    }
                }
                Some('{') => {
                    self.chars.next();
                    tokens.push(Token::LBrace);
                }
                Some('}') => {
                    self.chars.next();
                    tokens.push(Token::RBrace);
                }
                Some('(') => {
                    self.chars.next();
                    tokens.push(Token::LParen);
                }
                Some(')') => {
                    self.chars.next();
                    tokens.push(Token::RParen);
                }
                Some('!') => {
                    let mut peek2 = self.chars.clone();
                    peek2.next();
                    let boundary = match peek2.peek() {
                        None => true,
                        Some(&c) => c == ' ' || c == '\t' || c == '\n',
                    };
                    if boundary {
                        self.chars.next();
                        tokens.push(Token::Bang);
                    } else {
                        let w = self.read_word()?;
                        tokens.push(Token::Word(w));
                    }
                }
                Some('<') => {
                    self.chars.next();
                    tokens.push(Token::Less);
                }
                Some('>') => {
                    self.chars.next();
                    if self.chars.peek() == Some(&'>') {
                        self.chars.next();
                        tokens.push(Token::DGreat);
                    } else if self.chars.peek() == Some(&'&') {
                        self.chars.next();
                        tokens.push(Token::AndGreat);
                    } else {
                        tokens.push(Token::Great);
                    }
                }
                Some(c) if c.is_ascii_digit() => {
                    // could be fd-prefixed redirect: 2> 2>> 2>&1 1>&2, or just part of a word
                    let saved: Vec<char> = self.remaining_clone();
                    let mut digits = String::new();
                    let mut tmp = self.chars.clone();
                    while let Some(&d) = tmp.peek() {
                        if d.is_ascii_digit() {
                            digits.push(d);
                            tmp.next();
                        } else {
                            break;
                        }
                    }
                    if let Some(&next) = tmp.peek() {
                        if next == '>' {
                            // consume digits + '>'
                            for _ in 0..digits.len() {
                                self.chars.next();
                            }
                            self.chars.next(); // '>'
                            if digits == "2" {
                                if self.chars.peek() == Some(&'&') {
                                    let mut peek2 = self.chars.clone();
                                    peek2.next();
                                    if peek2.peek() == Some(&'1') {
                                        self.chars.next(); // &
                                        self.chars.next(); // 1
                                        tokens.push(Token::DupErrToOut);
                                        continue;
                                    }
                                }
                                if self.chars.peek() == Some(&'>') {
                                    self.chars.next();
                                    tokens.push(Token::ErrDGreat);
                                } else {
                                    tokens.push(Token::ErrGreat);
                                }
                            } else if digits == "1" {
                                if self.chars.peek() == Some(&'&') {
                                    let mut peek2 = self.chars.clone();
                                    peek2.next();
                                    if peek2.peek() == Some(&'2') {
                                        self.chars.next(); // &
                                        self.chars.next(); // 2
                                        tokens.push(Token::DupOutToErr);
                                        continue;
                                    }
                                }
                                if self.chars.peek() == Some(&'>') {
                                    self.chars.next();
                                    tokens.push(Token::DGreat);
                                } else {
                                    tokens.push(Token::Great);
                                }
                            } else {
                                // unsupported fd number, treat '>' as plain redirect
                                if self.chars.peek() == Some(&'>') {
                                    self.chars.next();
                                    tokens.push(Token::DGreat);
                                } else {
                                    tokens.push(Token::Great);
                                }
                            }
                            continue;
                        }
                    }
                    let _ = saved;
                    let w = self.read_word()?;
                    tokens.push(Token::Word(w));
                }
                Some(_) => {
                    let w = self.read_word()?;
                    tokens.push(Token::Word(w));
                }
            }
        }
        Ok(tokens)
    }

    fn remaining_clone(&self) -> Vec<char> {
        self.chars.clone().collect()
    }

    fn skip_spaces_and_comments(&mut self) {
        loop {
            while let Some(&c) = self.chars.peek() {
                if c == ' ' || c == '\t' {
                    self.chars.next();
                } else {
                    break;
                }
            }
            if self.chars.peek() == Some(&'#') {
                while let Some(&c) = self.chars.peek() {
                    if c == '\n' {
                        break;
                    }
                    self.chars.next();
                }
            } else {
                break;
            }
        }
    }

    fn is_word_boundary(c: char) -> bool {
        matches!(
            c,
            ' ' | '\t' | '\n' | '|' | '&' | ';' | '<' | '>' | '(' | ')' | '{' | '}'
        )
    }

    fn read_word(&mut self) -> Result<Word, String> {
        let mut parts: Vec<WordPart> = Vec::new();
        let mut lit = String::new();

        macro_rules! push_lit {
            () => {
                if !lit.is_empty() {
                    parts.push(WordPart::Literal(std::mem::take(&mut lit)));
                }
            };
        }

        loop {
            match self.chars.peek() {
                None => break,
                Some(&c) if Self::is_word_boundary(c) => break,
                Some(&'\'') => {
                    self.chars.next();
                    while let Some(&c) = self.chars.peek() {
                        if c == '\'' {
                            self.chars.next();
                            break;
                        }
                        lit.push(c);
                        self.chars.next();
                    }
                }
                Some(&'"') => {
                    self.chars.next();
                    loop {
                        match self.chars.peek() {
                            None => return Err("unterminated double-quoted string".into()),
                            Some(&'"') => {
                                self.chars.next();
                                break;
                            }
                            Some(&'\\') => {
                                self.chars.next();
                                match self.chars.peek() {
                                    Some(&c2) if c2 == '"' || c2 == '\\' || c2 == '$' => {
                                        lit.push(c2);
                                        self.chars.next();
                                    }
                                    Some(&'\n') => {
                                        self.chars.next();
                                    }
                                    Some(&c2) => {
                                        lit.push('\\');
                                        lit.push(c2);
                                        self.chars.next();
                                    }
                                    None => return Err("unterminated double-quoted string".into()),
                                }
                            }
                            Some(&'$') => {
                                push_lit!();
                                self.read_dollar(&mut parts)?;
                            }
                            Some(&c) => {
                                lit.push(c);
                                self.chars.next();
                            }
                        }
                    }
                }
                Some(&'\\') => {
                    self.chars.next();
                    match self.chars.next() {
                        Some(c2) => lit.push(c2),
                        None => return Err("unterminated escape".into()),
                    }
                }
                Some(&'$') => {
                    push_lit!();
                    self.read_dollar(&mut parts)?;
                }
                Some(&c) => {
                    lit.push(c);
                    self.chars.next();
                }
            }
        }
        push_lit!();
        if parts.is_empty() {
            parts.push(WordPart::Literal(String::new()));
        }
        Ok(parts)
    }

    fn read_dollar(&mut self, parts: &mut Vec<WordPart>) -> Result<(), String> {
        self.chars.next(); // consume $
        match self.chars.peek() {
            Some(&'(') => {
                self.chars.next();
                if self.chars.peek() == Some(&'(') {
                    self.chars.next();
                    let mut depth = 0;
                    let mut src = String::new();
                    loop {
                        match self.chars.peek().copied() {
                            None => return Err("unterminated arithmetic expansion".into()),
                            Some('(') => {
                                src.push('(');
                                self.chars.next();
                                depth += 1;
                            }
                            Some(')') => {
                                self.chars.next();
                                if depth == 0 {
                                    if self.chars.peek() == Some(&')') {
                                        self.chars.next();
                                        break;
                                    } else {
                                        src.push(')');
                                    }
                                } else {
                                    depth -= 1;
                                    src.push(')');
                                }
                            }
                            Some(c) => {
                                src.push(c);
                                self.chars.next();
                            }
                        }
                    }
                    parts.push(WordPart::Arith(src));
                } else {
                    let mut depth = 1;
                    let mut src = String::new();
                    while let Some(&c) = self.chars.peek() {
                        self.chars.next();
                        if c == '(' {
                            depth += 1;
                            src.push(c);
                        } else if c == ')' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            src.push(c);
                        } else {
                            src.push(c);
                        }
                    }
                    if depth != 0 {
                        return Err("unterminated command substitution".into());
                    }
                    parts.push(WordPart::CmdSub(src));
                }
            }
            Some(&'{') => {
                self.chars.next();
                let mut name = String::new();
                while let Some(&c) = self.chars.peek() {
                    if c == '}' {
                        self.chars.next();
                        break;
                    }
                    name.push(c);
                    self.chars.next();
                }
                parts.push(WordPart::Var(name));
            }
            Some(&c) if c == '?' || c == '$' || c == '#' || c == '!' || c == '@' || c == '*' => {
                self.chars.next();
                parts.push(WordPart::Var(c.to_string()));
            }
            Some(&c) if c.is_ascii_digit() => {
                self.chars.next();
                parts.push(WordPart::Var(c.to_string()));
            }
            Some(&c) if c.is_alphabetic() || c == '_' => {
                let mut name = String::new();
                while let Some(&c) = self.chars.peek() {
                    if c.is_alphanumeric() || c == '_' {
                        name.push(c);
                        self.chars.next();
                    } else {
                        break;
                    }
                }
                parts.push(WordPart::Var(name));
            }
            _ => {
                parts.push(WordPart::Literal("$".to_string()));
            }
        }
        Ok(())
    }
}
