//! Axon code generator: typed AST -> LLVM IR via C API -> native object file.

use std::collections::HashMap;
use std::ffi::{CStr, CString};

use crate::ast::*;
use crate::llvm::*;

type Slot = (LLVMValueRef, Type);
type Locals = HashMap<String, Slot>;

/// LLVM target registration is process-global; registering twice makes
/// LLVMGetTargetFromTriple fail with "Cannot choose between targets".
static TARGET_INIT: std::sync::Once = std::sync::Once::new();

fn init_target() {
    TARGET_INIT.call_once(|| unsafe {
        LLVMInitializeX86TargetInfo();
        LLVMInitializeX86Target();
        LLVMInitializeX86TargetMC();
        LLVMInitializeX86AsmPrinter();
    });
}

pub fn generate_ir_text(program: &Program, opt: bool, call_map: &HashMap<usize, String>) -> Result<String, String> {
    unsafe {
        let mut g = Gen::create();
        g.call_map = call_map.clone();
        let result = g.build_module(program, opt).map(|_| {
            let ir_c = LLVMPrintModuleToString(g.module);
            let ir = CStr::from_ptr(ir_c).to_string_lossy().into_owned();
            LLVMDisposeMessage(ir_c);
            ir
        });
        g.dispose();
        result
    }
}

pub fn generate_to_object(program: &Program, obj_path: &std::path::Path, opt: bool, call_map: &HashMap<usize, String>) -> Result<(), String> {
    unsafe {
        let mut g = Gen::create();
        g.call_map = call_map.clone();
        let result = g
            .build_module(program, opt)
            .and_then(|_| g.emit_object(obj_path));
        g.dispose();
        result
    }
}

struct FnInfo {
    ref_: LLVMValueRef,
    fn_ty: LLVMTypeRef,
    ret: Type,
    entry_bb: LLVMBasicBlockRef,
}

struct Gen {
    ctx: LLVMContextRef,
    module: LLVMModuleRef,
    builder: LLVMBuilderRef,
    void: LLVMTypeRef,
    i1: LLVMTypeRef,
    i8: LLVMTypeRef,
    i32: LLVMTypeRef,
    i64: LLVMTypeRef,
    f64t: LLVMTypeRef,
    ptr: LLVMTypeRef,
    cur_bb: LLVMBasicBlockRef,
    cur_fn: String,
    tm: LLVMTargetMachineRef,
    printf_ty: LLVMTypeRef,
    fns: HashMap<String, FnInfo>,
    structs: HashMap<String, LLVMTypeRef>,
    struct_fields: HashMap<String, Vec<(String, usize, Type)>>,
    strings: HashMap<String, LLVMValueRef>,
    fmts: HashMap<String, LLVMValueRef>,
    lit_temps: HashMap<usize, LLVMValueRef>,
    iter_temps: HashMap<usize, LLVMValueRef>,
    val_temps: HashMap<usize, LLVMValueRef>,
    externs: HashMap<String, (LLVMValueRef, LLVMTypeRef)>,
    /// stack of (break_target, continue_target) for nested loops
    loop_stack: Vec<(LLVMBasicBlockRef, LLVMBasicBlockRef)>,
    loop_count: usize,
    /// generic call node address -> mangled instance name (from typecheck)
    call_map: HashMap<usize, String>,
}

impl Gen {
    unsafe fn create() -> Gen {
        let ctx = LLVMContextCreate();
        let module_name = CString::new("axon_module").unwrap();
        let module = LLVMModuleCreateWithNameInContext(module_name.as_ptr(), ctx);
        let builder = LLVMCreateBuilderInContext(ctx);
        Gen {
            ctx,
            module,
            builder,
            void: LLVMVoidTypeInContext(ctx),
            i1: LLVMInt1TypeInContext(ctx),
            i8: LLVMInt8TypeInContext(ctx),
            i32: LLVMInt32TypeInContext(ctx),
            i64: LLVMInt64TypeInContext(ctx),
            f64t: LLVMDoubleTypeInContext(ctx),
            ptr: LLVMPointerTypeInContext(ctx, 0),
            cur_bb: std::ptr::null_mut(),
            cur_fn: String::new(),
            tm: std::ptr::null_mut(),
            printf_ty: std::ptr::null_mut(),
            fns: HashMap::new(),
            structs: HashMap::new(),
            struct_fields: HashMap::new(),
            strings: HashMap::new(),
            fmts: HashMap::new(),
            lit_temps: HashMap::new(),
            iter_temps: HashMap::new(),
            val_temps: HashMap::new(),
            externs: HashMap::new(),
            loop_stack: Vec::new(),
            loop_count: 0,
            call_map: HashMap::new(),
        }
    }

    unsafe fn dispose(&mut self) {
        LLVMDisposeBuilder(self.builder);
        LLVMDisposeModule(self.module);
        LLVMContextDispose(self.ctx);
    }

    unsafe fn cstr(&self, s: &str) -> CString {
        CString::new(s).unwrap_or_else(|_| CString::new("?").unwrap())
    }

    unsafe fn ty_of(&self, t: &Type) -> LLVMTypeRef {
        match t {
            Type::Int => self.i64,
            Type::Float => self.f64t,
            Type::Bool => self.i1,
            Type::Str => self.ptr,
            Type::Void => self.void,
            Type::Array { elem, len } => LLVMArrayType(self.ty_of(elem), *len as u32),
            Type::Struct(name) => *self.structs.get(name).unwrap_or(&std::ptr::null_mut()),
        }
    }

    unsafe fn const_int(&self, t: &Type, v: i64) -> LLVMValueRef {
        LLVMConstInt(self.ty_of(t), v as u64, 1)
    }

    unsafe fn terminated(&self) -> bool {
        !LLVMGetBasicBlockTerminator(self.cur_bb).is_null()
    }

    unsafe fn add_bb(&mut self, f: LLVMValueRef, name: &str) -> LLVMBasicBlockRef {
        let n = self.cstr(name);
        LLVMAppendBasicBlockInContext(self.ctx, f, n.as_ptr())
    }

    unsafe fn pos(&mut self, bb: LLVMBasicBlockRef) {
        self.cur_bb = bb;
        LLVMPositionBuilderAtEnd(self.builder, bb);
    }

    /// Allocas must live at the top of the function's entry block, even when
    /// the entry block is already terminated (e.g. by short-circuit branches).
    unsafe fn alloca_in_entry(&mut self, ty: LLVMTypeRef, name: &str) -> LLVMValueRef {
        let entry = self.fns[&self.cur_fn].entry_bb;
        let saved = self.cur_bb;
        let first = LLVMGetFirstInstruction(entry);
        if first.is_null() {
            LLVMPositionBuilderAtEnd(self.builder, entry);
        } else {
            LLVMPositionBuilderBefore(self.builder, first);
        }
        let n = self.cstr(name);
        let slot = LLVMBuildAlloca(self.builder, ty, n.as_ptr());
        self.pos(saved);
        slot
    }

    /// per-literal-site temp memory (hoisted to the entry block, reused per iteration)
    unsafe fn lit_temp(&mut self, key: usize, ty: LLVMTypeRef) -> LLVMValueRef {
        if let Some(v) = self.lit_temps.get(&key) {
            return *v;
        }
        let slot = self.alloca_in_entry(ty, &format!("lit{key}"));
        self.lit_temps.insert(key, slot);
        slot
    }

    /// per-replication-site loop counter (hoisted to the entry block)
    unsafe fn rep_iter(&mut self, key: usize) -> LLVMValueRef {
        if let Some(v) = self.iter_temps.get(&key) {
            return *v;
        }
        let slot = self.alloca_in_entry(self.i64, &format!("rep.iter{key}"));
        self.iter_temps.insert(key, slot);
        slot
    }

    /// temp for materializing a non-lvalue aggregate (keyed by expr node address)
    unsafe fn val_temp(&mut self, key: usize, ty: LLVMTypeRef) -> LLVMValueRef {
        if let Some(v) = self.val_temps.get(&key) {
            return *v;
        }
        let slot = self.alloca_in_entry(ty, &format!("tmp{key}"));
        self.val_temps.insert(key, slot);
        slot
    }

    unsafe fn string_lit(&mut self, s: &str) -> LLVMValueRef {
        if let Some(v) = self.strings.get(s) {
            return *v;
        }
        let c = self.cstr(s);
        let n = self.cstr(&format!(".str{}", self.strings.len()));
        let v = LLVMBuildGlobalStringPtr(self.builder, c.as_ptr(), n.as_ptr());
        self.strings.insert(s.to_string(), v);
        v
    }

