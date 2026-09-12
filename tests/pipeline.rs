//! End-to-end pipeline tests: compile Aoxn source -> native exe -> run -> check output.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use aoxn::build_exe;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// the in-Aoxn standard library (compiled together with stdlib tests)
fn stdlib_src() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("stdlib").join("stdlib.ax");
    std::fs::read_to_string(path).expect("stdlib/stdlib.ax not found")
}

fn build_and_run_with_stdlib(src: &str) -> String {
    let full = format!("{}\n{}", stdlib_src(), dedent(src));
    build_and_run(&full)
}

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
    let dir = std::env::temp_dir().join("Aoxn-tests");
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
    let dir = std::env::temp_dir().join("Aoxn-tests");
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
            print("hello, Aoxn")
            return 0
        "#,
    );
    assert_eq!(out, "hello, Aoxn\n");
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
    let dir = std::env::temp_dir().join("Aoxn-tests");
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
fn comparison_boundary_cases() {
    // regression: '>=' was once lexed as '>'
    let out = build_and_run(
        r#"
        def ge(c: int) -> bool:
            return c >= 65

        def le(c: int) -> bool:
            return c <= 65

        def main() -> int:
            print(5 >= 5)         # boundary: equal
            print(5 >= 6)
            print(5 > 5)          # strict: false
            print(5 <= 5)         # boundary: equal
            print(4 <= 5)
            print(5 < 5)
            print(ge(65))
            print(ge(64))
            print(ge(66))
            print(le(65))
            print(le(66))
            print(le(64))
            return 0
        "#,
    );
    assert_eq!(out, "true\nfalse\nfalse\ntrue\ntrue\nfalse\ntrue\nfalse\ntrue\ntrue\nfalse\ntrue\n");
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
    assert!(msg.contains("expected 'import', 'def' or 'struct'"), "{msg}");
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
            print(greet("Aoxn"))
            t = ""
            t = t + "x" + "y" + "z"
            print(t)
            print(len(s))
            print(len(t))
            return 0
        "#,
    );
    assert_eq!(out, "hello, world\nhi, Aoxn!\nxyz\n12\n3\n");
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

// ---- for loops / break / continue / f-strings (v0.5) ----

#[test]
fn for_range_forms() {
    let out = build_and_run(
        r#"
        def main() -> int:
            for i in range(4):
                print(i)
            for i in range(2, 5):
                print(i)
            for i in range(0, 10, 3):
                print(i)
            for i in range(5, 0, -1):
                print(i)
            return 0
        "#,
    );
    assert_eq!(out, "0\n1\n2\n3\n2\n3\n4\n0\n3\n6\n9\n5\n4\n3\n2\n1\n");
}

#[test]
fn for_array_iteration() {
    let out = build_and_run(
        r#"
        struct P:
            v: int

        def main() -> int:
            total = 0
            for x in [10, 20, 30]:
                total = total + x
            print(total)
            pts: [P; 2] = [P(v=5), P(v=7)]
            s = 0
            for p in pts:
                s = s + p.v
            print(s)
            names = ["ab", "cde"]
            lens = 0
            for n in names:
                lens = lens + len(n)
            print(lens)
            return 0
        "#,
    );
    assert_eq!(out, "60\n12\n5\n");
}

#[test]
fn for_nested_and_reuse() {
    let out = build_and_run(
        r#"
        def main() -> int:
            count = 0
            for i in range(3):
                for j in range(3):
                    count = count + 1
            print(count)
            for i in range(2):
                print(i)
            for i in range(2, 4):
                print(i)
            return 0
        "#,
    );
    assert_eq!(out, "9\n0\n1\n2\n3\n");
}

#[test]
fn break_and_continue() {
    let out = build_and_run(
        r#"
        def main() -> int:
            i = 0
            while True:
                i = i + 1
                if i == 3:
                    break
            print(i)
            total = 0
            for i in range(10):
                if i % 2 == 0:
                    continue
                total = total + i
            print(total)
            found = -1
            for j in range(20):
                if j * j > 50:
                    found = j
                    break
            print(found)
            return 0
        "#,
    );
    assert_eq!(out, "3\n25\n8\n");
}

