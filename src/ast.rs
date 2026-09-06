//! Axon abstract syntax tree.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Int,
    Float,
    Bool,
    Str,
    Void,
    Array { elem: Box<Type>, len: usize },
    Struct(String),
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Int => write!(f, "int"),
            Type::Float => write!(f, "float"),
            Type::Bool => write!(f, "bool"),
            Type::Str => write!(f, "string"),
            Type::Void => write!(f, "void"),
            Type::Array { elem, len } => write!(f, "[{elem}; {len}]"),
            Type::Struct(name) => write!(f, "{name}"),
        }
    }
}

impl Type {
    /// primitives that `print` accepts
    pub fn is_printable(&self) -> bool {
        matches!(self, Type::Int | Type::Float | Type::Bool | Type::Str)
    }

    pub fn is_compound(&self) -> bool {
        matches!(self, Type::Array { .. } | Type::Struct(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug)]
pub struct Program {
    pub structs: Vec<StructDecl>,
    pub funcs: Vec<FnDecl>,
}

#[derive(Debug)]
pub struct StructDecl {
    pub name: String,
    pub fields: Vec<Param>,
    pub pos: Pos,
}

#[derive(Debug)]
pub struct FnDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Type,
    pub body: Block,
    /// `extern def`: declared, body provided by the C runtime
    pub is_extern: bool,
    pub pos: Pos,
}

#[derive(Debug)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub pos: Pos,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum ForIter {
    /// `range(start, end, step)` — 1..3 int args
    Range(Vec<Expr>),
    /// iterate array elements (each copied into the loop variable)
    Array(Expr),
}

#[derive(Debug, Clone)]
pub enum Stmt {
    /// `x = e` (inferred) or `x: t = e` (annotated). First use declares the
    /// variable; subsequent uses are re-assignments with the same type.
    Let {
        name: String,
        ty: Option<Type>,
        expr: Expr,
        pos: Pos,
    },
    /// assignment to a compound lvalue: `a[i] = e` / `p.f = e`
    Assign {
        target: Expr,
        expr: Expr,
        pos: Pos,
    },
    If {
        cond: Expr,
        then_block: Block,
        else_block: Option<Block>,
        pos: Pos,
    },
    While {
        cond: Expr,
        body: Block,
        pos: Pos,
    },
    For {
        var: String,
        iter: ForIter,
        body: Block,
        pos: Pos,
    },
    Break {
        pos: Pos,
    },
    Continue {
        pos: Pos,
    },
    Return {
        expr: Option<Expr>,
        pos: Pos,
    },
    Pass,
    ExprStmt {
        expr: Expr,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Expr,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64, Pos),
    Float(f64, Pos),
    Str(String, Pos),
    Bool(bool, Pos),
    Var { name: String, pos: Pos },
    Call { name: String, args: Vec<Arg>, pos: Pos, lit_id: usize },
    Unary { op: UnOp, expr: Box<Expr>, pos: Pos },
    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr>, pos: Pos },
    Index { arr: Box<Expr>, idx: Box<Expr>, pos: Pos },
    Field { obj: Box<Expr>, name: String, pos: Pos },
    ArrayLit { elems: Vec<Expr>, lit_id: usize, pos: Pos },
    /// `[elem] * N` — single-element array replication (Python-style)
    ArrayRep { elem: Box<Expr>, count: usize, lit_id: usize, pos: Pos },
    StructLit { name: String, fields: Vec<(String, Expr)>, lit_id: usize, pos: Pos },
}

impl Expr {
    pub fn pos(&self) -> Pos {
        match self {
            Expr::Int(_, p) | Expr::Float(_, p) | Expr::Str(_, p) | Expr::Bool(_, p) => *p,
            Expr::Var { pos, .. }
            | Expr::Call { pos, .. }
            | Expr::Unary { pos, .. }
            | Expr::Binary { pos, .. }
            | Expr::Index { pos, .. }
            | Expr::Field { pos, .. }
            | Expr::ArrayLit { pos, .. }
            | Expr::ArrayRep { pos, .. }
            | Expr::StructLit { pos, .. } => *pos,
        }
    }

    /// A valid assignment target: variable, index, or field access.
    pub fn is_lvalue(&self) -> bool {
        matches!(self, Expr::Var { .. } | Expr::Index { .. } | Expr::Field { .. })
    }
}
