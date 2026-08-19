mod arith;
mod ast;
mod builtins;
mod exec;
mod lexer;
mod parser;
mod state;

use lexer::Lexer;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use state::Shell;

fn expand_prompt(shell: &Shell, tpl: &str) -> String {
    let mut out = String::new();
    let mut chars = tpl.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' {
            if let Some(&next) = chars.peek() {
                if next == '{' {
                    chars.next();
                    let mut name = String::new();
                    while let Some(&c2) = chars.peek() {
                        if c2 == '}' {
                            chars.next();
                            break;
                        }
                        name.push(c2);
                        chars.next();
                    }
                    if let Some(v) = shell.get_var(&name) {
                        out.push_str(&v);
                    }
                    continue;
                } else if next.is_alphabetic() || next == '_' {
                    let mut name = String::new();
                    while let Some(&c2) = chars.peek() {
                        if c2.is_alphanumeric() || c2 == '_' {
                            name.push(c2);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    if let Some(v) = shell.get_var(&name) {
                        out.push_str(&v);
                    }
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

fn needs_more_input(err: &str) -> bool {
    err.contains("expected 'fi'")
        || err.contains("expected 'then'")
        || err.contains("expected 'do'")
        || err.contains("expected 'done'")
        || err.contains("expected '}'")
        || err.contains("expected word")
        || err.contains("unterminated")
        || err.contains("expected command")
        || err.contains("syntax error near token 'Eof'")
}

fn try_parse(src: &str) -> Result<Vec<ast::Node>, String> {
    let tokens = Lexer::new(src).tokenize()?;
    parser::parse(tokens)
}

fn main() {
    let mut shell = Shell::new();

    let rc_path = dirs::home_dir().map(|h| h.join(".xshrc"));
    if let Some(path) = &rc_path {
        if let Ok(src) = std::fs::read_to_string(path) {
            shell.run_source(&src);
        }
    }

    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        let script = &args[1];
        match std::fs::read_to_string(script) {
            Ok(src) => {
                let status = shell.run_source(&src);
                std::process::exit(shell.should_exit.unwrap_or(status));
            }
            Err(e) => {
                eprintln!("xsh: {}: {}", script, e);
                std::process::exit(1);
            }
        }
    }

    let mut rl = DefaultEditor::new().expect("failed to init line editor");
    let history_path = dirs::home_dir().map(|h| h.join(".xsh_history"));
    if let Some(path) = &history_path {
        let _ = rl.load_history(path);
    }

    loop {
        if let Some(code) = shell.should_exit {
            if let Some(path) = &history_path {
                let _ = rl.save_history(path);
            }
            std::process::exit(code);
        }
        let prompt = expand_prompt(&shell, &shell.get_var("PS1").unwrap_or_default());
        let mut buffer = String::new();
        let mut current_prompt = prompt.clone();
        let line = loop {
            match rl.readline(&current_prompt) {
                Ok(line) => {
                    if !buffer.is_empty() {
                        buffer.push('\n');
                    }
                    buffer.push_str(&line);
                    match try_parse(&buffer) {
                        Ok(_) => break Some(buffer.clone()),
                        Err(e) => {
                            if needs_more_input(&e) {
                                current_prompt = "> ".to_string();
                                continue;
                            } else {
                                eprintln!("{}", e);
                                break None;
                            }
                        }
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    break None;
                }
                Err(ReadlineError::Eof) => {
                    if let Some(path) = &history_path {
                        let _ = rl.save_history(path);
                    }
                    std::process::exit(shell.last_status);
                }
                Err(e) => {
                    eprintln!("xsh: readline error: {}", e);
                    if let Some(path) = &history_path {
                        let _ = rl.save_history(path);
                    }
                    std::process::exit(1);
                }
            }
        };

        if let Some(src) = line {
            if !src.trim().is_empty() {
                let _ = rl.add_history_entry(src.as_str());
                shell.run_source(&src);
            }
        }
    }
}