#[test]
fn f_string_basics() {
    let out = build_and_run(
        r#"
        def main() -> int:
            name = "Aoxn"
            version = 5
            print(f"hello {name}!")
            print(f"v{version}, {version * 2}")
            print(f"{True}/{False}")
            print(f"braces: {{literal}}")
            print(f"expr: {[1, 2, 3][1] + len(name)}")
            print(f"")
            print(f"just text")
            return 0
        "#,
    );
    assert_eq!(out, "hello Aoxn!\nv5, 10\ntrue/false\nbraces: {literal}\nexpr: 6\n\njust text\n");
}

#[test]
fn f_string_with_function_calls() {
    let out = build_and_run(
        r#"
        def double(n: int) -> int:
            return n * 2

        def main() -> int:
            for i in range(1, 4):
                print(f"{i} -> {double(i)}")
            s = f"{double(21)}"
            print(s)
            print(len(s))
            return 0
        "#,
    );
    assert_eq!(out, "1 -> 2\n2 -> 4\n3 -> 6\n42\n2\n");
}

#[test]
fn str_builtin() {
    let out = build_and_run(
        r#"
        def main() -> int:
            print(str(42))
            print(str(-7))
            print(str(True))
            print(str(False))
            print(str(1.5))
            print(str("already"))
            print(len(str(12345)))
            print(str(2 + 3) == "5")
            return 0
        "#,
    );
    assert_eq!(out, "42\n-7\ntrue\nfalse\n1.500000\nalready\n5\ntrue\n");
}

#[test]
fn rejects_break_outside_loop() {
    let msg = expect_compile_error(
        "def main() -> int:\n    break\n    return 0",
    );
    assert!(msg.contains("'break' outside of a loop"), "{msg}");
}

#[test]
fn rejects_continue_outside_loop() {
    let msg = expect_compile_error(
        "def main() -> int:\n    continue\n    return 0",
    );
    assert!(msg.contains("'continue' outside of a loop"), "{msg}");
}

#[test]
fn rejects_range_float_arg() {
    let msg = expect_compile_error(
        "def main() -> int:\n    for i in range(1.5):\n        print(i)\n    return 0",
    );
    assert!(msg.contains("range arguments must be int"), "{msg}");
}

#[test]
fn rejects_for_over_int() {
    let msg = expect_compile_error(
        "def main() -> int:\n    for x in 5:\n        print(x)\n    return 0",
    );
    assert!(msg.contains("'for' can only iterate over arrays"), "{msg}");
}

#[test]
fn rejects_fstring_of_array() {
    let msg = expect_compile_error(
        "def main() -> int:\n    print(f\"{[1, 2]}\")\n    return 0",
    );
    assert!(msg.contains("cannot convert [int; 2] to string"), "{msg}");
}

// ---- standard library (written in Aoxn itself, v0.7 generics) ----

#[test]
fn stdlib_math() {
    let out = build_and_run_with_stdlib(
        r#"
        def main() -> int:
            print(sqrt(4.0))
            print(sqrt(2.0))
            print(isqrt(99))
            print(isqrt(100))
            print(pow_i(3, 4))
            print(pow_i(2, 10))
            print(gcd(48, 18))
            print(gcd(-48, 18))
            print(lcm(4, 6))
            print(lcm(0, 5))
            print(is_prime(97))
            print(is_prime(98))
            print(is_prime(2))
            print(abs_i(-5))
            print(hypot(3.0, 4.0))
            print(clamp_i(15, 0, 10))
            print(clamp_f(0.5, 1.0, 2.0))
            return 0
        "#,
    );
    assert_eq!(
        out,
        "2.000000\n1.414214\n9\n10\n81\n1024\n6\n6\n12\n0\ntrue\nfalse\ntrue\n5\n5.000000\n10\n1.000000\n"
    );
}

