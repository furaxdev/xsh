use crate::state::Shell;
use crate::test_cmd;
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use std::io::Write;

const BUILTINS: &[&str] = &[
    "cd", "pwd", "exit", "export", "unset", "alias", "unalias", "echo", "source", ".", "true",
    "false", "type", "read", ":", "wait", "test", "[", "printf", "history", "jobs", "kill",
    "env", "which", "pushd", "popd", "dirs", "eval", "exec", "command", "set",
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
        "test" => test_cmd::run(args),
        "[" => {
            if args.last().map(|s| s.as_str()) != Some("]") {
                eprintln!("xsh: [: missing ']'");
                return 2;
            }
            test_cmd::run(&args[..args.len() - 1])
        }
        "printf" => builtin_printf(args),
        "history" => {
            for (i, line) in shell.history.iter().enumerate() {
                println!("{:5}  {}", i + 1, line);
            }
            0
        }
        "jobs" => {
            let mut still_running = Vec::new();
            for pid in shell.bg_jobs.drain(..) {
                match waitpid(pid, Some(WaitPidFlag::WNOHANG)) {
                    Ok(WaitStatus::StillAlive) => {
                        println!("[{}]  Running", pid);
                        still_running.push(pid);
                    }
                    Ok(_) => {
                        println!("[{}]  Done", pid);
                    }
                    Err(_) => {}
                }
            }
            shell.bg_jobs = still_running;
            0
        }
        "kill" => builtin_kill(args),
        "env" => {
            let mut names: Vec<_> = shell.exported.iter().cloned().collect();
            names.sort();
            for k in names {
                if let Some(v) = shell.vars.get(&k) {
                    println!("{}={}", k, v);
                }
            }
            0
        }
        "which" => {
            let mut status = 0;
            for a in args {
                match find_in_path(shell, a) {
                    Some(path) => println!("{}", path),
                    None => {
                        println!("{} not found", a);
                        status = 1;
                    }
                }
            }
            status
        }
        "pushd" => {
            let Ok(cwd) = std::env::current_dir() else {
                return 1;
            };
            let target = match args.first() {
                Some(t) => t.clone(),
                None => {
                    eprintln!("xsh: pushd: no other directory");
                    return 1;
                }
            };
            match std::env::set_current_dir(&target) {
                Ok(_) => {
                    shell.dir_stack.push(cwd.to_string_lossy().to_string());
                    update_pwd(shell);
                    print_dirs(shell);
                    0
                }
                Err(e) => {
                    eprintln!("xsh: pushd: {}: {}", target, e);
                    1
                }
            }
        }
        "popd" => match shell.dir_stack.pop() {
            Some(dir) => match std::env::set_current_dir(&dir) {
                Ok(_) => {
                    update_pwd(shell);
                    print_dirs(shell);
                    0
                }
                Err(e) => {
                    eprintln!("xsh: popd: {}: {}", dir, e);
                    1
                }
            },
            None => {
                eprintln!("xsh: popd: directory stack empty");
                1
            }
        },
        "dirs" => {
            print_dirs(shell);
            0
        }
        "eval" => {
            let src = args.join(" ");
            shell.run_source(&src)
        }
        "exec" => {
            if args.is_empty() {
                return 0;
            }
            crate::exec::exec_and_replace(shell, args);
        }
        "command" => {
            if args.is_empty() {
                return 0;
            }
            if is_builtin(&args[0]) {
                run_builtin(shell, &args[0], &args[1..])
            } else {
                shell.exec_external(args, &[])
            }
        }
        "set" => {
            for a in args {
                match a.as_str() {
                    "-e" => shell.errexit = true,
                    "+e" => shell.errexit = false,
                    "-x" => shell.xtrace = true,
                    "+x" => shell.xtrace = false,
                    _ => {}
                }
            }
            if args.is_empty() {
                let mut names: Vec<_> = shell.vars.keys().cloned().collect();
                names.sort();
                for k in names {
                    println!("{}={}", k, shell.vars[&k]);
                }
            }
            0
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

fn builtin_printf(args: &[String]) -> i32 {
    let Some(fmt) = args.first() else {
        return 0;
    };
    let rest = &args[1..];
    let mut ai = 0;
    let mut chars = fmt.chars().peekable();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else if c == '%' {
            match chars.next() {
                Some('s') => {
                    out.push_str(rest.get(ai).map(|s| s.as_str()).unwrap_or(""));
                    ai += 1;
                }
                Some('d') => {
                    let v = rest
                        .get(ai)
                        .and_then(|s| s.trim().parse::<i64>().ok())
                        .unwrap_or(0);
                    out.push_str(&v.to_string());
                    ai += 1;
                }
                Some('%') => out.push('%'),
                Some(other) => {
                    out.push('%');
                    out.push(other);
                }
                None => out.push('%'),
            }
        } else {
            out.push(c);
        }
    }
    print!("{}", out);
    let _ = std::io::stdout().flush();
    0
}

fn builtin_kill(args: &[String]) -> i32 {
    let mut sig = Signal::SIGTERM;
    let mut rest = args;
    if let Some(first) = args.first() {
        if let Some(spec) = first.strip_prefix('-') {
            let parsed = spec
                .parse::<i32>()
                .ok()
                .and_then(|n| Signal::try_from(n).ok())
                .or_else(|| {
                    let name = format!("SIG{}", spec.to_uppercase());
                    Signal::iterator().find(|s| s.as_str() == name || s.as_str() == spec.to_uppercase())
                });
            if let Some(s) = parsed {
                sig = s;
                rest = &args[1..];
            }
        }
    }
    let mut status = 0;
    for a in rest {
        match a.parse::<i32>() {
            Ok(pid) => {
                if kill(Pid::from_raw(pid), sig).is_err() {
                    eprintln!("xsh: kill: ({}) - no such process", pid);
                    status = 1;
                }
            }
            Err(_) => {
                eprintln!("xsh: kill: {}: arguments must be process ids", a);
                status = 1;
            }
        }
    }
    status
}

fn update_pwd(shell: &mut Shell) {
    if let Ok(cwd) = std::env::current_dir() {
        let s = cwd.to_string_lossy().to_string();
        shell.vars.insert("PWD".to_string(), s);
        shell.exported.insert("PWD".to_string());
    }
}

fn print_dirs(shell: &Shell) {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut all = vec![cwd];
    all.extend(shell.dir_stack.iter().rev().cloned());
    println!("{}", all.join("  "));
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
