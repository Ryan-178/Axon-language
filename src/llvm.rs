//! Raw FFI bindings to the LLVM-C API (LLVM-C.lib / LLVM-C.dll).
//! Hand-written thin layer: no heavyweight bindings crates.
//! Every declaration here must match the LLVM C API exactly.

#![allow(non_snake_case, dead_code)]

use std::os::raw::{c_char, c_int, c_uint};

pub type LLVMContextRef = *mut ();
pub type LLVMModuleRef = *mut ();
pub type LLVMBuilderRef = *mut ();
pub type LLVMTypeRef = *mut ();
pub type LLVMValueRef = *mut ();
pub type LLVMBasicBlockRef = *mut ();
pub type LLVMTargetRef = *mut ();
pub type LLVMTargetMachineRef = *mut ();
pub type LLVMTargetDataRef = *mut ();
pub type LLVMBool = c_int;

// ---- Context / Module / Builder ----
extern "C" {
    pub fn LLVMContextCreate() -> LLVMContextRef;
    pub fn LLVMModuleCreateWithNameInContext(name: *const c_char, ctx: LLVMContextRef) -> LLVMModuleRef;
    // NOTE: LLVMBuilderCreate is missing from the LLVM-C.lib shipped by the
    // official Windows installer; LLVMCreateBuilderInContext is the same thing.
    pub fn LLVMCreateBuilderInContext(ctx: LLVMContextRef) -> LLVMBuilderRef;
    pub fn LLVMDisposeModule(m: LLVMModuleRef);
    pub fn LLVMDisposeBuilder(b: LLVMBuilderRef);
    pub fn LLVMContextDispose(c: LLVMContextRef);
}

// ---- Types ----
extern "C" {
    pub fn LLVMInt1TypeInContext(ctx: LLVMContextRef) -> LLVMTypeRef;
    pub fn LLVMInt8TypeInContext(ctx: LLVMContextRef) -> LLVMTypeRef;
    pub fn LLVMInt32TypeInContext(ctx: LLVMContextRef) -> LLVMTypeRef;
    pub fn LLVMInt64TypeInContext(ctx: LLVMContextRef) -> LLVMTypeRef;
    pub fn LLVMDoubleTypeInContext(ctx: LLVMContextRef) -> LLVMTypeRef;
    pub fn LLVMVoidTypeInContext(ctx: LLVMContextRef) -> LLVMTypeRef;
    pub fn LLVMArrayType(elem: LLVMTypeRef, count: c_uint) -> LLVMTypeRef;
    pub fn LLVMPointerTypeInContext(ctx: LLVMContextRef, address_space: c_uint) -> LLVMTypeRef;
    pub fn LLVMStructCreateNamed(ctx: LLVMContextRef, name: *const c_char) -> LLVMTypeRef;
    pub fn LLVMStructSetBody(s: LLVMTypeRef, fields: *mut LLVMTypeRef, field_count: c_uint, packed: LLVMBool);
    pub fn LLVMFunctionType(ret: LLVMTypeRef, params: *mut LLVMTypeRef, param_count: c_uint, is_var_arg: LLVMBool) -> LLVMTypeRef;
}

