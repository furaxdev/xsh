use crate::state::Shell;
use crate::test_cmd;
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use std::io::Write;

const BUILTINS: &[&str] = &[
    "cd", "pwd", "exit", "export", "unset", "alias", "unalias", "echo", "source", ".", "true",
    "false", "type", "read", ":", "wait", "test", "[", "printf", "history", "jobs", "kill",
    "env", "which", "pushd", "popd", "dirs", "eval", "exec", "command", "set", "local", "shift",
    "readonly", "let", "basename", "dirname", "trap", "umask", "getopts", "hash", "declare",
    "typeset", "builtin", "seq", "fg", "bg", "disown",
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
                if shell.readonly.contains(a) {
                    eprintln!("xsh: unset: {}: readonly variable", a);
                    return 1;
                }
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
            let mut interpret = false;
            let mut start = 0;
            while let Some(a) = args.get(start) {
                if a.len() >= 2 && a.starts_with('-') && a[1..].chars().all(|c| matches!(c, 'n' | 'e' | 'E')) {
                    for c in a[1..].chars() {
                        match c {
                            'n' => newline = false,
                            'e' => interpret = true,
                            'E' => interpret = false,
                            _ => {}
                        }
                    }
                    start += 1;
                } else {
                    break;
                }
            }
            let out = args[start..].join(" ");
            let out = if interpret { interpret_escapes(&out) } else { out };
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
        "local" => {
            let mut status = 0;
            for a in args {
                match a.split_once('=') {
                    Some((k, v)) => {
                        status |= shell.declare_local(k, Some(v.to_string()));
                    }
                    None => {
                        status |= shell.declare_local(a, None);
                    }
                }
            }
            status
        }
        "shift" => {
            let n: usize = args.first().and_then(|a| a.parse().ok()).unwrap_or(1);
            let count: usize = shell.get_var("#").and_then(|s| s.parse().ok()).unwrap_or(0);
            if n > count {
                return 1;
            }
            for i in 1..=(count - n) {
                match shell.vars.get(&(i + n).to_string()).cloned() {
                    Some(v) => {
                        shell.vars.insert(i.to_string(), v);
                    }
                    None => {
                        shell.vars.remove(&i.to_string());
                    }
                }
            }
            for i in (count - n + 1)..=count {
                shell.vars.remove(&i.to_string());
            }
            let new_count = count - n;
            let joined: Vec<String> = (1..=new_count)
                .filter_map(|i| shell.vars.get(&i.to_string()).cloned())
                .collect();
            shell.vars.insert("@".to_string(), joined.join(" "));
            shell.vars.insert("#".to_string(), new_count.to_string());
            0
        }
        "readonly" => {
            if args.is_empty() {
                let mut names: Vec<_> = shell.readonly.iter().cloned().collect();
                names.sort();
                for k in names {
                    println!("readonly {}={}", k, shell.vars.get(&k).cloned().unwrap_or_default());
                }
                return 0;
            }
            for a in args {
                if let Some((k, v)) = a.split_once('=') {
                    shell.vars.insert(k.to_string(), v.to_string());
                    shell.readonly.insert(k.to_string());
                } else {
                    shell.readonly.insert(a.clone());
                }
            }
            0
        }
        "let" => {
            if args.is_empty() {
                return 1;
            }
            let mut last = 0i64;
            for a in args {
                let result = if let Some((k, expr)) = a.split_once('=') {
                    let k = k.trim();
                    if shell.readonly.contains(k) {
                        eprintln!("xsh: {}: readonly variable", k);
                        return 1;
                    }
                    match crate::arith::eval(expr, &shell.vars) {
                        Ok(v) => {
                            shell.vars.insert(k.to_string(), v.to_string());
                            v
                        }
                        Err(e) => {
                            eprintln!("xsh: let: {}", e);
                            return 1;
                        }
                    }
                } else {
                    match crate::arith::eval(a, &shell.vars) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("xsh: let: {}", e);
                            return 1;
                        }
                    }
                };
                last = result;
            }
            if last != 0 {
                0
            } else {
                1
            }
        }
        "basename" => {
            let Some(path) = args.first() else {
                eprintln!("xsh: basename: missing operand");
                return 1;
            };
            let mut name = std::path::Path::new(path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone());
            if let Some(suffix) = args.get(1) {
                if name.ends_with(suffix.as_str()) && name != *suffix {
                    name.truncate(name.len() - suffix.len());
                }
            }
            println!("{}", name);
            0
        }
        "dirname" => {
            let Some(path) = args.first() else {
                eprintln!("xsh: dirname: missing operand");
                return 1;
            };
            let dir = std::path::Path::new(path)
                .parent()
                .map(|p| {
                    let s = p.to_string_lossy().to_string();
                    if s.is_empty() {
                        ".".to_string()
                    } else {
                        s
                    }
                })
                .unwrap_or_else(|| ".".to_string());
            println!("{}", dir);
            0
        }
        "trap" => {
            if args.is_empty() {
                if let Some(cmd) = &shell.trap_exit {
                    println!("trap -- '{}' EXIT", cmd);
                }
                return 0;
            }
            if args[0] == "-" {
                shell.trap_exit = None;
                return 0;
            }
            if args.len() >= 2 && args[1] == "EXIT" {
                shell.trap_exit = Some(args[0].clone());
                0
            } else {
                eprintln!("xsh: trap: only the EXIT signal is supported");
                1
            }
        }
        "umask" => {
            match args.first() {
                None => {
                    let cur = nix::sys::stat::umask(nix::sys::stat::Mode::empty());
                    nix::sys::stat::umask(cur);
                    println!("{:04o}", cur.bits());
                }
                Some(m) => match u32::from_str_radix(m, 8) {
                    Ok(bits) => {
                        if let Some(mode) = nix::sys::stat::Mode::from_bits(bits) {
                            nix::sys::stat::umask(mode);
                        }
                    }
                    Err(_) => {
                        eprintln!("xsh: umask: {}: invalid mode", m);
                        return 1;
                    }
                },
            }
            0
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
        "read" => builtin_read(shell, args),
        "getopts" => builtin_getopts(shell, args),
        "hash" => 0,
        "declare" | "typeset" => builtin_declare(shell, args),
        "builtin" => {
            if args.is_empty() {
                return 0;
            }
            if is_builtin(&args[0]) {
                run_builtin(shell, &args[0], &args[1..])
            } else {
                eprintln!("xsh: builtin: {}: not a shell builtin", args[0]);
                1
            }
        }
        "seq" => builtin_seq(args),
        "fg" => match shell.bg_jobs.pop() {
            Some(pid) => match waitpid(pid, None) {
                Ok(WaitStatus::Exited(_, code)) => code,
                _ => 0,
            },
            None => {
                eprintln!("xsh: fg: no current job");
                1
            }
        },
        "bg" => {
            if shell.bg_jobs.is_empty() {
                eprintln!("xsh: bg: no current job");
                1
            } else {
                0
            }
        }
        "disown" => {
            shell.bg_jobs.clear();
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
        if t == "-" {
            match shell.get_var("OLDPWD") {
                Some(p) => p,
                None => {
                    eprintln!("xsh: cd: OLDPWD not set");
                    return 1;
                }
            }
        } else {
            t.clone()
        }
    } else if let Some(home) = shell.get_var("HOME") {
        home
    } else {
        eprintln!("xsh: cd: HOME not set");
        return 1;
    };
    let prev = std::env::current_dir().ok();
    match std::env::set_current_dir(&target) {
        Ok(_) => {
            if let Some(prev) = prev {
                let s = prev.to_string_lossy().to_string();
                shell.vars.insert("OLDPWD".to_string(), s);
                shell.exported.insert("OLDPWD".to_string());
            }
            if let Ok(cwd) = std::env::current_dir() {
                let s = cwd.to_string_lossy().to_string();
                shell.vars.insert("PWD".to_string(), s.clone());
                shell.exported.insert("PWD".to_string());
                if args.first().map(|a| a == "-").unwrap_or(false) {
                    println!("{}", s);
                }
            }
            0
        }
        Err(e) => {
            eprintln!("xsh: cd: {}: {}", target, e);
            1
        }
    }
}