    unsafe fn fmt_lit(&mut self, s: &'static str) -> LLVMValueRef {
        if let Some(v) = self.fmts.get(s) {
            return *v;
        }
        let c = self.cstr(s);
        let n = self.cstr(&format!(".fmt{}", self.fmts.len()));
        let v = LLVMBuildGlobalStringPtr(self.builder, c.as_ptr(), n.as_ptr());
        self.fmts.insert(s.to_string(), v);
        v
    }

    // ---- module assembly ----

    unsafe fn build_module(&mut self, program: &Program, opt: bool) -> Result<(), String> {
        // C runtime entry first so the user's `main` gets the internal name `axon.main`.
        let main_i32 = LLVMFunctionType(self.i32, std::ptr::null_mut(), 0, 0);
        let wrapper_name = self.cstr("main");
        let wrapper = LLVMAddFunction(self.module, wrapper_name.as_ptr(), main_i32);

        // declare struct types (two-phase so structs may reference each other)
        for s in &program.structs {
            let name = self.cstr(&s.name);
            let ty = LLVMStructCreateNamed(self.ctx, name.as_ptr());
            self.structs.insert(s.name.clone(), ty);
        }
        for s in &program.structs {
            let field_tys: Vec<LLVMTypeRef> = s.fields.iter().map(|f| self.ty_of(&f.ty)).collect();
            let named = self.structs[&s.name];
            let mut tys = field_tys.clone();
            LLVMStructSetBody(named, tys.as_mut_ptr(), tys.len() as u32, 0);
            self.struct_fields.insert(
                s.name.clone(),
                s.fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| (f.name.clone(), i, f.ty.clone()))
                    .collect(),
            );
        }

        // declare all user functions (two-pass, enables mutual recursion)
        for f in &program.funcs {
            if !f.type_params.is_empty() {
                // generic declarations are never emitted directly; codegen
                // only sees their monomorphized instances
                continue;
            }
            let mut param_tys: Vec<LLVMTypeRef> = f.params.iter().map(|p| self.ty_of(&p.ty)).collect();
            let ret_ty = self.ty_of(&f.ret);
            let fn_ty = LLVMFunctionType(ret_ty, param_tys.as_mut_ptr(), param_tys.len() as u32, 0);
            let internal = if f.name == "main" { "axon.main" } else { f.name.as_str() };
            let name = self.cstr(internal);
            let ref_ = LLVMAddFunction(self.module, name.as_ptr(), fn_ty);
            // extern declarations get no entry block (no body will be emitted)
            let entry = if f.is_extern { std::ptr::null_mut() } else { self.add_bb(ref_, "entry") };
            self.fns.insert(
                f.name.clone(),
                FnInfo { ref_, fn_ty, ret: f.ret.clone(), entry_bb: entry },
            );
        }

        // emit bodies
        for f in &program.funcs {
            if f.is_extern {
                continue; // body lives in the C runtime; declaration is enough
            }
            let (fn_ref, entry_bb, fn_ret) = {
                let info = &self.fns[&f.name];
                (info.ref_, info.entry_bb, info.ret.clone())
            };
            self.pos(entry_bb);
            self.cur_fn = f.name.clone();

            let mut locals: Locals = HashMap::new();

            // parameters: alloca + store in entry block
            for (i, p) in f.params.iter().enumerate() {
                let arg = LLVMGetParam(fn_ref, i as u32);
                let pname = self.cstr(&format!("param.{}", p.name));
                let slot = LLVMBuildAlloca(self.builder, self.ty_of(&p.ty), pname.as_ptr());
                LLVMBuildStore(self.builder, arg, slot);
                locals.insert(p.name.clone(), (slot, p.ty.clone()));
            }

            self.emit_block_into(&f.body, &mut locals, &fn_ret)?;

            // void function falling off the end
            if !self.terminated() {
                if fn_ret == Type::Void {
                    LLVMBuildRetVoid(self.builder);
                } else {
                    // typecheck guarantees this cannot happen; keep a safe terminator anyway
                    LLVMBuildUnreachable(self.builder);
                }
            }
        }

        // wrapper body: call axon.main, return exit code
        if !program.funcs.iter().any(|f| f.name == "main") {
            return Err("internal error: main missing at codegen".into());
        }
        let (main_fn_ty, main_ref, main_ret) = {
            let info = &self.fns["main"];
            (info.fn_ty, info.ref_, info.ret.clone())
        };
        let bb = self.add_bb(wrapper, "entry");
        self.pos(bb);

        // deterministic output across platforms: put stdout in binary mode
        // (Windows CRT would otherwise translate \n to \r\n)
        if cfg!(windows) {
            let mut params = [self.i32, self.i32];
            let setmode_ty = LLVMFunctionType(self.i32, params.as_mut_ptr(), 2, 0);
            let name = self.cstr("_setmode");
            let setmode = LLVMAddFunction(self.module, name.as_ptr(), setmode_ty);
            // fd 1 = stdout, _O_BINARY = 0x8000
            let mut args = [LLVMConstInt(self.i32, 1, 0), LLVMConstInt(self.i32, 0x8000, 0)];
            LLVMBuildCall2(self.builder, setmode_ty, setmode, args.as_mut_ptr(), 2, self.cstr("").as_ptr());
        }

        let call = LLVMBuildCall2(self.builder, main_fn_ty, main_ref, std::ptr::null_mut(), 0, self.cstr("").as_ptr());
        if main_ret == Type::Int {
            let v = LLVMBuildTrunc(self.builder, call, self.i32, self.cstr("exitcode").as_ptr());
            LLVMBuildRet(self.builder, v);
        } else {
            LLVMBuildRet(self.builder, LLVMConstInt(self.i32, 0, 0));
        }

        // verify
        if std::env::var("AXON_DUMP_IR").is_ok() {
            let ir_c = LLVMPrintModuleToString(self.module);
            eprintln!("{}", CStr::from_ptr(ir_c).to_string_lossy());
            LLVMDisposeMessage(ir_c);
        }
        let mut msg: *mut std::os::raw::c_char = std::ptr::null_mut();
        if LLVMVerifyModule(self.module, VERIFY_RETURN_STATUS, &mut msg) != 0 {
            let m = if msg.is_null() {
                "module verification failed".to_string()
            } else {
                let s = CStr::from_ptr(msg).to_string_lossy().into_owned();
                LLVMDisposeMessage(msg);
                s
            };
            return Err(format!("internal error: module verification failed: {m}"));
        }

        // target setup
        let triple_c = LLVMGetDefaultTargetTriple();
        LLVMSetTarget(self.module, triple_c);

        init_target();

        let mut target: LLVMTargetRef = std::ptr::null_mut();
        let mut err: *mut std::os::raw::c_char = std::ptr::null_mut();
        if LLVMGetTargetFromTriple(triple_c, &mut target, &mut err) != 0 {
            let m = if err.is_null() {
                "unknown target".to_string()
            } else {
                let s = CStr::from_ptr(err).to_string_lossy().into_owned();
                LLVMDisposeMessage(err);
                s
            };
            LLVMDisposeMessage(triple_c);
            return Err(format!("internal error: cannot resolve target: {m}"));
        }

        let level = if opt { CODEGEN_LEVEL_AGGRESSIVE } else { CODEGEN_LEVEL_DEFAULT };
        let cpu = self.cstr("");
        let features = self.cstr("");
        let tm = LLVMCreateTargetMachine(target, triple_c, cpu.as_ptr(), features.as_ptr(), level, RELOC_DEFAULT, CODE_MODEL_DEFAULT);
        LLVMDisposeMessage(triple_c);
        if tm.is_null() {
            return Err("internal error: cannot create target machine".into());
        }
        let dl = LLVMCreateTargetDataLayout(tm);
        LLVMSetModuleDataLayout(self.module, dl);

        // IR-level optimization pipeline (O3)
        if opt {
            let opts = LLVMCreatePassBuilderOptions();
            let passes = self.cstr("default<O3>");
            let perr = LLVMRunPasses(self.module, passes.as_ptr(), tm, opts);
            LLVMDisposePassBuilderOptions(opts);
            if !perr.is_null() {
                let s = CStr::from_ptr(perr).to_string_lossy().into_owned();
                LLVMDisposeMessage(perr);
                return Err(format!("internal error: optimization pipeline failed: {s}"));
            }
        }

