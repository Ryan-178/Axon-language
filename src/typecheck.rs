//! Axon type checker: strict, no implicit conversions, deterministic errors.
//! Generic functions are monomorphized at call sites: arguments are unified
//! against the declared param types (T / length N), a concrete instance is
//! cloned+substituted, queued for checking, and codegen receives the instance.

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::Diag;

/// resolved struct layout: field name -> (index, type)
pub type StructTable = HashMap<String, Vec<(String, Type)>>;

#[derive(Debug, Clone)]
pub struct FnSig {
    pub params: Vec<Type>,
    pub ret: Type,
}

/// mutable checking context shared by the recursive walkers
pub struct Tc<'a> {
    pub sigs: HashMap<String, FnSig>,
    pub structs: &'a StructTable,
    pub generics: &'a HashMap<String, FnDecl>,
    /// file whose function body is currently being checked
    pub cur_file: u32,
    /// AST node address of a generic call -> mangled instance name
    pub call_map: HashMap<usize, String>,
    /// instances pending body checking
    queue: Vec<FnDecl>,
    /// instance names already created (dedup)
    done: HashSet<String>,
    /// all created instances, emitted by the codegen stage
    instances: Vec<FnDecl>,
}

pub struct CheckOutput {
    pub structs: StructTable,
    pub sigs: HashMap<String, FnSig>,
    pub instances: Vec<FnDecl>,
    pub call_map: HashMap<usize, String>,
}

pub fn check(program: &Program) -> Result<CheckOutput, Diag> {
    let structs = collect_structs(&program.structs)?;
    let mut sigs: HashMap<String, FnSig> = HashMap::new();
    let mut generics: HashMap<String, FnDecl> = HashMap::new();

    // pass 1: concrete functions (signatures first, enables mutual recursion)
    for f in &program.funcs {
        if !f.type_params.is_empty() {
            continue;
        }
        if f.is_extern && f.name == "main" {
            return Err(Diag {
                stage: "type",
                file: f.pos.file,
                line: f.pos.line,
                col: f.pos.col,
                message: "'main' cannot be declared extern".into(),
            });
        }
        if sigs.contains_key(&f.name) {
            return Err(Diag {
                stage: "type",
                file: f.pos.file,
                line: f.pos.line,
                col: f.pos.col,
                message: format!("function '{}' is defined more than once", f.name),
            });
        }
        if structs.contains_key(&f.name) {
            return Err(Diag {
                stage: "type",
                file: f.pos.file,
                line: f.pos.line,
                col: f.pos.col,
                message: format!("'{}' is already defined as a struct", f.name),
            });
        }
        for (i, p) in f.params.iter().enumerate() {
            if f.params[..i].iter().any(|q| q.name == p.name) {
                return Err(Diag {
                    stage: "type",
                    file: p.pos.file,
                    line: p.pos.line,
                    col: p.pos.col,
                    message: format!("duplicate parameter '{}' in function '{}'", p.name, f.name),
                });
            }
            resolve_ty(&p.ty, &structs).map_err(|m| Diag {
                stage: "type",
                file: p.pos.file,
                line: p.pos.line,
                col: p.pos.col,
                message: m,
            })?;
            if p.ty == Type::Void {
                return Err(Diag {
                    stage: "type",
                    file: p.pos.file,
                    line: p.pos.line,
                    col: p.pos.col,
                    message: format!("parameter '{}' cannot have type void", p.name),
                });
            }
        }
        resolve_ty(&f.ret, &structs).map_err(|m| Diag {
            stage: "type",
            file: f.pos.file,
            line: f.pos.line,
            col: f.pos.col,
            message: m,
        })?;
        if has_generic_len(&f.ret) {
            return Err(Diag {
                stage: "type",
                file: f.pos.file,
                line: f.pos.line,
                col: f.pos.col,
                message: "array length parameters are only valid inside generic functions".into(),
            });
        }
        for p in &f.params {
            if has_generic_len(&p.ty) {
                return Err(Diag {
                    stage: "type",
                    file: p.pos.file,
                    line: p.pos.line,
                    col: p.pos.col,
                    message: "array length parameters are only valid inside generic functions".into(),
                });
            }
        }
        sigs.insert(
            f.name.clone(),
            FnSig {
                params: f.params.iter().map(|p| p.ty.clone()).collect(),
                ret: f.ret.clone(),
            },
        );
    }

    // pass 2: generic declarations (no body checking until instantiation)
    for f in &program.funcs {
        if f.type_params.is_empty() {
            continue;
        }
        if f.is_extern {
            return Err(Diag {
                stage: "type",
                file: f.pos.file,
                line: f.pos.line,
                col: f.pos.col,
                message: "extern functions cannot be generic".into(),
            });
        }
        if f.name == "main" {
            return Err(Diag {
                stage: "type",
                file: f.pos.file,
                line: f.pos.line,
                col: f.pos.col,
                message: "'main' cannot be generic".into(),
            });
        }
        if sigs.contains_key(&f.name) || generics.contains_key(&f.name) {
            return Err(Diag {
                stage: "type",
                file: f.pos.file,
                line: f.pos.line,
                col: f.pos.col,
                message: format!("function '{}' is defined more than once", f.name),
            });
        }
        if structs.contains_key(&f.name) {
            return Err(Diag {
                stage: "type",
                file: f.pos.file,
                line: f.pos.line,
                col: f.pos.col,
                message: format!("'{}' is already defined as a struct", f.name),
            });
        }
        generics.insert(f.name.clone(), f.clone());
    }

    if !sigs.contains_key("main") {
        return Err(Diag {
            stage: "type",
            file: 0,
            line: 1,
            col: 1,
            message: "program has no 'main' function".into(),
        });
    }

    let mut tc = Tc {
        sigs,
        structs: &structs,
        generics: &generics,
        cur_file: 0,
        call_map: HashMap::new(),
        queue: Vec::new(),
        done: HashSet::new(),
        instances: Vec::new(),
    };

    // pass 3: check concrete non-extern bodies
    for f in &program.funcs {
        if f.type_params.is_empty() && !f.is_extern {
            tc.cur_file = f.pos.file;
            tc.check_fn_body(f)?;
        }
    }

    // pass 4: drain monomorphized instances (they may enqueue more)
    let mut qi = 0;
    while qi < tc.queue.len() {
        let inst = tc.queue[qi].clone();
        qi += 1;
        tc.cur_file = inst.pos.file;
        tc.check_fn_body(&inst)?;
    }
    let Tc { sigs, call_map, instances, .. } = tc;

    Ok(CheckOutput {
        structs,
        sigs,
        instances,
        call_map,
    })
}