#[test]
fn stdlib_generic_sort_search() {
    let out = build_and_run_with_stdlib(
        r#"
        def main() -> int:
            arr = [5, 3, 8, 1, 9, 2, 7, 4]
            sorted_arr = sort(arr)
            print(arr[0])                    # input untouched: value semantics
            print(sum_int(arr))
            print(max_of(arr))
            print(min_of(arr))
            print(linear_search(arr, 8))
            print(linear_search(arr, 42))
            print(binary_search(sorted_arr, 8))
            print(binary_search(sorted_arr, 6))
            print(reverse(arr)[0])
            # same functions, other types and lengths
            print(sort([3, 1])[0])
            print(sort([2.5, 1.5, 0.5])[0])
            print(sort(["pear", "apple", "fig"])[0])
            print(binary_search([10, 20, 30], 20))
            print(sum_float([1.5, 2.5]))
            print(max_of(["pear", "apple"]))
            return 0
        "#,
    );
    assert_eq!(out, "5\n39\n9\n1\n2\n-1\n6\n-1\n4\n1\n0.500000\napple\n1\n4.000000\npear\n");
}

#[test]
fn stdlib_sort_does_not_mutate() {
    let out = build_and_run_with_stdlib(
        r#"
        def main() -> int:
            a = [9, 8, 7]
            b = sort(a)
            print(a[0])
            print(b[0])
            print(b[2])
            return 0
        "#,
    );
    assert_eq!(out, "9\n7\n9\n");
}

#[test]
fn rejects_generic_struct_sort() {
    let msg = expect_compile_error(
        "def sort[T, N](arr: [T; N]) -> [T; N]:\n    result = arr\n    for i in range(N):\n        for j in range(N - 1 - i):\n            if result[j] > result[j + 1]:\n                t = result[j]\n                result[j] = result[j + 1]\n                result[j + 1] = t\n    return result\n\nstruct P:\n    v: int\n\ndef main() -> int:\n    r = sort([P(v=1)])\n    return 0",
    );
    assert!(msg.contains("requires two int, two float, or two string operands, found (P, P)"), "{msg}");
}

#[test]
fn rejects_extern_main() {
    let msg = expect_compile_error(
        "extern def main() -> int",
    );
    assert!(msg.contains("'main' cannot be declared extern"), "{msg}");
}

#[test]
fn rejects_generic_main() {
    let msg = expect_compile_error(
        "def main[T](x: T) -> T:\n    return x",
    );
    assert!(msg.contains("'main' cannot be generic"), "{msg}");
}

#[test]
fn rejects_len_param_outside_generic() {
    let msg = expect_compile_error(
        "def f(arr: [int; N]) -> int:\n    return 0\n\ndef main() -> int:\n    return 0",
    );
    assert!(msg.contains("unknown array length 'N'"), "{msg}");
}

#[test]
fn rejects_uninferable_len() {
    let msg = expect_compile_error(
        "def make[T, N]() -> [T; N]:\n    return [0]\n\ndef main() -> int:\n    print(make())\n    return 0",
    );
    assert!(msg.contains("cannot infer array length N"), "{msg}");
}

// ---- import / module system (v0.8) ----

fn tmp_dir(tag: &str) -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst) + std::process::id() as usize;
    let dir = std::env::temp_dir().join(format!("Aoxn-import-{tag}-{id}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn import_transitive_and_include_once() {
    let dir = tmp_dir("transitive");
    std::fs::write(
        dir.join("main.ax"),
        "import \"lib.ax\"\nimport \"lib.ax\"\nimport \"sub/deep.ax\"\n\ndef main() -> int:\n    print(helper() + deep())\n    return 0\n",
    )
    .unwrap();
    std::fs::write(dir.join("lib.ax"), "def helper() -> int:\n    return 40\n").unwrap();
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub").join("deep.ax"), "def deep() -> int:\n    return 2\n").unwrap();

    let exe = dir.join("out.exe");
    aoxn::build_paths_exe(
        &[dir.join("main.ax").display().to_string()],
        &exe,
        true,
    )
    .expect("compilation failed");
    let out = Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "42\n");
}

