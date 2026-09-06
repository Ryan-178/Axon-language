//! Axon type checker: strict, no implicit conversions, deterministic errors.

use std::collections::HashMap;

use crate::ast::*;
use crate::Diag;

/// resolved struct layout: field name -> (index, type)
pub type StructTable = HashMap<String, Vec<(String, Type)>>;

pub struct FnSig {
    pub params: Vec<Type>,
    pub ret: Type,
}

pub fn check(program: &Program) -> Result<(StructTable, HashMap<String, FnSig>), Diag> {
    let structs = collect_structs(&program.structs)?;

    let mut sigs: HashMap<String, FnSig> = HashMap::new();

    // collect signatures first (allows mutual recursion)
    for f in &program.funcs {
        if sigs.contains_key(&f.name) {
            return Err(Diag {
                stage: "type",
                line: f.pos.line,
                col: f.pos.col,
                message: format!("function '{}' is defined more than once", f.name),
            });
        }
        if structs.contains_key(&f.name) {
            return Err(Diag {
                stage: "type",
                line: f.pos.line,
                col: f.pos.col,
                message: format!("'{}' is already defined as a struct", f.name),
            });
        }
        for (i, p) in f.params.iter().enumerate() {
            if f.params[..i].iter().any(|q| q.name == p.name) {
                return Err(Diag {
                    stage: "type",
                    line: p.pos.line,
                    col: p.pos.col,
                    message: format!("duplicate parameter '{}' in function '{}'", p.name, f.name),
                });
            }
            resolve_ty(&p.ty, &structs).map_err(|m| Diag {
                stage: "type",
                line: p.pos.line,
                col: p.pos.col,
                message: m,
            })?;
            if p.ty == Type::Void {
                return Err(Diag {
                    stage: "type",
                    line: p.pos.line,
                    col: p.pos.col,
                    message: format!("parameter '{}' cannot have type void", p.name),
                });
            }
        }
        resolve_ty(&f.ret, &structs).map_err(|m| Diag {
            stage: "type",
            line: f.pos.line,
            col: f.pos.col,
            message: m,
        })?;
        sigs.insert(
            f.name.clone(),
            FnSig {
                params: f.params.iter().map(|p| p.ty.clone()).collect(),
                ret: f.ret.clone(),
            },
        );
    }

    if !sigs.contains_key("main") {
        return Err(Diag {
            stage: "type",
            line: 1,
            col: 1,
            message: "program has no 'main' function".into(),
        });
    }

    for f in &program.funcs {
        let mut scopes: Vec<(String, Type)> =
            f.params.iter().map(|p| (p.name.clone(), p.ty.clone())).collect();
        let returns_all = check_block(&f.body, f, &sigs, &structs, &mut scopes)?;
        // strict rule: non-void functions must return a value on every path
        if f.ret != Type::Void && !returns_all {
            return Err(Diag {
                stage: "type",
                line: f.pos.line,
                col: f.pos.col,
                message: format!(
                    "function '{}' returns {} but does not return a value on all paths",
                    f.name, f.ret
                ),
            });
        }
    }

    Ok((structs, sigs))
}

// ---- struct table construction ----