impl<'a> Tc<'a> {
    fn check_fn_body(&mut self, f: &FnDecl) -> Result<(), Diag> {
        let mut scopes: Vec<(String, Type)> =
            f.params.iter().map(|p| (p.name.clone(), p.ty.clone())).collect();
        let returns_all = self.check_block(&f.body, f, &mut scopes, 0)?;
        // strict rule: non-void functions must return a value on every path
        if f.ret != Type::Void && !returns_all {
            return Err(Diag {
                stage: "type",
                file: f.pos.file,
                line: f.pos.line,
                col: f.pos.col,
                message: format!(
                    "function '{}' returns {} but does not return a value on all paths",
                    f.name, f.ret
                ),
            });
        }
        Ok(())
    }

    fn check_block(
        &mut self,
        block: &Block,
        f: &FnDecl,
        scopes: &mut Vec<(String, Type)>,
        loop_depth: usize,
    ) -> Result<bool, Diag> {
        let mut guarantees_return = false;
        for stmt in &block.stmts {
            if guarantees_return {
                let pos = stmt_pos(stmt);
                return Err(Diag {
                    stage: "type", file: self.cur_file,
                    line: pos.line,
                    col: pos.col,
                    message: "unreachable statement after 'return'".into(),
                });
            }
            self.check_stmt(stmt, f, scopes, loop_depth)?;
            guarantees_return = stmt_guarantees_return(stmt);
        }
        Ok(guarantees_return)
    }

