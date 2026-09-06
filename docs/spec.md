# Axon Language Specification (v0.4)

Axon is an AI-native, statically typed, ahead-of-time compiled language with a
Python-style syntax. Design goals: minimal syntax, explicit semantics, native
(C++-class) speed via LLVM, self-hosting core libraries, and machine-friendly
tooling (JSON diagnostics, dumpable IR).

## Layout rules (Python-style)

- Blocks are defined by **indentation**, not braces. Any consistent width works;
  a tab counts as 4 columns.
- Statements end at a line break; no semicolons. `;` is a syntax error.
- `#` starts a comment (to end of line). Blank and comment-only lines are ignored.
- Inside `(...)` a line break is ignored (implicit line joining) — call
  arguments may span lines, trailing commas allowed.
- A single simple statement may follow `:` on the same line
  (`if n < 2: return n`). Compound statements (if/while/def) cannot.

## Types

| Type     | Meaning                  | Backend       |
|----------|--------------------------|---------------|
| `int`    | signed 64-bit integer    | i64           |
| `float`  | 64-bit IEEE float        | double        |
| `bool`   | `True` / `False`         | i1            |
| `string` | immutable byte string    | ptr to NUL-terminated bytes (heap when built, static when literal) |
| `void`   | absence of a value       | (function return only) |
| `[T; N]` | fixed-size array, N > 0  | [N x T]       |
| `Name`   | struct (see below)       | named %struct |

No implicit conversions. `int` and `float` never mix silently; `%` is int-only.
Integer division by zero is undefined (native crash, no runtime check — speed
first, matching C/C++). Array indexing is **unchecked** (C-style).

## Arrays

```axon
a = [10, 20, 30]              # inferred [int; 3]
xs: [int; 4] = [1, 2, 3, 4]   # annotated
m: [[int; 2]; 3] = [[1, 2], [3, 4], [5, 6]]   # nested
a[1] = 99                     # element assignment
print(len(a))                 # 3 (compile-time constant)
```

- Literals are non-empty; all elements must share one type (compounds allowed:
  arrays of structs, arrays of arrays).
- Indexing is 0-based with an `int`; **no bounds checking** (out-of-bounds is
  undefined behavior, like C).
- `len(x)` requires an array and returns its (static) length as `int`.

## Structs

```axon
struct Point:
    x: int
    y: int

p = Point(x=1, y=2)     # construct: every field, by name, exactly once
p.x = 10                # field assignment
d = p.x + p.y           # field access
```

- Fields are declared in an indented block; each needs a type annotation.
- Structs may reference structs defined later (forward references) and may
  contain arrays/other structs. A struct **cannot contain itself**, directly
  or indirectly.
- Construction requires all fields, by name, in any order; types must match.
- Struct and function names share one namespace.

## Strings

```axon
s = "hello" + ", " + "world"   # concatenation (runtime malloc + memcpy)
print(s)                       # hello, world
print(len(s))                  # 12 (bytes)
print("abc" == "abc")          # true
print("apple" < "banana")      # true (byte-wise lexicographic)
```

- `+` concatenates two strings (only strings can be concatenated).
- All six comparison operators work on `(string, string)` via byte-wise
  `strcmp` semantics; `len(s)` is the byte length.
- Strings are **immutable**. Literals live in static storage; concatenation
  results are heap-allocated and intentionally not freed (no GC yet — safe
  because strings never mutate, so sharing/aliasing is sound).
- Strings may appear in struct fields, array elements, parameters, and
  returns; copies share the underlying bytes (safe: immutability).
- No string indexing yet (returns no `char` type — roadmap).

## Value semantics (arrays and structs)

Assignment, parameter passing, and returns **copy** the whole value (lowered
to memcpy). There are no references or pointers yet:

```axon
b = a          # b is an independent copy
b[0] = 99      # does not change a
data: [int; 50000] = [0] * 50000    # Python-style replication
```

## Program structure

A program is a list of `struct` and `def` definitions; execution starts at `main`.

```axon
def main() -> int:
    print("hello, axon")
    return 0              # exit code (truncated to i32 by the C entry wrapper)
```

`main` returns `int` (process exit code) or `void` (exit 0).

## Bindings

```axon
x = 5               # inferred: int
x: int = 10         # annotated (Python-style); annotation must match
x = x + 5           # re-assignment keeps the declared type
```

- The first binding declares the variable; its type is fixed from the
  initializer (or checked against the annotation).
- Re-assignment must keep the declared type. Re-declaring with a different
  type is an error. No shadowing within a function (parameters included).
- `print(1)` prints `1`; `print(True)` prints `true` / `false` (readable).

## Functions

```axon
def fib(n: int) -> int:
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)
```

- Parameters require type annotations; the return annotation is optional
  (defaults to `void`).
- Recursion and mutual recursion are allowed; order of definition does not
  matter.

## Statements

- `if cond:` / `elif cond:` / `else:` — conditions must be `bool`.
- `while cond:` — the only loop (no `for` yet).
- `return` / `return expr`.
- `pass` — explicit empty statement.

## Expressions

```ebbnf
or    := and ("or"  | "||") and)*
and   := eq  (("and" | "&&") eq)*
eq    := rel (("==" | "!=") rel)*
rel   := add (("<" | "<=" | ">" | ">=") add)*
add   := mul (("+" | "-") mul)*
mul   := unary (("*" | "/" | "%") unary)*
unary := ("not" | "!" | "-") unary | primary
primary := INT | FLOAT | STRING | True | False
         | IDENT ("(" args ")")? | "(" expr ")"
```

- `and`/`or`/`not` and `&&`/`||`/`!` are synonyms. `and`/`or` short-circuit.
- `True`/`False` and `true`/`false` are synonyms.

## Semantics rules (strict, AI-verifiable)

- Every `if` / `while` condition must be `bool`.
- A non-void function must return a value on **all** paths (`if/elif/else`
  where every branch returns satisfies this).
- No unreachable statements (rejected at compile time).
- A name may be declared only once; annotations on re-assignment must match.

## Builtins

- `print(expr)` — one `int`, `float`, `bool`, or `string` (no arrays/structs);
  prints a trailing newline. Floats print with `%f` (6 decimals).
- `len(x)` — array: static length; string: byte length. Returns `int`.

## Tooling contract (AI-native)

- `axon build file.ax [-o out] [--O0]` — native executable (LLVM O3 default).
- `axon run file.ax [-- args...]` — compile and run.
- `axon ir file.ax` — print the optimized LLVM IR.
- `--json` — diagnostics as `{"ok":false,"errors":[{"stage","line","col","message"}]}`.
- `AXON_DUMP_IR=1` — dump unoptimized IR to stderr before verification.

Diagnostics stages: `lex`, `parse`, `type`, `internal`, `link`, `io`.

## Performance

Axon compiles through LLVM O3 to native machine code — measured equal to
`clang -O3` on the same program (see `examples/benchmark.ax`).

## Roadmap

1. String indexing / iteration (needs a `char` type or substring slices).
2. `for` loops over ranges/iterables; f-string interpolation.
3. Memory: string interning or arena freeing (currently concatenation leaks).
4. Core library written **in Axon itself** (algorithms, data structures).
5. Self-hosting: rewrite the compiler in Axon.
6. Standard library: math, crypto (保密性), collections, IO.
7. Top-level statements as an implicit `main` (module-script mode).
