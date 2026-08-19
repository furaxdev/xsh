use crate::ast::Node;
use nix::unistd::Pid;
use std::collections::{HashMap, HashSet};

pub struct Shell {
    pub vars: HashMap<String, String>,
    pub exported: HashSet<String>,
    pub aliases: HashMap<String, String>,
    pub functions: HashMap<String, Vec<Node>>,
    pub last_status: i32,
    pub shell_name: String,
    pub should_exit: Option<i32>,
    pub bg_jobs: Vec<Pid>,
}

impl Shell {
    pub fn new() -> Self {
        let mut vars = HashMap::new();
        let mut exported = HashSet::new();
        for (k, v) in std::env::vars() {
            exported.insert(k.clone());
            vars.insert(k, v);
        }
        if !vars.contains_key("PWD") {
            if let Ok(cwd) = std::env::current_dir() {
                vars.insert("PWD".to_string(), cwd.to_string_lossy().to_string());
                exported.insert("PWD".to_string());
            }
        }
        if !vars.contains_key("PS1") {
            vars.insert("PS1".to_string(), "xsh:$PWD$ ".to_string());
        }
        Shell {
            vars,
            exported,
            aliases: HashMap::new(),
            functions: HashMap::new(),
            last_status: 0,
            shell_name: "xsh".to_string(),
            should_exit: None,
            bg_jobs: Vec::new(),
        }
    }

    pub fn get_var(&self, name: &str) -> Option<String> {
        match name {
            "?" => Some(self.last_status.to_string()),
            "$" => Some(std::process::id().to_string()),
            "0" => Some(self.shell_name.clone()),
            _ => self.vars.get(name).cloned(),
        }
    }

    pub fn env_for_child(&self) -> Vec<(String, String)> {
        self.exported
            .iter()
            .filter_map(|k| self.vars.get(k).map(|v| (k.clone(), v.clone())))
            .collect()
    }
}