        self.tm = tm;
        Ok(())
    }

    /// `for var in range(...)` / `for var in array:` — start/end/step and the
    /// array pointer are evaluated once at loop entry (Python semantics).
    unsafe fn emit_for(
        &mut self,
        var: &str,
        iter: &ForIter,
        body: &Block,
        locals: &mut Locals,
        fn_ret: &Type,
    ) -> Result<(), String> {
        let fn_ref = self.fns[&self.cur_fn].ref_;

        // loop variable type + iteration source
        let (start, end, step, var_ty, arr_info) = match iter {
            ForIter::Range(args) => {
                // range(n) → 0..n · range(a, b) → a..b · range(a, b, step)
                let (s, e, st) = match args.len() {
                    1 => {
                        let (e, _) = self.emit_expr(&args[0], locals)?;
                        (self.const_int(&Type::Int, 0), e, self.const_int(&Type::Int, 1))
                    }
                    2 => {
                        let (s, _) = self.emit_expr(&args[0], locals)?;
                        let (e, _) = self.emit_expr(&args[1], locals)?;
                        (s, e, self.const_int(&Type::Int, 1))
                    }
                    _ => {
                        let (s, _) = self.emit_expr(&args[0], locals)?;
                        let (e, _) = self.emit_expr(&args[1], locals)?;
                        let (st, _) = self.emit_expr(&args[2], locals)?;
                        (s, e, st)
                    }
                };
                (s, e, st, Type::Int, None)
            }
            ForIter::Array(e) => {
                // evaluated once: pointer to the array + static length
                let (arr_ptr, arr_ty) = self.emit_aggregate_ptr(e, locals)?;
                let (elem, len) = match &arr_ty {
                    Type::Array { elem, len } => ((**elem).clone(), *len),
                    other => return Err(format!("internal error: iterating {other} at codegen")),
                };
                (
                    self.const_int(&Type::Int, 0),
                    self.const_int(&Type::Int, len as i64),
                    self.const_int(&Type::Int, 1),
                    elem.clone(),
                    Some((arr_ptr, len)),
                )
            }
        };

        // variable slot (declare on first use, reuse otherwise)
        let var_slot = match locals.get(var).cloned() {
            Some((slot, _dt)) => slot,
            None => {
                let slot = self.alloca_in_entry(self.ty_of(&var_ty), &format!("var.{var}"));
                locals.insert(var.to_string(), (slot, var_ty.clone()));
                slot
            }
        };

        // internal index for array iteration (the array pointer is an SSA
        // value and survives across blocks; no temp storage needed)
        let mut idx_slot: Option<LLVMValueRef> = None;
        let mut arr_ty_real: Option<Type> = None;
        let mut arr_base: LLVMValueRef = std::ptr::null_mut();
        if let Some((arr_ptr, len)) = arr_info {
            let id = self.loop_count;
            self.loop_count += 1;
            let idx = self.alloca_in_entry(self.i64, &format!("for.idx{id}"));
            LLVMBuildStore(self.builder, LLVMConstInt(self.i64, 0, 0), idx);
            idx_slot = Some(idx);
            arr_ty_real = Some(Type::Array { elem: Box::new(var_ty.clone()), len });
            arr_base = arr_ptr;
        }

        // range: initialize the variable with the start value
        if arr_info.is_none() {
            LLVMBuildStore(self.builder, start, var_slot);
        }

        // induction slot: the loop variable itself for ranges, the hidden
        // i64 index for array iteration (elements are copied in the body)
        let (ind_slot, ind_ty) = match &idx_slot {
            Some(idx) => (*idx, Type::Int),
            None => (var_slot, var_ty.clone()),
        };

        let cond_bb = self.add_bb(fn_ref, "for.cond");
        let body_bb = self.add_bb(fn_ref, "for.body");
        let cont_bb = self.add_bb(fn_ref, "for.cont");
        let end_bb = self.add_bb(fn_ref, "for.end");

        let zero = self.const_int(&Type::Int, 0);
        LLVMBuildBr(self.builder, cond_bb);

        // cond: (step > 0) ? (i < end) : (i > end)
        self.pos(cond_bb);
        let i = LLVMBuildLoad2(self.builder, self.ty_of(&ind_ty), ind_slot, self.cstr("for.i").as_ptr());
        let plus = LLVMBuildICmp(self.builder, INT_SLT, i, end, self.cstr("for.lt").as_ptr());
        let minus = LLVMBuildICmp(self.builder, INT_SGT, i, end, self.cstr("for.gt").as_ptr());
        let step_pos = LLVMBuildICmp(self.builder, INT_SGT, step, zero, self.cstr("for.steppos").as_ptr());
        let n = self.cstr("for.c");
        let c = LLVMBuildSelect(self.builder, step_pos, plus, minus, n.as_ptr());
        LLVMBuildCondBr(self.builder, c, body_bb, end_bb);

        // body
        self.pos(body_bb);
        if let (Some(idx), Some(arr_ty_real)) = (idx_slot, arr_ty_real.clone()) {
            let idx_v = LLVMBuildLoad2(self.builder, self.i64, idx, self.cstr("for.idxv").as_ptr());
            let mut indices = [zero, idx_v];
            let nn = self.cstr("for.elem");
            let elem_ptr = LLVMBuildGEP2(
                self.builder,
                self.ty_of(&arr_ty_real),
                arr_base,
                indices.as_mut_ptr(),
                2,
                nn.as_ptr(),
            );
            let ev = LLVMBuildLoad2(self.builder, self.ty_of(&var_ty), elem_ptr, self.cstr("for.val").as_ptr());
            LLVMBuildStore(self.builder, ev, var_slot);
        }
        self.loop_stack.push((end_bb, cont_bb));
        self.emit_block_into(body, locals, fn_ret)?;
        self.loop_stack.pop();
        if !self.terminated() {
            LLVMBuildBr(self.builder, cont_bb);
        }

        // continue target: increment the induction slot only
        self.pos(cont_bb);
        let i2 = LLVMBuildLoad2(self.builder, self.ty_of(&ind_ty), ind_slot, self.cstr("for.i").as_ptr());
        let next = LLVMBuildAdd(self.builder, i2, step, self.cstr("for.next").as_ptr());
        LLVMBuildStore(self.builder, next, ind_slot);
        LLVMBuildBr(self.builder, cond_bb);

        self.pos(end_bb);
        Ok(())
    }

    unsafe fn emit_object(&mut self, obj_path: &std::path::Path) -> Result<(), String> {
        let path_c = self.cstr(&obj_path.to_string_lossy());
        let mut err: *mut std::os::raw::c_char = std::ptr::null_mut();
        if LLVMTargetMachineEmitToFile(self.tm, self.module, path_c.as_ptr(), CODEGEN_OBJECT_FILE, &mut err) != 0 {
            let m = if err.is_null() {
                "unknown emission error".to_string()
            } else {
                let s = CStr::from_ptr(err).to_string_lossy().into_owned();
                LLVMDisposeMessage(err);
                s
            };
            return Err(format!("internal error: object emission failed: {m}"));
        }
        Ok(())
    }

    // ---- statement / expression emission ----

    unsafe fn emit_block_into(&mut self, block: &Block, locals: &mut Locals, fn_ret: &Type) -> Result<(), String> {
        for stmt in &block.stmts {
            if self.terminated() {
                break; // unreachable code was already rejected by typecheck
            }
            self.emit_stmt(stmt, locals, fn_ret)?;
        }
        Ok(())
    }

    unsafe fn emit_stmt(&mut self, stmt: &Stmt, locals: &mut Locals, fn_ret: &Type) -> Result<(), String> {
        match stmt {
            Stmt::Let { name, ty, expr, .. } => {
                let hint = self.type_hint(expr, locals)?;
                let bind_ty = match locals.get(name) {
                    Some((_slot, dt)) => dt.clone(),
                    None => ty.clone().unwrap_or(hint),
                };
                let slot = match locals.get(name).cloned() {
                    Some((slot, _)) => slot, // re-assignment of an existing binding
                    None => {
                        let slot = self.alloca_in_entry(self.ty_of(&bind_ty), &format!("var.{name}"));
                        locals.insert(name.clone(), (slot, bind_ty.clone()));
                        slot
                    }
                };
                if bind_ty.is_compound() {
                    // aggregate: copy via memcpy, never as a giant SSA value
                    // (huge load/store pairs choke the optimizer)
                    let (src, _aty) = self.emit_aggregate_ptr(expr, locals)?;
                    let size = LLVMSizeOf(self.ty_of(&bind_ty));
                    LLVMBuildMemCpy(self.builder, slot, 0, src, 0, size);
                } else {
                    let (v, _t) = self.emit_expr(expr, locals)?;
                    LLVMBuildStore(self.builder, v, slot);
                }
                Ok(())
            }
            Stmt::Assign { target, expr, .. } => {
                let (ptr, t) = self.emit_lvalue(target, locals)?;
                if t.is_compound() {
                    // aggregate element/field copy via memcpy
                    let (src, _aty) = self.emit_aggregate_ptr(expr, locals)?;
                    let size = LLVMSizeOf(self.ty_of(&t));
                    LLVMBuildMemCpy(self.builder, ptr, 0, src, 0, size);
                } else {
                    let (v, _t2) = self.emit_expr(expr, locals)?;
                    LLVMBuildStore(self.builder, v, ptr);
                }
                Ok(())
            }
            Stmt::If { cond, then_block, else_block, .. } => {
                let (c, _) = self.emit_expr(cond, locals)?;
                let fn_ref = self.fns[&self.cur_fn].ref_;
                let then_bb = self.add_bb(fn_ref, "if.then");
                let else_bb = self.add_bb(fn_ref, "if.else");
                LLVMBuildCondBr(self.builder, c, then_bb, else_bb);

                // end block is created lazily: if both branches return, an empty
                // merge block would be an unterminated basic block (broken module)
                let mut end_bb: LLVMBasicBlockRef = std::ptr::null_mut();

                self.pos(then_bb);
                self.emit_block_into(then_block, locals, fn_ret)?;
                if !self.terminated() {
                    end_bb = self.add_bb(fn_ref, "if.end");
                    LLVMBuildBr(self.builder, end_bb);
                }

                self.pos(else_bb);
                if let Some(eb) = else_block {
                    self.emit_block_into(eb, locals, fn_ret)?;
                }
                if !self.terminated() {
                    if end_bb.is_null() {
                        end_bb = self.add_bb(fn_ref, "if.end");
                    }
                    LLVMBuildBr(self.builder, end_bb);
                }

                if !end_bb.is_null() {
                    self.pos(end_bb);
                }
                Ok(())
            }
            Stmt::While { cond, body, .. } => {
                let fn_ref = self.fns[&self.cur_fn].ref_;
                let cond_bb = self.add_bb(fn_ref, "while.cond");
                let body_bb = self.add_bb(fn_ref, "while.body");
                let end_bb = self.add_bb(fn_ref, "while.end");
                LLVMBuildBr(self.builder, cond_bb);

                self.pos(cond_bb);
                let (c, _) = self.emit_expr(cond, locals)?;
                LLVMBuildCondBr(self.builder, c, body_bb, end_bb);

                self.pos(body_bb);
                self.loop_stack.push((end_bb, cond_bb));
                self.emit_block_into(body, locals, fn_ret)?;
                self.loop_stack.pop();
                if !self.terminated() {
                    LLVMBuildBr(self.builder, cond_bb);
                }

                self.pos(end_bb);
                Ok(())
            }
            Stmt::For { var, iter, body, .. } => self.emit_for(var, iter, body, locals, fn_ret),
            Stmt::Break { .. } => {
                let (brk, _cont) = *self
                    .loop_stack
                    .last()
                    .ok_or("internal error: break outside loop at codegen")?;
                LLVMBuildBr(self.builder, brk);
                Ok(())
            }
            Stmt::Continue { .. } => {
                let (_brk, cont) = *self
                    .loop_stack
                    .last()
                    .ok_or("internal error: continue outside loop at codegen")?;
                LLVMBuildBr(self.builder, cont);
                Ok(())
            }
            Stmt::Return { expr, .. } => match expr {
                None => {
                    LLVMBuildRetVoid(self.builder);
                    Ok(())
                }
                Some(e) => {
                    let (v, _) = self.emit_expr(e, locals)?;
                    if *fn_ret == Type::Void {
                        return Err("internal error: value return in void function".into());
                    }
                    LLVMBuildRet(self.builder, v);
                    Ok(())
                }
            },
            Stmt::Pass => Ok(()),
            Stmt::ExprStmt { expr } => {
                self.emit_expr(expr, locals)?;
                Ok(())
            }
        }
    }

    /// address of an aggregate-valued expression, materializing it into
    /// memory only when needed. Avoids giant SSA aggregate load/stores —
    /// those choke the optimizer (SROA) on large arrays.
    unsafe fn emit_aggregate_ptr(&mut self, expr: &Expr, locals: &mut Locals) -> Result<(LLVMValueRef, Type), String> {
        match expr {
            Expr::ArrayRep { elem, count, lit_id, .. } => {
                let (temp, ty) = self.fill_rep(elem, *count, *lit_id, locals)?;
                Ok((temp, ty))
            }
            _ if expr.is_lvalue() => self.emit_lvalue(expr, locals),
            _ => {
                let (v, t) = self.emit_expr(expr, locals)?;
                let key = expr as *const Expr as usize;
                let temp = self.val_temp(key, self.ty_of(&t));
                LLVMBuildStore(self.builder, v, temp);
                Ok((temp, t))
            }
        }
    }

    /// static type of an expression (codegen-side hint, no emission)
    fn type_hint(&self, expr: &Expr, locals: &Locals) -> Result<Type, String> {
        match expr {
            Expr::Int(..) => Ok(Type::Int),
            Expr::Float(..) => Ok(Type::Float),
            Expr::Bool(..) => Ok(Type::Bool),
            Expr::Str(..) => Ok(Type::Str),
            Expr::Var { name, .. } => locals
                .get(name)
                .map(|(_, t)| t.clone())
                .ok_or_else(|| format!("internal error: unknown variable '{name}' in type hint")),
            Expr::Call { name, .. } => {
                // generic calls resolve to their monomorphized instance
                let node = expr as *const Expr as usize;
                let eff = self.call_map.get(&node).cloned().unwrap_or_else(|| name.clone());
                if let Some(info) = self.fns.get(&eff) {
                    return Ok(info.ret.clone());
                }
                if self.struct_fields.contains_key(&eff) {
                    return Ok(Type::Struct(eff.clone()));
                }
                if eff == "print" {
                    return Ok(Type::Void);
                }
                if eff == "len" {
                    return Ok(Type::Int);
                }
                if eff == "str" {
                    return Ok(Type::Str);
                }
                Err(format!("internal error: unknown call '{eff}' in type hint"))
            }
            Expr::Unary { expr, .. } => self.type_hint(expr, locals),
            Expr::Binary { op, lhs, rhs, .. } => match op {
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::And | BinOp::Or => Ok(Type::Bool),
                _ => self.type_hint(lhs, locals).or_else(|_| self.type_hint(rhs, locals)),
            },
            Expr::Index { arr, .. } => match self.type_hint(arr, locals)? {
                Type::Array { elem, .. } => Ok(*elem),
                other => Err(format!("internal error: index on {other} in type hint")),
            },
            Expr::Field { obj, name, .. } => match self.type_hint(obj, locals)? {
                Type::Struct(sname) => self
                    .struct_fields
                    .get(&sname)
                    .and_then(|f| f.iter().find(|(n, _, _)| n == name))
                    .map(|(_, _, t)| t.clone())
                    .ok_or_else(|| format!("internal error: field '{name}' in type hint")),
                other => Err(format!("internal error: field access on {other} in type hint")),
            },
            Expr::ArrayLit { elems, .. } => {
                let elem = self.type_hint(&elems[0], locals)?;
                Ok(Type::Array { elem: Box::new(elem), len: elems.len() })
            }
            Expr::ArrayRep { elem, count, .. } => {
                let elem = self.type_hint(elem, locals)?;
                Ok(Type::Array { elem: Box::new(elem), len: *count })
            }
            Expr::StructLit { name, .. } => Ok(Type::Struct(name.clone())),
        }
    }

    /// fill a replication temp (element evaluated once, runtime fill loop)
    unsafe fn fill_rep(
        &mut self,
        elem: &Expr,
        count: usize,
        lit_id: usize,
        locals: &mut Locals,
    ) -> Result<(LLVMValueRef, Type), String> {
        let (v, t) = self.emit_expr(elem, locals)?;
        let arr_ty = self.ty_of(&Type::Array { elem: Box::new(t.clone()), len: count });
        let temp = self.lit_temp(lit_id, arr_ty);
        let iter = self.rep_iter(lit_id);
        let fn_ref = self.fns[&self.cur_fn].ref_;
        let cond_bb = self.add_bb(fn_ref, &format!("rep.cond{lit_id}"));
        let body_bb = self.add_bb(fn_ref, &format!("rep.body{lit_id}"));
        let end_bb = self.add_bb(fn_ref, &format!("rep.end{lit_id}"));

        LLVMBuildStore(self.builder, LLVMConstInt(self.i64, 0, 0), iter);
        LLVMBuildBr(self.builder, cond_bb);

        self.pos(cond_bb);
        let i = LLVMBuildLoad2(self.builder, self.i64, iter, self.cstr("rep.i").as_ptr());
        let n_const = LLVMConstInt(self.i64, count as u64, 0);
        let c = LLVMBuildICmp(self.builder, INT_SLT, i, n_const, self.cstr("rep.c").as_ptr());
        LLVMBuildCondBr(self.builder, c, body_bb, end_bb);

        self.pos(body_bb);
        let i2 = LLVMBuildLoad2(self.builder, self.i64, iter, self.cstr("rep.i").as_ptr());
        let zero = LLVMConstInt(self.i64, 0, 0);
        let mut indices = [zero, i2];
        let nn = self.cstr("rep.elem");
        let gep = LLVMBuildGEP2(self.builder, arr_ty, temp, indices.as_mut_ptr(), 2, nn.as_ptr());
        LLVMBuildStore(self.builder, v, gep);
        let one = LLVMConstInt(self.i64, 1, 0);
        let next = LLVMBuildAdd(self.builder, i2, one, self.cstr("rep.next").as_ptr());
        LLVMBuildStore(self.builder, next, iter);
        LLVMBuildBr(self.builder, cond_bb);

        self.pos(end_bb);
        Ok((temp, Type::Array { elem: Box::new(t), len: count }))
    }

    /// address of an lvalue: variable slot, array element, or struct field
    unsafe fn emit_lvalue(&mut self, target: &Expr, locals: &mut Locals) -> Result<(LLVMValueRef, Type), String> {
        match target {
            Expr::Var { name, .. } => locals
                .get(name)
                .cloned()
                .ok_or_else(|| format!("internal error: unknown variable '{name}' at codegen")),
            Expr::Index { arr, idx, .. } => {
                let (base_ptr, base_ty) = self.emit_lvalue(arr, locals)?;
                let elem = match &base_ty {
                    Type::Array { elem, .. } => (**elem).clone(),
                    other => return Err(format!("internal error: cannot index {other} at codegen")),
                };
                let (iv, _it) = self.emit_expr(idx, locals)?;
                let zero = LLVMConstInt(self.i64, 0, 0);
                let mut indices = [zero, iv];
                let n = self.cstr("elem.ptr");
                let gep = LLVMBuildGEP2(
                    self.builder,
                    self.ty_of(&base_ty),
                    base_ptr,
                    indices.as_mut_ptr(),
                    2,
                    n.as_ptr(),
                );
                Ok((gep, elem))
            }
            Expr::Field { obj, name, .. } => {
                let (base_ptr, base_ty) = self.emit_lvalue(obj, locals)?;
                let sname = match &base_ty {
                    Type::Struct(s) => s.clone(),
                    other => return Err(format!("internal error: cannot access field of {other} at codegen")),
                };
                let (fidx, fty) = self.struct_fields[&sname]
                    .iter()
                    .find(|(n, _, _)| n == name)
                    .map(|(_, i, t)| (*i, t.clone()))
                    .ok_or_else(|| format!("internal error: unknown field '{name}' of '{sname}' at codegen"))?;
                // struct GEP indices must be i32 constants (LangRef rule)
                let zero = LLVMConstInt(self.i32, 0, 0);
                let fconst = LLVMConstInt(self.i32, fidx as u64, 0);
                let mut indices = [zero, fconst];
                let n = self.cstr("field.ptr");
                let gep = LLVMBuildGEP2(
                    self.builder,
                    self.ty_of(&base_ty),
                    base_ptr,
                    indices.as_mut_ptr(),
                    2,
                    n.as_ptr(),
                );
                Ok((gep, fty))
            }
            other => Err(format!(
                "internal error: invalid lvalue '{}' at codegen",
                other.pos().line
            )),
        }
    }

    unsafe fn emit_expr(&mut self, expr: &Expr, locals: &mut Locals) -> Result<(LLVMValueRef, Type), String> {
        match expr {
            Expr::Int(v, _) => Ok((self.const_int(&Type::Int, *v), Type::Int)),
            Expr::Float(v, _) => Ok((LLVMConstReal(self.f64t, *v), Type::Float)),
            Expr::Bool(v, _) => Ok((LLVMConstInt(self.i1, *v as u64, 0), Type::Bool)),
            Expr::Str(s, _) => {
                let v = self.string_lit(s);
                Ok((v, Type::Str))
            }
            Expr::Var { name, .. } => {
                let (slot, t) = locals
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("internal error: unknown variable '{name}' at codegen"))?;
                let v = LLVMBuildLoad2(self.builder, self.ty_of(&t), slot, self.cstr("load").as_ptr());
                Ok((v, t))
            }
            Expr::Index { arr, idx, .. } => {
                let (ptr, t) = if arr.is_lvalue() {
                    self.emit_lvalue(expr, locals)?
                } else {
                    let (agg_ptr, agg_ty) = self.emit_aggregate_ptr(arr, locals)?;
                    let (iv, _it) = self.emit_expr(idx, locals)?;
                    let elem = match &agg_ty {
                        Type::Array { elem, .. } => (**elem).clone(),
                        other => return Err(format!("internal error: cannot index {other} at codegen")),
                    };
                    let zero = LLVMConstInt(self.i64, 0, 0);
                    let mut indices = [zero, iv];
                    let n = self.cstr("elem.ptr");
                    let gep = LLVMBuildGEP2(
                        self.builder,
                        self.ty_of(&agg_ty),
                        agg_ptr,
                        indices.as_mut_ptr(),
                        2,
                        n.as_ptr(),
                    );
                    (gep, elem)
                };
                let v = LLVMBuildLoad2(self.builder, self.ty_of(&t), ptr, self.cstr("elem").as_ptr());
                Ok((v, t))
            }
            Expr::Field { obj, name, .. } => {
                let (ptr, t) = if obj.is_lvalue() {
                    self.emit_lvalue(expr, locals)?
                } else {
                    let (agg_ptr, agg_ty) = self.emit_aggregate_ptr(obj, locals)?;
                    let sname = match &agg_ty {
                        Type::Struct(s) => s.clone(),
                        other => return Err(format!("internal error: cannot access field of {other} at codegen")),
                    };
                    let (fidx, fty) = self.struct_fields[&sname]
                        .iter()
                        .find(|(n, _, _)| n == name)
                        .map(|(_, i, t)| (*i, t.clone()))
                        .ok_or_else(|| format!("internal error: unknown field '{name}' of '{sname}' at codegen"))?;
                    // struct GEP indices must be i32 constants (LangRef rule)
                    let zero = LLVMConstInt(self.i32, 0, 0);
                    let fconst = LLVMConstInt(self.i32, fidx as u64, 0);
                    let mut indices = [zero, fconst];
                    let n = self.cstr("field.ptr");
                    let gep = LLVMBuildGEP2(
                        self.builder,
                        self.ty_of(&agg_ty),
                        agg_ptr,
                        indices.as_mut_ptr(),
                        2,
                        n.as_ptr(),
                    );
                    (gep, fty)
                };
                let v = LLVMBuildLoad2(self.builder, self.ty_of(&t), ptr, self.cstr("field").as_ptr());
                Ok((v, t))
            }
            Expr::ArrayLit { elems, lit_id, .. } => {
                // evaluate all elements first
                let mut vals: Vec<LLVMValueRef> = Vec::with_capacity(elems.len());
                let mut elem_ty: Option<Type> = None;
                for e in elems {
                    let (v, t) = self.emit_expr(e, locals)?;
                    if let Some(et) = &elem_ty {
                        if &t != et {
                            return Err("internal error: mixed element types in array literal".into());
                        }
                    }
                    elem_ty = Some(t);
                    vals.push(v);
                }
                let elem_ty = elem_ty.unwrap();
                let n = elems.len();
                let arr_ty = self.ty_of(&Type::Array { elem: Box::new(elem_ty.clone()), len: n });
                let temp = self.lit_temp(*lit_id, arr_ty);
                for (i, v) in vals.iter().enumerate() {
                    let zero = LLVMConstInt(self.i64, 0, 0);
                    let idx = LLVMConstInt(self.i64, i as u64, 0);
                    let mut indices = [zero, idx];
                    let nn = self.cstr("lit.elem");
                    let gep = LLVMBuildGEP2(self.builder, arr_ty, temp, indices.as_mut_ptr(), 2, nn.as_ptr());
                    LLVMBuildStore(self.builder, *v, gep);
                }
                let agg = LLVMBuildLoad2(self.builder, arr_ty, temp, self.cstr("lit.val").as_ptr());
                Ok((agg, Type::Array { elem: Box::new(elem_ty), len: n }))
            }
            Expr::ArrayRep { elem, count, lit_id, .. } => {
                // value context: fill the temp, then load (assignment contexts
                // bypass this via emit_aggregate_ptr + memcpy)
                let (temp, ty) = self.fill_rep(elem, *count, *lit_id, locals)?;
                let arr_ty = self.ty_of(&ty);
                let agg = LLVMBuildLoad2(self.builder, arr_ty, temp, self.cstr("rep.val").as_ptr());
                Ok((agg, ty))
            }
            Expr::StructLit { name, fields, lit_id, .. } => {
                self.emit_struct_construction(name, fields, *lit_id, locals)
            }
            Expr::Call { name, args, pos, lit_id } => {
                // generic calls are routed to their monomorphized instance
                let node = expr as *const Expr as usize;
                let routed: Option<String> = self.call_map.get(&node).cloned();
                let name: &str = match &routed {
                    Some(s) => s.as_str(),
                    None => name.as_str(),
                };
                // builtins
                if name == "print" {
                    return self.print_builtin(args, locals);
                }
                if name == "len" {
                    if args.len() != 1 {
                        return Err("internal error: len expects 1 argument".into());
                    }
                    let (v, t) = self.emit_expr(&args[0].value, locals)?;
                    return match t {
                        Type::Array { len, .. } => Ok((self.const_int(&Type::Int, len as i64), Type::Int)),
                        Type::Str => {
                            let (strlen_f, strlen_ty) = self.get_extern("strlen", self.i64, &[self.ptr], false);
                            let mut sargs = [v];
                            let n = LLVMBuildCall2(self.builder, strlen_ty, strlen_f, sargs.as_mut_ptr(), 1, self.cstr("str.len").as_ptr());
                            Ok((n, Type::Int))
                        }
                        other => Err(format!("internal error: len on {other} at codegen")),
                    };
                }
                if name == "str" {
                    if args.len() != 1 {
                        return Err("internal error: str expects 1 argument".into());
                    }
                    let (v, t) = self.emit_expr(&args[0].value, locals)?;
                    return match t {
                        Type::Str => Ok((v, Type::Str)),
                        Type::Int | Type::Float => {
                            let (malloc_f, malloc_ty) = self.get_extern("malloc", self.ptr, &[self.i64], false);
                            let (snprintf_f, snprintf_ty) =
                                self.get_extern("snprintf", self.i32, &[self.ptr, self.i64, self.ptr], true);
                            let cap = if t == Type::Int { 32u64 } else { 64 };
                            let mut margs = [LLVMConstInt(self.i64, cap, 0)];
                            let buf = LLVMBuildCall2(self.builder, malloc_ty, malloc_f, margs.as_mut_ptr(), 1, self.cstr("str.buf").as_ptr());
                            let fmt = if t == Type::Int { "%lld" } else { "%f" };
                            let fmt_ptr = self.fmt_lit(fmt);
                            let mut sargs = [buf, LLVMConstInt(self.i64, cap, 0), fmt_ptr, v];
                            LLVMBuildCall2(self.builder, snprintf_ty, snprintf_f, sargs.as_mut_ptr(), 4, self.cstr("").as_ptr());
                            Ok((buf, Type::Str))
                        }
                        Type::Bool => {
                            // branch to static "true"/"false", phi the pointers
                            let st = self.string_lit("true");
                            let sf = self.string_lit("false");
                            let fn_ref = self.fns[&self.cur_fn].ref_;
                            let tbb = self.add_bb(fn_ref, "str.true");
                            let fbb = self.add_bb(fn_ref, "str.false");
                            let end = self.add_bb(fn_ref, "str.end");
                            LLVMBuildCondBr(self.builder, v, tbb, fbb);
                            self.pos(tbb);
                            LLVMBuildBr(self.builder, end);
                            self.pos(fbb);
                            LLVMBuildBr(self.builder, end);
                            self.pos(end);
                            let phi = LLVMBuildPhi(self.builder, self.ptr, self.cstr("str.bool").as_ptr());
                            let mut vals = [st, sf];
                            let mut bbs = [tbb, fbb];
                            LLVMAddIncoming(phi, vals.as_mut_ptr(), bbs.as_mut_ptr(), 2);
                            Ok((phi, Type::Str))
                        }
                        other => Err(format!("internal error: str on {other} at codegen")),
                    };
                }
                // raw memory primitives: int -> ptr, then load/store
                if name == "load_i64" || name == "load_f64" || name == "load_u8" {
                    if name == "load_u8" {
                        // load_u8(base: int|string, offset: int)
                        let (base, bt) = self.emit_expr(&args[0].value, locals)?;
                        let (off, _ot) = self.emit_expr(&args[1].value, locals)?;
                        let mut indices = [off];
                        let ptr = match bt {
                            Type::Str => {
                                let n = self.cstr("mem.strptr");
                                LLVMBuildGEP2(self.builder, self.i8, base, indices.as_mut_ptr(), 1, n.as_ptr())
                            }
                            _ => {
                                let n = self.cstr("mem.ptr");
                                let total = LLVMBuildAdd(self.builder, base, off, self.cstr("mem.addr").as_ptr());
                                LLVMBuildIntToPtr(self.builder, total, self.ptr, n.as_ptr())
                            }
                        };
                        let i8v = LLVMBuildLoad2(self.builder, self.i8, ptr, self.cstr("mem.u8").as_ptr());
                        let ext = LLVMBuildZExt(self.builder, i8v, self.i64, self.cstr("mem.u8.ext").as_ptr());
                        return Ok((ext, Type::Int));
                    }
                    if args.len() != 1 {
                        return Err(format!("internal error: {name} expects 1 argument"));
                    }
                    let (v, t) = self.emit_expr(&args[0].value, locals)?;
                    if t != Type::Int {
                        return Err(format!("internal error: {name} address must be int"));
                    }
                    let n = self.cstr("mem.ptr");
                    let ptr = LLVMBuildIntToPtr(self.builder, v, self.ptr, n.as_ptr());
                    let (ty, out) = if name == "load_f64" {
                        (self.f64t, Type::Float)
                    } else {
                        (self.i64, Type::Int)
                    };
                    let v2 = LLVMBuildLoad2(self.builder, ty, ptr, self.cstr("mem.load").as_ptr());
                    return Ok((v2, out));
                }
                if name == "store_i64" || name == "store_f64" {
                    if args.len() != 2 {
                        return Err(format!("internal error: {name} expects 2 arguments"));
                    }
                    let (addr, at) = self.emit_expr(&args[0].value, locals)?;
                    let (val, vt) = self.emit_expr(&args[1].value, locals)?;
                    if at != Type::Int {
                        return Err(format!("internal error: {name} address must be int"));
                    }
                    let want = if name == "store_f64" { Type::Float } else { Type::Int };
                    if vt != want {
                        return Err(format!("internal error: {name} value must be {want}"));
                    }
                    let n = self.cstr("mem.ptr");
                    let ptr = LLVMBuildIntToPtr(self.builder, addr, self.ptr, n.as_ptr());
                    LLVMBuildStore(self.builder, val, ptr);
                    return Ok((std::ptr::null_mut(), Type::Void));
                }
                if name == "store_u8" {
                    // store_u8(base: int|string, offset: int, value: int)
                    if args.len() != 3 {
                        return Err("internal error: store_u8 expects 3 arguments".into());
                    }
                    let (base, bt) = self.emit_expr(&args[0].value, locals)?;
                    let (off, _ot) = self.emit_expr(&args[1].value, locals)?;
                    let (val, vt) = self.emit_expr(&args[2].value, locals)?;
                    if vt != Type::Int {
                        return Err("internal error: store_u8 value must be int".into());
                    }
                    let mut indices = [off];
                    let ptr = match bt {
                        Type::Str => {
                            let n = self.cstr("mem.strptr");
                            LLVMBuildGEP2(self.builder, self.i8, base, indices.as_mut_ptr(), 1, n.as_ptr())
                        }
                        _ => {
                            let n = self.cstr("mem.ptr");
                            let total = LLVMBuildAdd(self.builder, base, off, self.cstr("mem.addr").as_ptr());
                            LLVMBuildIntToPtr(self.builder, total, self.ptr, n.as_ptr())
                        }
                    };
                    let trunc = LLVMBuildTrunc(self.builder, val, self.i8, self.cstr("mem.u8.t").as_ptr());
                    LLVMBuildStore(self.builder, trunc, ptr);
                    return Ok((std::ptr::null_mut(), Type::Void));
                }
                // pointer reinterpretation: int <-> string (same 8 bytes)
                if name == "as_string" {
                    if args.len() != 1 {
                        return Err("internal error: as_string expects 1 argument".into());
                    }
                    let (v, t) = self.emit_expr(&args[0].value, locals)?;
                    if t != Type::Int {
                        return Err("internal error: as_string expects int".into());
                    }
                    let n = self.cstr("mem.asstr");
                    let p = LLVMBuildIntToPtr(self.builder, v, self.ptr, n.as_ptr());
                    return Ok((p, Type::Str));
                }
                if name == "as_ptr" {
                    if args.len() != 1 {
                        return Err("internal error: as_ptr expects 1 argument".into());
                    }
                    let (v, t) = self.emit_expr(&args[0].value, locals)?;
                    if t != Type::Str {
                        return Err("internal error: as_ptr expects string".into());
                    }
                    let n = self.cstr("mem.asint");
                    let p = LLVMBuildPtrToInt(self.builder, v, self.i64, n.as_ptr());
                    return Ok((p, Type::Int));
                }
                // struct construction: Name(field=value, ...)
                if !self.fns.contains_key(name) {
                    if self.struct_fields.contains_key(name) {
                        let mut kws: Vec<(String, Expr)> = Vec::with_capacity(args.len());
                        for a in args {
                            match &a.name {
                                Some(n) => kws.push((n.clone(), a.value.clone())),
                                None => {
                                    return Err(format!(
                                        "internal error: unnamed argument at line {}",
                                        pos.line
                                    ))
                                }
                            }
                        }
                        return self.emit_struct_construction(name, &kws, *lit_id, locals);
                    }
                    return Err(format!("internal error: unknown callable '{name}' at codegen"));
                }
                let (fn_ty, fn_ref, ret_ty) = {
                    let info = self.fns.get(name).unwrap();
                    (info.fn_ty, info.ref_, info.ret.clone())
                };
                let mut vals: Vec<LLVMValueRef> = Vec::with_capacity(args.len());
                for a in args {
                    let (v, _) = self.emit_expr(&a.value, locals)?;
                    vals.push(v);
                }
                let r = LLVMBuildCall2(self.builder, fn_ty, fn_ref, vals.as_mut_ptr(), vals.len() as u32, self.cstr("").as_ptr());
                if ret_ty == Type::Void {
                    Ok((r, Type::Void))
                } else {
                    Ok((r, ret_ty))
                }
            }
            Expr::Unary { op, expr, .. } => {
                let (v, t) = self.emit_expr(expr, locals)?;
                match op {
                    UnOp::Not => {
                        let r = LLVMBuildNot(self.builder, v, self.cstr("not").as_ptr());
                        Ok((r, Type::Bool))
                    }
                    UnOp::Neg => {
                        let r = if t == Type::Float {
                            LLVMBuildFNeg(self.builder, v, self.cstr("neg").as_ptr())
                        } else {
                            let zero = self.const_int(&t, 0);
                            LLVMBuildSub(self.builder, zero, v, self.cstr("neg").as_ptr())
                        };
                        Ok((r, t))
                    }
                }
            }
            Expr::Binary { op, lhs, rhs, pos: _ } => self.emit_binary(*op, lhs, rhs, locals),
        }
    }

    unsafe fn emit_struct_construction(
        &mut self,
        name: &str,
        fields: &[(String, Expr)],
        lit_id: usize,
        locals: &mut Locals,
    ) -> Result<(LLVMValueRef, Type), String> {
        let layout = self
            .struct_fields
            .get(name)
            .cloned()
            .ok_or_else(|| format!("internal error: unknown struct '{name}' at codegen"))?;
        let struct_ty = self.structs[name];
        let temp = self.lit_temp(lit_id, struct_ty);
        for (fname, fexpr) in fields {
            let (fidx, _fty) = layout
                .iter()
                .find(|(n, _, _)| n == fname)
                .map(|(_, i, t)| (*i, t.clone()))
                .ok_or_else(|| format!("internal error: unknown field '{fname}' of '{name}'"))?;
            let (v, _t) = self.emit_expr(fexpr, locals)?;
            let zero = LLVMConstInt(self.i32, 0, 0);
            let fconst = LLVMConstInt(self.i32, fidx as u64, 0);
            let mut indices = [zero, fconst];
            let nn = self.cstr("ctor.field");
            let gep = LLVMBuildGEP2(self.builder, struct_ty, temp, indices.as_mut_ptr(), 2, nn.as_ptr());
            LLVMBuildStore(self.builder, v, gep);
        }
        let agg = LLVMBuildLoad2(self.builder, struct_ty, temp, self.cstr("ctor.val").as_ptr());
        Ok((agg, Type::Struct(name.to_string())))
    }

    unsafe fn get_printf(&mut self) -> LLVMValueRef {
        let existing = LLVMGetNamedFunction(self.module, self.cstr("printf").as_ptr());
        if !existing.is_null() {
            return existing;
        }
        let mut params = [self.ptr];
        let ty = LLVMFunctionType(self.i32, params.as_mut_ptr(), 1, 1);
        self.printf_ty = ty;
        LLVMAddFunction(self.module, self.cstr("printf").as_ptr(), ty)
    }

    /// declare (or look up) a C runtime function
    unsafe fn get_extern(
        &mut self,
        name: &str,
        ret: LLVMTypeRef,
        params: &[LLVMTypeRef],
        varargs: bool,
    ) -> (LLVMValueRef, LLVMTypeRef) {
        if let Some(hit) = self.externs.get(name) {
            return *hit;
        }
        let mut ps: Vec<LLVMTypeRef> = params.to_vec();
        let ty = LLVMFunctionType(ret, ps.as_mut_ptr(), ps.len() as u32, varargs as LLVMBool);
        let existing = LLVMGetNamedFunction(self.module, self.cstr(name).as_ptr());
        let f = if existing.is_null() {
            LLVMAddFunction(self.module, self.cstr(name).as_ptr(), ty)
        } else {
            existing
        };
        self.externs.insert(name.to_string(), (f, ty));
        (f, ty)
    }

    /// string concatenation: malloc(len1+len2+1), copy both, NUL-terminate.
    /// Strings are immutable and intentionally not freed (no GC yet).
    unsafe fn emit_str_concat(&mut self, l: LLVMValueRef, r: LLVMValueRef) -> Result<(LLVMValueRef, Type), String> {
        let (strlen_f, strlen_ty) = self.get_extern("strlen", self.i64, &[self.ptr], false);
        let (malloc_f, malloc_ty) = self.get_extern("malloc", self.ptr, &[self.i64], false);

        let mut largs = [l];
        let ll = LLVMBuildCall2(self.builder, strlen_ty, strlen_f, largs.as_mut_ptr(), 1, self.cstr("str.len1").as_ptr());
        let mut rargs = [r];
        let rl = LLVMBuildCall2(self.builder, strlen_ty, strlen_f, rargs.as_mut_ptr(), 1, self.cstr("str.len2").as_ptr());

        let sum = LLVMBuildAdd(self.builder, ll, rl, self.cstr("str.sum").as_ptr());
        let one = LLVMConstInt(self.i64, 1, 0);
        let total1 = LLVMBuildAdd(self.builder, sum, one, self.cstr("str.cap").as_ptr());
        let mut margs = [total1];
        let buf = LLVMBuildCall2(self.builder, malloc_ty, malloc_f, margs.as_mut_ptr(), 1, self.cstr("str.buf").as_ptr());

        LLVMBuildMemCpy(self.builder, buf, 0, l, 0, ll);
        let mut mid_idx = [ll];
        let mid = LLVMBuildGEP2(self.builder, self.i8, buf, mid_idx.as_mut_ptr(), 1, self.cstr("str.mid").as_ptr());
        LLVMBuildMemCpy(self.builder, mid, 0, r, 0, rl);
        let mut end_idx = [sum];
        let endp = LLVMBuildGEP2(self.builder, self.i8, buf, end_idx.as_mut_ptr(), 1, self.cstr("str.end").as_ptr());
        LLVMBuildStore(self.builder, LLVMConstInt(self.i8, 0, 0), endp);
        Ok((buf, Type::Str))
    }

    /// lexicographic byte comparison via strcmp
    unsafe fn emit_str_cmp(&mut self, op: BinOp, l: LLVMValueRef, r: LLVMValueRef) -> Result<(LLVMValueRef, Type), String> {
        let (strcmp_f, strcmp_ty) = self.get_extern("strcmp", self.i32, &[self.ptr, self.ptr], false);
        let mut cargs = [l, r];
        let res = LLVMBuildCall2(self.builder, strcmp_ty, strcmp_f, cargs.as_mut_ptr(), 2, self.cstr("str.cmp").as_ptr());
        let zero = LLVMConstInt(self.i32, 0, 0);
        let pred = match op {
            BinOp::Eq => INT_EQ,
            BinOp::Ne => INT_NE,
            BinOp::Lt => INT_SLT,
            BinOp::Le => INT_SLE,
            BinOp::Gt => INT_SGT,
            BinOp::Ge => INT_SGE,
            _ => return Err("internal error: bad string comparison".into()),
        };
        let n = self.cstr("str.bool");
        let v = LLVMBuildICmp(self.builder, pred, res, zero, n.as_ptr());
        Ok((v, Type::Bool))
    }

    unsafe fn print_builtin(&mut self, args: &[Arg], locals: &mut Locals) -> Result<(LLVMValueRef, Type), String> {
        let (v, t) = self.emit_expr(&args[0].value, locals)?;
        let printf = self.get_printf();

        if t == Type::Bool {
            // print readable `true` / `false` instead of 1 / 0
            let str_true = self.string_lit("true");
            let str_false = self.string_lit("false");
            let fp = self.fmt_lit("%s\n");
            let fn_ref = self.fns[&self.cur_fn].ref_;
            let tbb = self.add_bb(fn_ref, "print.true");
            let fbb = self.add_bb(fn_ref, "print.false");
            let end = self.add_bb(fn_ref, "print.end");
            LLVMBuildCondBr(self.builder, v, tbb, fbb);

            self.pos(tbb);
            let mut a1 = [fp, str_true];
            LLVMBuildCall2(self.builder, self.printf_ty, printf, a1.as_mut_ptr(), 2, self.cstr("").as_ptr());
            LLVMBuildBr(self.builder, end);

            self.pos(fbb);
            let mut a2 = [fp, str_false];
            LLVMBuildCall2(self.builder, self.printf_ty, printf, a2.as_mut_ptr(), 2, self.cstr("").as_ptr());
            LLVMBuildBr(self.builder, end);

            self.pos(end);
            return Ok((std::ptr::null_mut(), Type::Void));
        }

        let fmt = match t {
            Type::Int => "%lld\n",
            Type::Float => "%f\n",
            Type::Str => "%s\n",
            _ => return Err("internal error: unexpected print type".into()),
        };
        let fmt_ptr = self.fmt_lit(fmt);
        let mut argv: [LLVMValueRef; 2] = [fmt_ptr, v];
        let fn_ty = self.printf_ty;
        let r = LLVMBuildCall2(self.builder, fn_ty, printf, argv.as_mut_ptr(), 2, self.cstr("").as_ptr());
        Ok((r, Type::Void))
    }

    unsafe fn emit_binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, locals: &mut Locals) -> Result<(LLVMValueRef, Type), String> {
        use BinOp::*;
        match op {
            And | Or => self.emit_short_circuit(op, lhs, rhs, locals),
            _ => {
                let (l, lt) = self.emit_expr(lhs, locals)?;
                let (r, _rt) = self.emit_expr(rhs, locals)?;
                let t = lt; // typecheck guarantees lhs and rhs have the same type
                if t == Type::Str {
                    match op {
                        BinOp::Add => return self.emit_str_concat(l, r),
                        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                            return self.emit_str_cmp(op, l, r)
                        }
                        _ => return Err("internal error: invalid string operator".into()),
                    }
                }
                let n = self.cstr("tmp");
                let v = match (op, &t) {
                    (Add, Type::Int) => LLVMBuildAdd(self.builder, l, r, n.as_ptr()),
                    (Sub, Type::Int) => LLVMBuildSub(self.builder, l, r, n.as_ptr()),
                    (Mul, Type::Int) => LLVMBuildMul(self.builder, l, r, n.as_ptr()),
                    (Div, Type::Int) => LLVMBuildSDiv(self.builder, l, r, n.as_ptr()),
                    (Mod, Type::Int) => LLVMBuildSRem(self.builder, l, r, n.as_ptr()),
                    (Add, _) => LLVMBuildFAdd(self.builder, l, r, n.as_ptr()),
                    (Sub, _) => LLVMBuildFSub(self.builder, l, r, n.as_ptr()),
                    (Mul, _) => LLVMBuildFMul(self.builder, l, r, n.as_ptr()),
                    (Div, _) => LLVMBuildFDiv(self.builder, l, r, n.as_ptr()),
                    (Mod, _) => return Err("internal error: float mod".into()),
                    (Eq, Type::Int) => LLVMBuildICmp(self.builder, INT_EQ, l, r, n.as_ptr()),
                    (Ne, Type::Int) => LLVMBuildICmp(self.builder, INT_NE, l, r, n.as_ptr()),
                    (Lt, Type::Int) => LLVMBuildICmp(self.builder, INT_SLT, l, r, n.as_ptr()),
                    (Le, Type::Int) => LLVMBuildICmp(self.builder, INT_SLE, l, r, n.as_ptr()),
                    (Gt, Type::Int) => LLVMBuildICmp(self.builder, INT_SGT, l, r, n.as_ptr()),
                    (Ge, Type::Int) => LLVMBuildICmp(self.builder, INT_SGE, l, r, n.as_ptr()),
                    (Eq, Type::Bool) => LLVMBuildICmp(self.builder, INT_EQ, l, r, n.as_ptr()),
                    (Ne, Type::Bool) => LLVMBuildICmp(self.builder, INT_NE, l, r, n.as_ptr()),
                    (Eq, _) => LLVMBuildFCmp(self.builder, REAL_OEQ, l, r, n.as_ptr()),
                    (Ne, _) => LLVMBuildFCmp(self.builder, REAL_UNE, l, r, n.as_ptr()),
                    (Lt, _) => LLVMBuildFCmp(self.builder, REAL_OLT, l, r, n.as_ptr()),
                    (Le, _) => LLVMBuildFCmp(self.builder, REAL_OLE, l, r, n.as_ptr()),
                    (Gt, _) => LLVMBuildFCmp(self.builder, REAL_OGT, l, r, n.as_ptr()),
                    (Ge, _) => LLVMBuildFCmp(self.builder, REAL_OGE, l, r, n.as_ptr()),
                    (And | Or, _) => return Err("internal error: boolean operator reached arithmetic codegen".into()),
                };
                let out_ty = match op {
                    Eq | Ne | Lt | Le | Gt | Ge => Type::Bool,
                    _ => t,
                };
                Ok((v, out_ty))
            }
        }
    }

    unsafe fn emit_short_circuit(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, locals: &mut Locals) -> Result<(LLVMValueRef, Type), String> {
        let (l, _) = self.emit_expr(lhs, locals)?;
        let lhs_end_bb = self.cur_bb;
        let fn_ref = self.fns[&self.cur_fn].ref_;
        let rhs_bb = self.add_bb(fn_ref, "sc.rhs");
        let end_bb = self.add_bb(fn_ref, "sc.end");

        match op {
            BinOp::And => LLVMBuildCondBr(self.builder, l, rhs_bb, end_bb),
            _ => LLVMBuildCondBr(self.builder, l, end_bb, rhs_bb),
        };

        self.pos(rhs_bb);
        let (r, _) = self.emit_expr(rhs, locals)?;
        let rhs_end_bb = self.cur_bb;
        if !self.terminated() {
            LLVMBuildBr(self.builder, end_bb);
        }

        self.pos(end_bb);
        let phi = LLVMBuildPhi(self.builder, self.i1, self.cstr("sc.val").as_ptr());
        let mut vals: Vec<LLVMValueRef> = Vec::with_capacity(2);
        let mut bbs: Vec<LLVMBasicBlockRef> = Vec::with_capacity(2);

        match op {
            BinOp::And => {
                vals.push(LLVMConstInt(self.i1, 0, 0));
                bbs.push(lhs_end_bb);
            }
            _ => {
                vals.push(LLVMConstInt(self.i1, 1, 0));
                bbs.push(lhs_end_bb);
            }
        }
        vals.push(r);
        bbs.push(rhs_end_bb);

        LLVMAddIncoming(phi, vals.as_mut_ptr(), bbs.as_mut_ptr(), vals.len() as u32);
        Ok((phi, Type::Bool))
    }
}
