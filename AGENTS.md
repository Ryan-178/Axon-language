# AGENTS.md — Axon compiler

Axon: AI-native compiled language with Python-style syntax (indentation
blocks, `def`, `elif`, `#` comments) compiled through LLVM 18 to native code —
measured at parity with `clang -O3`. This repo IS the compiler (v0.6, written
in Rust). Language sources use the `.ax` extension. Language rules live in
`docs/spec.md` — keep it in sync with `src/parser.rs` + `src/typecheck.rs`
when the grammar changes.

## Commands

```powershell
cargo build                                   # build the `axon` compiler
cargo test                                    # 65 integration tests (compile .ax -> exe -> run)
cargo run -- run examples\hello.ax            # compile+run an Axon program
cargo run -- run examples\stdlib_demo.ax stdlib\stdlib.ax   # multi-file: merged namespace
cargo run -- build examples\fib.ax -o f.exe   # emit a native executable (LLVM O3 default)
cargo run -- ir examples\fib.ax               # dump optimized LLVM IR
cargo run -- run bad.ax --json                # diagnostics as JSON for agent consumption
```

Single test: `cargo test --test pipeline recursion_fib`.
Debug codegen: set `AXON_DUMP_IR=1` to print pre-verify IR to stderr.

## Environment facts (non-obvious, verified)

- **LLVM 23.1.0 at `C:\Program Files\LLVM`** (standard winget install;
  originally 18.1.8 lived repo-locally at `LLVM/`). `build.rs` searches:
  `AXON_LLVM_DIR` env → `<repo>/LLVM` → `C:\Program Files\LLVM`. If you
  change the LLVM install, touch `build.rs` (or `cargo clean`) so the build
  script re-runs — the old LIBPATH is cached otherwise.
- The Windows installer ships only the C API: `LLVM-C.lib` + `LLVM-C.dll`.
  Our FFI was written against the LLVM 18 C API and works unchanged on 23 —
  before relying on any new symbol, verify it exists:
  `findstr /c:"LLVMFoo" "C:\Program Files\LLVM\lib\LLVM-C.lib"`.
- **Do NOT use inkwell/llvm-sys** — they need full static LLVM libs which
  this install does not have. We use hand-written FFI in `src/llvm.rs`
  against the C API only.
- Use `LLVMCreateBuilderInContext` (not `LLVMBuilderCreate`): the 18.1.8
  installer's LLVM-C.lib was missing that symbol; the context variant works
  on every version.
- **`LLVM-C.dll` must be next to any binary that links LLVM-C.lib.** `build.rs`
  copies it into `target/{debug,release}{,/deps,/examples}` — if you see a DLL
  load error, add the LLVM `bin` directory to PATH.
- **Final linking shells out to clang** (resolution order: `AXON_CLANG` env →
  PATH → `LLVM\bin\clang.exe` → `C:\Program Files\LLVM\bin\clang.exe`). clang
  auto-detects MSVC Build Tools, which must be installed (Rust host is
  x86_64-pc-windows-msvc). `lld-link` exists in `LLVM\bin` but needs manual
  LIB paths — prefer clang.
- LLVM target registration is process-global: `LLVMInitializeX86*` must run
  exactly once per process (`init_target()` with `Once` in codegen.rs), or
  `LLVMGetTargetFromTriple` fails with "Cannot choose between targets".

## Architecture (pipeline order)

```
.ax source
  → src/lexer.rs      tokens with line/col; emits NEWLINE/INDENT/DEDENT
  → src/parser.rs     recursive-descent AST (src/ast.rs), Python-style layout
  → src/typecheck.rs  strict check + FnSig table (allows mutual recursion)
  → src/codegen.rs    LLVM IR via C API → object file (LLVMTargetMachineEmitToFile, O3 via LLVMRunPasses)
  → src/lib.rs        link via clang → executable
src/main.rs           CLI (build / run / ir), --json diagnostics
```

- Diagnostics: `Diag { stage, line, col, message }` in lib.rs; stages are
  `lex | parse | type | internal | link | io`. Compiler-internal failures must
  surface as `internal` diags, never panics, except LLVM's own fatal errors.
- Zero external crate dependencies. Any new LLVM feature = add `extern "C"`
  to `src/llvm.rs` + verify the symbol exists in LLVM-C.lib first.
- Multi-file: `parse_sources` merges all inputs into one Program (shared
  namespace, duplicate detection applies). `extern def` = bodyless C FFI
  declaration — no entry block, no body emission; declaring `extern main`
  is a type error.

## Lexer invariants (Python-style layout)

- The lexer maintains an indent stack and emits `Indent`/`Dedent` tokens;
  the parser consumes `:` Newline Indent stmt* Dedent (or one simple stmt).