fn interpret_escapes(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('a') => out.push('\u{7}'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn builtin_read(shell: &mut Shell, args: &[String]) -> i32 {
    let mut prompt: Option<String> = None;
    let mut silent = false;
    let mut varname: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-p" => {
                i += 1;
                prompt = args.get(i).cloned();
                i += 1;
            }
            "-s" => {
                silent = true;
                i += 1;
            }
            other => {
                varname = Some(other.to_string());
                i += 1;
            }
        }
    }
    if let Some(p) = &prompt {
        print!("{}", p);
        let _ = std::io::stdout().flush();
    }

    let orig_termios = if silent {
        use nix::sys::termios::{tcgetattr, tcsetattr, LocalFlags, SetArg};
        match tcgetattr(std::io::stdin()) {
            Ok(t) => {
                let mut raw = t.clone();
                raw.local_flags.remove(LocalFlags::ECHO);
                let _ = tcsetattr(std::io::stdin(), SetArg::TCSANOW, &raw);
                Some(t)
            }
            Err(_) => None,
        }
    } else {
        None
    };

    let mut line = String::new();
    let n = std::io::stdin().read_line(&mut line).unwrap_or(0);

    if let Some(t) = orig_termios {
        use nix::sys::termios::{tcsetattr, SetArg};
        let _ = tcsetattr(std::io::stdin(), SetArg::TCSANOW, &t);
        println!();
    }

    if n == 0 {
        return 1;
    }
    let line = line.trim_end_matches('\n');
    shell
        .vars
        .insert(varname.unwrap_or_else(|| "REPLY".to_string()), line.to_string());
    0
}

