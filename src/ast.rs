#[derive(Debug, Clone, PartialEq)]
pub enum WordPart {
    Literal(String),
    Var(String),
    CmdSub(String),
    Arith(String),
}

pub type Word = Vec<WordPart>;

#[derive(Debug, Clone)]
pub enum RedirectKind {
    In,          // <
    Out,         // >
    Append,      // >>
    ErrOut,      // 2>
    ErrAppend,   // 2>>
    Both,        // &> or >&
    DupErrToOut, // 2>&1
    DupOutToErr, // 1>&2
}

#[derive(Debug, Clone)]
pub struct Redirect {
    pub kind: RedirectKind,
    pub target: Word,
}

#[derive(Debug, Clone)]
pub struct SimpleCommand {
    pub assignments: Vec<(String, Word)>,
    pub words: Vec<Word>,
    pub redirects: Vec<Redirect>,
}

#[derive(Debug, Clone)]
pub enum Node {
    Simple(SimpleCommand),
    Pipeline(Vec<Node>, bool /* negate */),
    List(Vec<(Node, Sep)>),
    If {
        branches: Vec<(Vec<Node>, Vec<Node>)>, // (cond stmts, body) for if + elifs
        else_branch: Option<Vec<Node>>,
    },
    For {
        var: String,
        words: Vec<Word>,
        body: Vec<Node>,
    },
    While {
        cond: Vec<Node>,
        body: Vec<Node>,
        until: bool,
    },
    FunctionDef {
        name: String,
        body: Vec<Node>,
    },
    Subshell(Vec<Node>),
    Background(Box<Node>),
    Return(Option<Word>),
    Break,
    Continue,
    Case {
        word: Word,
        arms: Vec<(Vec<Word>, Vec<Node>)>,
    },
    Timed(Box<Node>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sep {
    Seq,
    And,
    Or,
}