    fn check_stmt(
        &mut self,
        stmt: &Stmt,
        f: &FnDecl,
        scopes: &mut Vec<(String, Type)>,
        loop_depth: usize,
    ) -> Result<(), Diag> {
        match stmt {
            Stmt::Let { name, ty, expr, pos } => {
                let t = self.check_expr(expr, scopes)?;
                if t == Type::Void {
                    return Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: format!("cannot bind a void expression to '{name}'"),
                    });
                }
                match lookup(scopes, name) {
                    Some(dt) => {
                        // re-assignment: type is fixed at first binding
                        if let Some(ann) = ty {
                            if *ann != dt {
                                return Err(Diag {
                                    stage: "type", file: self.cur_file,
                                    line: pos.line,
                                    col: pos.col,
                                    message: format!(
                                        "cannot re-declare '{name}' as {ann}: it is already {dt}"
                                    ),
                                });
                            }
                        }
                        if t != dt {
                            return Err(Diag {
                                stage: "type", file: self.cur_file,
                                line: pos.line,
                                col: pos.col,
                                message: format!("cannot assign a value of type {t} to '{name}: {dt}'"),
                            });
                        }
                    }
                    None => {
                        // first binding: annotation (if present) must match the initializer
                        if let Some(ann) = ty {
                            if *ann != t {
                                return Err(Diag {
                                    stage: "type", file: self.cur_file,
                                    line: pos.line,
                                    col: pos.col,
                                    message: format!(
                                        "cannot initialize '{name}: {ann}' with an expression of type {t}"
                                    ),
                                });
                            }
                        }
                        scopes.push((name.clone(), t));
                    }
                }
                Ok(())
            }
            Stmt::Assign { target, expr, pos } => {
                if !target.is_lvalue() {
                    return Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: "invalid assignment target".into(),
                    });
                }
                let dt = self.lvalue_type(target, scopes)?;
                let t = self.check_expr(expr, scopes)?;
                if t != dt {
                    return Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: format!("cannot assign a value of type {t} to a target of type {dt}"),
                    });
                }
                Ok(())
            }
            Stmt::If { cond, then_block, else_block, pos } => {
                let t = self.check_expr(cond, scopes)?;
                if t != Type::Bool {
                    return Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: format!("'if' condition must be bool, found {t}"),
                    });
                }
                self.check_block(then_block, f, scopes, loop_depth)?;
                if let Some(eb) = else_block {
                    self.check_block(eb, f, scopes, loop_depth)?;
                }
                Ok(())
            }
            Stmt::While { cond, body, pos } => {
                let t = self.check_expr(cond, scopes)?;
                if t != Type::Bool {
                    return Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: format!("'while' condition must be bool, found {t}"),
                    });
                }
                self.check_block(body, f, scopes, loop_depth + 1)?;
                Ok(())
            }
            Stmt::For { var, iter, body, pos } => {
                let var_ty = match iter {
                    ForIter::Range(args) => {
                        if args.is_empty() || args.len() > 3 {
                            return Err(Diag {
                                stage: "type", file: self.cur_file,
                                line: pos.line,
                                col: pos.col,
                                message: format!("range expects 1 to 3 arguments, found {}", args.len()),
                            });
                        }
                        for a in args {
                            let t = self.check_expr(a, scopes)?;
                            if t != Type::Int {
                                return Err(Diag {
                                    stage: "type", file: self.cur_file,
                                    line: a.pos().line,
                                    col: a.pos().col,
                                    message: format!("range arguments must be int, found {t}"),
                                });
                            }
                        }
                        Type::Int
                    }
                    ForIter::Array(e) => {
                        let t = self.check_expr(e, scopes)?;
                        match t {
                            Type::Array { elem, .. } => *elem,
                            other => {
                                return Err(Diag {
                                    stage: "type", file: self.cur_file,
                                    line: pos.line,
                                    col: pos.col,
                                    message: format!("'for' can only iterate over arrays, found {other}"),
                                })
                            }
                        }
                    }
                };
                match lookup(scopes, var) {
                    Some(dt) => {
                        if dt != var_ty {
                            return Err(Diag {
                                stage: "type", file: self.cur_file,
                                line: pos.line,
                                col: pos.col,
                                message: format!("loop variable '{var}' is already {dt}, cannot reuse as {var_ty}"),
                            });
                        }
                    }
                    None => {
                        scopes.push((var.clone(), var_ty));
                    }
                }
                self.check_block(body, f, scopes, loop_depth + 1)?;
                Ok(())
            }
            Stmt::Break { pos } => {
                if loop_depth == 0 {
                    return Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: "'break' outside of a loop".into(),
                    });
                }
                Ok(())
            }
            Stmt::Continue { pos } => {
                if loop_depth == 0 {
                    return Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: "'continue' outside of a loop".into(),
                    });
                }
                Ok(())
            }
            Stmt::Return { expr, pos } => {
                match (expr, &f.ret) {
                    (None, Type::Void) => Ok(()),
                    (None, ret) => Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: format!("'return' must return a value of type {ret}"),
                    }),
                    (Some(_), Type::Void) => Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: "void function cannot return a value".into(),
                    }),
                    (Some(e), ret) => {
                        let t = self.check_expr(e, scopes)?;
                        if t != *ret {
                            return Err(Diag {
                                stage: "type", file: self.cur_file,
                                line: pos.line,
                                col: pos.col,
                                message: format!("'return' type mismatch: expected {ret}, found {t}"),
                            });
                        }
                        Ok(())
                    }
                }
            }
            Stmt::Pass => Ok(()),
            Stmt::ExprStmt { expr } => {
                self.check_expr(expr, scopes)?;
                Ok(())
            }
        }
    }

    /// type of an assignment target (already validated as lvalue by the parser)
    fn lvalue_type(&mut self, target: &Expr, scopes: &mut Vec<(String, Type)>) -> Result<Type, Diag> {
        match target {
            Expr::Var { name, pos } => lookup(scopes, name).ok_or_else(|| Diag {
                stage: "type", file: self.cur_file,
                line: pos.line,
                col: pos.col,
                message: format!("assignment to undeclared variable '{name}'"),
            }),
            Expr::Index { arr, idx, pos } => {
                let at = self.check_expr(arr, scopes)?;
                let it = self.check_expr(idx, scopes)?;
                if it != Type::Int {
                    return Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: format!("array index must be int, found {it}"),
                    });
                }
                match at {
                    Type::Array { elem, .. } => Ok(*elem),
                    other => Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: format!("cannot index a value of type {other}"),
                    }),
                }
            }
            Expr::Field { obj, name, pos } => {
                let ot = self.check_expr(obj, scopes)?;
                field_type(&ot, name, self.structs).ok_or_else(|| Diag {
                    stage: "type", file: self.cur_file,
                    line: pos.line,
                    col: pos.col,
                    message: format!("type {ot} has no field '{name}'"),
                })
            }
            other => {
                let pos = other.pos();
                Err(Diag {
                    stage: "type", file: self.cur_file,
                    line: pos.line,
                    col: pos.col,
                    message: "invalid assignment target".into(),
                })
            }
        }
    }

    fn check_expr(&mut self, expr: &Expr, scopes: &mut Vec<(String, Type)>) -> Result<Type, Diag> {
        match expr {
            Expr::Int(..) => Ok(Type::Int),
            Expr::Float(..) => Ok(Type::Float),
            Expr::Str(..) => Ok(Type::Str),
            Expr::Bool(..) => Ok(Type::Bool),
            Expr::Var { name, pos } => lookup(scopes, name).ok_or_else(|| Diag {
                stage: "type", file: self.cur_file,
                line: pos.line,
                col: pos.col,
                message: format!("unknown variable '{name}'"),
            }),
            Expr::Index { arr, idx, pos } => {
                let at = self.check_expr(arr, scopes)?;
                let it = self.check_expr(idx, scopes)?;
                if it != Type::Int {
                    return Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: format!("array index must be int, found {it}"),
                    });
                }
                match at {
                    Type::Array { elem, .. } => Ok(*elem),
                    other => Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: format!("cannot index a value of type {other} (only [T; N] arrays are indexable)"),
                    }),
                }
            }
            Expr::Field { obj, name, pos } => {
                let ot = self.check_expr(obj, scopes)?;
                field_type(&ot, name, self.structs).ok_or_else(|| Diag {
                    stage: "type", file: self.cur_file,
                    line: pos.line,
                    col: pos.col,
                    message: format!("type {ot} has no field '{name}'"),
                })
            }
            Expr::ArrayLit { elems, pos, .. } => {
                if elems.is_empty() {
                    return Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: "empty array literals are not allowed".into(),
                    });
                }
                let elem = self.check_expr(&elems[0], scopes)?;
                for e in &elems[1..] {
                    let t = self.check_expr(e, scopes)?;
                    if t != elem {
                        return Err(Diag {
                            stage: "type", file: self.cur_file,
                            line: e.pos().line,
                            col: e.pos().col,
                            message: format!("array literal elements must share one type: found {elem} and {t}"),
                        });
                    }
                }
                Ok(Type::Array { elem: Box::new(elem), len: elems.len() })
            }
            Expr::ArrayRep { elem, count, pos, .. } => {
                if *count == 0 {
                    return Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: "array replication count must be positive".into(),
                    });
                }
                let t = self.check_expr(elem, scopes)?;
                Ok(Type::Array { elem: Box::new(t), len: *count })
            }
            Expr::StructLit { name, fields, pos, .. } => {
                let layout = self.structs.get(name).ok_or_else(|| Diag {
                    stage: "type", file: self.cur_file,
                    line: pos.line,
                    col: pos.col,
                    message: format!("unknown struct '{name}'"),
                })?;
                // every field exactly once (any order), types must match
                for (fname, fexpr) in fields {
                    let fty = layout
                        .iter()
                        .find(|(n, _)| n == fname)
                        .map(|(_, t)| t.clone())
                        .ok_or_else(|| Diag {
                            stage: "type", file: self.cur_file,
                            line: fexpr.pos().line,
                            col: fexpr.pos().col,
                            message: format!("struct '{name}' has no field '{fname}'"),
                        })?;
                    let t = self.check_expr(fexpr, scopes)?;
                    if t != fty {
                        return Err(Diag {
                            stage: "type", file: self.cur_file,
                            line: fexpr.pos().line,
                            col: fexpr.pos().col,
                            message: format!(
                                "field '{fname}' of '{name}' must be {fty}, found {t}"
                            ),
                        });
                    }
                }
                if fields.len() != layout.len() {
                    let missing: Vec<String> = layout
                        .iter()
                        .filter(|(n, _)| !fields.iter().any(|f| f.0 == *n))
                        .map(|(n, _)| n.clone())
                        .collect();
                    return Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: format!(
                            "struct literal '{}' is missing field(s): {}",
                            name,
                            missing.join(", ")
                        ),
                    });
                }
                // duplicate field names in the literal
                for (i, (fname, _)) in fields.iter().enumerate() {
                    if fields[..i].iter().any(|(n, _)| n == fname) {
                        let fpos = fields.iter().find(|(n, _)| n == fname).unwrap().1.pos();
                        return Err(Diag {
                            stage: "type", file: self.cur_file,
                            line: fpos.line,
                            col: fpos.col,
                            message: format!("field '{fname}' given more than once in struct literal"),
                        });
                    }
                }
                Ok(Type::Struct(name.clone()))
            }
            Expr::Call { name, args, pos, .. } => {
                // builtins first (they are not in the function table)
                if name == "print" {
                    if args.len() != 1 || args[0].name.is_some() {
                        return Err(Diag {
                            stage: "type", file: self.cur_file,
                            line: pos.line,
                            col: pos.col,
                            message: "print expects exactly 1 positional argument".into(),
                        });
                    }
                    let t = self.check_expr(&args[0].value, scopes)?;
                    if !t.is_printable() {
                        return Err(Diag {
                            stage: "type", file: self.cur_file,
                            line: pos.line,
                            col: pos.col,
                            message: format!("print requires int, float, bool, or string, found {t}"),
                        });
                    }
                    return Ok(Type::Void);
                }
                if name == "len" {
                    if args.len() != 1 || args[0].name.is_some() {
                        return Err(Diag {
                            stage: "type", file: self.cur_file,
                            line: pos.line,
                            col: pos.col,
                            message: "len expects exactly 1 positional argument".into(),
                        });
                    }
                    let t = self.check_expr(&args[0].value, scopes)?;
                    if !matches!(t, Type::Array { .. } | Type::Str) {
                        return Err(Diag {
                            stage: "type", file: self.cur_file,
                            line: pos.line,
                            col: pos.col,
                            message: format!("len requires an array or string, found {t}"),
                        });
                    }
                    return Ok(Type::Int);
                }
                if name == "str" {
                    if args.len() != 1 || args[0].name.is_some() {
                        return Err(Diag {
                            stage: "type", file: self.cur_file,
                            line: pos.line,
                            col: pos.col,
                            message: "str expects exactly 1 positional argument".into(),
                        });
                    }
                    let t = self.check_expr(&args[0].value, scopes)?;
                    if !t.is_printable() {
                        return Err(Diag {
                            stage: "type", file: self.cur_file,
                            line: pos.line,
                            col: pos.col,
                            message: format!("cannot convert {t} to string"),
                        });
                    }
                    return Ok(Type::Str);
                }
                // generic call: unify arguments, monomorphize
                if self.sigs.get(name).is_none() {
                    if self.generics.contains_key(name) {
                        return self.instantiate_call(name, args, *pos, expr, scopes);
                    }
                    if let Some(layout) = self.structs.get(name) {
                        let layout = layout.clone();
                        return self.check_struct_construction(name, &layout, args, *pos, scopes);
                    }
                    return Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: format!("call to undefined function or struct '{name}'"),
                    });
                }
                let sig = self.sigs[name].clone();
                if args.iter().any(|a| a.name.is_some()) {
                    return Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: format!("function '{}' takes positional arguments only", name),
                    });
                }
                if args.len() != sig.params.len() {
                    return Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: format!(
                            "function '{}' expects {} argument(s), found {}",
                            name,
                            sig.params.len(),
                            args.len()
                        ),
                    });
                }
                for (i, a) in args.iter().enumerate() {
                    let t = self.check_expr(&a.value, scopes)?;
                    if t != sig.params[i] {
                        return Err(Diag {
                            stage: "type", file: self.cur_file,
                            line: a.value.pos().line,
                            col: a.value.pos().col,
                            message: format!(
                                "argument {} of '{}' must be {}, found {}",
                                i + 1,
                                name,
                                sig.params[i],
                                t
                            ),
                        });
                    }
                }
                Ok(sig.ret.clone())
            }
            Expr::Unary { op, expr, pos } => {
                let t = self.check_expr(expr, scopes)?;
                match (op, t) {
                    (UnOp::Not, Type::Bool) => Ok(Type::Bool),
                    (UnOp::Not, other) => Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: format!("'!' requires bool, found {other}"),
                    }),
                    (UnOp::Neg, Type::Int) => Ok(Type::Int),
                    (UnOp::Neg, Type::Float) => Ok(Type::Float),
                    (UnOp::Neg, other) => Err(Diag {
                        stage: "type", file: self.cur_file,
                        line: pos.line,
                        col: pos.col,
                        message: format!("unary '-' requires int or float, found {other}"),
                    }),
                }
            }
            Expr::Binary { op, lhs, rhs, pos } => {
                let lt = self.check_expr(lhs, scopes)?;
                let rt = self.check_expr(rhs, scopes)?;
                use BinOp::*;
                match op {
                    And | Or => {
                        if lt == Type::Bool && rt == Type::Bool {
                            Ok(Type::Bool)
                        } else {
                            Err(Diag {
                                stage: "type", file: self.cur_file,
                                line: pos.line,
                                col: pos.col,
                                message: format!("'{}' requires bool operands, found ({lt}, {rt})", op_str(*op)),
                            })
                        }
                    }
                    Eq | Ne => {
                        if lt.is_compound() || rt.is_compound() {
                            Err(Diag {
                                stage: "type", file: self.cur_file,
                                line: pos.line,
                                col: pos.col,
                                message: format!("cannot compare compound type {lt} with {rt}"),
                            })
                        } else if lt == rt && lt != Type::Void {
                            Ok(Type::Bool)
                        } else {
                            Err(Diag {
                                stage: "type", file: self.cur_file,
                                line: pos.line,
                                col: pos.col,
                                message: format!("cannot compare {lt} with {rt}"),
                            })
                        }
                    }
                    Lt | Le | Gt | Ge => {
                        if lt == Type::Str && rt == Type::Str {
                            Ok(Type::Bool)
                        } else {
                            let numeric = (lt == Type::Int || lt == Type::Float) && lt == rt;
                            if numeric {
                                Ok(Type::Bool)
                            } else {
                                Err(Diag {
                                    stage: "type", file: self.cur_file,
                                    line: pos.line,
                                    col: pos.col,
                                    message: format!(
                                        "'{}' requires two int, two float, or two string operands, found ({lt}, {rt})",
                                        op_str(*op)
                                    ),
                                })
                            }
                        }
                    }
                    Add => {
                        if lt == Type::Str || rt == Type::Str {
                            if lt == Type::Str && rt == Type::Str {
                                Ok(Type::Str)
                            } else {
                                Err(Diag {
                                    stage: "type", file: self.cur_file,
                                    line: pos.line,
                                    col: pos.col,
                                    message: format!("cannot concatenate string with {rt}"),
                                })
                            }
                        } else if (lt == Type::Int || lt == Type::Float) && lt == rt {
                            Ok(lt)
                        } else {
                            Err(Diag {
                                stage: "type", file: self.cur_file,
                                line: pos.line,
                                col: pos.col,
                                message: format!(
                                    "'+' requires two int or two float operands, found ({lt}, {rt})"
                                ),
                            })
                        }
                    }
                    Sub | Mul | Div => {
                        if (lt == Type::Int || lt == Type::Float) && lt == rt {
                            Ok(lt)
                        } else {
                            Err(Diag {
                                stage: "type", file: self.cur_file,
                                line: pos.line,
                                col: pos.col,
                                message: format!(
                                    "'{}' requires two int or two float operands, found ({lt}, {rt})",
                                    op_str(*op)
                                ),
                            })
                        }
                    }
                    Mod => {
                        if lt == Type::Int && rt == Type::Int {
                            Ok(Type::Int)
                        } else {
                            Err(Diag {
                                stage: "type", file: self.cur_file,
                                line: pos.line,
                                col: pos.col,
                                message: format!("'%' requires two int operands, found ({lt}, {rt})"),
                            })
                        }
                    }
                }
            }
        }
    }

    /// unify arguments against the generic declaration, create/reuse the
    /// monomorphized instance, and route codegen to it via call_map
    fn instantiate_call(
        &mut self,
        name: &str,
        args: &[Arg],
        pos: Pos,
        call_expr: &Expr,
        scopes: &mut Vec<(String, Type)>,
    ) -> Result<Type, Diag> {
        let generic = self.generics[name].clone();
        if args.iter().any(|a| a.name.is_some()) {
            return Err(Diag {
                stage: "type", file: self.cur_file,
                line: pos.line,
                col: pos.col,
                message: format!("function '{name}' takes positional arguments only"),
            });
        }
        if args.len() != generic.params.len() {
            return Err(Diag {
                stage: "type", file: self.cur_file,
                line: pos.line,
                col: pos.col,
                message: format!(
                    "function '{name}' expects {} argument(s), found {}",
                    generic.params.len(),
                    args.len()
                ),
            });
        }
        // check arguments, then unify against declared param types
        let mut arg_types: Vec<Type> = Vec::with_capacity(args.len());
        for a in args {
            arg_types.push(self.check_expr(&a.value, scopes)?);
        }
        let mut subst_t: HashMap<String, Type> = HashMap::new();
        let mut n: Option<usize> = None;
        for (i, at) in arg_types.iter().enumerate() {
            if !unify(&generic.params[i].ty, at, &generic.type_params, &mut subst_t, &mut n) {
                return Err(Diag {
                    stage: "type", file: self.cur_file,
                    line: args[i].value.pos().line,
                    col: args[i].value.pos().col,
                    message: format!(
                        "argument {} of '{}' must be {}, found {}",
                        i + 1,
                        name,
                        generic.params[i].ty,
                        at
                    ),
                });
            }
        }
        let ret = subst_type(&generic.ret, &subst_t, n).map_err(|m| Diag {
            stage: "type", file: self.cur_file,
            line: pos.line,
            col: pos.col,
            message: m,
        })?;
        let key = mangle(name, &generic.type_params, &subst_t, n);
        self.call_map.insert(call_expr as *const Expr as usize, key.clone());
        if !self.done.contains(&key) {
            self.done.insert(key.clone());
            let mut inst = generic.clone();
            inst.name = key;
            inst.type_params = Vec::new();
            for p in &mut inst.params {
                p.ty = subst_type(&p.ty, &subst_t, n).map_err(|m| Diag {
                    stage: "type",
                    file: p.pos.file,
                    line: p.pos.line,
                    col: p.pos.col,
                    message: m,
                })?;
            }
            inst.ret = ret.clone();
            let lp = generic.len_param.clone();
            subst_block_types(&mut inst.body, &subst_t, n, lp.as_deref()).map_err(|m| Diag {
                stage: "type", file: self.cur_file,
                line: pos.line,
                col: pos.col,
                message: m,
            })?;
            self.instances.push(inst.clone());
            self.queue.push(inst);
        }
        Ok(ret)
    }

    #[allow(clippy::too_many_arguments)]
    fn check_struct_construction(
        &mut self,
        name: &str,
        layout: &[(String, Type)],
        args: &[Arg],
        pos: Pos,
        scopes: &mut Vec<(String, Type)>,
    ) -> Result<Type, Diag> {
        // all arguments must be named
        for a in args {
            if a.name.is_none() {
                return Err(Diag {
                    stage: "type", file: self.cur_file,
                    line: a.value.pos().line,
                    col: a.value.pos().col,
                    message: format!("struct '{name}' must be constructed with named fields: {name}(field=value, ...)"),
                });
            }
        }
        for a in args {
            let fname = a.name.as_ref().unwrap();
            let fty = layout
                .iter()
                .find(|(n, _)| n == fname)
                .map(|(_, t)| t.clone())
                .ok_or_else(|| Diag {
                    stage: "type", file: self.cur_file,
                    line: a.value.pos().line,
                    col: a.value.pos().col,
                    message: format!("struct '{name}' has no field '{fname}'"),
                })?;
            let t = self.check_expr(&a.value, scopes)?;
            if t != fty {
                return Err(Diag {
                    stage: "type", file: self.cur_file,
                    line: a.value.pos().line,
                    col: a.value.pos().col,
                    message: format!("field '{fname}' of '{name}' must be {fty}, found {t}"),
                });
            }
        }
        // duplicates
        for (i, a) in args.iter().enumerate() {
            let an = a.name.as_ref().unwrap();
            if args[..i].iter().any(|x| x.name.as_deref() == Some(an.as_str())) {
                return Err(Diag {
                    stage: "type", file: self.cur_file,
                    line: a.value.pos().line,
                    col: a.value.pos().col,
                    message: format!("field '{an}' given more than once in '{name}'"),
                });
            }
        }
        // completeness
        if args.len() != layout.len() {
            let missing: Vec<String> = layout
                .iter()
                .filter(|(n, _)| !args.iter().any(|a| a.name.as_deref() == Some(n.as_str())))
                .map(|(n, _)| n.clone())
                .collect();
            return Err(Diag {
                stage: "type", file: self.cur_file,
                line: pos.line,
                col: pos.col,
                message: format!(
                    "struct literal '{name}' is missing field(s): {}",
                    missing.join(", ")
                ),
            });
        }
        Ok(Type::Struct(name.to_string()))
    }
}