// regression: per-file literal ids restart at 0; codegen literal temps must
// be keyed per AST site, not per id (aggregate literals in two files collided
// and referenced an alloca from the other function)
#[test]
fn import_aggregate_literals_across_files() {
    let dir = tmp_dir("litids");
    std::fs::write(dir.join("a.ax"), "def f() -> [int; 2]:\n    return [1, 2]\n").unwrap();
    std::fs::write(
        dir.join("main.ax"),
        "import \"a.ax\"\n\ndef h() -> [int; 2]:\n    return [3, 4]\n\ndef main() -> int:\n    x = [5, 6]\n    return f()[0] + h()[0] + x[0]\n",
    )
    .unwrap();

    let exe = dir.join("out.exe");
    aoxn::build_paths_exe(&[dir.join("main.ax").display().to_string()], &exe, true)
        .expect("compilation failed");
    let out = Command::new(&exe).output().unwrap();
    assert_eq!(out.status.code(), Some(9), "1 + 3 + 5");
}

#[test]
fn import_cycle_detected() {
    let dir = tmp_dir("cycle");
    std::fs::write(dir.join("a.ax"), "import \"b.ax\"\n\ndef fa() -> int:\n    return fb() + 1\n").unwrap();
    std::fs::write(dir.join("b.ax"), "import \"a.ax\"\n\ndef fb() -> int:\n    return 1\n").unwrap();

    let exe = dir.join("out.exe");
    match aoxn::build_paths_exe(&[dir.join("a.ax").display().to_string()], &exe, true) {
        Ok(()) => panic!("expected circular import error"),
        Err(diags) => {
            assert!(diags[0].message.contains("circular import"), "{:?}", diags[0]);
            assert!(diags[0].message.contains("a.ax"), "{:?}", diags[0]);
        }
    }
}

#[test]
fn import_missing_file() {
    let dir = tmp_dir("missing");
    std::fs::write(dir.join("main.ax"), "import \"nope.ax\"\n\ndef main() -> int:\n    return 0\n").unwrap();
    let exe = dir.join("out.exe");
    match aoxn::build_paths_exe(&[dir.join("main.ax").display().to_string()], &exe, true) {
        Ok(()) => panic!("expected import error"),
        Err(diags) => assert!(diags[0].message.contains("cannot open"), "{:?}", diags[0]),
    }
}

#[test]
fn import_error_reports_importing_file() {
    let dir = tmp_dir("errfile");
    std::fs::write(dir.join("main.ax"), "import \"lib.ax\"\n\ndef main() -> int:\n    print(helper())\n    return 0\n").unwrap();
    std::fs::write(dir.join("lib.ax"), "def helper() -> int:\n    print(unknown_var)\n    return 0\n").unwrap();
    let exe = dir.join("out.exe");
    match aoxn::build_paths_exe(&[dir.join("main.ax").display().to_string()], &exe, true) {
        Ok(()) => panic!("expected compile error"),
        Err(diags) => {
            let file = aoxn::files::name(diags[0].file);
            assert!(file.contains("lib.ax"), "{:?} / {file}", diags[0]);
            assert_eq!(diags[0].line, 2, "{:?}", diags[0]);
            assert!(diags[0].message.contains("unknown variable 'unknown_var'"), "{:?}", diags[0]);
        }
    }
}

#[test]
fn string_sources_reject_imports() {
    let msg = expect_compile_error("import \"somewhere.ax\"\n\ndef main() -> int:\n    return 0");
    assert!(msg.contains("requires compiling from files"), "{msg}");
}

// ---- stdlib v0.9: raw memory, Vec, buffers, file IO ----

#[test]
fn stdlib_vec_grow_and_slots() {
    let out = build_and_run_with_stdlib(
        r#"
        def main() -> int:
            v = vec_new()
            for i in range(100):
                v = vec_push(v, i * i)
            print(v.len)
            print(vec_get(v, 0))
            print(vec_get(v, 10))
            print(vec_get(v, 99))
            vec_set(v, 50, 7)
            print(vec_get(v, 50))
            vec_free(v)
            # strings via as_ptr / as_string
            sv = vec_new()
            sv = vec_push(sv, as_ptr("alpha"))
            sv = vec_push(sv, as_ptr("beta"))
            print(as_string(vec_get(sv, 0)))
            print(as_string(vec_get(sv, 1)))
            vec_free(sv)
            return 0
        "#,
    );
    assert_eq!(out, "100\n0\n100\n9801\n7\nalpha\nbeta\n");
}

