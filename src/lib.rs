//! Axon compiler pipeline: lex -> parse -> typecheck -> codegen -> native object -> link.

pub mod ast;
pub mod codegen;
pub mod files;
pub mod lexer;
pub mod llvm;
pub mod parser;
pub mod typecheck;

use crate::ast::{FnDecl, Program, StructDecl};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Diag {
    pub stage: &'static str, // "lex" | "parse" | "type" | "internal" | "link" | "io"
    pub file: u32,           // index into the compilation file registry
    pub line: usize,
    pub col: usize,
    pub message: String,
}

impl Diag {
    fn internal(message: impl Into<String>) -> Diag {
        Diag { stage: "internal", file: u32::MAX, line: 0, col: 0, message: message.into() }
    }

    fn to_json(&self) -> String {
        let esc = self.message.replace('\\', "\\\\").replace('"', "\\\"");
        format!(
            "{{\"stage\":\"{}\",\"file\":\"{}\",\"line\":{},\"col\":{},\"message\":\"{}\"}}",
            self.stage,
            files::name(self.file),
            self.line,
            self.col,
            esc
        )
    }
}

pub fn diags_to_json(diags: &[Diag]) -> String {
    let items: Vec<String> = diags.iter().map(|d| d.to_json()).collect();
    format!("{{\"ok\":false,\"errors\":[{}]}}", items.join(","))
}

/// human-readable one-line form used by the CLI
pub fn diag_to_string(d: &Diag) -> String {
    let file = files::name(d.file);
    let loc = if d.line > 0 {
        format!("{file}:{}:{}: ", d.line, d.col)
    } else if file != "?" {
        format!("{file}: ")
    } else {
        String::new()
    };
    format!("[{}] {}{}", d.stage, loc, d.message)
}

/// Full compile pipeline from Axon source text to a native object file.
pub fn compile_to_object(src: &str, obj_path: &Path, opt: bool) -> Result<(), Vec<Diag>> {
    compile_sources_to_object(&[src.to_string()], obj_path, opt)
}

/// Compile multiple Axon sources as one program (merged namespace).
pub fn compile_sources_to_object(sources: &[String], obj_path: &Path, opt: bool) -> Result<(), Vec<Diag>> {
    let program = parse_sources(sources)?;
    finish_to_object(program, obj_path, opt)
}

/// Compile from file paths, resolving `import "..."` recursively
/// (include-once per canonical path, circular imports rejected).
pub fn compile_paths_to_object(paths: &[String], obj_path: &Path, opt: bool) -> Result<(), Vec<Diag>> {
    let program = load_program(paths)?;
    finish_to_object(program, obj_path, opt)
}

/// Full compile pipeline from Axon source text to LLVM IR text (for `axon ir`).
pub fn compile_to_ir(src: &str, opt: bool) -> Result<String, Vec<Diag>> {
    compile_sources_to_ir(&[src.to_string()], opt)
}

/// Multiple sources 鈫?LLVM IR text.
pub fn compile_sources_to_ir(sources: &[String], opt: bool) -> Result<String, Vec<Diag>> {
    let program = parse_sources(sources)?;
    finish_to_ir(program, opt)
}

/// File paths (with imports) 鈫?LLVM IR text.
pub fn compile_paths_to_ir(paths: &[String], opt: bool) -> Result<String, Vec<Diag>> {
    let program = load_program(paths)?;
    finish_to_ir(program, opt)
}

fn finish_to_object(program: Program, obj_path: &Path, opt: bool) -> Result<(), Vec<Diag>> {
    let out = typecheck::check(&program).map_err(|d| vec![d])?;
    let mut program = program;
    // only concrete functions reach codegen: drop generic declarations,
    // append their monomorphized instances
    program.funcs.retain(|f| f.type_params.is_empty());
    program.funcs.extend(out.instances);
    codegen::generate_to_object(&program, obj_path, opt, &out.call_map).map_err(|m| vec![Diag::internal(m)])?;
    Ok(())
}

