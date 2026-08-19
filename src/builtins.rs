use crate::state::Shell;
use nix::sys::wait::waitpid;
use std::io::Write;

const BUILTINS: &[&str] = &[
    "cd", "pwd", "exit", "export", "unset", "alias", "unalias", "echo", "source", ".", "true",
    "false", "type", "read", ":", "wait",
];

pub fn is_builtin(name: &str) -> bool {
    BUILTINS.contains(&name)
}

pub fn run_builtin(shell: &mut Shell, name: &str, args: &[String]) -> i32 {
    match name {
        "cd" => builtin_cd(shell, args),
        "pwd" => {
            match std::env::current_dir() {
                Ok(p) => println!("{}", p.display()),
                Err(e) => {
                    eprintln!("xsh: pwd: {}", e);
                    return 1;
                }
            }
            0
        }
        "exit" => {
            let code = args
                .first()
                .and_then(|a| a.parse::<i32>().ok())
                .unwrap_or(shell.last_status);
            shell.should_exit = Some(code);
            code
        }
        "export" => {
            for a in args {
                if let Some((k, v)) = a.split_once('=') {
                    shell.vars.insert(k.to_string(), v.to_string());
                    shell.exported.insert(k.to_string());
                } else {
                    shell.exported.insert(a.clone());
                }
            }
            0
        }
        "unset" => {
            for a in args {
                shell.vars.remove(a);
                shell.exported.remove(a);
                shell.functions.remove(a);
            }
            0
        }
        "alias" => {
            if args.is_empty() {
                let mut names: Vec<_> = shell.aliases.keys().cloned().collect();
                names.sort();
                for k in names {
                    println!("alias {}='{}'", k, shell.aliases[&k]);
                }
                return 0;
            }
            for a in args {
                if let Some((k, v)) = a.split_once('=') {
                    shell.aliases.insert(k.to_string(), v.to_string());
                } else if let Some(v) = shell.aliases.get(a) {
                    println!("alias {}='{}'", a, v);
                }
            }
            0
        }
        "unalias" => {
            for a in args {
                shell.aliases.remove(a);
            }
            0
        }
        "echo" => {
            let mut newline = true;
            let mut start = 0;
            if args.first().map(|s| s.as_str()) == Some("-n") {
                newline = false;
                start = 1;
            }
            let out = args[start..].join(" ");
            if newline {
                println!("{}", out);
            } else {
                print!("{}", out);
                let _ = std::io::stdout().flush();
            }
            0
        }
        "source" | "." => {
            if let Some(path) = args.first() {
                match std::fs::read_to_string(path) {
                    Ok(src) => shell.run_source(&src),
                    Err(e) => {
                        eprintln!("xsh: {}: {}", path, e);
                        1
                    }
                }
            } else {
                eprintln!("xsh: source: filename argument required");
                1
            }
        }
        "wait" => {
            for pid in shell.bg_jobs.drain(..) {
                let _ = waitpid(pid, None);
            }
            0
        }
        "true" | ":" => 0,
        "false" => 1,
        "type" => {
            let mut status = 0;
            for a in args {
                if shell.aliases.contains_key(a) {
                    println!("{} is an alias for '{}'", a, shell.aliases[a]);
                } else if shell.functions.contains_key(a) {
                    println!("{} is a function", a);
                } else if is_builtin(a) {
                    println!("{} is a shell builtin", a);
                } else if let Some(path) = find_in_path(shell, a) {
                    println!("{} is {}", a, path);
                } else {
                    println!("xsh: type: {}: not found", a);
                    status = 1;
                }
            }
            status
        }
        "read" => {
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
                return 1;
            }
            let line = line.trim_end_matches('\n');
            if let Some(varname) = args.first() {
                shell.vars.insert(varname.clone(), line.to_string());
            } else {
                shell.vars.insert("REPLY".to_string(), line.to_string());
            }
            0
        }
        _ => {
            eprintln!("xsh: {}: not a builtin", name);
            1
        }
    }
}

fn builtin_cd(shell: &mut Shell, args: &[String]) -> i32 {
    let target = if let Some(t) = args.first() {
        t.clone()
    } else if let Some(home) = shell.get_var("HOME") {
        home
    } else {
        eprintln!("xsh: cd: HOME not set");
        return 1;
    };
    match std::env::set_current_dir(&target) {
        Ok(_) => {
            if let Ok(cwd) = std::env::current_dir() {
                let s = cwd.to_string_lossy().to_string();
                shell.vars.insert("PWD".to_string(), s.clone());
                shell.exported.insert("PWD".to_string());
            }
            0
        }
        Err(e) => {
            eprintln!("xsh: cd: {}: {}", target, e);
            1
        }
    }
}

fn find_in_path(shell: &Shell, name: &str) -> Option<String> {
    let path = shell.get_var("PATH")?;
    for dir in path.split(':') {
        let candidate = std::path::Path::new(dir).join(name);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}
