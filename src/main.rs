//! Aoxn compiler driver.
//!
//! Usage:
//!   Aoxn build <file.ax> [-o out.exe] [--O0] [--json]
//!   Aoxn run   <file.ax> [args...] [--O0] [--json]
//!   Aoxn ir    <file.ax> [--O0] [--json]
//!   Aoxn --help

use std::path::PathBuf;
use std::process::Command;

use aoxn::Diag;

struct Opts {
    out: Option<String>,
    o0: bool,
    json: bool,
    positional: Vec<String>,
    // everything after "--" (used by `run` to pass args to the compiled program)
    passthrough: Vec<String>,
    /// additional libraries to link (`-l LLVM-C`), repeatable
    libs: Vec<String>,
    /// additional library search paths (`-L C:\...\lib`), repeatable
    lib_paths: Vec<String>,
}

fn parse_opts(args: &[String]) -> Opts {
    let mut opts = Opts {
        out: None,
        o0: false,
        json: false,
        positional: Vec::new(),
        passthrough: Vec::new(),
        libs: Vec::new(),
        lib_paths: Vec::new(),
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" if i + 1 < args.len() => {
                opts.out = Some(args[i + 1].clone());
                i += 2;
            }
            "-l" if i + 1 < args.len() => {
                opts.libs.push(args[i + 1].clone());
                i += 2;
            }
            "-L" if i + 1 < args.len() => {
                opts.lib_paths.push(args[i + 1].clone());
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
            eprintln!("error: unknown command '{other}' (try: Aoxn --help)");
            std::process::exit(2);
        }
    }
}

fn print_help() {
    println!(
        "Aoxn compiler v{}\n\n\
         USAGE:\n  \
         Aoxn build <file.ax> [-o out] [--O0] [--json]   compile to a native executable\n  \
         Aoxn run <file.ax> [--O0] [--json] [-- args...]  compile and run in one step\n  \
         Aoxn ir <file.ax> [--O0] [--json]               print the LLVM IR\n\n\
         FLAGS:\n  \
         -o <path>   output executable path (default: <file>.exe)\n  \
         --O0        disable optimizations (default: O3)\n  \
         --json      emit diagnostics as JSON (AI-agent friendly)",
        env!("CARGO_PKG_VERSION")
    );
}

fn report(diags: &[Diag], json: bool) {
    if json {
        eprintln!("{}", aoxn::diags_to_json(diags));
    } else {
        for d in diags {
            eprintln!("{}", aoxn::diag_to_string(d));
        }
    }
}

fn cmd_build(args: &[String]) {
    let opts = parse_opts(args);
    if opts.positional.is_empty() {
        eprintln!("error: 'Aoxn build' needs an input file (.ax)");
        std::process::exit(2);
    }
    // the first file is the entry; `import "..."` pulls in the rest
    let exe = opts.out.map(PathBuf::from).unwrap_or_else(|| default_exe(&opts.positional[0]));
    match aoxn::build_paths_opts(&opts.positional, &exe, !opts.o0, &opts.libs, &opts.lib_paths) {
        Ok(()) => println!("{}", exe.display()),
        Err(diags) => {
            report(&diags, opts.json);
            std::process::exit(1);
        }
    }
}

fn cmd_run(args: &[String]) {
    let opts = parse_opts(args);
    if opts.positional.is_empty() {
        eprintln!("error: 'Aoxn run' needs an input file (.ax)");
        std::process::exit(2);
    }
    // program args: anything after "--"
    let prog_args = &opts.passthrough;
    let exe = temp_exe(&opts.positional[0]);
    if let Err(diags) = aoxn::build_paths_opts(&opts.positional, &exe, !opts.o0, &opts.libs, &opts.lib_paths) {
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
    if opts.positional.is_empty() {
        eprintln!("error: 'Aoxn ir' needs an input file (.ax)");
        std::process::exit(2);
    }
    match aoxn::compile_paths_to_ir(&opts.positional, !opts.o0) {
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
    let dir = std::env::temp_dir().join("Aoxn-run");
    let _ = std::fs::create_dir_all(&dir);
    let unique = format!("{stem}-{}.exe", std::process::id());
    dir.join(unique)
}



