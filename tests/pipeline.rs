//! End-to-end pipeline tests: compile Axon source -> native exe -> run -> check output.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use axon::build_exe;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Strip the common leading indentation (and leading blank lines) from an
/// embedded source string, so tests can stay indented inside Rust code.
fn dedent(src: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let body: Vec<&&str> = lines
        .iter()
        .skip_while(|l| l.trim().is_empty())
        .collect();
    let min_indent = body
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut out: Vec<String> = body
        .iter()
        .map(|l| if l.len() >= min_indent { l[min_indent..].to_string() } else { l.to_string() })
        .collect();
    // drop trailing blank lines, keep exactly one trailing newline
    while out.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        out.pop();
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

fn build_and_run(src: &str) -> String {
    let src = &dedent(src);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst) + std::process::id() as usize;
    let dir = std::env::temp_dir().join("axon-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let exe: PathBuf = dir.join(format!("t{id}.exe"));

    match build_exe(src, &exe, true) {
        Ok(()) => {}
        Err(diags) => panic!("compilation failed: {:?}", diags),
    }

    let out = Command::new(&exe).output().expect("failed to run compiled program");
    let _ = std::fs::remove_file(&exe);
    let _ = std::fs::remove_file(exe.with_extension("obj"));
    assert!(
        out.status.success(),
        "program exited with {:?}, stderr: {:?}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn expect_compile_error(src: &str) -> String {
    let src = &dedent(src);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst) + std::process::id() as usize;
    let dir = std::env::temp_dir().join("axon-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let exe: PathBuf = dir.join(format!("t{id}.exe"));
    match build_exe(src, &exe, true) {
        Ok(()) => panic!("expected compilation to fail, but it succeeded"),
        Err(diags) => diags[0].message.clone(),
    }
}

#[test]
fn hello_world() {
    let out = build_and_run(
        r#"
        # the classic
        def main() -> int:
            print("hello, axon")
            return 0
        "#,
    );
    assert_eq!(out, "hello, axon\n");
}

#[test]
fn arithmetic_and_precedence() {
    let out = build_and_run(
        r#"
        def main() -> int:
            a = 2 + 3 * 4        # 14
            b = (2 + 3) * 4      # 20
            c = 17 / 5           # 3
            d = 17 % 5           # 2
            print(a)
            print(b)
            print(c)
            print(d)
            return 0
        "#,
    );
    assert_eq!(out, "14\n20\n3\n2\n");
}

#[test]
fn floats_and_negative() {
    let out = build_and_run(
        r#"
        def main() -> int:
            x: float = 1.5 * 2.0
            y = -x
            print(x)
            print(y)
            return 0
        "#,
    );
    assert_eq!(out, "3.000000\n-3.000000\n");
}

#[test]
fn recursion_fib() {
    let out = build_and_run(
        r#"
        def fib(n: int) -> int:
            if n < 2:
                return n
            return fib(n - 1) + fib(n - 2)

        def main() -> int:
            print(fib(20))
            return 0
        "#,
    );
    assert_eq!(out, "6765\n");
}

#[test]
fn while_loop_and_mutation() {
    let out = build_and_run(
        r#"
        def main() -> int:
            total = 0
            i = 1
            while i <= 100:
                total = total + i
                i = i + 1
            print(total)
            return 0
        "#,
    );
    assert_eq!(out, "5050\n");
}

#[test]
fn elif_else_chains() {
    let out = build_and_run(
        r#"
        def classify(n: int) -> string:
            if n < 0:
                return "negative"
            elif n == 0:
                return "zero"
            else:
                return "positive"

        def main() -> int:
            print(classify(-5))
            print(classify(0))
            print(classify(7))
            return 0
        "#,
    );
    assert_eq!(out, "negative\nzero\npositive\n");
}

#[test]
fn bool_logic_and_short_circuit() {
    let out = build_and_run(
        r#"
        def boom() -> bool:
            print("BOOM")
            return True

        def main() -> int:
            a = True and True
            b = False or False
            print(a)
            print(b)
            print(not a)
            if False and boom():
                print("unreachable")
            if True or boom():
                print("ok")
            return 0
        "#,
    );
    assert_eq!(out, "true\nfalse\nfalse\nok\n");
}

#[test]
fn mutual_recursion() {
    let out = build_and_run(
        r#"
        def is_even(n: int) -> bool:
            if n == 0:
                return True
            return is_odd(n - 1)

        def is_odd(n: int) -> bool:
            if n == 0:
                return False
            return is_even(n - 1)

        def main() -> int:
            print(is_even(10))
            print(is_even(7))
            return 0
        "#,
    );
    assert_eq!(out, "true\nfalse\n");
}

#[test]
fn exit_code_propagates() {
    let dir = std::env::temp_dir().join("axon-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join(format!("exit-{}.exe", std::process::id()));
    build_exe("def main() -> int: return 42", &exe, true).unwrap();
    let status = Command::new(&exe).status().unwrap();
    let _ = std::fs::remove_file(&exe);
    assert_eq!(status.code(), Some(42));
}

#[test]
fn comparison_ops() {
    let out = build_and_run(
        r#"
        def main() -> int:
            print(1 < 2)
            print(2 <= 2)
            print(3 > 4)
            print(4 >= 5)
            print(1 == 1)
            print(1 != 1)
            print(1.5 < 2.5)
            return 0
        "#,
    );
    assert_eq!(out, "true\ntrue\nfalse\nfalse\ntrue\nfalse\ntrue\n");
}

#[test]
fn pass_statement() {
    let out = build_and_run(
        r#"
        def main() -> int:
            # empty branches need `pass`
            if False:
                pass
            else:
                print("else ran")
            return 0
        "#,
    );
    assert_eq!(out, "else ran\n");
}

#[test]
fn multiline_call_arguments() {
    let out = build_and_run(
        r#"
        def add(a: int, b: int) -> int:
            return a + b

        def main() -> int:
            print(
                add(
                    20,
                    22,
                )
            )
            return 0
        "#,
    );
    assert_eq!(out, "42\n");
}

#[test]
fn annotated_binding_reassign() {
    let out = build_and_run(
        r#"
        def main() -> int:
            x: int = 10
            x = x + 5          # re-assignment keeps the type
            print(x)
            return 0
        "#,
    );
    assert_eq!(out, "15\n");
}

// ---- compile error tests (strict type system) ----

#[test]
fn rejects_int_float_mixing() {
    let msg = expect_compile_error("def main() -> int:\n    x: int = 1 + 1.5\n    return 0");
    assert!(msg.contains("requires two int or two float"), "{msg}");
}

#[test]
fn rejects_non_bool_condition() {
    let msg = expect_compile_error("def main() -> int:\n    if 1:\n        print(1)\n    return 0");
    assert!(msg.contains("'if' condition must be bool"), "{msg}");
}

#[test]
fn rejects_undefined_variable() {
    let msg = expect_compile_error("def main() -> int:\n    print(x)\n    return 0");
    assert!(msg.contains("unknown variable 'x'"), "{msg}");
}

#[test]
fn rejects_wrong_arg_type() {
    let msg = expect_compile_error(
        "def f(x: int) -> int:\n    return x\n\ndef main() -> int:\n    print(f(1.5))\n    return 0",
    );
    assert!(msg.contains("argument 1 of 'f' must be int, found float"), "{msg}");
}

#[test]
fn rejects_missing_return() {
    let msg = expect_compile_error("def f() -> int:\n    x = 1\n\ndef main() -> int:\n    return 0");
    assert!(msg.contains("does not return a value on all paths"), "{msg}");
}

#[test]
fn rejects_unreachable_code() {
    let msg = expect_compile_error("def main() -> int:\n    return 0\n    print(1)");
    assert!(msg.contains("unreachable"), "{msg}");
}

#[test]
fn rejects_type_change_on_rebind() {
    let msg = expect_compile_error(
        "def main() -> int:\n    x = 1\n    x: float = 2.0\n    return 0",
    );
    assert!(msg.contains("cannot re-declare 'x' as float: it is already int"), "{msg}");
}

#[test]
fn rejects_assign_wrong_type() {
    let msg = expect_compile_error(
        "def main() -> int:\n    x = 1\n    x = \"hello\"\n    return 0",
    );
    assert!(msg.contains("cannot assign a value of type string to 'x: int'"), "{msg}");
}

#[test]
fn rejects_missing_main() {
    let msg = expect_compile_error("def helper() -> int:\n    return 1");
    assert!(msg.contains("no 'main' function"), "{msg}");
}

#[test]
fn rejects_bad_indentation() {
    let msg = expect_compile_error(
        "def main() -> int:\n    x = 1\n   y = 2\n    return 0",
    );
    assert!(msg.contains("unindent does not match"), "{msg}");
}

#[test]
fn rejects_top_level_statement() {
    let msg = expect_compile_error("print(1)");
    assert!(msg.contains("expected 'def' or 'struct'"), "{msg}");
}

// ---- arrays (v0.3) ----

#[test]
fn array_literal_index_len() {
    let out = build_and_run(
        r#"
        def main() -> int:
            a = [10, 20, 30]
            print(a[0])
            print(a[2])
            print(len(a))
            a[1] = 99
            print(a[1])
            return 0
        "#,
    );
    assert_eq!(out, "10\n30\n3\n99\n");
}

#[test]
fn array_types_and_params() {
    let out = build_and_run(
        r#"
        def total(xs: [int; 4]) -> int:
            t = 0
            i = 0
            while i < len(xs):
                t = t + xs[i]
                i = i + 1
            return t

        def make() -> [int; 4]:
            return [5, 10, 15, 20]

        def main() -> int:
            xs: [int; 4] = [1, 2, 3, 4]
            print(total(xs))
            print(total(make()))
            ys = [1.5, 2.5]
            print(ys[0] + ys[1])
            return 0
        "#,
    );
    assert_eq!(out, "10\n50\n4.000000\n");
}

#[test]
fn array_value_semantics() {
    let out = build_and_run(
        r#"
        def main() -> int:
            a = [1, 2, 3]
            b = a          # copy, not reference
            b[0] = 99
            print(a[0])
            print(b[0])
            return 0
        "#,
    );
    assert_eq!(out, "1\n99\n");
}

#[test]
fn nested_arrays() {
    let out = build_and_run(
        r#"
        def main() -> int:
            m: [[int; 2]; 3] = [[1, 2], [3, 4], [5, 6]]
            m[1][0] = 30
            print(m[1][0] + m[0][1] + m[2][1])
            return 0
        "#,
    );
    assert_eq!(out, "38\n");
}

// ---- structs (v0.3) ----

#[test]
fn struct_basic() {
    let out = build_and_run(
        r#"
        struct Point:
            x: int
            y: int

        def main() -> int:
            p = Point(x=1, y=2)
            print(p.x + p.y)
            p.x = 10
            print(p.x)
            return 0
        "#,
    );
    assert_eq!(out, "3\n10\n");
}

#[test]
fn struct_value_semantics() {
    let out = build_and_run(
        r#"
        struct Point:
            x: int
            y: int

        def main() -> int:
            p = Point(x=1, y=2)
            q = p           # copy
            q.x = 99
            print(p.x)
            print(q.x)
            return 0
        "#,
    );
    assert_eq!(out, "1\n99\n");
}

#[test]
fn struct_params_and_return() {
    let out = build_and_run(
        r#"
        struct Point:
            x: int
            y: int

        def add(a: Point, b: Point) -> Point:
            return Point(x=a.x + b.x, y=a.y + b.y)

        def main() -> int:
            r = add(Point(x=1, y=2), Point(x=10, y=20))
            print(r.x)
            print(r.y)
            return 0
        "#,
    );
    assert_eq!(out, "11\n22\n");
}

#[test]
fn struct_nested_and_forward_reference() {
    let out = build_and_run(
        r#"
        # Outer references Inner before it is defined
        struct Outer:
            inner: Inner
            flag: bool

        struct Inner:
            v: int

        def main() -> int:
            o = Outer(inner=Inner(v=41), flag=True)
            o.inner.v = o.inner.v + 1
            print(o.inner.v)
            print(o.flag)
            return 0
        "#,
    );
    assert_eq!(out, "42\ntrue\n");
}

#[test]
fn struct_with_array_field() {
    let out = build_and_run(
        r#"
        struct Vec3:
            data: [float; 3]
            tag: int

        def dot(a: Vec3, b: Vec3) -> float:
            return a.data[0] * b.data[0] + a.data[1] * b.data[1] + a.data[2] * b.data[2]

        def main() -> int:
            u = Vec3(data=[1.0, 2.0, 3.0], tag=1)
            v = Vec3(data=[4.0, 5.0, 6.0], tag=2)
            print(dot(u, v))
            print(u.data[2])
            return 0
        "#,
    );
    assert_eq!(out, "32.000000\n3.000000\n");
}

#[test]
fn array_of_structs() {
    let out = build_and_run(
        r#"
        struct Point:
            x: int
            y: int

        def main() -> int:
            pts: [Point; 2] = [Point(x=1, y=2), Point(x=3, y=4)]
            pts[1].x = 30
            print(pts[0].x + pts[1].x + pts[1].y)
            return 0
        "#,
    );
    assert_eq!(out, "35\n");
}

#[test]
fn struct_used_in_conditional() {
    let out = build_and_run(
        r#"
        struct Box:
            w: int
            h: int

        def area(b: Box) -> int:
            return b.w * b.h

        def main() -> int:
            b = Box(w=3, h=4)
            if area(b) > 10:
                print("big")
            else:
                print("small")
            return 0
        "#,
    );
    assert_eq!(out, "big\n");
}

// ---- array/struct compile error tests ----

#[test]
fn rejects_print_struct() {
    let msg = expect_compile_error(
        "struct P:\n    x: int\n\ndef main() -> int:\n    print(P(x=1))\n    return 0",
    );
    assert!(msg.contains("print requires int, float, bool, or string"), "{msg}");
}

#[test]
fn rejects_compare_compound() {
    let msg = expect_compile_error(
        "def main() -> int:\n    a = [1, 2]\n    b = [1, 2]\n    if a == b:\n        print(1)\n    return 0",
    );
    assert!(msg.contains("cannot compare compound type"), "{msg}");
}

#[test]
fn rejects_unknown_struct_field() {
    let msg = expect_compile_error(
        "struct P:\n    x: int\n\ndef main() -> int:\n    p = P(x=1)\n    print(p.z)\n    return 0",
    );
    assert!(msg.contains("has no field 'z'"), "{msg}");
}

#[test]
fn rejects_missing_struct_field() {
    let msg = expect_compile_error(
        "struct P:\n    x: int\n    y: int\n\ndef main() -> int:\n    p = P(x=1)\n    return 0",
    );
    assert!(msg.contains("missing field(s): y"), "{msg}");
}

#[test]
fn rejects_recursive_struct() {
    let msg = expect_compile_error(
        "struct Node:\n    next: Node\n\ndef main() -> int:\n    return 0",
    );
    assert!(msg.contains("recursive struct"), "{msg}");
}

#[test]
fn rejects_index_non_array() {
    let msg = expect_compile_error(
        "def main() -> int:\n    x = 5\n    print(x[0])\n    return 0",
    );
    assert!(msg.contains("cannot index a value of type int"), "{msg}");
}

#[test]
fn rejects_zero_len_array() {
    let msg = expect_compile_error(
        "def main() -> int:\n    a: [int; 0] = [1]\n    return 0",
    );
    assert!(msg.contains("array length must be a positive integer"), "{msg}");
}

#[test]
fn rejects_struct_kwarg_on_fn() {
    let msg = expect_compile_error(
        "def f(x: int) -> int:\n    return x\n\ndef main() -> int:\n    print(f(x=1))\n    return 0",
    );
    assert!(msg.contains("takes positional arguments only"), "{msg}");
}

// ---- strings (v0.4) ----

#[test]
fn string_concat() {
    let out = build_and_run(
        r#"
        def greet(name: string) -> string:
            return "hi, " + name + "!"

        def main() -> int:
            s = "hello" + ", " + "world"
            print(s)
            print(greet("axon"))
            t = ""
            t = t + "x" + "y" + "z"
            print(t)
            print(len(s))
            print(len(t))
            return 0
        "#,
    );
    assert_eq!(out, "hello, world\nhi, axon!\nxyz\n12\n3\n");
}

#[test]
fn string_equality() {
    let out = build_and_run(
        r#"
        def same(a: string, b: string) -> bool:
            return a == b

        def main() -> int:
            print("abc" == "abc")
            print("abc" == "abd")
            print("abc" != "abd")
            print(same("hello", "hello"))
            print(same("hello", "hell"))
            print("" == "")
            return 0
        "#,
    );
    assert_eq!(out, "true\nfalse\ntrue\ntrue\nfalse\ntrue\n");
}

#[test]
fn string_ordering() {
    let out = build_and_run(
        r#"
        def main() -> int:
            print("apple" < "banana")
            print("banana" < "apple")
            print("abc" <= "abc")
            print("abd" > "abc")
            print("abc" >= "abd")
            print("" < "a")
            return 0
        "#,
    );
    assert_eq!(out, "true\nfalse\ntrue\ntrue\nfalse\ntrue\n");
}

#[test]
fn strings_in_structs_and_arrays() {
    let out = build_and_run(
        r#"
        struct User:
            name: string
            id: int

        def main() -> int:
            u = User(name="li lei", id=7)
            print(u.name)
            print(u.id)
            tag = "user#" + u.name
            print(tag)
            names: [string; 3] = ["al", "bo", "cy"]
            print(names[1])
            print(names[0] < names[1])
            print(len(names))
            reps: [string; 2] = ["hi"] * 2
            print(reps[0] + reps[1])
            return 0
        "#,
    );
    assert_eq!(out, "li lei\n7\nuser#li lei\nbo\ntrue\n3\nhihi\n");
}

#[test]
fn string_concat_in_loop() {
    let out = build_and_run(
        r#"
        def join3(a: string, b: string, c: string) -> string:
            return a + b + c

        def main() -> int:
            acc = ""
            i = 0
            while i < 3:
                acc = acc + "ab"
                i = i + 1
            print(acc)
            print(len(acc))
            print(join3("x", "y", "z"))
            return 0
        "#,
    );
    assert_eq!(out, "ababab\n6\nxyz\n");
}

#[test]
fn rejects_concat_string_int() {
    let msg = expect_compile_error(
        "def main() -> int:\n    s = \"a\" + 1\n    return 0",
    );
    assert!(msg.contains("cannot concatenate string with int"), "{msg}");
}

#[test]
fn rejects_compare_string_int() {
    let msg = expect_compile_error(
        "def main() -> int:\n    if \"a\" < 1:\n        print(1)\n    return 0",
    );
    assert!(msg.contains("two int, two float, or two string"), "{msg}");
}
