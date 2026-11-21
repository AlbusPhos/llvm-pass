// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! 必须复用 LLVM C++ 实现的 IR 辅助函数。
//!
//! LLVM C API 没有提供这些能力（或无法等价实现），因此通过 `cxx` 转发到
//! LLVM C++ 实现。函数参数均为裸指针，调用方必须保证生命周期与对象类型正确。
//!
//! 本模块不依赖 `inkwell` 特性。

use std::ffi::c_void;
use std::pin::Pin;

use crate::ffi;

/// 将指定指令替换为 `unreachable`，并删除该基本块内位于其后的指令。
///
/// 返回被删除的指令数量。
///
/// # Safety
///
/// `inst` 必须是有效的 `llvm::Instruction*`。返回后，`inst` 以及同一基本块中
/// 位于其后的所有指令均已被销毁，不得再次访问。
#[inline]
pub unsafe fn change_to_unreachable(inst: *mut c_void) -> u32 {
    // SAFETY: 调用方保证指针有效。
    unsafe { ffi::ir_change_to_unreachable(inst.cast()) }
}

/// 清空函数体（`dropAllReferences` + `deleteBody`）。
///
/// # Safety
///
/// `func` 必须是有效的 `llvm::Function*`。调用后，函数内所有基本块与指令均失效。
#[inline]
pub unsafe fn reset_function_body(func: *mut c_void) {
    // SAFETY: 调用方保证指针有效；`Pin` 仅用于匹配 cxx 桥接签名，不移动对象。
    let func = unsafe { Pin::new_unchecked(&mut *(func as *mut ffi::Function)) };
    ffi::ir_delete_function_body(func);
}

/// 设置全局值的 `dso_local` 标志（`GlobalValue::setDSOLocal`）。
///
/// # Safety
///
/// `gv` 必须是有效的 `llvm::Value*`。
#[inline]
pub unsafe fn set_dso_local(gv: *mut c_void, dso_local: bool) {
    // SAFETY: 调用方保证指针有效。
    unsafe { ffi::ir_set_dso_local(gv.cast(), dso_local) }
}

/// 在 `split_pt` 处切分基本块 `bb`，返回新建的后继基本块。
///
/// # Safety
///
/// `bb` 与 `split_pt` 必须有效，且 `split_pt` 属于 `bb`。
#[inline]
pub unsafe fn split_block(bb: *mut c_void, split_pt: *mut c_void, name: &str) -> *mut c_void {
    // SAFETY: 调用方保证指针有效。
    unsafe { ffi::ir_split_basic_block(bb.cast(), split_pt.cast(), name).cast() }
}

/// 返回基本块的首个合法插入点；若到达块末尾则返回空指针。
///
/// # Safety
///
/// `bb` 必须是有效的 `llvm::BasicBlock*`。
#[inline]
pub unsafe fn first_insertion_pt(bb: *mut c_void) -> *mut c_void {
    // SAFETY: 调用方保证指针有效。
    unsafe { ffi::ir_first_insertion_pt(bb.cast()).cast() }
}

/// 返回首条非 PHI、非调试记录、非静态 alloca 的指令；若不存在则返回空指针。
///
/// # Safety
///
/// `bb` 必须是有效的 `llvm::BasicBlock*`。
#[inline]
pub unsafe fn first_non_phi_or_dbg_or_alloca(bb: *mut c_void) -> *mut c_void {
    // SAFETY: 调用方保证指针有效。
    unsafe { ffi::ir_first_non_phi_or_dbg_or_alloca(bb.cast()).cast() }
}

/// 将 `inst` 移动到 `before` 之前。
///
/// # Safety
///
/// `inst` 与 `before` 必须是已插入基本块的有效指令。
#[inline]
pub unsafe fn move_inst_before(inst: *mut c_void, before: *mut c_void) {
    // SAFETY: 调用方保证指针有效。
    unsafe { ffi::ir_move_inst_before(inst.cast(), before.cast()) }
}

/// 将 `inst` 移动到 `bb` 的首个合法插入点。
///
/// # Safety
///
/// `inst` 与 `bb` 必须有效。
#[inline]
pub unsafe fn move_inst_to_first_insertion_pt(inst: *mut c_void, bb: *mut c_void) {
    // SAFETY: 调用方保证指针有效。
    unsafe { ffi::ir_move_inst_to_first_insertion_pt(inst.cast(), bb.cast()) }
}

/// 将 SSA 指令结果降级为栈槽（`DemoteRegToStack`），返回生成的 alloca。
///
/// 若 `inst` 没有任何使用者，LLVM 会直接销毁它并返回空指针。
///
/// # Safety
///
/// `inst` 必须是有效的 `llvm::Instruction*`；`alloca_pt` 可为空。
#[inline]
pub unsafe fn demote_reg_to_stack(
    inst: *mut c_void,
    volatile_loads: bool,
    alloca_pt: *mut c_void,
) -> *mut c_void {
    // SAFETY: 调用方保证指针有效。
    unsafe { ffi::ir_demote_reg_to_stack(inst.cast(), volatile_loads, alloca_pt.cast()).cast() }
}

/// 将 `PHINode` 降级为栈槽（`DemotePHIToStack`），返回生成的 alloca。
///
/// # Safety
///
/// `phi` 必须是有效的 `llvm::PHINode*`；调用后原 PHI 指令会被销毁。
#[inline]
pub unsafe fn demote_phi_to_stack(phi: *mut c_void, alloca_pt: *mut c_void) -> *mut c_void {
    // SAFETY: 调用方保证指针有效。
    unsafe { ffi::ir_demote_phi_to_stack(phi.cast(), alloca_pt.cast()).cast() }
}

/// 判断指令是否可能抛出异常。
///
/// # Safety
///
/// `inst` 必须是有效的 `llvm::Instruction*`。
#[inline]
pub unsafe fn may_throw(inst: *const c_void) -> bool {
    // SAFETY: 调用方保证指针有效。
    unsafe { ffi::ir_may_throw(inst.cast()) }
}

/// 判断指令是否可能具有副作用。
///
/// # Safety
///
/// `inst` 必须是有效的 `llvm::Instruction*`。
#[inline]
pub unsafe fn may_have_side_effects(inst: *const c_void) -> bool {
    // SAFETY: 调用方保证指针有效。
    unsafe { ffi::ir_may_have_side_effects(inst.cast()) }
}

/// 判断指令是否为静态 `alloca`。
///
/// # Safety
///
/// `inst` 必须是有效的 `llvm::Instruction*`。
#[inline]
pub unsafe fn is_static_alloca(inst: *const c_void) -> bool {
    // SAFETY: 调用方保证指针有效。
    unsafe { ffi::ir_is_static_alloca(inst.cast()) }
}

/// 判断指令结果是否在指定基本块外部被使用。
///
/// # Safety
///
/// `inst` 与 `bb` 必须有效。
#[inline]
pub unsafe fn is_used_outside_of_block(inst: *const c_void, bb: *const c_void) -> bool {
    // SAFETY: 调用方保证指针有效。
    unsafe { ffi::ir_is_used_outside_of_block(inst.cast(), bb.cast()) }
}

/// 判断 GEP 的第 `operand_no` 个操作数（从 1 开始）是否索引进结构体字段。
///
/// # Safety
///
/// `gep` 必须是有效的 `llvm::Instruction*`。
#[inline]
pub unsafe fn gep_index_is_struct(gep: *const c_void, operand_no: u32) -> bool {
    // SAFETY: 调用方保证指针有效。
    unsafe { ffi::ir_gep_index_is_struct(gep.cast(), operand_no) }
}