// ---- Values / Functions / Blocks ----
extern "C" {
    pub fn LLVMAddFunction(m: LLVMModuleRef, name: *const c_char, fn_ty: LLVMTypeRef) -> LLVMValueRef;
    pub fn LLVMGetNamedFunction(m: LLVMModuleRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMConstInt(ty: LLVMTypeRef, val: u64, sign_extend: LLVMBool) -> LLVMValueRef;
    pub fn LLVMConstReal(ty: LLVMTypeRef, val: f64) -> LLVMValueRef;
    pub fn LLVMGetParam(f: LLVMValueRef, index: c_uint) -> LLVMValueRef;
    pub fn LLVMAppendBasicBlockInContext(ctx: LLVMContextRef, f: LLVMValueRef, name: *const c_char) -> LLVMBasicBlockRef;
    pub fn LLVMPositionBuilderAtEnd(b: LLVMBuilderRef, bb: LLVMBasicBlockRef);
    pub fn LLVMPositionBuilderBefore(b: LLVMBuilderRef, instr: LLVMValueRef);
    pub fn LLVMGetFirstInstruction(bb: LLVMBasicBlockRef) -> LLVMValueRef;
    pub fn LLVMGetBasicBlockTerminator(bb: LLVMBasicBlockRef) -> LLVMValueRef;
}

// ---- Build instructions ----
extern "C" {
    pub fn LLVMBuildAlloca(b: LLVMBuilderRef, ty: LLVMTypeRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildGEP2(b: LLVMBuilderRef, ty: LLVMTypeRef, ptr: LLVMValueRef, indices: *mut LLVMValueRef, index_count: c_uint, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildMemCpy(b: LLVMBuilderRef, dst: LLVMValueRef, dst_align: c_uint, src: LLVMValueRef, src_align: c_uint, size: LLVMValueRef) -> LLVMValueRef;
    pub fn LLVMSizeOf(ty: LLVMTypeRef) -> LLVMValueRef;
    pub fn LLVMBuildStore(b: LLVMBuilderRef, val: LLVMValueRef, ptr: LLVMValueRef) -> LLVMValueRef;
    pub fn LLVMBuildLoad2(b: LLVMBuilderRef, ty: LLVMTypeRef, ptr: LLVMValueRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildAdd(b: LLVMBuilderRef, l: LLVMValueRef, r: LLVMValueRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildSub(b: LLVMBuilderRef, l: LLVMValueRef, r: LLVMValueRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildMul(b: LLVMBuilderRef, l: LLVMValueRef, r: LLVMValueRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildSDiv(b: LLVMBuilderRef, l: LLVMValueRef, r: LLVMValueRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildSRem(b: LLVMBuilderRef, l: LLVMValueRef, r: LLVMValueRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildFAdd(b: LLVMBuilderRef, l: LLVMValueRef, r: LLVMValueRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildFSub(b: LLVMBuilderRef, l: LLVMValueRef, r: LLVMValueRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildFMul(b: LLVMBuilderRef, l: LLVMValueRef, r: LLVMValueRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildFDiv(b: LLVMBuilderRef, l: LLVMValueRef, r: LLVMValueRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildICmp(b: LLVMBuilderRef, pred: c_uint, l: LLVMValueRef, r: LLVMValueRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildSelect(b: LLVMBuilderRef, cond: LLVMValueRef, if_true: LLVMValueRef, if_false: LLVMValueRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildFCmp(b: LLVMBuilderRef, pred: c_uint, l: LLVMValueRef, r: LLVMValueRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildNot(b: LLVMBuilderRef, v: LLVMValueRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildFNeg(b: LLVMBuilderRef, v: LLVMValueRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildZExt(b: LLVMBuilderRef, v: LLVMValueRef, ty: LLVMTypeRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildTrunc(b: LLVMBuilderRef, v: LLVMValueRef, ty: LLVMTypeRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildBr(b: LLVMBuilderRef, dest: LLVMBasicBlockRef) -> LLVMValueRef;
    pub fn LLVMBuildCondBr(b: LLVMBuilderRef, cond: LLVMValueRef, then_bb: LLVMBasicBlockRef, else_bb: LLVMBasicBlockRef) -> LLVMValueRef;
    pub fn LLVMBuildRet(b: LLVMBuilderRef, v: LLVMValueRef) -> LLVMValueRef;
    pub fn LLVMBuildRetVoid(b: LLVMBuilderRef) -> LLVMValueRef;
    pub fn LLVMBuildUnreachable(b: LLVMBuilderRef) -> LLVMValueRef;
    pub fn LLVMBuildCall2(b: LLVMBuilderRef, fn_ty: LLVMTypeRef, callee: LLVMValueRef, args: *mut LLVMValueRef, arg_count: c_uint, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMBuildPhi(b: LLVMBuilderRef, ty: LLVMTypeRef, name: *const c_char) -> LLVMValueRef;
    pub fn LLVMAddIncoming(phi: LLVMValueRef, vals: *mut LLVMValueRef, blocks: *mut LLVMBasicBlockRef, count: c_uint);
    pub fn LLVMBuildGlobalStringPtr(b: LLVMBuilderRef, s: *const c_char, name: *const c_char) -> LLVMValueRef;
}

// ICmp predicates
pub const INT_EQ: c_uint = 32;
pub const INT_NE: c_uint = 33;
pub const INT_SGT: c_uint = 38;
pub const INT_SGE: c_uint = 39;
pub const INT_SLT: c_uint = 40;
pub const INT_SLE: c_uint = 41;

// FCmp predicates
pub const REAL_OEQ: c_uint = 1;
pub const REAL_OGT: c_uint = 2;
pub const REAL_OGE: c_uint = 3;
pub const REAL_OLT: c_uint = 4;
pub const REAL_OLE: c_uint = 5;
pub const REAL_UNE: c_uint = 6;

// ---- Verification / Debug printing ----
extern "C" {
    pub fn LLVMVerifyModule(m: LLVMModuleRef, action: c_uint, out_message: *mut *mut c_char) -> LLVMBool;
    pub fn LLVMPrintModuleToString(m: LLVMModuleRef) -> *mut c_char;
    pub fn LLVMDisposeMessage(msg: *mut c_char);
}

pub const VERIFY_RETURN_STATUS: c_uint = 0;

// ---- Target / code emission ----
extern "C" {
    pub fn LLVMInitializeX86TargetInfo();
    pub fn LLVMInitializeX86Target();
    pub fn LLVMInitializeX86TargetMC();
    pub fn LLVMInitializeX86AsmPrinter();
    pub fn LLVMGetDefaultTargetTriple() -> *mut c_char;
    pub fn LLVMGetTargetFromTriple(triple: *const c_char, target: *mut LLVMTargetRef, err: *mut *mut c_char) -> LLVMBool;
    pub fn LLVMCreateTargetMachine(t: LLVMTargetRef, triple: *const c_char, cpu: *const c_char, features: *const c_char, level: c_uint, reloc: c_uint, code_model: c_uint) -> LLVMTargetMachineRef;
    pub fn LLVMCreateTargetDataLayout(tm: LLVMTargetMachineRef) -> LLVMTargetDataRef;
    pub fn LLVMSetTarget(m: LLVMModuleRef, triple: *const c_char);
    pub fn LLVMSetModuleDataLayout(m: LLVMModuleRef, dl: LLVMTargetDataRef);
    pub fn LLVMTargetMachineEmitToFile(tm: LLVMTargetMachineRef, m: LLVMModuleRef, filename: *const c_char, code_type: c_uint, err: *mut *mut c_char) -> LLVMBool;
}

// CodeGenOptLevel
pub const CODEGEN_LEVEL_DEFAULT: c_uint = 2;
pub const CODEGEN_LEVEL_AGGRESSIVE: c_uint = 3;
// RelocMode
pub const RELOC_DEFAULT: c_uint = 0;
// CodeModel
pub const CODE_MODEL_DEFAULT: c_uint = 0;
// LLVMCodeGenFileType
pub const CODEGEN_OBJECT_FILE: c_uint = 1;

// ---- New Pass Builder (IR optimization) ----
extern "C" {
    pub fn LLVMCreatePassBuilderOptions() -> *mut ();
    pub fn LLVMRunPasses(m: LLVMModuleRef, passes: *const c_char, tm: LLVMTargetMachineRef, options: *mut ()) -> *mut c_char;
    pub fn LLVMDisposePassBuilderOptions(opts: *mut ());
}
