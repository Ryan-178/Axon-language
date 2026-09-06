//! Axon compiler driver.
//!
//! Usage:
//!   axon build <file.ax> [-o out.exe] [--O0] [--json]
//!   axon run   <file.ax> [args...] [--O0] [--json]
//!   axon ir    <file.ax> [--O0] [--json]
//!   axon --help

use std::path::PathBuf;
use std::process::Command;

use axon::Diag;

struct Opts {
    out: Option<String>,
    o0: bool,
    json: bool,
    positional: Vec<String>,
    // everything after "--" (used by `run` to pass args to the compiled program)
    passthrough: Vec<String>,
}

fn parse_opts(args: &[String]) -> Opts {
    let mut opts = Opts { out: None, o0: false, json: false, positional: Vec::new(), passthrough: Vec::new() };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" if i + 1 < args.len() => {
                opts.out = Some(args[i + 1].clone());
                i += 2;
            }
            "--O0" => {
                opts.o0 = true;
                i += 1;
            }
            "--json" => {
                opts.json = true;
                i += 1;
            }
            "--" => {
                opts.passthrough = args[i + 1..].to_vec();
                break;
            }
            other => {
                opts.positional.push(other.to_string());
                i += 1;
            }
        }
    }
    opts
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print_help();
        std::process::exit(2);
    }
    match args[0].as_str() {
        "--help" | "-h" | "help" => print_help(),
        "build" => cmd_build(&args[1..]),
        "run" => cmd_run(&args[1..]),
        "ir" => cmd_ir(&args[1..]),
        other => {
            eprintln!("error: unknown command '{other}' (try: axon --help)");
            std::process::exit(2);
        }
    }
}

fn print_help() {
    println!(
        "Axon compiler v{}\n\n\
         USAGE:\n  \
         axon build <file.ax> [-o out] [--O0] [--json]   compile to a native executable\n  \
         axon run <file.ax> [--O0] [--json] [-- args...]  compile and run in one step\n  \
         axon ir <file.ax> [--O0] [--json]               print the LLVM IR\n\n\
         FLAGS:\n  \
         -o <path>   output executable path (default: <file>.exe)\n  \
         --O0        disable optimizations (default: O3)\n  \
         --json      emit diagnostics as JSON (AI-agent friendly)",
        env!("CARGO_PKG_VERSION")
    );
}

fn report(diags: &[Diag], json: bool) {
    if json {
        eprintln!("{}", axon::diags_to_json(diags));
    } else {
        for d in diags {
            if d.line > 0 {
                eprintln!("[{}] {}:{}: {}", d.stage, d.line, d.col, d.message);
            } else {
                eprintln!("[{}] {}", d.stage, d.message);
            }
        }
    }
}

fn read_source(path: &str, json: bool) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Some(s),
        Err(e) => {
            let d = vec![Diag { stage: "io", line: 0, col: 0, message: format!("cannot read '{path}': {e}") }];
            report(&d, json);
            None
        }
    }
}

fn cmd_build(args: &[String]) {
    let opts = parse_opts(args);
    let Some(input) = opts.positional.first() else {
        eprintln!("error: 'axon build' needs an input file (.ax)");
        std::process::exit(2);
    };
    let Some(src) = read_source(input, opts.json) else { std::process::exit(1) };
    let exe = opts.out.map(PathBuf::from).unwrap_or_else(|| default_exe(input));
    match axon::build_exe(&src, &exe, !opts.o0) {
        Ok(()) => println!("{}", exe.display()),
        Err(diags) => {
            report(&diags, opts.json);
            std::process::exit(1);
        }
    }
}

fn cmd_run(args: &[String]) {
    let opts = parse_opts(args);
    let Some(input) = opts.positional.first() else {
        eprintln!("error: 'axon run' needs an input file (.ax)");
        std::process::exit(2);
    };
    // program args: anything after "--", or extra positionals after the input file
    let prog_args: Vec<String> = if !opts.passthrough.is_empty() || args.contains(&"--".to_string()) {
        opts.passthrough.clone()
    } else {
        opts.positional[1..].to_vec()
    };
    let Some(src) = read_source(input, opts.json) else { std::process::exit(1) };
    let exe = temp_exe(input);
    if let Err(diags) = axon::build_exe(&src, &exe, !opts.o0) {
        report(&diags, opts.json);
        std::process::exit(1);
    }
    let status = Command::new(&exe)
        .args(prog_args)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("error: failed to run compiled program: {e}");
            std::process::exit(1);
        });
    let _ = std::fs::remove_file(&exe);
    std::process::exit(status.code().unwrap_or(1));
}

fn cmd_ir(args: &[String]) {
    let opts = parse_opts(args);
    let Some(input) = opts.positional.first() else {
        eprintln!("error: 'axon ir' needs an input file (.ax)");
        std::process::exit(2);
    };
    let Some(src) = read_source(input, opts.json) else { std::process::exit(1) };
    match axon::compile_to_ir(&src, !opts.o0) {
        Ok(ir) => print!("{ir}"),
        Err(diags) => {
            report(&diags, opts.json);
            std::process::exit(1);
        }
    }
}

fn default_exe(input: &str) -> PathBuf {
    let mut p = PathBuf::from(input);
    p.set_extension(if cfg!(windows) { "exe" } else { "" });
    p
}

fn temp_exe(input: &str) -> PathBuf {
    let stem = PathBuf::from(input)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "program".into());
    let dir = std::env::temp_dir().join("axon-run");
    let _ = std::fs::create_dir_all(&dir);
    let unique = format!("{stem}-{}.exe", std::process::id());
    dir.join(unique)
}
