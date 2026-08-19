use crate::ast::*;
use crate::builtins;
use crate::lexer::Lexer;
use crate::parser;
use crate::state::Shell;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{execvp, fork, pipe, ForkResult, Pid};
use std::collections::HashSet;
use std::ffi::CString;
use std::fs::OpenOptions;
use std::os::fd::{IntoRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;

fn raw_dup2(old: RawFd, new: RawFd) {
    unsafe {
        nix::libc::dup2(old, new);
    }
}

fn raw_close(fd: RawFd) {
    unsafe {
        nix::libc::close(fd);
    }
}

fn raw_dup(fd: RawFd) -> RawFd {
    unsafe { nix::libc::dup(fd) }
}

pub enum Flow {
    Normal,
    Break,
    Continue,
    Return(i32),
}

impl Shell {
    pub fn expand_word(&mut self, word: &Word) -> Result<String, String> {
        let mut out = String::new();
        for part in word {
            match part {
                WordPart::Literal(s) => out.push_str(s),
                WordPart::Var(name) => {
                    if let Some(v) = self.get_var(name) {
                        out.push_str(&v);
                    }
                }
                WordPart::CmdSub(src) => {
                    let captured = self.capture_output(src)?;
                    let trimmed = captured.trim_end_matches('\n');
                    out.push_str(trimmed);
                }
                WordPart::Arith(src) => {
                    let v = crate::arith::eval(src, &self.vars)?;
                    out.push_str(&v.to_string());
                }
            }
        }
        Ok(out)
    }

    fn capture_output(&mut self, src: &str) -> Result<String, String> {
        let (r, w) = pipe().map_err(|e| e.to_string())?;
        let r_fd = r.into_raw_fd();
        let w_fd = w.into_raw_fd();
        match unsafe { fork() }.map_err(|e| e.to_string())? {
            ForkResult::Child => {
                raw_close(r_fd);
                raw_dup2(w_fd, 1);
                raw_close(w_fd);
                let status = self.run_source(src);
                std::process::exit(status);
            }
            ForkResult::Parent { child } => {
                raw_close(w_fd);
                use std::io::Read;
                let mut f = unsafe { <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(r_fd) };
                let mut buf = String::new();
                let _ = f.read_to_string(&mut buf);
                let _ = waitpid(child, None);
                Ok(buf)
            }
        }
    }

    pub fn run_source(&mut self, src: &str) -> i32 {
        let tokens = match Lexer::new(src).tokenize() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("xsh: {}", e);
                return 2;
            }
        };
        let prog = match parser::parse(tokens) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{}", e);
                return 2;
            }
        };
        for n in &prog {
            if self.should_exit.is_some() {
                break;
            }
            self.run_node(n);
            if self.errexit && self.last_status != 0 {
                self.should_exit = Some(self.last_status);
                break;
            }
        }
        self.last_status
    }

    pub fn run_list(&mut self, nodes: &[Node]) -> Flow {
        for n in nodes {
            if self.should_exit.is_some() {
                return Flow::Normal;
            }
            match self.run_node(n) {
                Flow::Normal => {}
                other => return other,
            }
        }
        Flow::Normal
    }

    fn run_node(&mut self, node: &Node) -> Flow {
        match node {
            Node::Simple(sc) => {
                self.last_status = self.run_simple(sc);
                Flow::Normal
            }
            Node::Pipeline(cmds, negate) => {
                let st = self.run_pipeline(cmds);
                self.last_status = if *negate {
                    if st == 0 {
                        1
                    } else {
                        0
                    }
                } else {
                    st
                };
                Flow::Normal
            }
            Node::List(items) => self.run_and_or_list(items),
            Node::If {
                branches,
                else_branch,
            } => {
                for (cond, body) in branches {
                    match self.run_list(cond) {
                        Flow::Normal => {}
                        other => return other,
                    }
                    if self.last_status == 0 {
                        return self.run_list(body);
                    }
                }
                if let Some(b) = else_branch {
                    return self.run_list(b);
                }
                self.last_status = 0;
                Flow::Normal
            }
            Node::For { var, words, body } => {
                let mut values = Vec::new();
                for w in words {
                    match self.expand_word(w) {
                        Ok(s) => values.push(s),
                        Err(e) => {
                            eprintln!("xsh: {}", e);
                            self.last_status = 1;
                            return Flow::Normal;
                        }
                    }
                }
                for v in values {
                    self.vars.insert(var.clone(), v);
                    match self.run_list(body) {
                        Flow::Break => break,
                        Flow::Continue => continue,
                        Flow::Normal => {}
                        other => return other,
                    }
                }
                self.last_status = 0;
                Flow::Normal
            }
            Node::While { cond, body, until } => {
                loop {
                    match self.run_list(cond) {
                        Flow::Normal => {}
                        other => return other,
                    }
                    let ok = self.last_status == 0;
                    let should_run = if *until { !ok } else { ok };
                    if !should_run {
                        break;
                    }
                    match self.run_list(body) {
                        Flow::Break => break,
                        Flow::Continue => continue,
                        Flow::Normal => {}
                        other => return other,
                    }
                }
                self.last_status = 0;
                Flow::Normal
            }
            Node::FunctionDef { name, body } => {
                self.functions.insert(name.clone(), body.clone());
                self.last_status = 0;
                Flow::Normal
            }
            Node::Subshell(body) => {
                self.last_status = self.run_in_subshell(body);
                Flow::Normal
            }
            Node::Background(inner) => {
                self.run_background(inner);
                self.last_status = 0;
                Flow::Normal
            }
            Node::Return(w) => {
                let code = match w {
                    Some(word) => match self.expand_word(word) {
                        Ok(s) => s.trim().parse().unwrap_or(0),
                        Err(_) => 0,
                    },
                    None => self.last_status,
                };
                Flow::Return(code)
            }
            Node::Break => Flow::Break,
            Node::Continue => Flow::Continue,
            Node::Case { word, arms } => {
                let subject = match self.expand_word(word) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("xsh: {}", e);
                        self.last_status = 1;
                        return Flow::Normal;
                    }
                };
                for (patterns, body) in arms {
                    for p in patterns {
                        let pat = match self.expand_word(p) {
                            Ok(s) => s,
                            Err(e) => {
                                eprintln!("xsh: {}", e);
                                self.last_status = 1;
                                return Flow::Normal;
                            }
                        };
                        if crate::glob::matches(&pat, &subject) {
                            return self.run_list(body);
                        }
                    }
                }
                self.last_status = 0;
                Flow::Normal
            }
            Node::Timed(inner) => {
                let start = std::time::Instant::now();
                let flow = self.run_node(inner);
                let elapsed = start.elapsed();
                eprintln!("real\t{:.3}s", elapsed.as_secs_f64());
                flow
            }
        }
    }

    fn run_and_or_list(&mut self, items: &[(Node, Sep)]) -> Flow {
        let mut idx = 0;
        let mut skip_next = false;
        let mut prev_status = self.last_status;
        while idx < items.len() {
            let (node, sep) = &items[idx];
            if !skip_next {
                match self.run_node(node) {
                    Flow::Normal => {}
                    other => return other,
                }
                prev_status = self.last_status;
            }
            skip_next = match sep {
                Sep::And => prev_status != 0,
                Sep::Or => prev_status == 0,
                Sep::Seq => false,
            };
            idx += 1;
        }
        self.last_status = prev_status;
        Flow::Normal
    }

    pub fn call_function(&mut self, body: &[Node], args: &[String]) -> i32 {
        let mut frame: Vec<(String, Option<String>)> = Vec::new();
        for name in ["@", "#"]
            .into_iter()
            .map(String::from)
            .chain((1..=9).map(|i: usize| i.to_string()))
        {
            frame.push((name.clone(), self.vars.get(&name).cloned()));
        }
        for (i, a) in args.iter().enumerate() {
            self.vars.insert((i + 1).to_string(), a.clone());
        }
        self.vars.insert("@".to_string(), args.join(" "));
        self.vars.insert("#".to_string(), args.len().to_string());

        self.local_stack.push(frame);
        let flow = self.run_list(body);
        let frame = self.local_stack.pop().unwrap_or_default();
        for (name, old) in frame {
            match old {
                Some(v) => {
                    self.vars.insert(name, v);
                }
                None => {
                    self.vars.remove(&name);
                }
            }
        }

        match flow {
            Flow::Return(code) => code,
            _ => self.last_status,
        }
    }

    pub fn declare_local(&mut self, name: &str, value: Option<String>) -> i32 {
        if self.local_stack.is_empty() {
            eprintln!("xsh: local: can only be used inside a function");
            return 1;
        }
        let already_recorded = self
            .local_stack
            .last()
            .unwrap()
            .iter()
            .any(|(n, _)| n == name);
        if !already_recorded {
            let prev = self.vars.get(name).cloned();
            self.local_stack
                .last_mut()
                .unwrap()
                .push((name.to_string(), prev));
        }
        self.vars.insert(name.to_string(), value.unwrap_or_default());
        0
    }

    pub fn run_exit_trap(&mut self) {
        if let Some(cmd) = self.trap_exit.take() {
            self.run_source(&cmd);
        }
    }

    fn run_simple(&mut self, sc: &SimpleCommand) -> i32 {
        let mut saved: Vec<(String, Option<String>)> = Vec::new();
        for (name, word) in &sc.assignments {
            if self.readonly.contains(name) {
                eprintln!("xsh: {}: readonly variable", name);
                return 1;
            }
            let val = match self.expand_word(word) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("xsh: {}", e);
                    return 1;
                }
            };
            saved.push((name.clone(), self.vars.get(name).cloned()));
            self.vars.insert(name.clone(), val);
        }

        let mut argv: Vec<String> = Vec::new();
        for w in &sc.words {
            match self.expand_word(w) {
                Ok(v) => argv.push(v),
                Err(e) => {
                    eprintln!("xsh: {}", e);
                    self.restore(saved);
                    return 1;
                }
            }
        }

        let is_pure_assignment = sc.words.is_empty();
        let status = if argv.is_empty() {
            0
        } else {
            let argv = match self.resolve_alias_argv(argv) {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("xsh: {}", e);
                    self.restore(saved);
                    return 1;
                }
            };
            if argv.is_empty() {
                0
            } else {
                if self.xtrace {
                    eprintln!("+ {}", argv.join(" "));
                }
                self.run_command(&argv, &sc.redirects)
            }
        };

        if !is_pure_assignment {
            self.restore(saved);
        }
        status
    }

    fn restore(&mut self, saved: Vec<(String, Option<String>)>) {
        for (name, old) in saved {
            match old {
                Some(v) => {
                    self.vars.insert(name, v);
                }
                None => {
                    self.vars.remove(&name);
                }
            }
        }
    }

    fn resolve_alias_argv(&mut self, argv: Vec<String>) -> Result<Vec<String>, String> {
        let mut argv = argv;
        let mut seen = HashSet::new();
        loop {
            if argv.is_empty() {
                return Ok(argv);
            }
            let head = argv[0].clone();
            if seen.contains(&head) {
                break;
            }
            let alias_val = match self.aliases.get(&head) {
                Some(v) => v.clone(),
                None => break,
            };
            seen.insert(head);
            let tokens = Lexer::new(&alias_val).tokenize()?;
            let prog = parser::parse(tokens)?;
            if let Some(Node::Simple(sc0)) = prog.into_iter().next() {
                let mut new_argv = Vec::new();
                for w in &sc0.words {
                    new_argv.push(self.expand_word(w)?);
                }
                new_argv.extend(argv[1..].iter().cloned());
                argv = new_argv;
            } else {
                break;
            }
        }
        Ok(argv)
    }

    fn run_command(&mut self, argv: &[String], redirects: &[Redirect]) -> i32 {
        let name = &argv[0];
        if let Some(body) = self.functions.get(name).cloned() {
            let saved_out = match self.save_and_apply_redirects(redirects) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("xsh: {}", e);
                    return 1;
                }
            };
            let status = self.call_function(&body, &argv[1..]);
            self.restore_redirects(saved_out);
            return status;
        }
        if builtins::is_builtin(name) {
            let saved_out = match self.save_and_apply_redirects(redirects) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("xsh: {}", e);
                    return 1;
                }
            };
            let status = builtins::run_builtin(self, name, &argv[1..]);
            self.restore_redirects(saved_out);
            return status;
        }
        self.exec_external(argv, redirects)
    }

    pub(crate) fn exec_external(&mut self, argv: &[String], redirects: &[Redirect]) -> i32 {
        match unsafe { fork() } {
            Ok(ForkResult::Child) => {
                if let Err(e) = self.apply_redirects_in_child(redirects) {
                    eprintln!("xsh: {}", e);
                    std::process::exit(1);
                }
                exec_and_replace(&mut *self, argv);
            }
            Ok(ForkResult::Parent { child }) => wait_for(child),
            Err(e) => {
                eprintln!("xsh: fork failed: {}", e);
                1
            }
        }
    }

    fn run_pipeline(&mut self, cmds: &[Node]) -> i32 {
        if cmds.len() == 1 {
            self.run_node(&cmds[0]);
            return self.last_status;
        }

        let n = cmds.len();
        let mut pipe_fds: Vec<(RawFd, RawFd)> = Vec::new();
        for _ in 0..n - 1 {
            match pipe() {
                Ok((r, w)) => pipe_fds.push((r.into_raw_fd(), w.into_raw_fd())),
                Err(e) => {
                    eprintln!("xsh: pipe failed: {}", e);
                    return 1;
                }
            }
        }

        let mut children: Vec<Pid> = Vec::new();

        for i in 0..n {
            match unsafe { fork() } {
                Ok(ForkResult::Child) => {
                    if i > 0 {
                        let (r, _) = pipe_fds[i - 1];
                        raw_dup2(r, 0);
                    }
                    if i < n - 1 {
                        let (_, w) = pipe_fds[i];
                        raw_dup2(w, 1);
                    }
                    for &(r, w) in &pipe_fds {
                        raw_close(r);
                        raw_close(w);
                    }
                    let status = self.run_node_in_child(&cmds[i]);
                    std::process::exit(status);
                }
                Ok(ForkResult::Parent { child }) => {
                    children.push(child);
                }
                Err(e) => {
                    eprintln!("xsh: fork failed: {}", e);
                    return 1;
                }
            }
        }

        for &(r, w) in &pipe_fds {
            raw_close(r);
            raw_close(w);
        }

        let mut last_status = 0;
        for (idx, pid) in children.into_iter().enumerate() {
            let st = wait_for(pid);
            if idx == n - 1 {
                last_status = st;
            }
        }
        last_status
    }

    fn run_node_in_child(&mut self, node: &Node) -> i32 {
        match node {
            Node::Simple(sc) => self.run_simple_no_fork_external(sc),
            _ => {
                self.run_node(node);
                self.last_status
            }
        }
    }

    fn run_simple_no_fork_external(&mut self, sc: &SimpleCommand) -> i32 {
        for (name, word) in &sc.assignments {
            if let Ok(val) = self.expand_word(word) {
                self.vars.insert(name.clone(), val);
            }
        }
        let mut argv = Vec::new();
        for w in &sc.words {
            match self.expand_word(w) {
                Ok(v) => argv.push(v),
                Err(e) => {
                    eprintln!("xsh: {}", e);
                    return 1;
                }
            }
        }
        if argv.is_empty() {
            return 0;
        }
        let argv = match self.resolve_alias_argv(argv) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("xsh: {}", e);
                return 1;
            }
        };
        if argv.is_empty() {
            return 0;
        }
        let name = argv[0].clone();
        if let Some(body) = self.functions.get(&name).cloned() {
            if let Err(e) = self.apply_redirects_in_child(&sc.redirects) {
                eprintln!("xsh: {}", e);
                return 1;
            }
            return self.call_function(&body, &argv[1..]);
        }
        if builtins::is_builtin(&name) {
            if let Err(e) = self.apply_redirects_in_child(&sc.redirects) {
                eprintln!("xsh: {}", e);
                return 1;
            }
            return builtins::run_builtin(self, &name, &argv[1..]);
        }
        if let Err(e) = self.apply_redirects_in_child(&sc.redirects) {
            eprintln!("xsh: {}", e);
            return 1;
        }
        exec_and_replace(self, &argv);
    }

    fn run_in_subshell(&mut self, body: &[Node]) -> i32 {
        match unsafe { fork() } {
            Ok(ForkResult::Child) => {
                self.run_list(body);
                std::process::exit(self.last_status);
            }
            Ok(ForkResult::Parent { child }) => wait_for(child),
            Err(e) => {
                eprintln!("xsh: fork failed: {}", e);
                1
            }
        }
    }

    fn run_background(&mut self, inner: &Node) {
        match unsafe { fork() } {
            Ok(ForkResult::Child) => {
                self.run_node(inner);
                std::process::exit(self.last_status);
            }
            Ok(ForkResult::Parent { child }) => {
                eprintln!("[{}]", child);
                self.bg_jobs.push(child);
            }
            Err(e) => {
                eprintln!("xsh: fork failed: {}", e);
            }
        }
    }

    fn save_and_apply_redirects(
        &mut self,
        redirects: &[Redirect],
    ) -> Result<Vec<(RawFd, RawFd)>, String> {
        if redirects.is_empty() {
            return Ok(Vec::new());
        }
        let mut saved = Vec::new();
        for fd in [0, 1, 2] {
            let dup = raw_dup(fd);
            saved.push((fd, dup));
        }
        if let Err(e) = self.apply_redirects_in_child(redirects) {
            for (fd, dup) in &saved {
                raw_dup2(*dup, *fd);
                raw_close(*dup);
            }
            return Err(e);
        }
        Ok(saved)
    }

    fn restore_redirects(&mut self, saved: Vec<(RawFd, RawFd)>) {
        for (fd, dup) in saved {
            raw_dup2(dup, fd);
            raw_close(dup);
        }
    }

    fn apply_redirects_in_child(&mut self, redirects: &[Redirect]) -> Result<(), String> {
        for r in redirects {
            match r.kind {
                RedirectKind::In => {
                    let path = self.expand_word(&r.target)?;
                    let f = OpenOptions::new()
                        .read(true)
                        .open(&path)
                        .map_err(|e| format!("{}: {}", path, e))?;
                    let fd = f.into_raw_fd();
                    raw_dup2(fd, 0);
                    raw_close(fd);
                }
                RedirectKind::Out => {
                    let path = self.expand_word(&r.target)?;
                    let f = OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .mode(0o644)
                        .open(&path)
                        .map_err(|e| format!("{}: {}", path, e))?;
                    let fd = f.into_raw_fd();
                    raw_dup2(fd, 1);
                    raw_close(fd);
                }
                RedirectKind::Append => {
                    let path = self.expand_word(&r.target)?;
                    let f = OpenOptions::new()
                        .write(true)
                        .create(true)
                        .append(true)
                        .mode(0o644)
                        .open(&path)
                        .map_err(|e| format!("{}: {}", path, e))?;
                    let fd = f.into_raw_fd();
                    raw_dup2(fd, 1);
                    raw_close(fd);
                }
                RedirectKind::ErrOut => {
                    let path = self.expand_word(&r.target)?;
                    let f = OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .mode(0o644)
                        .open(&path)
                        .map_err(|e| format!("{}: {}", path, e))?;
                    let fd = f.into_raw_fd();
                    raw_dup2(fd, 2);
                    raw_close(fd);
                }
                RedirectKind::ErrAppend => {
                    let path = self.expand_word(&r.target)?;
                    let f = OpenOptions::new()
                        .write(true)
                        .create(true)
                        .append(true)
                        .mode(0o644)
                        .open(&path)
                        .map_err(|e| format!("{}: {}", path, e))?;
                    let fd = f.into_raw_fd();
                    raw_dup2(fd, 2);
                    raw_close(fd);
                }
                RedirectKind::Both => {
                    let path = self.expand_word(&r.target)?;
                    let f = OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .mode(0o644)
                        .open(&path)
                        .map_err(|e| format!("{}: {}", path, e))?;
                    let fd = f.into_raw_fd();
                    raw_dup2(fd, 1);
                    raw_dup2(fd, 2);
                    raw_close(fd);
                }
                RedirectKind::DupErrToOut => {
                    raw_dup2(1, 2);
                }
                RedirectKind::DupOutToErr => {
                    raw_dup2(2, 1);
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn exec_and_replace(shell: &mut Shell, argv: &[String]) -> ! {
    let cname = match CString::new(argv[0].as_str()) {
        Ok(c) => c,
        Err(_) => std::process::exit(127),
    };
    let cargs: Vec<CString> = argv
        .iter()
        .map(|a| CString::new(a.as_str()).unwrap_or_default())
        .collect();
    for (k, v) in shell.env_for_child() {
        unsafe {
            std::env::set_var(k, v);
        }
    }
    match execvp(&cname, &cargs) {
        Ok(_) => unreachable!(),
        Err(nix::errno::Errno::ENOENT) => {
            eprintln!("xsh: {}: command not found", argv[0]);
            std::process::exit(127);
        }
        Err(e) => {
            eprintln!("xsh: {}: {}", argv[0], e);
            std::process::exit(126);
        }
    }
}

fn wait_for(pid: Pid) -> i32 {
    match waitpid(pid, None) {
        Ok(WaitStatus::Exited(_, code)) => code,
        Ok(WaitStatus::Signaled(_, sig, _)) => 128 + sig as i32,
        _ => 1,
    }
}
