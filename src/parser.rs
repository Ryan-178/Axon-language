//! Axon parser: tokens -> AST (recursive descent, Python-style layout).

use crate::ast::*;
use crate::lexer::{FStrPart, Tok, Token};
use crate::Diag;

pub fn parse(tokens: Vec<Token>) -> Result<Program, Diag> {
    Parser { toks: tokens, idx: 0, next_lit_id: 0, cur_type_params: Vec::new(), cur_len_params: Vec::new() }.parse_program()
}

struct Parser {
    toks: Vec<Token>,
    idx: usize,
    next_lit_id: usize,
    /// type/length parameters of the fn currently being parsed
    cur_type_params: Vec<String>,
    /// length parameter names actually used in the current fn (v0.7: max 1)
    cur_len_params: Vec<String>,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.idx].tok
    }

    fn peek2(&self) -> &Tok {
        let n = (self.idx + 1).min(self.toks.len() - 1);
        &self.toks[n].tok
    }

    fn pos(&self) -> Pos {
        self.toks[self.idx].pos
    }

    fn bump(&mut self) -> Tok {
        let t = self.toks[self.idx].tok.clone();
        if self.idx < self.toks.len() - 1 {
            self.idx += 1;
        }
        t
    }

    fn eat(&mut self, expected: &Tok) -> Result<(), Diag> {
        if self.peek() == expected {
            self.bump();
            Ok(())
        } else {
            Err(self.unexpected(expected))
        }
    }

    fn unexpected(&self, expected: &Tok) -> Diag {
        Diag {
            stage: "parse",
            line: self.pos().line,
            col: self.pos().col,
            message: format!("expected {expected:?}, found {:?}", self.peek()),
        }
    }

    fn lit_id(&mut self) -> usize {
        let id = self.next_lit_id;
        self.next_lit_id += 1;
        id
    }

    // ---- top level ----

    fn parse_program(&mut self) -> Result<Program, Diag> {
        let mut structs = Vec::new();
        let mut funcs = Vec::new();
        while *self.peek() == Tok::Newline {
            self.bump();
        }
        while *self.peek() != Tok::Eof {
            match self.peek() {
                Tok::Def => {
                    funcs.push(self.fn_decl(false)?);
                    while *self.peek() == Tok::Newline {
                        self.bump();
                    }
                }
                Tok::Extern => {
                    funcs.push(self.fn_decl(true)?);
                    while *self.peek() == Tok::Newline {
                        self.bump();
                    }
                }
                Tok::Struct => {
                    structs.push(self.struct_decl()?);
                    while *self.peek() == Tok::Newline {
                        self.bump();
                    }
                }
                _ => return Err(Diag {
                    stage: "parse",
                    line: self.pos().line,
                    col: self.pos().col,
                    message: "expected 'def' or 'struct' at top level".into(),
                }),
            }
        }
        Ok(Program { structs, funcs })
    }

    fn struct_decl(&mut self) -> Result<StructDecl, Diag> {
        let pos = self.pos();
        self.eat(&Tok::Struct)?;
        let name = match self.bump() {
            Tok::Ident(n) => n,
            _ => return Err(self.unexpected(&Tok::Ident("name".into()))),
        };
        self.eat(&Tok::Colon)?;
        // fields are an indented block only
        self.eat(&Tok::Newline)?;
        self.eat(&Tok::Indent)?;
        let mut fields = Vec::new();
        while *self.peek() != Tok::Dedent {
            if *self.peek() == Tok::Eof {
                return Err(Diag {
                    stage: "parse",
                    line: self.pos().line,
                    col: self.pos().col,
                    message: "unexpected end of file inside struct body".into(),
                });
            }
            let fpos = self.pos();
            let fname = match self.bump() {
                Tok::Ident(n) => n,
                _ => return Err(self.unexpected(&Tok::Ident("field".into()))),
            };
            self.eat(&Tok::Colon)?;
            let ty = self.ty()?;
            fields.push(Param { name: fname, ty, pos: fpos });
            if *self.peek() == Tok::Newline {
                self.bump();
            }
        }
        self.bump(); // Dedent
        Ok(StructDecl { name, fields, pos })
    }

    fn fn_decl(&mut self, is_extern: bool) -> Result<FnDecl, Diag> {
        let pos = self.pos();
        if is_extern {
            self.eat(&Tok::Extern)?;
        }
        self.eat(&Tok::Def)?;
        let name = match self.bump() {
            Tok::Ident(n) => n,
            _ => return Err(self.unexpected(&Tok::Ident("name".into()))),
        };
        // optional generic header: `def name[T, N](...)`
        let mut type_params: Vec<String> = Vec::new();
        if *self.peek() == Tok::LBracket {
            self.bump();
            loop {
                match self.bump() {
                    Tok::Ident(n) => type_params.push(n),
                    _ => return Err(self.unexpected(&Tok::Ident("type parameter".into()))),
                }
                match self.peek() {
                    Tok::Comma => {
                        self.bump();
                    }
                    Tok::RBracket => break,
                    _ => return Err(self.unexpected(&Tok::RBracket)),
                }
            }
            self.eat(&Tok::RBracket)?;
        }
        self.cur_type_params = type_params.clone();
        self.eat(&Tok::LParen)?;
        let mut params = Vec::new();
        if *self.peek() != Tok::RParen {
            loop {
                let ppos = self.pos();
                let pname = match self.bump() {
                    Tok::Ident(n) => n,
                    _ => return Err(self.unexpected(&Tok::Ident("param".into()))),
                };
                self.eat(&Tok::Colon)?;
                let ty = self.ty()?;
                params.push(Param { name: pname, ty, pos: ppos });
                match self.peek() {
                    Tok::Comma => {
                        self.bump();
                        if *self.peek() == Tok::RParen {
                            break; // trailing comma
                        }
                    }
                    Tok::RParen => break,
                    _ => return Err(self.unexpected(&Tok::RParen)),
                }
            }
        }
        self.eat(&Tok::RParen)?;
        let ret = if *self.peek() == Tok::Arrow {
            self.bump();
            self.ty()?
        } else {
            Type::Void
        };
        let body = if is_extern {
            // extern declarations have no body; the statement ends at the newline
            Block { stmts: vec![] }
        } else {
            self.block()?
        };
        self.cur_type_params = Vec::new();
        let len_param = self.cur_len_params.first().cloned();
        if self.cur_len_params.len() > 1 {
            let n = self.cur_len_params[1].clone();
            self.cur_len_params = Vec::new();
            return Err(Diag {
                stage: "parse",
                line: pos.line,
                col: pos.col,
                message: format!("only one length parameter is supported (found '{n}' as well)"),
            });
        }
        self.cur_len_params = Vec::new();
        Ok(FnDecl { name, type_params, len_param, params, ret, body, is_extern, pos })
    }

    fn ty(&mut self) -> Result<Type, Diag> {
        let t = match self.bump() {
            Tok::TyInt => Type::Int,
            Tok::TyFloat => Type::Float,
            Tok::TyBool => Type::Bool,
            Tok::TyString => Type::Str,
            Tok::TyVoid => Type::Void,
            Tok::Ident(n) => Type::Struct(n),
            Tok::LBracket => {
                let elem = Box::new(self.ty()?);
                self.eat(&Tok::Semi)?;
                let len = match self.bump() {
                    Tok::Int(n) if n > 0 => n as usize,
                    Tok::Int(_) => {
                        return Err(Diag {
                            stage: "parse",
                            line: self.pos().line,
                            col: self.pos().col,
                            message: "array length must be a positive integer".into(),
                        })
                    }
                    // `[T; N]` inside a generic declaration: length parameter
                    Tok::Ident(n) if self.cur_type_params.contains(&n) => {
                        if !self.cur_len_params.contains(&n) {
                            self.cur_len_params.push(n);
                        }
                        crate::ast::GENERIC_LEN
                    }
                    Tok::Ident(n) => {
                        return Err(Diag {
                            stage: "parse",
                            line: self.pos().line,
                            col: self.pos().col,
                            message: format!(
                                "unknown array length '{n}' (length parameters must be declared in the fn header, e.g. def f[T, N](arr: [T; N]))"
                            ),
                        })
                    }
                    _ => return Err(self.unexpected(&Tok::Int(0))),
                };
                self.eat(&Tok::RBracket)?;
                Type::Array { elem, len }
            }
            _ => {
                return Err(Diag {
                    stage: "parse",
                    line: self.pos().line,
                    col: self.pos().col,
                    message: "expected a type: int | float | bool | string | void | name | [T; N]".into(),
                })
            }
        };
        Ok(t)
    }

    // ---- blocks and statements ----

    /// `:` NEWLINE INDENT stmt* DEDENT   |   `:` simple_stmt NEWLINE
    fn block(&mut self) -> Result<Block, Diag> {
        self.eat(&Tok::Colon)?;
        if *self.peek() == Tok::Newline {
            self.bump();
            self.eat(&Tok::Indent)?;
            let mut stmts = Vec::new();
            while *self.peek() != Tok::Dedent {
                if *self.peek() == Tok::Eof {
                    return Err(Diag {
                        stage: "parse",
                        line: self.pos().line,
                        col: self.pos().col,
                        message: "unexpected end of file inside an indented block".into(),
                    });
                }
                stmts.push(self.stmt()?);
                if *self.peek() == Tok::Newline {
                    self.bump();
                }
            }
            self.bump(); // Dedent
            Ok(Block { stmts })
        } else {
            // single-line body: only simple statements allowed
            let stmts = vec![self.simple_stmt()?];
            self.eat(&Tok::Newline)?;
            Ok(Block { stmts })
        }
    }

    fn stmt(&mut self) -> Result<Stmt, Diag> {
        let pos = self.pos();
        match self.peek() {
            Tok::If => self.if_stmt(),
            Tok::While => {
                self.bump();
                let cond = self.expr()?;
                let body = self.block()?;
                Ok(Stmt::While { cond, body, pos })
            }
            Tok::For => {
                self.bump();
                let var = match self.bump() {
                    Tok::Ident(n) => n,
                    _ => return Err(self.unexpected(&Tok::Ident("loop variable".into()))),
                };
                self.eat(&Tok::In)?;
                // `range(...)` is contextual: an identifier followed by '('
                let iter = if matches!(self.peek(), Tok::Ident(n) if n == "range")
                    && *self.peek2() == Tok::LParen
                {
                    self.bump(); // 'range'
                    self.bump(); // '('
                    let args = self.call_args()?;
                    self.eat(&Tok::RParen)?;
                    let vals: Vec<Expr> = args.into_iter().map(|a| a.value).collect();
                    ForIter::Range(vals)
                } else {
                    let e = self.expr()?;
                    ForIter::Array(e)
                };
                let body = self.block()?;
                Ok(Stmt::For { var, iter, body, pos })
            }
            Tok::Break => {
                self.bump();
                Ok(Stmt::Break { pos })
            }
            Tok::Continue => {
                self.bump();
                Ok(Stmt::Continue { pos })
            }
            Tok::Return => {
                self.bump();
                if *self.peek() == Tok::Newline {
                    Ok(Stmt::Return { expr: None, pos })
                } else {
                    let expr = self.expr()?;
                    Ok(Stmt::Return { expr: Some(expr), pos })
                }
            }
            Tok::Pass => {
                self.bump();
                Ok(Stmt::Pass)
            }
            Tok::Ident(_) if *self.peek2() == Tok::Colon => {
                // annotated binding: x: t = e
                let name = match self.bump() {
                    Tok::Ident(n) => n,
                    _ => unreachable!(),
                };
                self.bump(); // ':'
                let ty = self.ty()?;
                self.eat(&Tok::Assign)?;
                let expr = self.expr()?;
                Ok(Stmt::Let { name, ty: Some(ty), expr, pos })
            }
            _ => {
                // assignment or expression statement
                let expr = self.expr()?;
                if *self.peek() == Tok::Assign {
                    if !expr.is_lvalue() {
                        return Err(Diag {
                            stage: "parse",
                            line: pos.line,
                            col: pos.col,
                            message: "invalid assignment target".into(),
                        });
                    }
                    self.bump(); // '='
                    let value = self.expr()?;
                    match expr {
                        // `x = e` is a binding/re-assignment; compound targets are Assign
                        Expr::Var { name, pos } => Ok(Stmt::Let { name, ty: None, expr: value, pos }),
                        target => Ok(Stmt::Assign { target, expr: value, pos }),
                    }
                } else {
                    Ok(Stmt::ExprStmt { expr })
                }
            }
        }
    }

    /// simple statements allowed after `:` on the same line
    fn simple_stmt(&mut self) -> Result<Stmt, Diag> {
        match self.peek() {
            Tok::If | Tok::While | Tok::For | Tok::Def | Tok::Struct => Err(Diag {
                stage: "parse",
                line: self.pos().line,
                col: self.pos().col,
                message: "compound statements cannot appear on the same line after ':'".into(),
            }),
            _ => self.stmt(),
        }
    }

    fn if_stmt(&mut self) -> Result<Stmt, Diag> {
        let pos = self.pos();
        self.eat(&Tok::If)?;
        let cond = self.expr()?;
        let then_block = self.block()?;

        // collect elif branches and the final else
        let mut branches: Vec<(Expr, Block)> = vec![(cond, then_block)];
        while *self.peek() == Tok::Elif {
            self.bump();
            let c = self.expr()?;
            let b = self.block()?;
            branches.push((c, b));
        }
        let mut else_cur: Option<Block> = if *self.peek() == Tok::Else {
            self.bump();
            Some(self.block()?)
        } else {
            None
        };

        // fold elifs into nested ifs (right-assoc), keeps the AST tiny:
        // innermost branch first, outermost (`branches[0]`) last
        for (c, b) in branches.iter().skip(1).rev() {
            else_cur = Some(Block {
                stmts: vec![Stmt::If {
                    cond: c.clone(),
                    then_block: b.clone(),
                    else_block: else_cur.take(),
                    pos,
                }],
            });
        }
        let (cond, then_block) = branches.swap_remove(0);
        Ok(Stmt::If { cond, then_block, else_block: else_cur, pos })
    }

    /// f"pre{expr}post" 鈫?"pre" + str(expr) + "post"
    fn desugar_fstring(&mut self, parts: Vec<FStrPart>, pos: Pos) -> Result<Expr, Diag> {
        let mut exprs: Vec<Expr> = Vec::new();
        for p in parts {
            match p {
                FStrPart::Lit(s) => {
                    if !s.is_empty() {
                        exprs.push(Expr::Str(s, pos));
                    }
                }
                FStrPart::ExprTokens(toks) => {
                    let mut sub = Parser { toks, idx: 0, next_lit_id: self.next_lit_id, cur_type_params: Vec::new(), cur_len_params: Vec::new() };
                    let value = sub.expr()?;
                    self.next_lit_id = sub.next_lit_id;
                    let lit_id = self.lit_id();
                    exprs.push(Expr::Call { name: "str".into(), args: vec![Arg { name: None, value }], pos, lit_id });
                }
            }
        }
        if exprs.is_empty() {
            return Ok(Expr::Str(String::new(), pos));
        }
        let mut acc = exprs.remove(0);
        for e in exprs {
            acc = Expr::Binary { op: BinOp::Add, lhs: Box::new(acc), rhs: Box::new(e), pos };
        }
        Ok(acc)
    }

    // ---- expressions, precedence climbing ----

    fn expr(&mut self) -> Result<Expr, Diag> {
        self.or_expr()
    }

    fn or_expr(&mut self) -> Result<Expr, Diag> {
        let mut lhs = self.and_expr()?;
        while *self.peek() == Tok::OrOr {
            let pos = self.pos();
            self.bump();
            let rhs = self.and_expr()?;
            lhs = Expr::Binary { op: BinOp::Or, lhs: Box::new(lhs), rhs: Box::new(rhs), pos };
        }
        Ok(lhs)
    }

    fn and_expr(&mut self) -> Result<Expr, Diag> {
        let mut lhs = self.eq_expr()?;
        while *self.peek() == Tok::AndAnd {
            let pos = self.pos();
            self.bump();
            let rhs = self.eq_expr()?;
            lhs = Expr::Binary { op: BinOp::And, lhs: Box::new(lhs), rhs: Box::new(rhs), pos };
        }
        Ok(lhs)
    }

    fn eq_expr(&mut self) -> Result<Expr, Diag> {
        let mut lhs = self.rel_expr()?;
        loop {
            let op = match self.peek() {
                Tok::Eq => BinOp::Eq,
                Tok::Ne => BinOp::Ne,
                _ => break,
            };
            let pos = self.pos();
            self.bump();
            let rhs = self.rel_expr()?;
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), pos };
        }
        Ok(lhs)
    }

    fn rel_expr(&mut self) -> Result<Expr, Diag> {
        let mut lhs = self.add_expr()?;
        loop {
            let op = match self.peek() {
                Tok::Lt => BinOp::Lt,
                Tok::Le => BinOp::Le,
                Tok::Gt => BinOp::Gt,
                Tok::Ge => BinOp::Ge,
                _ => break,
            };
            let pos = self.pos();
            self.bump();
            let rhs = self.add_expr()?;
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), pos };
        }
        Ok(lhs)
    }

    fn add_expr(&mut self) -> Result<Expr, Diag> {
        let mut lhs = self.mul_expr()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => break,
            };
            let pos = self.pos();
            self.bump();
            let rhs = self.mul_expr()?;
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), pos };
        }
        Ok(lhs)
    }

    fn mul_expr(&mut self) -> Result<Expr, Diag> {
        let mut lhs = self.unary_expr()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Percent => BinOp::Mod,
                _ => break,
            };
            let pos = self.pos();
            self.bump();
            let rhs = self.unary_expr()?;
            // Python-style array replication: [e] * N  (or N * [e])
            let rep: Option<(Expr, usize)> = match (&lhs, &rhs) {
                (Expr::ArrayLit { elems, .. }, Expr::Int(n, _)) if elems.len() == 1 && *n > 0 => {
                    Some((elems[0].clone(), *n as usize))
                }
                (Expr::Int(n, _), Expr::ArrayLit { elems, .. }) if elems.len() == 1 && *n > 0 => {
                    Some((elems[0].clone(), *n as usize))
                }
                _ => None,
            };
            if let Some((elem, count)) = rep {
                let lit_id = self.lit_id();
                lhs = Expr::ArrayRep { elem: Box::new(elem), count, lit_id, pos };
                continue;
            }
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), pos };
        }
        Ok(lhs)
    }

    fn unary_expr(&mut self) -> Result<Expr, Diag> {
        let pos = self.pos();
        match self.peek() {
            Tok::Minus => {
                self.bump();
                let e = self.unary_expr()?;
                Ok(Expr::Unary { op: UnOp::Neg, expr: Box::new(e), pos })
            }
            Tok::Bang => {
                self.bump();
                let e = self.unary_expr()?;
                Ok(Expr::Unary { op: UnOp::Not, expr: Box::new(e), pos })
            }
            _ => self.postfix(),
        }
    }

    /// primary followed by call / index / field-access suffixes
    fn postfix(&mut self) -> Result<Expr, Diag> {
        let mut e = self.primary()?;
        loop {
            match self.peek() {
                Tok::LParen => {
                    // only bare names are callable
                    let name = match &e {
                        Expr::Var { name, .. } => name.clone(),
                        other => {
                            let p = other.pos();
                            return Err(Diag {
                                stage: "parse",
                                line: p.line,
                                col: p.col,
                                message: "only named functions can be called".into(),
                            });
                        }
                    };
                    self.bump(); // '('
                    let args = self.call_args()?;
                    self.eat(&Tok::RParen)?;
                    let pos = e.pos();
                    let lit_id = self.lit_id();
                    e = Expr::Call { name, args, pos, lit_id };
                }
                Tok::LBracket => {
                    self.bump();
                    let idx = self.expr()?;
                    self.eat(&Tok::RBracket)?;
                    let pos = e.pos();
                    e = Expr::Index { arr: Box::new(e), idx: Box::new(idx), pos };
                }
                Tok::Dot => {
                    self.bump();
                    let name = match self.bump() {
                        Tok::Ident(n) => n,
                        _ => return Err(self.unexpected(&Tok::Ident("field".into()))),
                    };
                    let pos = e.pos();
                    e = Expr::Field { obj: Box::new(e), name, pos };
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn call_args(&mut self) -> Result<Vec<Arg>, Diag> {
        let mut args = Vec::new();
        if *self.peek() == Tok::RParen {
            return Ok(args);
        }
        loop {
            let arg = if matches!(*self.peek(), Tok::Ident(_)) && *self.peek2() == Tok::Assign {
                let name = match self.bump() {
                    Tok::Ident(n) => n,
                    _ => unreachable!(),
                };
                self.bump(); // '='
                Arg { name: Some(name), value: self.expr()? }
            } else {
                Arg { name: None, value: self.expr()? }
            };
            args.push(arg);
            match self.peek() {
                Tok::Comma => {
                    self.bump();
                    if *self.peek() == Tok::RParen {
                        break; // trailing comma
                    }
                }
                Tok::RParen => break,
                _ => return Err(self.unexpected(&Tok::RParen)),
            }
        }
        Ok(args)
    }

    fn primary(&mut self) -> Result<Expr, Diag> {
        let pos = self.pos();
        match self.bump() {
            Tok::Int(v) => Ok(Expr::Int(v, pos)),
            Tok::Float(v) => Ok(Expr::Float(v, pos)),
            Tok::Str(s) => Ok(Expr::Str(s, pos)),
            Tok::True => Ok(Expr::Bool(true, pos)),
            Tok::False => Ok(Expr::Bool(false, pos)),
            Tok::Ident(name) => Ok(Expr::Var { name, pos }),
            Tok::FStr(parts) => self.desugar_fstring(parts, pos),
            Tok::LParen => {
                let e = self.expr()?;
                self.eat(&Tok::RParen)?;
                Ok(e)
            }
            Tok::LBracket => {
                // array literal: [e0, e1, ...]
                let mut elems = Vec::new();
                if *self.peek() != Tok::RBracket {
                    loop {
                        elems.push(self.expr()?);
                        match self.peek() {
                            Tok::Comma => {
                                self.bump();
                                if *self.peek() == Tok::RBracket {
                                    break; // trailing comma
                                }
                            }
                            Tok::RBracket => break,
                            _ => return Err(self.unexpected(&Tok::RBracket)),
                        }
                    }
                }
                self.eat(&Tok::RBracket)?;
                if elems.is_empty() {
                    return Err(Diag {
                        stage: "parse",
                        line: pos.line,
                        col: pos.col,
                        message: "empty array literals are not allowed (element type could not be inferred)".into(),
                    });
                }
                let lit_id = self.lit_id();
                Ok(Expr::ArrayLit { elems, lit_id, pos })
            }
            _ => Err(Diag {
                stage: "parse",
                line: pos.line,
                col: pos.col,
                message: format!("expected an expression, found {:?}", self.toks[self.idx.saturating_sub(1)].tok),
            }),
        }
    }
}