// ---- generic helpers ----

/// unify a declared (possibly generic) type against a concrete type;
/// T-params fill subst_t, the length param fills n. Returns false on mismatch.
fn unify(
    declared: &Type,
    actual: &Type,
    params: &[String],
    subst_t: &mut HashMap<String, Type>,
    n: &mut Option<usize>,
) -> bool {
    match declared {
        Type::Struct(d) if params.contains(d) => match subst_t.get(d) {
            Some(prev) => prev == actual,
            None => {
                subst_t.insert(d.clone(), actual.clone());
                true
            }
        },
        Type::Array { elem: de, len: GENERIC_LEN } => match actual {
            Type::Array { elem: ae, len: al } => {
                if let Some(prev) = *n {
                    if prev != *al {
                        return false;
                    }
                } else {
                    *n = Some(*al);
                }
                unify(de, ae, params, subst_t, n)
            }
            _ => false,
        },
        Type::Array { elem: de, len: dl } => match actual {
            Type::Array { elem: ae, len: al } if dl == al => unify(de, ae, params, subst_t, n),
            _ => false,
        },
        _ => declared == actual,
    }
}

/// substitute type params (T) and the length param (GENERIC_LEN sentinel)
fn subst_type(t: &Type, subst_t: &HashMap<String, Type>, n: Option<usize>) -> Result<Type, String> {
    match t {
        Type::Struct(d) => Ok(subst_t.get(d).cloned().unwrap_or_else(|| t.clone())),
        Type::Array { elem, len } => {
            let e = subst_type(elem, subst_t, n)?;
            let l = if *len == GENERIC_LEN {
                match n {
                    Some(v) => v,
                    None => return Err("cannot infer array length N (use it in a parameter)".into()),
                }
            } else {
                *len
            };
            Ok(Type::Array { elem: Box::new(e), len: l })
        }
        other => Ok(other.clone()),
    }
}