#[test]
fn infer_binding_from_as_string_and_as_ptr() {
    // regression: unannotated bindings need codegen type hints for
    // as_string (-> string) and as_ptr (-> int)
    let out = build_and_run_with_stdlib(
        r#"
        def main() -> int:
            s = "roundtrip"
            p = as_ptr(s)
            t = as_string(p)
            print(t)
            print(len(t))
            return 0
        "#,
    );
    assert_eq!(out, "roundtrip\n9\n");
}

#[test]
fn stdlib_str_bytes_and_classes() {
    let out = build_and_run_with_stdlib(
        r#"
        def main() -> int:
            s = "Aoxn9!"
            print(str_get(s, 0))          # 'A' = 65
            print(str_get(s, 4))          # '9' = 57
            print(is_digit(str_get(s, 4)))
            print(is_digit(str_get(s, 0)))
            print(is_alpha(str_get(s, 0)))
            print(is_space(str_get(s, 5)))
            print(len(s))
            return 0
        "#,
    );
    assert_eq!(out, "65\n57\ntrue\nfalse\ntrue\nfalse\n6\n");
}

#[test]
fn stdlib_file_io_roundtrip() {
    let out = build_and_run_with_stdlib(
        r#"
        def main() -> int:
            path = "AOXN_stdlib_test.txt"
            ok = write_file(path, "hello from Aoxn")
            print(ok)
            content = read_file(path)
            print(content)
            print(len(content))
            print(read_file("definitely_missing_file_xyz.txt") == "")
            return 0
        "#,
    );
    assert!(out.starts_with("true\nhello from Aoxn\n15\ntrue\n"), "{out}");
}

#[test]
fn stdlib_system_spawn() {
    let out = build_and_run_with_stdlib(
        r#"
        def main() -> int:
            code = system("exit /b 7")
            print(code)
            return 0
        "#,
    );
    assert_eq!(out, "7\n");
}

// ---- self-hosting stage 1: the Aoxn lexer written in Aoxn (v0.10) ----

#[test]
fn selfhost_lexer_token_stream() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let demo = manifest.join("selfhost").join("lex_demo.ax");
    let exe = std::env::temp_dir()
        .join("axon-tests")
        .join(format!("selfhost-lex-{}.exe", std::process::id()));
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();

    aoxn::build_paths_exe(&[demo.display().to_string()], &exe, true)
        .expect("self-host lexer demo failed to compile");
    let out = Command::new(&exe).output().expect("failed to run");
    let _ = std::fs::remove_file(&exe);
    assert!(out.status.success(), "self-host lexer demo crashed: {:?}", out.status.code());

    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "DEF\nIDENT main\nLPAREN\nRPAREN\nARROW\nTYINT\nCOLON\nNEWLINE\n\
         INDENT\n\
         IDENT x\nASSIGN\nINT 1\nNEWLINE\n\
         IDENT print\nLPAREN\nSTR hi\nCOMMA\nFLOAT 2.5\nRPAREN\nNEWLINE\n\
         RETURN\nIDENT x\nNEWLINE\n\
         DEDENT\n\
         EOF\n"
    );
}

// ---- self-hosting stage 2: the Aoxn parser written in Aoxn (v0.11) ----