fn finish_to_ir(program: Program, opt: bool) -> Result<String, Vec<Diag>> {
    let out = typecheck::check(&program).map_err(|d| vec![d])?;
    let mut program = program;
    program.funcs.retain(|f| f.type_params.is_empty());
    program.funcs.extend(out.instances);
    codegen::generate_ir_text(&program, opt, &out.call_map).map_err(|m| vec![Diag::internal(m)])
}

/// source-code based entry (no import resolution; imports are an error)
fn parse_sources(sources: &[String]) -> Result<Program, Vec<Diag>> {
    files::clear();
    let mut imports = Vec::new();
    let mut structs = Vec::new();
    let mut funcs = Vec::new();
    for src in sources {
        let file_id = files::register("<source>");
        let tokens = lexer::lex(src, file_id).map_err(|d| vec![d])?;
        let program = parser::parse(tokens).map_err(|d| vec![d])?;
        imports.extend(program.imports);
        structs.extend(program.structs);
        funcs.extend(program.funcs);
    }
    if let Some(imp) = imports.first() {
        return Err(vec![Diag {
            stage: "io",
            file: imp.pos.file,
            line: imp.pos.line,
            col: imp.pos.col,
            message: format!(
                "import \"{}\" requires compiling from files (imports resolve relative to the importing file)",
                imp.path
            ),
        }]);
    }
    Ok(Program { imports: vec![], structs, funcs })
}

/// file-path based entry: resolve imports recursively
fn load_program(entries: &[String]) -> Result<Program, Vec<Diag>> {
    files::clear();
    let mut state = LoadState {
        visited: HashSet::new(),
        stack: Vec::new(),
        structs: Vec::new(),
        funcs: Vec::new(),
    };
    for entry in entries {
        load_file(Path::new(entry), &mut state)?;
    }
    Ok(Program { imports: vec![], structs: state.structs, funcs: state.funcs })
}

struct LoadState {
    visited: HashSet<PathBuf>,
    stack: Vec<PathBuf>,
    structs: Vec<StructDecl>,
    funcs: Vec<FnDecl>,
}

fn load_file(path: &Path, state: &mut LoadState) -> Result<(), Vec<Diag>> {
    let canonical = std::fs::canonicalize(path).map_err(|e| {
        vec![Diag {
            stage: "io",
            file: u32::MAX,
            line: 0,
            col: 0,
            message: format!("cannot open '{}': {e}", path.display()),
        }]
    })?;
    if state.stack.contains(&canonical) {
        let cycle: Vec<String> = state
            .stack
            .iter()
            .chain(std::iter::once(&canonical))
            .map(|p| p.display().to_string())
            .collect();
        return Err(vec![Diag {
            stage: "io",
            file: u32::MAX,
            line: 0,
            col: 0,
            message: format!("circular import: {}", cycle.join(" -> ")),
        }]);
    }
    if state.visited.contains(&canonical) {
        return Ok(()); // include-once
    }
    state.visited.insert(canonical.clone());
    state.stack.push(canonical.clone());

    let src = std::fs::read_to_string(path).map_err(|e| {
        vec![Diag {
            stage: "io",
            file: u32::MAX,
            line: 0,
            col: 0,
            message: format!("cannot read '{}': {e}", path.display()),
        }]
    })?;
    let file_id = files::register(path.display().to_string());
    let tokens = lexer::lex(&src, file_id).map_err(|d| vec![d])?;
    let program = parser::parse(tokens).map_err(|d| vec![d])?;

    // resolve this file's imports relative to its own directory
    let dir = canonical.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    for imp in &program.imports {
        let imp_path = resolve_import(&dir, &imp.path);
        load_file(&imp_path, state).map_err(|diags| {
            // attach the import site to resolution errors that lack one
            let out: Vec<Diag> = diags
                .into_iter()
                .map(|mut d| {
                    if d.file == u32::MAX && d.line == 0 {
                        d.file = imp.pos.file;
                        d.line = imp.pos.line;
                        d.col = imp.pos.col;
                    }
                    d
                })
                .collect();
            out
        })?;
    }

    state.structs.extend(program.structs);
    state.funcs.extend(program.funcs);
    state.stack.pop();
    Ok(())
}