- Blank lines and comment-only lines produce **no tokens** (no indent changes).
- Inside parentheses `paren_depth > 0`: newlines and indentation are ignored
  (implicit line joining).
- EOF: flush trailing `Newline`, then pending `Dedent`s, then `Eof`.
- `//` is NOT a comment (it is int division); comments are `#` only.
- Indent/dedent mismatch → lex error "unindent does not match any outer
  indentation level".
- Tests embed indented sources; `dedent()` in tests/pipeline.rs strips the
  common leading whitespace before compiling.

## Codegen invariants (learned the hard way)

- Allocas must be inserted at the **top** of the entry block — the entry block
  may already be terminated by short-circuit branches when a `let` is
  emitted. Always use `Gen::alloca_in_entry`, never position at entry end.
- `if` end-blocks are created **lazily**: when both branches return, an
  eagerly-created empty merge block would be an unterminated basic block.
- Short-circuit `&&`/`||` produce phi nodes; incoming constant is `false` for
  `&&` / `true` for `||` (the skip path's value).
- The C entry wrapper is emitted BEFORE user functions so LLVM names it `main`;
  the user's `main` is renamed `axon.main`. Don't flip the order.
- `print(bool)` emits branch + `%s` with `true`/`false` globals (readable
  output), stdout is set to binary mode via `_setmode(1, 0x8000)` for
  deterministic `\n` endings on Windows.
- **Struct GEP indices must be i32 constants** (LangRef rule) — i64 field
  indices fail with "Invalid indices for GEP pointer type". Array indices are
  i64. Struct types are built in two phases (LLVMStructCreateNamed for all,
  then LLVMStructSetBody) so structs can forward-reference each other.
- Array/struct literals get **entry-hoisted temp allocas** keyed by parse-time
  site id (`lit_temps`) — never alloca inside loops (stack grows per
  iteration). Non-lvalue aggregate bases (`f()[0]`) are materialized through
  `val_temps` keyed by AST node address.
- Aggregates use **value semantics**: params/returns/assignment copy whole
  values. Bindings/assignments of compound types copy via **LLVMBuildMemCpy**
  (`emit_aggregate_ptr` + size from `LLVMSizeOf`) — loading a big array as a
  single SSA value (e.g. `load [50000 x i64]`) makes SROA/O3 blow up
  quadratically (compiler hangs/crashes). Small aggregates returned from
  functions are still plain load/ret.
- `[e] * N` replication fills its temp with a **runtime loop** (`fill_rep`),
  never N-unrolled stores (same optimizer blow-up).
- Strings: literals are static globals; `a + b` is malloc + two memcpys +
  NUL (`emit_str_concat`); comparisons/ordering go through C runtime
  `strcmp`; `len(str)` is `strlen`. Concat results are **never freed**
  (immutable strings, no GC yet — documented behavior). C runtime functions
  (malloc/strlen/strcmp/snprintf) are declared lazily via `Gen::get_extern`.
- `for` loops: ranges and arrays share one emission path; the **induction
  slot is separate from the loop variable** for arrays (hidden i64 index; the
  element variable is a plain copy written in the body preamble). start/end/
  step and the array pointer are evaluated once at entry. `break`/`continue`
  target blocks ride the `loop_stack`.
- f-strings lex into `FStrPart` lists (sub-expressions re-lexed in parens to
  defeat the indent tracker, padding preserved for column-accurate errors),
  then desugar in the parser to `"lit" + str(expr) + ...` chains. `str()`
  uses snprintf (int/float) or a branch-phi over static "true"/"false".

## Language semantics (enforced, keep strict)

- No implicit int/float conversions, no re-typing a bound name, `bool`
  conditions only, all-paths-return, no unreachable code. This strictness is
  a feature (deterministic, AI-verifiable); do not relax it casually.
- Python-style surface, native semantics: `def`/`elif`/`pass`, `x = 5`
  infers, `x: int = 5` checks, re-assignment keeps the type, `and`/`or`/`not`
  + `True`/`False` are aliases of the symbolic operators.
- Arrays `[T; N]` and structs are first-class value types (copy on
  assignment/param/return). Indexing is unchecked (C-style); struct fields by
  name; construction requires every field by name (`Point(x=1, y=2)`).
- Strings are immutable byte sequences: `+` concatenates, all six comparisons
  work byte-wise, `len()` is bytes. They live in struct fields/arrays by
  shared pointer (safe: immutability); concat results leak by design.
- When grammar/semantics change: update `docs/spec.md`, parser, typecheck,
  codegen, and add a test in `tests/pipeline.rs` in the same change.

## Repo layout

- `examples/*.ax` — demo programs (hello, fib, primes, vectors, strings, benchmarks, stdlib_demo)
- `stdlib/stdlib.ax` — the standard library, written in Axon itself
- `docs/spec.md` — language spec + roadmap (self-hosting, in-Axon stdlib)