#[test]
fn selfhost_parser_ast_dump() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let demo = manifest.join("selfhost").join("parse_demo.ax");
    let exe = std::env::temp_dir()
        .join("axon-tests")
        .join(format!("selfhost-parse-{}.exe", std::process::id()));
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();

    aoxn::build_paths_exe(&[demo.display().to_string()], &exe, true)
        .expect("self-host parser demo failed to compile");
    let out = Command::new(&exe).output().expect("failed to run");
    let _ = std::fs::remove_file(&exe);
    assert!(out.status.success(), "self-host parser demo crashed: {:?}", out.status.code());

    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "BLOCK program\n\
         IMPORT \nSTR lib.ax\n\
         STRUCT P\nFIELD v\nTY-int \n\
         FN scale\n\
         tag4 \nVAR P2\n\
         PARAM arr\nTY-array  len=2\nTY-int \n\
         PARAM k\nTY-int \n\
         TY-int \n\
         BLOCK \n\
         LET total\nINT 0 0\n\
         tag11 x\nVAR arr\n\
         BLOCK \n\
         LET total\nBINARY \nVAR total\nBINARY \nVAR x\nVAR k\n\
         LET label\nBINARY \nSTR total=\nCALL str\nARG \nVAR total\n\
         IF \nBINARY \nVAR total\nINT 0 0\n\
         BLOCK \nRETURN \nVAR total\n\
         RETURN \ntag28 \nINT 1 1\n"
    );
}

// ---- self-hosting stage 3: the Aoxn type checker written in Aoxn (v0.11) ----

#[test]
fn selfhost_typechecker_accepts_and_rejects() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let demo = manifest.join("selfhost").join("tycheck_demo.ax");
    let exe = std::env::temp_dir()
        .join("axon-tests")
        .join(format!("selfhost-tycheck-{}.exe", std::process::id()));
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();

    aoxn::build_paths_exe(&[demo.display().to_string()], &exe, true)
        .expect("self-host typechecker demo failed to compile");
    let out = Command::new(&exe).output().expect("failed to run");
    let _ = std::fs::remove_file(&exe);
    assert!(out.status.success(), "self-host typechecker demo crashed: {:?}", out.status.code());

    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "typecheck OK\n\
         bad program rejected: '+' requires two int or two float operands, found (int, bool)\n\
         generics OK, instances:\n\
         \x20 wrap.i\n\
         \x20 wrap.b\n\
         \x20 id.i\n\
         \x20 id.b\n\
         bad generic call rejected: argument 2 of 'pair' must be T, found bool\n\
         length generics OK: first.i.?.3\n"
    );
}

// stage-2 exit criterion: the self-hosted front end (lexer + parser +
// checker) handles the entire stdlib in one program
#[test]
fn selfhost_frontend_handles_stdlib() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let stdlib = std::fs::read_to_string(manifest.join("stdlib").join("stdlib.ax")).unwrap();
    let mut checked = stdlib;
    checked.push_str("\ndef main() -> int:\n    return 0\n");
    let esc = checked
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");

    let abs = |p: &str| manifest.join(p).display().to_string().replace('\\', "/");
    let driver_src = format!(
        "import \"{}\"\nimport \"{}\"\nimport \"{}\"\n\n\
         def main() -> int:\n    \
         src = \"{esc}\"\n    \
         c = checker_new(lex_state_new(src))\n    \
         c = check_all(c)\n    \
         if c.err == 1:\n        \
         print(\"CHECK ERROR: \" + c.errmsg)\n        \
         return 1\n    \
         print(\"stdlib typechecks OK\")\n    \
         return 0\n",
        abs("selfhost/lexer.ax"),
        abs("selfhost/parser.ax"),
        abs("selfhost/typecheck.ax"),
    );

    let dir = std::env::temp_dir().join("axon-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let driver = dir.join(format!("selfhost-stdlib-{}.ax", std::process::id()));
    let exe = dir.join(format!("selfhost-stdlib-{}.exe", std::process::id()));
    std::fs::write(&driver, driver_src).unwrap();

    aoxn::build_paths_exe(&[driver.display().to_string()], &exe, true)
        .expect("self-host stdlib checker failed to compile");
    let out = Command::new(&exe).output().expect("failed to run");
    let _ = std::fs::remove_file(&exe);
    let _ = std::fs::remove_file(&driver);
    assert!(out.status.success(), "self-host stdlib checker crashed: {:?}", out.status.code());

    assert_eq!(String::from_utf8_lossy(&out.stdout), "stdlib typechecks OK\n");
}