fn has_generic_len(t: &Type) -> bool {
    match t {
        Type::Array { elem, len } => *len == GENERIC_LEN || has_generic_len(elem),
        _ => false,
    }
}

fn subst_block_types(block: &mut Block, subst_t: &HashMap<String, Type>, n: Option<usize>, len_param: Option<&str>) -> Result<(), String> {
    for stmt in &mut block.stmts {
        subst_stmt_types(stmt, subst_t, n, len_param)?;
    }
    Ok(())
}

fn subst_stmt_types(stmt: &mut Stmt, subst_t: &HashMap<String, Type>, n: Option<usize>, len_param: Option<&str>) -> Result<(), String> {
    match stmt {
        Stmt::Let { ty: Some(t), expr, .. } => {
            *t = subst_type(t, subst_t, n)?;
            subst_expr(expr, n, len_param);
            Ok(())
        }
        Stmt::Let { expr, .. } => {
            subst_expr(expr, n, len_param);
            Ok(())
        }
        Stmt::Assign { target, expr, .. } => {
            subst_expr(target, n, len_param);
            subst_expr(expr, n, len_param);
            Ok(())
        }
        Stmt::If { cond, then_block, else_block, .. } => {
            subst_expr(cond, n, len_param);
            subst_block_types(then_block, subst_t, n, len_param)?;
            if let Some(eb) = else_block {
                subst_block_types(eb, subst_t, n, len_param)?;
            }
            Ok(())
        }
        Stmt::While { cond, body, .. } => {
            subst_expr(cond, n, len_param);
            subst_block_types(body, subst_t, n, len_param)
        }
        Stmt::For { iter, body, .. } => {
            match iter {
                ForIter::Range(args) => {
                    for a in args {
                        subst_expr(a, n, len_param);
                    }
                }
                ForIter::Array(e) => subst_expr(e, n, len_param),
            }
            subst_block_types(body, subst_t, n, len_param)
        }
        Stmt::Return { expr: Some(e), .. } => {
            subst_expr(e, n, len_param);
            Ok(())
        }
        Stmt::ExprStmt { expr } => {
            subst_expr(expr, n, len_param);
            Ok(())
        }
        _ => Ok(()),
    }
}