fn builtin_getopts(shell: &mut Shell, args: &[String]) -> i32 {
    if args.len() < 2 {
        eprintln!("xsh: getopts: usage: getopts optstring name [arg...]");
        return 2;
    }
    let optstring = &args[0];
    let name = &args[1];
    let extra = &args[2..];
    let positional: Vec<String> = if !extra.is_empty() {
        extra.to_vec()
    } else {
        let count: usize = shell.get_var("#").and_then(|s| s.parse().ok()).unwrap_or(0);
        (1..=count)
            .filter_map(|i| shell.vars.get(&i.to_string()).cloned())
            .collect()
    };
    let optind: usize = shell.get_var("OPTIND").and_then(|s| s.parse().ok()).unwrap_or(1);
    let idx = optind.saturating_sub(1);

    if idx >= positional.len() {
        shell.vars.insert(name.clone(), "?".to_string());
        return 1;
    }
    let cur = &positional[idx];
    if !cur.starts_with('-') || cur == "--" || cur.len() < 2 {
        shell.vars.insert(name.clone(), "?".to_string());
        return 1;
    }
    let opt_char = cur.chars().nth(1).unwrap_or('?');
    if !optstring.contains(opt_char) {
        shell.vars.insert(name.clone(), "?".to_string());
        shell.vars.insert("OPTIND".to_string(), (optind + 1).to_string());
        return 0;
    }
    let needs_arg = optstring
        .chars()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|w| w[0] == opt_char && w[1] == ':');
    shell.vars.insert(name.clone(), opt_char.to_string());
    if needs_arg {
        if idx + 1 < positional.len() {
            shell.vars.insert("OPTARG".to_string(), positional[idx + 1].clone());
            shell.vars.insert("OPTIND".to_string(), (optind + 2).to_string());
        } else {
            eprintln!("xsh: getopts: option requires an argument -- '{}'", opt_char);
            shell.vars.insert("OPTIND".to_string(), (optind + 1).to_string());
        }
    } else {
        shell.vars.insert("OPTIND".to_string(), (optind + 1).to_string());
    }
    0
}

fn builtin_declare(shell: &mut Shell, args: &[String]) -> i32 {
    let mut do_export = false;
    let mut do_readonly = false;
    let mut rest = Vec::new();
    for a in args {
        if let Some(flags) = a.strip_prefix('-') {
            if flags.contains('x') {
                do_export = true;
            }
            if flags.contains('r') {
                do_readonly = true;
            }
            if !flags.is_empty() {
                continue;
            }
        }
        rest.push(a.clone());
    }
    for a in rest {
        let (k, v) = match a.split_once('=') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (a.clone(), None),
        };
        if let Some(v) = v {
            shell.vars.insert(k.clone(), v);
        }
        if do_export {
            shell.exported.insert(k.clone());
        }
        if do_readonly {
            shell.readonly.insert(k.clone());
        }
    }
    0
}

fn builtin_seq(args: &[String]) -> i32 {
    let nums: Vec<f64> = args.iter().filter_map(|a| a.parse().ok()).collect();
    let (start, step, end) = match nums.len() {
        1 => (1.0, 1.0, nums[0]),
        2 => (nums[0], 1.0, nums[1]),
        3 => (nums[0], nums[1], nums[2]),
        _ => {
            eprintln!("xsh: seq: usage: seq [first [incr]] last");
            return 1;
        }
    };
    if step == 0.0 {
        eprintln!("xsh: seq: zero increment");
        return 1;
    }
    let mut v = start;
    if step > 0.0 {
        while v <= end {
            print_seq_num(v);
            v += step;
        }
    } else {
        while v >= end {
            print_seq_num(v);
            v += step;
        }
    }
    0
}

fn print_seq_num(v: f64) {
    if v.fract() == 0.0 {
        println!("{}", v as i64);
    } else {
        println!("{}", v);
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
