# Axon

**Axon** is an AI-native, statically typed, ahead-of-time compiled programming
language. Python-style syntax on the surface, C++-class native performance
underneath: programs compile through LLVM (O3) straight to machine code.

```axon
struct Vec2:
    x: int
    y: int

def add(a: Vec2, b: Vec2) -> Vec2:
    return Vec2(x=a.x + b.x, y=a.y + b.y)

def main() -> int:
    total = add(Vec2(x=1, y=2), Vec2(x=3, y=4))
    print(total.x + total.y)   # 10
    return 0
```

## Why Axon

- **Python-style syntax** — indentation blocks, `def` / `elif` / `pass`,
  `#` comments, `x = 5` type inference, `and` / `or` / `not`, `[0] * n`
- **Native speed** — LLVM O3 backend; measured at parity with `clang -O3`
  (benchmarks below)
- **AI-native tooling** — every diagnostic can be emitted as structured JSON
  (`--json`), the optimized IR is dumpable (`axon ir`), and the semantics are
  deliberately strict and deterministic so AI-generated code is verifiable
- **Value semantics** — arrays, structs, and strings copy by value; no hidden
  references; array indexing is unchecked, C-style

## Quick start

Prerequisites (Windows): Rust (msvc host), LLVM (C API), clang, MSVC Build
Tools. `build.rs` locates LLVM automatically (`AXON_LLVM_DIR` overrides).

```powershell
cargo build
cargo run -- run examples\hello.ax
```

Emit a standalone native executable:

```powershell
cargo run -- build examples\fib.ax -o fib.exe
.\fib.exe
```

More: `cargo run -- ir examples\fib.ax` dumps the optimized LLVM IR;
`cargo run -- run examples\primes.ax --json` emits machine-readable
diagnostics; `--O0` disables optimization.

## Language tour

```axon
# arrays, structs, strings — all value types
struct Particle:
    x: float
    y: float
    mass: float

def energy(p: Particle) -> float:
    return (p.x * p.x + p.y * p.y) * p.mass

def main() -> int:
    ps: [Particle; 1000] = [Particle(x=1.5, y=0.5, mass=2.0)] * 1000

    total = 0.0
    i = 0
    while i < len(ps):
        total = total + energy(ps[i])
        i = i + 1

    print(total)
    print("a" < "b")     # true — strings compare byte-wise
    return 0
```

Types are strict and every expression is checked: `int`/`float` never mix
implicitly, conditions must be `bool`, array indexing is unchecked (C-style),
and every function must return a value on all paths. Strictness is a feature:
the guarantees are simple enough for a machine to reason about.

## Performance

Same-algorithm comparisons against `clang -O3` on the same machine (warm
runs, best of 3):

| Benchmark | Scale | Axon | clang C++ |
|---|---|---|---|
| Loop sum | 2×10⁸ iterations | ~15 ms | ~23 ms |
| Array fill + scan | 2×10⁸ reads | 86 ms | 99 ms |
| Struct copies (by value) | 7.5×10⁷ copies | 237 ms | 206 ms |

Native code is native code — Axon sits within noise of clang.

## Project layout

| Path | Contents |
|---|---|
| `src/` | the compiler: lexer → parser → typecheck → LLVM codegen → clang link |
| `src/llvm.rs` | hand-written LLVM-C FFI (no inkwell/llvm-sys) |
| `examples/*.ax` | demo programs (hello, fib, primes, vectors, strings, benchmarks) |
| `tests/pipeline.rs` | 50 end-to-end tests: compile → run → verify output |
| `docs/spec.md` | full language specification and roadmap |

## Status

v0.4 · Windows-first · 50/50 tests green.

Roadmap: `for` loops & f-strings → standard library written in Axon itself →
self-hosting (compiler rewritten in Axon).

See [`docs/spec.md`](docs/spec.md) for the complete language specification.

## License

Apache-2.0 — see [`LICENSE`](LICENSE).