fn resolve_import(dir: &Path, import: &str) -> PathBuf {
    let p = Path::new(import);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        dir.join(p)
    }
}

/// Locate the clang driver used for final linking.
/// Order: AXON_CLANG env -> PATH -> repo-local LLVM -> standard install dir.
pub fn find_clang() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("AXON_CLANG") {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Some(path);
        }
    }
    let names: Vec<&str> = if cfg!(windows) { vec!["clang.exe", "clang"] } else { vec!["clang"] };
    for name in names {
        if let Some(path) = which(name) {
            return Some(path);
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    // compile-time repo-local toolchain layout: <repo>/LLVM/bin/clang.exe
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("LLVM").join("bin").join("clang.exe"));
    candidates.push(PathBuf::from(r"C:\Program Files\LLVM\bin\clang.exe"));
    candidates.into_iter().find(|p| p.is_file())
}

fn which(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    let sep = if cfg!(windows) { ';' } else { ':' };
    for dir in path_var.split(sep) {
        if dir.is_empty() {
            continue;
        }
        let candidate = PathBuf::from(dir).join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Compile Axon source files to an executable at `exe_path` (links via clang).
pub fn build_exe(src: &str, exe_path: &Path, opt: bool) -> Result<(), Vec<Diag>> {
    build_sources_exe(&[src.to_string()], exe_path, opt)
}

/// Compile multiple sources (merged namespace) into an executable.
pub fn build_sources_exe(sources: &[String], exe_path: &Path, opt: bool) -> Result<(), Vec<Diag>> {
    let obj_path = exe_path.with_extension("obj");
    compile_sources_to_object(sources, &obj_path, opt)?;
    let link_result = link(&obj_path, exe_path);
    if link_result.is_ok() {
        let _ = std::fs::remove_file(&obj_path);
    }
    link_result
}

/// Compile from file paths (with import resolution) into an executable.
pub fn build_paths_exe(paths: &[String], exe_path: &Path, opt: bool) -> Result<(), Vec<Diag>> {
    let obj_path = exe_path.with_extension("obj");
    compile_paths_to_object(paths, &obj_path, opt)?;
    let link_result = link(&obj_path, exe_path);
    if link_result.is_ok() {
        let _ = std::fs::remove_file(&obj_path);
    }
    link_result
}

/// Link a native object file into an executable using clang.
pub fn link(obj_path: &Path, exe_path: &Path) -> Result<(), Vec<Diag>> {
    let clang = find_clang().ok_or_else(|| vec![Diag {
        stage: "link",
        file: u32::MAX,
        line: 0,
        col: 0,
        message: "cannot find clang for linking. Set AXON_CLANG to the clang executable \
                  or add LLVM's bin directory to PATH."
            .into(),
    }])?;

    let status = Command::new(&clang)
        .arg(obj_path)
        .arg("-o")
        .arg(exe_path)
        // 8 MB stack: large fixed-size arrays live on the stack (allocas)
        .arg("-Wl,/STACK:8388608")
        .status()
        .map_err(|e| vec![Diag {
            stage: "link",
            file: u32::MAX,
            line: 0,
            col: 0,
            message: format!("failed to spawn {}: {e}", clang.display()),
        }])?;

    if !status.success() {
        return Err(vec![Diag {
            stage: "link",
            file: u32::MAX,
            line: 0,
            col: 0,
            message: format!("clang linking failed with exit code {:?}", status.code()),
        }]);
    }
    Ok(())
}