/// replace the length parameter with its concrete int constant in expressions
fn subst_expr(e: &mut Expr, n: Option<usize>, len_param: Option<&str>) {
    let (lp, v) = match (len_param, n) {
        (Some(lp), Some(v)) => (lp, v),
        _ => return,
    };
    match e {
        Expr::Var { name, .. } if name == lp => {
            *e = Expr::Int(v as i64, Pos { line: 0, col: 0, file: u32::MAX });
        }
        Expr::Unary { expr, .. } => subst_expr(expr, n, len_param),
        Expr::Binary { lhs, rhs, .. } => {
            subst_expr(lhs, n, len_param);
            subst_expr(rhs, n, len_param);
        }
        Expr::Index { arr, idx, .. } => {
            subst_expr(arr, n, len_param);
            subst_expr(idx, n, len_param);
        }
        Expr::Field { obj, .. } => subst_expr(obj, n, len_param),
        Expr::ArrayLit { elems, .. } => {
            for x in elems {
                subst_expr(x, n, len_param);
            }
        }
        Expr::ArrayRep { elem, .. } => subst_expr(elem, n, len_param),
        Expr::Call { args, .. } => {
            for a in args {
                subst_expr(&mut a.value, n, len_param);
            }
        }
        Expr::StructLit { fields, .. } => {
            for (_, fe) in fields {
                subst_expr(fe, n, len_param);
            }
        }
        _ => {}
    }
}

