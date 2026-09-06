# Changelog

Notable changes to the Axon compiler and language. Axon follows semver-ish
minor bumps while pre-1.0: each minor version is a language milestone.

## [0.8.1] - 2026-09-06

### Added
- `-l NAME` / `-L DIR` link flags: user programs can link arbitrary C
  libraries (LLVM-C, crypto, ...).
- Self-hosting proof of concept: `examples/ffi_llvm.ax` drives the LLVM-C
  API from Axon (pointers pass as `int`, ABI-identical on x86-64) and emits
  real IR — `define i64 @answer() { ret i64 42 }`.
- Self-hosting feasibility assessment: `docs/selfhost.md` (capability
  matrix, gap analysis, staged bootstrap plan, verdict: feasible).

## [0.8.0] - 2026-09-06

### Added
- **Import / module system**: `import "../stdlib/stdlib.ax"` — paths resolve
  relative to the importing file; include-once per canonical path; circular
  imports rejected with the full cycle chain.
- **File-aware diagnostics**: every error now names its source file
  (`[type] stdlib/stdlib.ax:130:20: ...`, also in `--json` output).
- `axon build / run / ir` are now import-aware — `axon run examples\stdlib_demo.ax`
  alone pulls in the standard library.

## [0.7.0] - 2026-09-06

### Added
- **Generic functions**: `def sort[T, N](arr: [T; N]) -> [T; N]` — type
  parameters `T` and array-length parameters `N`, monomorphized at every call
  site (deterministic mangled instances like `sort.i.8`).
- The length parameter is usable as an `int` constant inside generic bodies
  (`range(N)`, `N - 1`).
- Standard library rewritten with generics: `sort`, `linear_search`,
  `binary_search`, `max_of`, `min_of`, `reverse`, `sum_int`, `sum_float` —
  replacing the fixed-size `_8` functions.

### Fixed
- Generic declarations no longer reach codegen (a `[T; usize::MAX]` type
  crashed LLVM's array-type construction).

## [0.6.0] - 2026-09-06

### Added
- `extern def` — C FFI declarations (bodyless, resolved at link time).
- Multi-file compilation: `axon build main.ax stdlib/stdlib.ax` merges all
  inputs into one namespace (build / run / ir).
- `stdlib/stdlib.ax` — the standard library written in Axon itself: math
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
- `str()` builtin (int → decimal, float → `%f`, bool → `true`/`false`).

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
  freed (no GC yet — documented behavior).

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
- Large aggregates are never materialized as SSA values — `load [50000 x i64]`
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
  (no inkwell/llvm-sys), pipeline lexer → parser → strict typecheck → LLVM O3
  → object file → clang link.
- Primitives (int/float/bool/string), functions (mutual recursion), control
  flow, `print` builtin.
- `axon build / run / ir` CLI with `--json` diagnostics for AI agents.
- Benchmarks: parity with `clang -O3` on identical algorithms.
