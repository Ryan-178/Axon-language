//! Axon compiler pipeline: lex -> parse -> typecheck -> codegen -> native object -> link.

pub mod ast;
pub mod codegen;
pub mod lexer;
pub mod llvm;
pub mod parser;
pub mod typecheck;

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Diag {
    pub stage: &'static str, // "lex" | "parse" | "type" | "internal" | "link" | "io"
    pub line: usize,
    pub col: usize,
    pub message: String,
}

impl Diag {
    fn internal(message: impl Into<String>) -> Diag {
        Diag { stage: "internal", line: 0, col: 0, message: message.into() }
    }

    fn to_json(&self) -> String {
        let esc = self.message.replace('\\', "\\\\").replace('"', "\\\"");
        format!("{{\"stage\":\"{}\",\"line\":{},\"col\":{},\"message\":\"{}\"}}", self.stage, self.line, self.col, esc)
    }
}

pub fn diags_to_json(diags: &[Diag]) -> String {
    let items: Vec<String> = diags.iter().map(|d| d.to_json()).collect();
    format!("{{\"ok\":false,\"errors\":[{}]}}", items.join(","))
}

/// Full compile pipeline from Axon source text to a native object file.
pub fn compile_to_object(src: &str, obj_path: &Path, opt: bool) -> Result<(), Vec<Diag>> {
    let tokens = lexer::lex(src).map_err(|d| vec![d])?;
    let program = parser::parse(tokens).map_err(|d| vec![d])?;
    typecheck::check(&program).map_err(|d| vec![d])?;
    codegen::generate_to_object(&program, obj_path, opt).map_err(|m| vec![Diag::internal(m)])?;
    Ok(())
}

/// Full compile pipeline from Axon source text to LLVM IR text (for `axon ir`).
pub fn compile_to_ir(src: &str, opt: bool) -> Result<String, Vec<Diag>> {
    let tokens = lexer::lex(src).map_err(|d| vec![d])?;
    let program = parser::parse(tokens).map_err(|d| vec![d])?;
    typecheck::check(&program).map_err(|d| vec![d])?;
    codegen::generate_ir_text(&program, opt).map_err(|m| vec![Diag::internal(m)])
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

/// Compile Axon source to an executable at `exe_path` (links via clang).
pub fn build_exe(src: &str, exe_path: &Path, opt: bool) -> Result<(), Vec<Diag>> {
    let obj_path = exe_path.with_extension("obj");
    compile_to_object(src, &obj_path, opt)?;
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
            line: 0,
            col: 0,
            message: format!("failed to spawn {}: {e}", clang.display()),
        }])?;

    if !status.success() {
        return Err(vec![Diag {
            stage: "link",
            line: 0,
            col: 0,
            message: format!("clang linking failed with exit code {:?}", status.code()),
        }]);
    }
    Ok(())
}