/// deterministic mangled name for an instance: `name.<slug>.<len>`
fn mangle(name: &str, params: &[String], subst_t: &HashMap<String, Type>, n: Option<usize>) -> String {
    let mut key = String::from(name);
    for p in params {
        match subst_t.get(p) {
            Some(t) => {
                key.push('.');
                key.push_str(&type_slug(t));
            }
            None => {
                // param unused in the body 鈥?still part of the key
                key.push_str(".?");
            }
        }
    }
    if let Some(len) = n {
        key.push('.');
        key.push_str(&len.to_string());
    }
    key
}

fn type_slug(t: &Type) -> String {
    match t {
        Type::Int => "i".into(),
        Type::Float => "f".into(),
        Type::Bool => "b".into(),
        Type::Str => "s".into(),
        Type::Void => "v".into(),
        Type::Array { elem, len } => format!("a{}x{}", type_slug(elem), len),
        Type::Struct(n) => format!("s_{n}"),
    }
}

// ---- struct table construction ----

fn collect_structs(decls: &[StructDecl]) -> Result<StructTable, Diag> {
    let mut table: StructTable = HashMap::new();

    // pass 1: reserve all names (allows forward references between structs)
    for s in decls {
        if table.contains_key(&s.name) {
            return Err(Diag {
                stage: "type", file: s.pos.file,
                line: s.pos.line,
                col: s.pos.col,
                message: format!("struct '{}' is defined more than once", s.name),
            });
        }
        table.insert(s.name.clone(), Vec::new());
    }

    // pass 2: resolve field types
    for s in decls {
        let mut fields: Vec<(String, Type)> = Vec::new();
        for (i, f) in s.fields.iter().enumerate() {
            if s.fields[..i].iter().any(|q| q.name == f.name) {
                return Err(Diag {
                    stage: "type",
                    file: f.pos.file,
                    line: f.pos.line,
                    col: f.pos.col,
                    message: format!("duplicate field '{}' in struct '{}'", f.name, s.name),
                });
            }
            resolve_ty(&f.ty, &table).map_err(|m| Diag {
                stage: "type",
                file: f.pos.file,
                line: f.pos.line,
                col: f.pos.col,
                message: m,
            })?;
            if f.ty == Type::Void {
                return Err(Diag {
                    stage: "type",
                    file: f.pos.file,
                    line: f.pos.line,
                    col: f.pos.col,
                    message: format!("field '{}' cannot have type void", f.name),
                });
            }
            fields.push((f.name.clone(), f.ty.clone()));
        }
        table.insert(s.name.clone(), fields);
    }

    // pass 3: reject recursive structs (they would have infinite size)
    for s in decls {
        let mut stack: Vec<String> = vec![s.name.clone()];
        let mut seen: Vec<String> = vec![s.name.clone()];
        while let Some(name) = stack.pop() {
            if let Some(fields) = table.get(&name) {
                for (_, t) in fields {
                    if let Type::Struct(inner) = t {
                        if seen.contains(inner) {
                            return Err(Diag {
                                stage: "type", file: s.pos.file,
                                line: s.pos.line,
                                col: s.pos.col,
                                message: format!(
                                    "recursive struct '{}' (a struct cannot contain itself, directly or indirectly)",
                                    s.name
                                ),
                            });
                        }
                        seen.push(inner.clone());
                        stack.push(inner.clone());
                    }
                }
            }
        }
    }

    Ok(table)
}

