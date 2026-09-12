# Self-Hosting Feasibility Assessment

> Can Aoxn be rewritten in Aoxn? Short answer: **yes — and stages 1–3 are
> underway**: the Aoxn lexer, parser and type checker written in Aoxn live in
> `selfhost/` (verified by exact token-stream / AST-dump tests and by
> checking the entire `stdlib.ax`), and generics are monomorphized just like
> in the Rust compiler. The riskiest assumption was validated first by a
> working proof of concept (`examples/ffi_llvm.ax`): an Aoxn program drives
> the LLVM-C API through `extern def` and emits real IR.

## 1. Evidence: the FFI proof of concept

`examples/ffi_llvm.ax` calls 12 LLVM-C functions from plain Aoxn code and
produces:

```llvm
define i64 @answer() {
entry:
  ret i64 42          ; LLVMBuildAdd(20, 22) 鈥?O3 constant-folded
}
```

Key facts established by this experiment:

- **Pointers work as `int`**: LLVM handles (`LLVMContextRef`, `TypeRef`,
  ...) are 8-byte values; Aoxn `int` is i64 鈥?ABI-identical on x86-64. No
  pointer type is needed to *hold* opaque handles.
- **Strings work as `char*`**: Aoxn string literals/heap strings are
  NUL-terminated byte buffers 鈥?exactly what the C API expects.
- **Any C library can be linked**: `-l LLVM-C -L <dir>` flags pass through
  to clang (v0.8).
- Scalable pattern: the entire LLVM-C surface the compiler needs is
  addressable with `extern def` + int/string parameters.

## 2. What the compiler needs vs what Aoxn has

| Requirement (Rust impl today) | Aoxn now | Gap |
|---|---|---|
| Unicode/ASCII scanning, char classes | int arithmetic, comparisons | none |
| Token vector (grow) | fixed arrays only | **dynamic arrays** |
| Token/AST enums (`enum Tok { ... }`) | no sum types | **tagged unions or enums** |
| Structs with payloads | structs | works (tagged-union style) |
| HashMap for symbol tables | none | dynamic arrays + linear/probing |
| String building (lexer) | `+` concat, no indexing | **string indexing / char** |
| Mutating buffers (lexer, codegen strings) | immutable strings | **mutable byte buffers** |
| LLVM-C FFI | `extern def` (int/string) | proven 鉁?|
| Spawn clang for linking | none | `extern def system(...)` or CreateProcess |
| Read source files | none | `fopen/fread` via FFI (needs ptr-as-int) |
| Generics for containers | 鉁?monomorphized | none |
| Modules, file loading | 鉁?imports | none |
| Diagnostics w/ files, JSON output | 鉁?| none |

## 3. Gaps, ranked

### B1 鈥?dynamic arrays (blocker)
A growable `Vec[T]` in the stdlib, built on `malloc`/`realloc` via FFI.
Generics already exist, so `Vec[T]` is a stdlib artifact, not a compiler
feature. Needs: pointer-holding (int is fine), malloc/realloc/free externs,
and **addressability** 鈥?growing a vec must write back its pointer:
`v = vec_push(v, item)` (functional style, value semantics preserved).
Effort: ~150 lines of stdlib. Unblocks token vectors, AST nodes, scopes.

### B2 鈥?sum types or tagged-union ergonomics (blocker for sane AST)
The AST is enums all the way down (`Expr::Binary{op,lhs,rhs}`). Without sum
types, use the C pattern:

```Aoxn
struct Expr:
    tag: int          # EXPR_INT, EXPR_VAR, EXPR_BINARY, ...
    ival: int
    sval: string
    lhs: int          # indices into an arena instead of pointers
    rhs: int
```

Arena allocation (`Vec[Expr]` + integer indices) sidesteps pointers entirely
and is *simpler* than Rust's ownership model for a self-hosted compiler.
Alternative: add real `enum`/`match` to the language (bigger language
change, better long-term). Recommendation: **arena + tagged structs first,
`match` syntax later**.

### B3 鈥?string indexing / mutable byte buffers (major)
The lexer needs `s[i]`, `s[i] = c`, char鈫攊nt. Options:
- stdlib functions over malloc'd buffers (`buf_get`, `buf_set`, `buf_len`)
  using int-as-pointer FFI 鈥?no language change;
