# Aoxn

**Aoxn** is an AI-native, statically typed, ahead-of-time compiled programming
language. Python-style syntax on the surface, C++-class native performance
underneath: programs compile through LLVM (O3) straight to machine code.

```Aoxn
def sort[T, N](arr: [T; N]) -> [T; N]:      # generic, monomorphized
    result = arr
    for i in range(N):
        for j in range(N - 1 - i):
            if result[j] > result[j + 1]:
                t = result[j]
                result[j] = result[j + 1]
                result[j + 1] = t
    return result

def main() -> int:
    nums = [5, 3, 8, 1]
    print(f"sorted: {sort(nums)[0]}")   # sorted: 1
    print(sort(["pear", "apple"])[0])   # apple
    return 0
```

## Why Aoxn

- **Python-style syntax** 鈥?indentation blocks, `def` / `elif` / `pass`,
  `#` comments, `x = 5` type inference, `and` / `or` / `not`, `[0] * n`
- **Native speed** 鈥?LLVM O3 backend; measured at parity with `clang -O3`
  (benchmarks below)
- **Generics** 鈥?`def sort[T, N](arr: [T; N])` monomorphized per call site;
  no boxing, no runtime overhead
- **AI-native tooling** 鈥?every diagnostic can be emitted as structured JSON
  (`--json`), the optimized IR is dumpable (`Aoxn ir`), and the semantics are
  deliberately strict and deterministic so AI-generated code is verifiable
- **Value semantics** 鈥?arrays, structs, and strings copy by value; no hidden
  references; array indexing is unchecked, C-style

## Quick start

Prerequisites (Windows): Rust (msvc host), LLVM (C API), clang, MSVC Build
Tools. `build.rs` locates LLVM automatically (`AOXN_LLVM_DIR` overrides).

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

```Aoxn
# arrays, structs, strings 鈥?all value types
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
    print("a" < "b")     # true 鈥?strings compare byte-wise
    return 0
```

Types are strict and every expression is checked: `int`/`float` never mix
implicitly, conditions must be `bool`, array indexing is unchecked (C-style),
and every function must return a value on all paths. Strictness is a feature:
the guarantees are simple enough for a machine to reason about.

## Performance

Same-algorithm comparisons against `clang -O3` on the same machine (warm
runs, best of 3):

| Benchmark | Scale | Aoxn | clang C++ |
|---|---|---|---|
| Loop sum | 2脳10鈦?iterations | ~15 ms | ~23 ms |
| Array fill + scan | 2脳10鈦?reads | 86 ms | 99 ms |
| Struct copies (by value) | 7.5脳10鈦?copies | 237 ms | 206 ms |

Native code is native code 鈥?Aoxn sits within noise of clang.

## Project layout

| Path | Contents |
|---|---|
| `src/` | the compiler: lexer 鈫?parser 鈫?typecheck 鈫?LLVM codegen 鈫?clang link |
| `src/llvm.rs` | hand-written LLVM-C FFI (no inkwell/llvm-sys) |
| `stdlib/stdlib.ax` | the standard library, written in Aoxn itself (generics) |
| `examples/*.ax` | demo programs (hello, fib, primes, vectors, strings, benchmarks, stdlib_demo) |
| `tests/pipeline.rs` | 70 end-to-end tests: compile 鈫?run 鈫?verify output |
| `docs/spec.md` | full language specification and roadmap |

## Self-hosting

Aoxn driving LLVM-C from inside Aoxn (first brick of self-hosting): see
examples/ffi_llvm.ax and the full assessment in
[docs/selfhost.md](docs/selfhost.md).

## Status

v0.7 路 Windows-first 路 70/70 tests green 路 CI on every push.

Roadmap: import/module system 鈫?self-hosting (compiler rewritten in Aoxn).

See [`docs/spec.md`](docs/spec.md) for the complete language specification
and [`CHANGELOG.md`](CHANGELOG.md) for the release history.

## License

Apache-2.0 鈥?see [`LICENSE`](LICENSE).