/// validate a type annotation: struct names must exist, arrays recurse
fn resolve_ty(ty: &Type, structs: &StructTable) -> Result<(), String> {
    match ty {
        Type::Int | Type::Float | Type::Bool | Type::Str | Type::Void => Ok(()),
        Type::Array { elem, .. } => resolve_ty(elem, structs),
        Type::Struct(name) => {
            if structs.contains_key(name) {
                Ok(())
            } else {
                Err(format!("unknown type '{name}'"))
            }
        }
    }
}

fn lookup(scopes: &[(String, Type)], name: &str) -> Option<Type> {
    scopes.iter().rev().find(|(n, _)| n == name).map(|(_, t)| t.clone())
}

/// True if control flow cannot continue past this statement.
fn stmt_guarantees_return(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Return { .. } => true,
        Stmt::If { then_block, else_block, .. } => {
            block_returns_all(then_block)
                && else_block.as_ref().map(|e| block_returns_all(e)).unwrap_or(false)
        }
        _ => false,
    }
}

fn block_returns_all(block: &Block) -> bool {
    block.stmts.iter().any(stmt_guarantees_return)
}

fn stmt_pos(s: &Stmt) -> Pos {
    match s {
        Stmt::Let { pos, .. }
        | Stmt::Assign { pos, .. }
        | Stmt::If { pos, .. }
        | Stmt::While { pos, .. }
        | Stmt::For { pos, .. }
        | Stmt::Break { pos }
        | Stmt::Continue { pos }
        | Stmt::Return { pos, .. } => *pos,
        Stmt::Pass => Pos { line: 0, col: 0, file: u32::MAX },
        Stmt::ExprStmt { expr } => expr.pos(),
    }
}

fn field_type(t: &Type, name: &str, structs: &StructTable) -> Option<Type> {
    if let Type::Struct(sname) = t {
        if let Some(fields) = structs.get(sname) {
            for (fname, fty) in fields {
                if fname == name {
                    return Some(fty.clone());
                }
            }
        }
    }
    None
}

fn op_str(op: BinOp) -> &'static str {
    use BinOp::*;
    match op {
        Add => "+",
        Sub => "-",
        Mul => "*",
        Div => "/",
        Mod => "%",
        Eq => "==",
        Ne => "!=",
        Lt => "<",
        Le => "<=",
        Gt => ">",
        Ge => ">=",
        And => "&&",
        Or => "||",
    }
}