- or first-class `char` type + indexing (language change, on roadmap anyway).
Recommendation: buffer functions in stdlib now, `char` later.

### B4 鈥?file I/O + process spawn (major, small)
`fopen/fread/fclose` and `system`/`CreateProcess` via `extern def` 鈥?all
pointer-as-int friendly. ~40 lines of stdlib. Unblocks reading sources and
invoking clang.

### M1 鈥?unsigned/byte types (minor)
i64 covers char-classification and sizes; `u8` is a nicety, not a need.

### M2 鈥?compile-time evaluation (not needed)
The Rust compiler does no comptime; self-hosting doesn't require it.

## 4. Bootstrap plan

```
stage 0  axonc.rust     current compiler (the crutch)
stage 1  axonc.axon     compiler written in Aoxn
         axon build axonc.ax -l LLVM-C -L <llvm>\lib   → stage1.exe
stage 2  stage1.exe compiles axonc.ax   → stage2.exe
         verify: stage1 and stage2 produce identical IR for the test suite
stage 3  CI keeps the fixed point green: stage2 builds axonc.ax, outputs match
```

Progress: **stages 1–3 are complete** — the lexer (`selfhost/lexer.ax`), the
parser (`selfhost/parser.ax`) and the type checker (`selfhost/typecheck.ax`,
with generic monomorphization) run green in CI, including a full check of
`stdlib.ax`. Multi-file imports are resolved in Aoxn too (`selfhost/load.ax`),
so the front end consumes real `.ax` files. Stage 4 has started:
`selfhost/codegen.ax` emits native objects for int/bool/string programs
through LLVM-C, including `print`, `len`/`str` and f-strings. Remaining:
floats/structs/arrays, then the CLI/driver.

The Rust compiler remains the bootstrap crutch until stage 2 is stable, then
becomes a test oracle only. The self-hosted compiler does NOT need to link
LLVM-C via the C API forever — it can reuse the same emission path
(`LLVMTargetMachineEmitToFile`) since it talks to the same DLL.

## 5. Scale estimate

| Component | Rust today | Aoxn estimate |
|---|---|---|
| lexer.rs | ~700 | ~900 (tagged structs + buffer fns) |
| parser.rs | ~640 | ~800 |
| typecheck.rs | ~1450 | ~1800 (monomorphizer included) |
| codegen.rs | ~1100 | ~1400 (LLVM-C FFI calls) |
| main/lib (CLI, link) | ~350 | ~400 |
| **total** | **~4200 Rust** | **~5300 Aoxn** |

At the current velocity (one feature-milestone per session, each with full
test coverage), the plan is:

1. **v0.9**: Vec[T] + buffer/string utils + file IO + process spawn (stdlib
   work, no language changes except maybe `char`).
2. **v1.0**: arena-tagged-union discipline proven by writing a REAL tool in
   Aoxn that parses Aoxn syntax (the stage-1 lexer only) 鈥?the dress
   rehearsal.
3. **v1.1鈥搗1.3**: parser 鈫?typecheck 鈫?codegen in Aoxn, ported
   component-by-component, each keeping the 75-test pipeline green from both
   compilers.
4. **v1.4**: fixed-point CI (stage2 == stage1), self-hosting achieved.

## 6. Risks

- **Speed of the Aoxn compiler compiled by itself**: the pipeline is
  dominated by LLVM O3 (a C library), not by our Rust code 鈥?the risk is
  small; the interpreter-free design helps.
- **Value semantics copying large AST nodes**: arena + indices (B2) avoids
  big copies by design 鈥?this is why the arena is the recommended pattern.
- **Debugging without a debugger story**: Aoxn diagnostics must be excellent
  (they are, `--json`) because stage-1 bugs will surface as Aoxn-side
  panics; mitigate with a `assert(cond, msg)` builtin early.
- **Language churn during porting**: freeze language changes once stage-1
  work starts; the compiler being ported must not chase a moving target.

## 7. Verdict

Feasible. The two genuinely new compiler features are dynamic arrays (stdlib)
and a disciplined arena/tagged-union style (no language change required);
everything else the pipeline needs 鈥?FFI, generics, modules, file-aware
diagnostics 鈥?already exists and the LLVM-C path is proven end-to-end by a
working example in the repository.

