# Changelog

Notable changes to the Aoxn compiler and language. Aoxn follows semver-ish
minor bumps while pre-1.0: each minor version is a language milestone.

## [0.14.0] - 2026-09-12

### Added
- **Self-hosting: multi-file import resolution in the Aoxn front end**
  (`selfhost/load.ax`). `load_program(path)` reads a real `.ax` file, resolves
  `import "..."` recursively (paths relative to the importing file), includes
  each file once, rejects cycles with an import stack, and parses all files
  into one shared arena so node indices stay valid across files. Top-level
  nodes are spliced into a single program root (careful `next`-chain
  detaching); `checker_state()` + `check_all` now accept an already-parsed
  state. Verified by `selfhost/load_demo.ax` +
  `selfhost_frontend_handles_imports` (diamond include-once, cycle rejection,
  missing-file rejection, and `examples/stdlib_demo.ax` typechecking as a
  real 2-file program with 10 monomorphized instances).

## [0.13.0] - 2026-09-12

### Added
- **Self-hosting stage 4, first slice: the Aoxn code generator written in
  Aoxn** (`selfhost/codegen.ax`). Drives the LLVM-C API through `extern def`
  and emits a native object file from the self-hosted checker's output:
  int/void functions, int locals, arithmetic and comparisons, `if`/`else`,
  `while`, `for`-range, `break`/`continue`, direct calls and **monomorphized
  generic instances** (routed through the checker's `call_node`/`call_fni`).
  Target setup, `default<O3>`, verification and object emission included.
- `selfhost/codegen_demo.ax`: parses, checks and code-generates a sample
  program (generic `twice`, `fact` recursion, loops) to `selfhost_out.obj`.
- `selfhost_codegen_int_slice` test: builds the demo with `-l LLVM-C`,
  links its object with clang, runs it and compares the exit code with the
  Rust compiler's output for the same source (both 65).

### Fixed
- **Rust codegen literal-temp collision across files** (latent since v0.8
  imports): per-file parse-time literal ids restart at 0, but `lit_temps`
  cached hoisted allocas globally by id — an aggregate literal in one file
  reused an alloca from another function ("Referring to an instruction in
  another function", LLVM abort). Temps/replication iterators are now keyed
  by AST node address (as generic call routing already was). Regression test
  `import_aggregate_literals_across_files`.
- Codegen type hints for unannotated `load_i64`/`load_u8` (int) and
  `load_f64` (float) bindings.

## [0.12.0] - 2026-09-12

### Added
- **Self-hosting stage 3: monomorphization in the Aoxn type checker**
  (`selfhost/typecheck.ax`). Generic declarations are collected with `TY_VAR`
  type parameters and length-`N` arrays; every generic call unifies argument
  types against the declared parameter types, builds a deterministic mangled
  instance name (`id.i`, `first.i.?.3` — same scheme as the Rust compiler),
  clones the declaration's AST with the type parameters and `N` substituted,
  registers the instance as a normal signature, and queues its body for
  checking. Instances are deduplicated (recursive generics terminate) and
  each cloned call site is routed to its instance via `call_node`/`call_fni`.
- Self-hosted front-end exit test: the Aoxn lexer + parser + checker now
  process the **entire `stdlib.ax`** in one program
  (`selfhost_frontend_handles_stdlib`), plus generic accept/reject cases in
  `tycheck_demo.ax`.

### Fixed
- Self-hosted lexer keyword parity: `and` / `or` / `not` now map to `&&` /
  `||` / `!` (as in the Rust lexer) instead of distinct tokens the parser
  did not understand — this blocked parsing any real program using them.
- Self-hosted checker argument indexing for multi-argument builtins
  (`load_u8`, `store_i64`, `store_f64`, `store_u8`): it followed the sibling
  chain of the first argument's *value* instead of the argument list,
  producing bogus "missing expression" errors.
- Self-hosted checker signature collection: extern declarations were
  reported as "malformed function" because the return type was assumed to
  sit before a body block; externs return the last child.
- Self-hosted parser records the length-parameter name on `[T; N]` nodes so
  the checker can substitute `N` inside cloned bodies.

## [0.11.0] - 2026-09-12

### Added
- **Self-hosting stage 2: the Aoxn parser written in Aoxn**
  (`selfhost/parser.ax`, ~1100 lines) — arena AST (tag / sval / ival / child /
  next in parallel Vecs, first-child + next-sibling), full grammar port:
  declarations (import / struct / def+extern, generic headers, array-length
  params), statements (let / annotated let / assignment / if-elif-else folding
  / while / for-range / for-array / break / continue / return / pass),
  precedence-climbing expressions, call arguments, indexing, field access,
  array literals + `[e] * N` replication, and f-string desugaring via
  sub-lexing. Verified by `selfhost/parse_demo.ax` plus an exact AST-dump
  regression test (`selfhost_parser_ast_dump`).
- **Self-hosting stage 3, first slice: the Aoxn type checker written in Aoxn**
  (`selfhost/typecheck.ax`, ~1000 lines) — strict rules ported: struct
  collection (duplicate + cycle detection), signature collection, scope and
  struct tables, all-paths-return / unreachable-code analysis, expression
  typing, builtins, struct literals. Generic functions are reported as
  unsupported for now. Verified by `selfhost/tycheck_demo.ax` (accepts a
  well-typed program, rejects an ill-typed one) plus
  `selfhost_typechecker_accepts_and_rejects`.
- Self-hosted lexer: `;` token for array types, and f-strings now emit the
  raw literal source in the FSTR token so the parser can re-lex
  interpolations (matching the Rust lexer's literal/expr split).

### Fixed
- **Self-hosted parser segfault (the v0.11 WIP known issue).** Helpers such as
  `new_node` mutated a pass-by-value `PState` copy and discarded the write-back
  (`p.n_tag = vec_push(...)`), so the caller's arena Vecs stayed at `data=0`
  and the first `vec_set` stored through NULL. Rewritten in write-back style:
  every mutating function returns the updated `PState`, and multi-value
  results travel through `p.res_node` / `p.res_vec`.
- Self-hosted type checker: same write-back bug in `alloc_ty`; additionally
  the type arena's `t_elem` / `t_len` / `t_sname` Vecs were index-misaligned
  (seeded with a dummy slot while `t_tag` was not), making `ty_sname` return
  the wrong struct names. Fixed; struct-field counting no longer includes the
  reservation rows.
- Codegen: an unannotated binding of `as_string(...)` / `as_ptr(...)`
  (`x = as_string(p)`) failed with "unknown call in type hint"; both builtins
  now carry type hints (`string` / `int`).

## [0.10.0] - 2026-09-06

### Changed
- **Project renamed: Axon → Aoxn** (crate, binary, docs, examples).

### Added
- **Self-hosting stage 1: the Aoxn lexer written in Aoxn**
  (`selfhost/lexer.ax`, ~770 lines) — a faithful port of the Rust lexer:
  Python-style layout (NEWLINE/INDENT/DEDENT with an indent stack), comments,
  paren continuation, all operators, string escapes, floats, f-string raw
  tokens. Verified by an exact token-stream test (`selfhost/lex_demo.ax`).
- stdlib: `vec_pop`.

### Fixed
- Struct cycle detection: a struct appearing in multiple sibling fields was
  falsely reported as recursive; replaced with proper gray/black DFS.
- Runtime crash in the demo pipeline: an empty indent stack (seed value
  missing) made `vec_get(stack, -1)` dereference NULL.

## [0.9.0] - 2026-09-06

### Added
- **Raw memory builtins** (self-hosting foundation): `load_i64`/`store_i64`,
  `load_f64`/`store_f64`, `load_u8`/`store_u8` (byte access on `int`
  addresses and on `string` bytes), and pointer reinterpretation
  `as_string`/`as_ptr`. Addresses are plain `int`; unsafe by design.
- stdlib: growable `Vec` (8-byte slots, write-back style: `v = vec_push(v, x)`,
  `vec_get`/`vec_set`/`vec_free`), byte buffers (`buf_new`, `fill_zero`),
  string byte access (`str_get`), char classification (`is_digit`, `is_alpha`,
  `is_space`), file IO (`read_file`, `write_file` via FFI), and `system(cmd)`
  for process spawning.

### Fixed
- Lexer: `>=` was lexed as `>` (a latent bug no earlier test caught 鈥?  `4 >= 5` is false under both). Added boundary-equality regression tests.

## [0.8.1] - 2026-09-06

### Added
- `-l NAME` / `-L DIR` link flags: user programs can link arbitrary C
  libraries (LLVM-C, crypto, ...).
- Self-hosting proof of concept: `examples/ffi_llvm.ax` drives the LLVM-C
  API from Aoxn (pointers pass as `int`, ABI-identical on x86-64) and emits
  real IR 鈥?`define i64 @answer() { ret i64 42 }`.
- Self-hosting feasibility assessment: `docs/selfhost.md` (capability
  matrix, gap analysis, staged bootstrap plan, verdict: feasible).

## [0.8.0] - 2026-09-06

### Added
- **Import / module system**: `import "../stdlib/stdlib.ax"` 鈥?paths resolve
  relative to the importing file; include-once per canonical path; circular
  imports rejected with the full cycle chain.
- **File-aware diagnostics**: every error now names its source file
  (`[type] stdlib/stdlib.ax:130:20: ...`, also in `--json` output).
- `Aoxn build / run / ir` are now import-aware 鈥?`Aoxn run examples\stdlib_demo.ax`
  alone pulls in the standard library.

## [0.7.0] - 2026-09-06

### Added
- **Generic functions**: `def sort[T, N](arr: [T; N]) -> [T; N]` 鈥?type
  parameters `T` and array-length parameters `N`, monomorphized at every call
  site (deterministic mangled instances like `sort.i.8`).
- The length parameter is usable as an `int` constant inside generic bodies
  (`range(N)`, `N - 1`).
- Standard library rewritten with generics: `sort`, `linear_search`,
  `binary_search`, `max_of`, `min_of`, `reverse`, `sum_int`, `sum_float` 鈥?  replacing the fixed-size `_8` functions.

### Fixed
- Generic declarations no longer reach codegen (a `[T; usize::MAX]` type
  crashed LLVM's array-type construction).

## [0.6.0] - 2026-09-06

### Added
- `extern def` 鈥?C FFI declarations (bodyless, resolved at link time).
- Multi-file compilation: `Aoxn build main.ax stdlib/stdlib.ax` merges all
  inputs into one namespace (build / run / ir).
- `stdlib/stdlib.ax` 鈥?the standard library written in Aoxn itself: math
  (`abs/min/max/clamp/pow_i/gcd/lcm/isqrt/is_prime/hypot` + `sqrt/floor/ceil`
  via FFI), search, sort.
- `examples/stdlib_demo.ax`.

## [0.5.0] - 2026-09-06

### Added
- `for` loops: `range(n)`, `range(a, b)`, `range(a, b, step)` (negative step
  supported), and array iteration (`for x in arr`). start/end/step evaluated
  once at loop entry (Python semantics).
- `break` / `continue`.
- f-strings: `f"hello {name}, {1 + 2}"` with `{{`/`}}` escapes and arbitrary
  expressions; desugars to `"lit" + str(expr) + ...`.
- `str()` builtin (int 鈫?decimal, float 鈫?`%f`, bool 鈫?`true`/`false`).

### Added (CI)
- GitHub Actions: windows-latest, winget LLVM, full test suite + smoke test.

## [0.4.0] - 2026-09-05

### Added
- String operations: `+` concatenation (runtime malloc + memcpy + NUL), all
  six comparison operators (byte-wise `strcmp`), `len(str)` (bytes).
- Strings in struct fields, array elements, parameters, and returns.
- C runtime functions (malloc/strlen/strcmp/snprintf) declared lazily.

### Design
- Strings are immutable; concatenation results are heap-allocated and never
  freed (no GC yet 鈥?documented behavior).

## [0.3.0] - 2026-09-05

### Added
- Fixed-size arrays `[T; N]`: literals, unchecked indexing, `[e] * N`
  replication (element evaluated once, runtime fill loop).
- Structs: Python-style indented fields, named-field construction
  (`Point(x=1, y=2)`), field access/assignment, nesting, forward references,
  recursion rejected.
- Value semantics: assignment/params/returns copy whole aggregates (memcpy).

### Fixed
- Struct GEP field indices must be i32 constants (LangRef rule).
- Large aggregates are never materialized as SSA values 鈥?`load [50000 x i64]`
  made SROA/O3 hang; assignment now goes through memcpy.

## [0.2.0] - 2026-09-05

### Changed
- Python-style surface syntax: indentation-delimited blocks, `def` / `elif` /
  `pass`, `#` comments, no braces or semicolons, `//` is integer division.
- Lexer emits NEWLINE / INDENT / DEDENT; blank and comment-only lines produce
  no tokens; newlines inside parentheses ignored (implicit line joining).
- `and` / `or` / `not` and `True` / `False` as aliases of the symbolic forms.
- Bindings: `x = 5` infers, `x: int = 5` checks, re-assignment keeps the type.

### Removed
- `fn` / `let` keywords, braces, semicolons, block comments.

## [0.1.0] - 2026-09-05

### Added
- Initial compiler in Rust with zero external crates: hand-written LLVM-C FFI
  (no inkwell/llvm-sys), pipeline lexer 鈫?parser 鈫?strict typecheck 鈫?LLVM O3
  鈫?object file 鈫?clang link.
- Primitives (int/float/bool/string), functions (mutual recursion), control
  flow, `print` builtin.
- `Aoxn build / run / ir` CLI with `--json` diagnostics for AI agents.
- Benchmarks: parity with `clang -O3` on identical algorithms.