fn collect_structs(decls: &[StructDecl]) -> Result<StructTable, Diag> {
    let mut table: StructTable = HashMap::new();

    // pass 1: reserve all names (allows forward references between structs)
    for s in decls {
        if table.contains_key(&s.name) {
            return Err(Diag {
                stage: "type",
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
                    line: f.pos.line,
                    col: f.pos.col,
                    message: format!("duplicate field '{}' in struct '{}'", f.name, s.name),
                });
            }
            resolve_ty(&f.ty, &table).map_err(|m| Diag {
                stage: "type",
                line: f.pos.line,
                col: f.pos.col,
                message: m,
            })?;
            if f.ty == Type::Void {
                return Err(Diag {
                    stage: "type",
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
                                stage: "type",
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

fn check_block(
    block: &Block,
    f: &FnDecl,
    sigs: &HashMap<String, FnSig>,
    structs: &StructTable,
    scopes: &mut Vec<(String, Type)>,
) -> Result<bool, Diag> {
    let mut guarantees_return = false;
    for stmt in &block.stmts {
        if guarantees_return {
            let pos = stmt_pos(stmt);
            return Err(Diag {
                stage: "type",
                line: pos.line,
                col: pos.col,
                message: "unreachable statement after 'return'".into(),
            });
        }
        check_stmt(stmt, f, sigs, structs, scopes)?;
        guarantees_return = stmt_guarantees_return(stmt);
    }
    Ok(guarantees_return)
}

fn stmt_pos(s: &Stmt) -> Pos {
    match s {
        Stmt::Let { pos, .. }
        | Stmt::Assign { pos, .. }
        | Stmt::If { pos, .. }
        | Stmt::While { pos, .. }
        | Stmt::Return { pos, .. } => *pos,
        Stmt::Pass => Pos { line: 0, col: 0 },
        Stmt::ExprStmt { expr } => expr.pos(),
    }
}

fn check_stmt(
    stmt: &Stmt,
    f: &FnDecl,
    sigs: &HashMap<String, FnSig>,
    structs: &StructTable,
    scopes: &mut Vec<(String, Type)>,
) -> Result<(), Diag> {
    match stmt {
        Stmt::Let { name, ty, expr, pos } => {
            let t = check_expr(expr, sigs, structs, scopes)?;
            if t == Type::Void {
                return Err(Diag {
                    stage: "type",
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
                                stage: "type",
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
                            stage: "type",
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
                                stage: "type",
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
                    stage: "type",
                    line: pos.line,
                    col: pos.col,
                    message: "invalid assignment target".into(),
                });
            }
            let dt = lvalue_type(target, sigs, structs, scopes)?;
            let t = check_expr(expr, sigs, structs, scopes)?;
            if t != dt {
                return Err(Diag {
                    stage: "type",
                    line: pos.line,
                    col: pos.col,
                    message: format!("cannot assign a value of type {t} to a target of type {dt}"),
                });
            }
            Ok(())
        }
        Stmt::If { cond, then_block, else_block, pos } => {
            let t = check_expr(cond, sigs, structs, scopes)?;
            if t != Type::Bool {
                return Err(Diag {
                    stage: "type",
                    line: pos.line,
                    col: pos.col,
                    message: format!("'if' condition must be bool, found {t}"),
                });
            }
            check_block(then_block, f, sigs, structs, scopes)?;
            if let Some(eb) = else_block {
                check_block(eb, f, sigs, structs, scopes)?;
            }
            Ok(())
        }
        Stmt::While { cond, body, pos } => {
            let t = check_expr(cond, sigs, structs, scopes)?;
            if t != Type::Bool {
                return Err(Diag {
                    stage: "type",
                    line: pos.line,
                    col: pos.col,
                    message: format!("'while' condition must be bool, found {t}"),
                });
            }
            check_block(body, f, sigs, structs, scopes)?;
            Ok(())
        }
        Stmt::Return { expr, pos } => {
            match (expr, &f.ret) {
                (None, Type::Void) => Ok(()),
                (None, ret) => Err(Diag {
                    stage: "type",
                    line: pos.line,
                    col: pos.col,
                    message: format!("'return' must return a value of type {ret}"),
                }),
                (Some(_), Type::Void) => Err(Diag {
                    stage: "type",
                    line: pos.line,
                    col: pos.col,
                    message: "void function cannot return a value".into(),
                }),
                (Some(e), ret) => {
                    let t = check_expr(e, sigs, structs, scopes)?;
                    if t != *ret {
                        return Err(Diag {
                            stage: "type",
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
            check_expr(expr, sigs, structs, scopes)?;
            Ok(())
        }
    }
}

/// type of an assignment target (already validated as lvalue by the parser)
fn lvalue_type(
    target: &Expr,
    sigs: &HashMap<String, FnSig>,
    structs: &StructTable,
    scopes: &mut Vec<(String, Type)>,
) -> Result<Type, Diag> {
    match target {
        Expr::Var { name, pos } => lookup(scopes, name).ok_or_else(|| Diag {
            stage: "type",
            line: pos.line,
            col: pos.col,
            message: format!("assignment to undeclared variable '{name}'"),
        }),
        Expr::Index { arr, idx, pos } => {
            let at = check_expr(arr, sigs, structs, scopes)?;
            let it = check_expr(idx, sigs, structs, scopes)?;
            if it != Type::Int {
                return Err(Diag {
                    stage: "type",
                    line: pos.line,
                    col: pos.col,
                    message: format!("array index must be int, found {it}"),
                });
            }
            match at {
                Type::Array { elem, .. } => Ok(*elem),
                other => Err(Diag {
                    stage: "type",
                    line: pos.line,
                    col: pos.col,
                    message: format!("cannot index a value of type {other}"),
                }),
            }
        }
        Expr::Field { obj, name, pos } => {
            let ot = check_expr(obj, sigs, structs, scopes)?;
            field_type(&ot, name, structs).ok_or_else(|| Diag {
                stage: "type",
                line: pos.line,
                col: pos.col,
                message: format!("type {ot} has no field '{name}'"),
            })
        }
        other => {
            let pos = other.pos();
            Err(Diag {
                stage: "type",
                line: pos.line,
                col: pos.col,
                message: "invalid assignment target".into(),
            })
        }
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

fn check_expr(
    expr: &Expr,
    sigs: &HashMap<String, FnSig>,
    structs: &StructTable,
    scopes: &mut Vec<(String, Type)>,
) -> Result<Type, Diag> {
    match expr {
        Expr::Int(..) => Ok(Type::Int),
        Expr::Float(..) => Ok(Type::Float),
        Expr::Str(..) => Ok(Type::Str),
        Expr::Bool(..) => Ok(Type::Bool),
        Expr::Var { name, pos } => lookup(scopes, name).ok_or_else(|| Diag {
            stage: "type",
            line: pos.line,
            col: pos.col,
            message: format!("unknown variable '{name}'"),
        }),
        Expr::Index { arr, idx, pos } => {
            let at = check_expr(arr, sigs, structs, scopes)?;
            let it = check_expr(idx, sigs, structs, scopes)?;
            if it != Type::Int {
                return Err(Diag {
                    stage: "type",
                    line: pos.line,
                    col: pos.col,
                    message: format!("array index must be int, found {it}"),
                });
            }
            match at {
                Type::Array { elem, .. } => Ok(*elem),
                other => Err(Diag {
                    stage: "type",
                    line: pos.line,
                    col: pos.col,
                    message: format!("cannot index a value of type {other} (only [T; N] arrays are indexable)"),
                }),
            }
        }
        Expr::Field { obj, name, pos } => {
            let ot = check_expr(obj, sigs, structs, scopes)?;
            field_type(&ot, name, structs).ok_or_else(|| Diag {
                stage: "type",
                line: pos.line,
                col: pos.col,
                message: format!("type {ot} has no field '{name}'"),
            })
        }
        Expr::ArrayLit { elems, pos, .. } => {
            if elems.is_empty() {
                return Err(Diag {
                    stage: "type",
                    line: pos.line,
                    col: pos.col,
                    message: "empty array literals are not allowed".into(),
                });
            }
            let elem = check_expr(&elems[0], sigs, structs, scopes)?;
            for e in &elems[1..] {
                let t = check_expr(e, sigs, structs, scopes)?;
                if t != elem {
                    return Err(Diag {
                        stage: "type",
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
                    stage: "type",
                    line: pos.line,
                    col: pos.col,
                    message: "array replication count must be positive".into(),
                });
            }
            let t = check_expr(elem, sigs, structs, scopes)?;
            Ok(Type::Array { elem: Box::new(t), len: *count })
        }
        Expr::StructLit { name, fields, pos, .. } => {
            let layout = structs.get(name).ok_or_else(|| Diag {
                stage: "type",
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
                        stage: "type",
                        line: fexpr.pos().line,
                        col: fexpr.pos().col,
                        message: format!("struct '{name}' has no field '{fname}'"),
                    })?;
                let t = check_expr(fexpr, sigs, structs, scopes)?;
                if t != fty {
                    return Err(Diag {
                        stage: "type",
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
                    .filter(|(n, _)| !fields.iter().any(|f| &f.0 == n))
                    .map(|(n, _)| n.clone())
                    .collect();
                return Err(Diag {
                    stage: "type",
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
                        stage: "type",
                        line: fpos.line,
                        col: fpos.col,
                        message: format!("field '{fname}' given more than once in struct literal"),
                    });
                }
            }
            Ok(Type::Struct(name.clone()))
        }
        Expr::Call { name, args, pos, lit_id: _ } => {
            // builtins first (they are not in the function table)
            if name == "print" {
                if args.len() != 1 || args[0].name.is_some() {
                    return Err(Diag {
                        stage: "type",
                        line: pos.line,
                        col: pos.col,
                        message: "print expects exactly 1 positional argument".into(),
                    });
                }
                let t = check_expr(&args[0].value, sigs, structs, scopes)?;
                if !t.is_printable() {
                    return Err(Diag {
                        stage: "type",
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
                        stage: "type",
                        line: pos.line,
                        col: pos.col,
                        message: "len expects exactly 1 positional argument".into(),
                    });
                }
                let t = check_expr(&args[0].value, sigs, structs, scopes)?;
                if !matches!(t, Type::Array { .. } | Type::Str) {
                    return Err(Diag {
                        stage: "type",
                        line: pos.line,
                        col: pos.col,
                        message: format!("len requires an array or string, found {t}"),
                    });
                }
                return Ok(Type::Int);
            }
            // struct construction: Name(field=value, ...)
            if sigs.get(name).is_none() {
                if let Some(layout) = structs.get(name) {
                    return check_struct_construction(name, layout, args, *pos, sigs, structs, scopes);
                }
                return Err(Diag {
                    stage: "type",
                    line: pos.line,
                    col: pos.col,
                    message: format!("call to undefined function or struct '{name}'"),
                });
            }
            let sig = &sigs[name];
            if args.iter().any(|a| a.name.is_some()) {
                return Err(Diag {
                    stage: "type",
                    line: pos.line,
                    col: pos.col,
                    message: format!("function '{}' takes positional arguments only", name),
                });
            }
            if args.len() != sig.params.len() {
                return Err(Diag {
                    stage: "type",
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
                let t = check_expr(&a.value, sigs, structs, scopes)?;
                if t != sig.params[i] {
                    return Err(Diag {
                        stage: "type",
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
            let t = check_expr(expr, sigs, structs, scopes)?;
            match (op, t) {
                (UnOp::Not, Type::Bool) => Ok(Type::Bool),
                (UnOp::Not, other) => Err(Diag {
                    stage: "type",
                    line: pos.line,
                    col: pos.col,
                    message: format!("'!' requires bool, found {other}"),
                }),
                (UnOp::Neg, Type::Int) => Ok(Type::Int),
                (UnOp::Neg, Type::Float) => Ok(Type::Float),
                (UnOp::Neg, other) => Err(Diag {
                    stage: "type",
                    line: pos.line,
                    col: pos.col,
                    message: format!("unary '-' requires int or float, found {other}"),
                }),
            }
        }
        Expr::Binary { op, lhs, rhs, pos } => {
            let lt = check_expr(lhs, sigs, structs, scopes)?;
            let rt = check_expr(rhs, sigs, structs, scopes)?;
            use BinOp::*;
            match op {
                And | Or => {
                    if lt == Type::Bool && rt == Type::Bool {
                        Ok(Type::Bool)
                    } else {
                        Err(Diag {
                            stage: "type",
                            line: pos.line,
                            col: pos.col,
                            message: format!("'{}' requires bool operands, found ({lt}, {rt})", op_str(*op)),
                        })
                    }
                }
                Eq | Ne => {
                    if lt.is_compound() || rt.is_compound() {
                        Err(Diag {
                            stage: "type",
                            line: pos.line,
                            col: pos.col,
                            message: format!("cannot compare compound type {lt} with {rt}"),
                        })
                    } else if lt == rt && lt != Type::Void {
                        Ok(Type::Bool)
                    } else {
                        Err(Diag {
                            stage: "type",
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
                                stage: "type",
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
                                stage: "type",
                                line: pos.line,
                                col: pos.col,
                                message: format!("cannot concatenate string with {rt}"),
                            })
                        }
                    } else if (lt == Type::Int || lt == Type::Float) && lt == rt {
                        Ok(lt)
                    } else {
                        Err(Diag {
                            stage: "type",
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
                            stage: "type",
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
                            stage: "type",
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

#[allow(clippy::too_many_arguments)]
fn check_struct_construction(
    name: &str,
    layout: &[(String, Type)],
    args: &[Arg],
    pos: Pos,
    sigs: &HashMap<String, FnSig>,
    structs: &StructTable,
    scopes: &mut Vec<(String, Type)>,
) -> Result<Type, Diag> {
    // all arguments must be named
    for a in args {
        if a.name.is_none() {
            return Err(Diag {
                stage: "type",
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
                stage: "type",
                line: a.value.pos().line,
                col: a.value.pos().col,
                message: format!("struct '{name}' has no field '{fname}'"),
            })?;
        let t = check_expr(&a.value, sigs, structs, scopes)?;
        if t != fty {
            return Err(Diag {
                stage: "type",
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
                stage: "type",
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
            stage: "type",
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