// ---- self-hosting stage 4: codegen slice 1 (int functions -> native exe) ----

fn llvm_dir() -> Option<PathBuf> {
    for key in ["AOXN_LLVM_DIR", "AXON_LLVM_DIR"] {
        if let Ok(v) = std::env::var(key) {
            let p = PathBuf::from(v);
            if p.exists() {
                return Some(p);
            }
        }
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("LLVM");
    if repo.exists() {
        return Some(repo);
    }
    let p = PathBuf::from("C:/Program Files/LLVM");
    if p.exists() {
        return Some(p);
    }
    None
}

#[test]
fn selfhost_codegen_int_slice() {
    let llvm = llvm_dir().expect("LLVM install not found (set AOXN_LLVM_DIR)");
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let demo = manifest.join("selfhost").join("codegen_demo.ax");
    let dir = std::env::temp_dir().join("axon-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join(format!("selfhost-cg-{}.exe", std::process::id()));

    aoxn::build_paths_opts(
        &[demo.display().to_string()],
        &exe,
        true,
        &["LLVM-C".to_string()],
        &[llvm.join("lib").display().to_string()],
    )
    .expect("self-host codegen demo failed to compile");

    let path = format!(
        "{};{}",
        llvm.join("bin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = Command::new(&exe)
        .current_dir(&dir)
        .env("PATH", &path)
        .output()
        .expect("failed to run codegen demo");
    let _ = std::fs::remove_file(&exe);
    assert!(
        out.status.success(),
        "self-host codegen demo crashed: {:?} {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "codegen OK\n");

    // link the self-hosted object and run it
    let obj = dir.join("selfhost_out.obj");
    assert!(obj.exists(), "self-hosted object was not written");
    let exe_self = dir.join(format!("selfhost-cg-out-{}.exe", std::process::id()));
    aoxn::link(&obj, &exe_self).expect("linking the self-hosted object failed");
    let out_self = Command::new(&exe_self).output().expect("failed to run self-hosted exe");
    let _ = std::fs::remove_file(&exe_self);
    let _ = std::fs::remove_file(&obj);

    // parity: the Rust compiler must produce the same stdout and exit code
    let src = "def twice[T](x: T) -> T:\n    return x + x\n\ndef add(a: int, b: int) -> int:\n    return a + b\n\ndef fact(n: int) -> int:\n    if n <= 1:\n        return 1\n    return n * fact(n - 1)\n\ndef main() -> int:\n    t = twice(3)\n    for i in range(1, 6):\n        t = t + i * i\n    while t > 50:\n        t = t - 10\n    ok = t == 41\n    print(t)\n    print(fact(4))\n    print(t > 40)\n    print(ok)\n    print(-t)\n    return add(t, fact(4)) % 100\n";
    let exe_rust = dir.join(format!("selfhost-cg-rs-{}.exe", std::process::id()));
    aoxn::build_exe(src, &exe_rust, true).expect("rust reference compile failed");
    let out_rust = Command::new(&exe_rust).output().expect("failed to run rust reference");
    let _ = std::fs::remove_file(&exe_rust);

    assert_eq!(String::from_utf8_lossy(&out_self.stdout), "41\n24\ntrue\ntrue\n-41\n");
    assert_eq!(out_self.status.code(), Some(65));
    assert_eq!(out_rust.stdout, out_self.stdout);
    assert_eq!(out_rust.status.code(), out_self.status.code());
}

// ---- self-hosting: multi-file import resolution in the Aoxn front end ----

#[test]
fn selfhost_frontend_handles_imports() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let abs = |p: &str| manifest.join(p).display().to_string().replace('\\', "/");

    // fixture tree: diamond include-once, a cycle, and a missing import.
    // NB: the self-hosted loader opens paths with narrow fopen, so keep the
    // fixture directory ASCII (temp_dir can contain non-ASCII user names).
    let dir = manifest
        .join("target")
        .join(format!("shload-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("main.ax"),
        "import \"b.ax\"\nimport \"c.ax\"\n\ndef main() -> int:\n    return b_val() + c_val()\n",
    )
    .unwrap();
    std::fs::write(dir.join("b.ax"), "import \"d.ax\"\n\ndef b_val() -> int:\n    return d_val() + 1\n").unwrap();
    std::fs::write(dir.join("c.ax"), "import \"d.ax\"\n\ndef c_val() -> int:\n    return d_val() + 2\n").unwrap();
    std::fs::write(dir.join("d.ax"), "def d_val() -> int:\n    return 10\n").unwrap();
    std::fs::write(dir.join("cyc_a.ax"), "import \"cyc_b.ax\"\n\ndef fa() -> int:\n    return 0\n").unwrap();
    std::fs::write(dir.join("cyc_b.ax"), "import \"cyc_a.ax\"\n\ndef fb() -> int:\n    return 0\n").unwrap();
    std::fs::write(dir.join("miss.ax"), "import \"nope.ax\"\n\ndef fm() -> int:\n    return 0\n").unwrap();
    let fp = |name: &str| dir.join(name).display().to_string().replace('\\', "/");

    let driver_src = format!(
        "import \"{}\"\nimport \"{}\"\nimport \"{}\"\nimport \"{}\"\n\n\
         def main() -> int:\n    \
         r1 = load_program(\"{demo}\")\n    \
         if r1.err == 1:\n        \
         print(\"demo load err: \" + r1.errmsg)\n        \
         return 1\n    \
         c1 = checker_state()\n    \
         c1.p = r1.p\n    \
         c1 = check_all(c1)\n    \
         if c1.err == 1:\n        \
         print(\"demo check err: \" + c1.errmsg)\n        \
         return 1\n    \
         print(\"demo ok files=\" + str(r1.files.len) + \" instances=\" + str(c1.inst_names.len))\n    \
         r2 = load_program(\"{diamond}\")\n    \
         if r2.err == 1:\n        \
         print(\"diamond load err: \" + r2.errmsg)\n        \
         return 1\n    \
         c2 = checker_state()\n    \
         c2.p = r2.p\n    \
         c2 = check_all(c2)\n    \
         if c2.err == 1:\n        \
         print(\"diamond check err: \" + c2.errmsg)\n        \
         return 1\n    \
         print(\"diamond ok files=\" + str(r2.files.len))\n    \
         r3 = load_program(\"{cyc}\")\n    \
         if r3.err == 0:\n        \
         print(\"BUG: cycle accepted\")\n        \
         return 1\n    \
         print(\"cycle: \" + r3.errmsg)\n    \
         r4 = load_program(\"{miss}\")\n    \
         if r4.err == 0:\n        \
         print(\"BUG: missing import accepted\")\n        \
         return 1\n    \
         print(\"missing: \" + r4.errmsg)\n    \
         return 0\n",
        abs("selfhost/lexer.ax"),
        abs("selfhost/parser.ax"),
        abs("selfhost/typecheck.ax"),
        abs("selfhost/load.ax"),
        demo = abs("examples/stdlib_demo.ax"),
        diamond = fp("main.ax"),
        cyc = fp("cyc_a.ax"),
        miss = fp("miss.ax"),
    );

    let out_dir = std::env::temp_dir().join("axon-tests");
    std::fs::create_dir_all(&out_dir).unwrap();
    let driver = out_dir.join(format!("selfhost-load-{}.ax", std::process::id()));
    let exe = out_dir.join(format!("selfhost-load-{}.exe", std::process::id()));
    std::fs::write(&driver, driver_src).unwrap();

    aoxn::build_paths_exe(&[driver.display().to_string()], &exe, true)
        .expect("self-host loader driver failed to compile");
    let out = Command::new(&exe).output().expect("failed to run");
    let _ = std::fs::remove_file(&exe);
    let _ = std::fs::remove_file(&driver);
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "loader driver crashed: {:?}\n{text}", out.status.code());

    assert!(text.contains("demo ok files=2 instances=10"), "{text}");
    assert!(text.contains("diamond ok files=4"), "{text}");
    assert!(text.contains("cycle: circular import"), "{text}");
    assert!(text.contains("missing: cannot open"), "{text}");
}


