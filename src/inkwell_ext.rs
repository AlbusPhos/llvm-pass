// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! 基于 `inkwell` / `llvm-sys` 的 LLVM IR 扩展 Trait。
//!
//! 这些 Trait 处理 token、operand bundle、switch、DataLayout 等 `inkwell`
//! 未封装的能力。仅在启用 `inkwell` 特性时参与编译。

use crate::{ffi, ir_utils};
use std::ffi::CString;

use inkwell::basic_block::BasicBlock;
use inkwell::llvm_sys::LLVMTypeKind;
use inkwell::llvm_sys::comdat::LLVMSetComdat;
use inkwell::llvm_sys::core::{
    LLVMAddCase, LLVMBuildCallWithOperandBundles, LLVMCreateBuilderInContext, LLVMDisposeBuilder,
    LLVMDisposeOperandBundle, LLVMGetBasicBlockParent, LLVMGetCalledValue, LLVMGetGlobalParent,
    LLVMGetInlineAsmAsmString, LLVMGetInstructionCallConv, LLVMGetModuleContext,
    LLVMGetNumArgOperands, LLVMGetNumOperandBundles, LLVMGetNumOperands, LLVMGetNumSuccessors,
    LLVMGetOperand, LLVMGetOperandBundleAtIndex, LLVMGetSuccessor, LLVMGetSwitchDefaultDest,
    LLVMGetTypeKind, LLVMIsACallBrInst, LLVMIsACallInst, LLVMIsAInlineAsm, LLVMIsAInvokeInst,
    LLVMPositionBuilderBefore, LLVMSetInstructionCallConv, LLVMSetSuccessor, LLVMTypeOf,
};
use inkwell::llvm_sys::prelude::{LLVMOperandBundleRef, LLVMValueRef};
use inkwell::llvm_sys::target::{LLVMGetModuleDataLayout, LLVMIntPtrTypeForASInContext};
use inkwell::module::Module;
use inkwell::types::{AsTypeRef, IntType};
use inkwell::values::{
    AnyValueEnum, AsValueRef, BasicValue, BasicValueEnum, BasicValueUse, CallSiteValue,
    FunctionValue, GlobalValue, InstructionOpcode, InstructionValue, IntValue, PointerValue,
};

#[inline]
unsafe fn raw_value_is_token(value: LLVMValueRef) -> bool {
    // SAFETY: 调用方保证 value 是有效 LLVM 值。
    !value.is_null()
        && unsafe { LLVMGetTypeKind(LLVMTypeOf(value)) == LLVMTypeKind::LLVMTokenTypeKind }
}

#[inline]
unsafe fn is_call_base(inst: LLVMValueRef) -> bool {
    // SAFETY: 调用方保证 inst 是有效指令。
    unsafe {
        !LLVMIsACallInst(inst).is_null()
            || !LLVMIsAInvokeInst(inst).is_null()
            || !LLVMIsACallBrInst(inst).is_null()
    }
}

/// 针对 [`inkwell::values::AnyValueEnum`] 的指令提取扩展。
///
/// `BasicValueUse::get_user()` 返回的是按返回值类型分类的 `AnyValueEnum`，
/// 非 void 调用通常表现为 `IntValue` / `PointerValue` 等，而不是
/// `InstructionValue`。本 Trait 可统一还原为指令。
#[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
pub trait AnyValueExt<'ctx> {
    /// 若该值本质上是一条 LLVM 指令，则返回其 [`InstructionValue`]。
    fn as_instruction_value(&self) -> Option<InstructionValue<'ctx>>;
}

impl<'ctx> AnyValueExt<'ctx> for AnyValueEnum<'ctx> {
    #[inline]
    fn as_instruction_value(&self) -> Option<InstructionValue<'ctx>> {
        match self {
            AnyValueEnum::InstructionValue(inst) => Some(*inst),
            AnyValueEnum::IntValue(value) => value.as_instruction(),
            AnyValueEnum::FloatValue(value) => value.as_instruction(),
            AnyValueEnum::PointerValue(value) => value.as_instruction(),
            AnyValueEnum::ArrayValue(value) => value.as_instruction(),
            AnyValueEnum::StructValue(value) => value.as_instruction(),
            AnyValueEnum::VectorValue(value) => value.as_instruction(),
            AnyValueEnum::ScalableVectorValue(value) => value.as_instruction(),
            AnyValueEnum::PhiValue(value) => Some(value.as_instruction()),
            AnyValueEnum::FunctionValue(_) | AnyValueEnum::MetadataValue(_) => None,
        }
    }
}

/// 针对 [`inkwell::values::InstructionValue`] 的 LLVM IR 扩展方法。
#[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
#[allow(clippy::wrong_self_convention)] // InstructionValue 是 Copy，按值接收更便于链式调用
pub trait InstructionExt<'ctx> {
    /// 判断指令返回值是否为 `token` 类型（不触发 inkwell 的 `AnyTypeEnum` panic）。
    fn is_token_type(self) -> bool;

    /// 判断指令自身或其任一操作数是否为 `token` 类型。
    fn has_token(self) -> bool;

    /// 判断指令是否可能抛出异常。
    fn may_throw(self) -> bool;

    /// 判断指令是否可能具有副作用。
    fn may_have_side_effects(self) -> bool;

    /// 判断指令是否为静态 `alloca`。
    fn is_static_alloca(self) -> bool;

    /// 判断指令结果是否在基本块 `bb` 外部被使用。
    fn is_used_outside_of_block(self, bb: BasicBlock<'ctx>) -> bool;

    /// 若该指令为 GEP，判断第 `operand_no` 个操作数（从 1 开始）是否索引结构体。
    fn gep_index_is_struct(self, operand_no: u32) -> bool;

    /// 若该指令是调用类指令，返回实参数量。
    fn call_arg_count(self) -> Option<u32>;

    /// 判断第 `operand_no` 个操作数是否为调用实参。
    fn is_arg_operand(self, operand_no: u32) -> bool;

    /// 判断第 `operand_no` 个操作数是否为 operand bundle 的输入。
    ///
    /// 注意：`InvokeInst` / `CallBrInst` 在 bundle 与 callee 之间还有基本块操作数，
    /// 不能简单用 `arg_count..total-1` 判断。
    fn is_bundle_operand(self, operand_no: u32) -> bool;

    /// 判断第 `operand_no` 个操作数是否为被调函数（Callee）。
    fn is_callee_operand(self, operand_no: u32) -> bool;

    /// 判断该调用是否为内联汇编调用。
    fn is_inline_asm_call(self) -> bool;

    /// 若为内联汇编调用，返回汇编模板字节串。
    ///
    /// 返回拥有所有权的 `Vec<u8>`，避免引用 LLVM 内部 `InlineAsm` 对象的生命周期。
    fn inline_asm_bytes(self) -> Option<Vec<u8>>;

    /// 若为内联汇编调用且模板为合法 UTF-8，返回汇编模板字符串。
    fn inline_asm_string(self) -> Option<String>;

    /// 将当前指令移动到 `before` 之前。
    fn move_before(self, before: InstructionValue<'ctx>);

    /// 将当前指令移动到基本块 `bb` 的首个合法插入点。
    fn move_to_first_insertion_pt(self, bb: BasicBlock<'ctx>);

    /// 将 SSA 指令结果降级为栈槽，返回新 alloca 指针。
    ///
    /// # Safety
    ///
    /// `self` 必须是有效的非终结指令。若该指令没有任何使用者，
    /// `DemoteRegToStack` 会直接销毁它并返回空指针。
    unsafe fn demote_reg_to_stack(
        self,
        volatile_loads: bool,
        alloca_pt: Option<InstructionValue<'ctx>>,
    ) -> Option<PointerValue<'ctx>>;

    /// 将 `PHI` 节点降级为栈槽，返回新 alloca 指针。
    ///
    /// # Safety
    ///
    /// `self` 必须是 `PHI` 指令；调用后原 PHI 指令会被销毁，不得再次访问。
    unsafe fn demote_phi_to_stack(
        self,
        alloca_pt: Option<InstructionValue<'ctx>>,
    ) -> PointerValue<'ctx>;

    /// 在当前调用指令之前插入对 `callee` 的直接调用，并保留原调用的所有
    /// operand bundle 与调用约定。
    ///
    /// 若 `callee` 返回 `void`，`name` 会被忽略（LLVM 禁止为 void 指令命名）。
    ///
    /// # Safety
    ///
    /// `self` 必须是位于基本块内的 `CallInst` 或 `InvokeInst`；`args` 必须匹配
    /// `callee` 的签名。调用后原指令仍然有效。
    unsafe fn build_call_preserving_bundles(
        self,
        callee: FunctionValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
        name: &str,
    ) -> CallSiteValue<'ctx>;

    /// 将当前指令替换为 `unreachable`，并删除同一基本块内位于其后的指令。
    ///
    /// # Safety
    ///
    /// 调用后，当前指令以及同一基本块内位于其后的所有指令均失效。
    unsafe fn change_to_unreachable(self) -> u32;

    /// 向 `switch` 追加一个分支。
    fn switch_add_case(self, on_val: IntValue<'ctx>, dest: BasicBlock<'ctx>);

    /// 获取 `switch` 的非默认分支数量。
    fn switch_num_cases(self) -> u32;

    /// 获取第 `idx` 个分支的匹配常量与目标基本块。
    fn switch_get_case(self, idx: u32) -> Option<(IntValue<'ctx>, BasicBlock<'ctx>)>;

    /// 获取 `switch` 的默认目标基本块。
    fn switch_default_dest(self) -> Option<BasicBlock<'ctx>>;

    /// 设置 `switch` 的默认目标基本块。
    fn switch_set_default_dest(self, dest: BasicBlock<'ctx>);

    /// 获取终结指令的后继基本块数量。
    fn num_successors(self) -> u32;

    /// 获取第 `idx` 个后继基本块。
    fn get_successor(self, idx: u32) -> Option<BasicBlock<'ctx>>;

    /// 设置第 `idx` 个后继基本块。
    fn set_successor(self, idx: u32, dest: BasicBlock<'ctx>);
}

#[allow(clippy::wrong_self_convention)] // 与 Trait 声明保持一致
impl<'ctx> InstructionExt<'ctx> for InstructionValue<'ctx> {
    #[inline]
    fn is_token_type(self) -> bool {
        // SAFETY: self 是有效指令。
        unsafe { raw_value_is_token(self.as_value_ref()) }
    }

    #[inline]
    fn has_token(self) -> bool {
        let raw = self.as_value_ref();
        // SAFETY: self 是有效指令，所有操作数均有效。
        unsafe {
            if raw_value_is_token(raw) {
                return true;
            }
            let count = LLVMGetNumOperands(raw) as u32;
            (0..count).any(|index| raw_value_is_token(LLVMGetOperand(raw, index)))
        }
    }

    #[inline]
    fn may_throw(self) -> bool {
        // SAFETY: self 是有效指令。
        unsafe { ir_utils::may_throw(self.as_value_ref().cast()) }
    }

    #[inline]
    fn may_have_side_effects(self) -> bool {
        // SAFETY: self 是有效指令。
        unsafe { ir_utils::may_have_side_effects(self.as_value_ref().cast()) }
    }

    #[inline]
    fn is_static_alloca(self) -> bool {
        // SAFETY: self 是有效指令。
        unsafe { ir_utils::is_static_alloca(self.as_value_ref().cast()) }
    }

    #[inline]
    fn is_used_outside_of_block(self, bb: BasicBlock<'ctx>) -> bool {
        // SAFETY: self 与 bb 均有效，且同一函数内。
        unsafe {
            ir_utils::is_used_outside_of_block(self.as_value_ref().cast(), bb.as_mut_ptr().cast())
        }
    }

    #[inline]
    fn gep_index_is_struct(self, operand_no: u32) -> bool {
        // SAFETY: self 是有效指令。
        unsafe { ir_utils::gep_index_is_struct(self.as_value_ref().cast(), operand_no) }
    }

    #[inline]
    fn call_arg_count(self) -> Option<u32> {
        let raw = self.as_value_ref();
        // SAFETY: self 是有效指令。
        if unsafe { is_call_base(raw) } {
            // SAFETY: 已确认是 CallBase。
            Some(unsafe { LLVMGetNumArgOperands(raw) })
        } else {
            None
        }
    }

    #[inline]
    fn is_arg_operand(self, operand_no: u32) -> bool {
        self.call_arg_count()
            .is_some_and(|argument_count| operand_no < argument_count)
    }

    #[inline]
    fn is_bundle_operand(self, operand_no: u32) -> bool {
        let Some(argument_count) = self.call_arg_count() else {
            return false;
        };
        let total = self.get_num_operands();
        // Call: 0；Invoke: 2；CallBr: default + indirect dests。
        let extra_operands = self.num_successors();
        let bundle_end = total.saturating_sub(extra_operands + 1);
        operand_no >= argument_count && operand_no < bundle_end
    }

    #[inline]
    fn is_callee_operand(self, operand_no: u32) -> bool {
        self.call_arg_count().is_some() && operand_no + 1 == self.get_num_operands()
    }

    #[inline]
    fn is_inline_asm_call(self) -> bool {
        let raw = self.as_value_ref();
        // SAFETY: self 是有效指令。
        unsafe {
            if !is_call_base(raw) {
                return false;
            }
            let callee = LLVMGetCalledValue(raw);
            !callee.is_null() && !LLVMIsAInlineAsm(callee).is_null()
        }
    }

    #[inline]
    fn inline_asm_bytes(self) -> Option<Vec<u8>> {
        let raw = self.as_value_ref();
        // SAFETY: self 是有效指令，返回的字符串在复制期间有效。
        unsafe {
            if !is_call_base(raw) {
                return None;
            }
            let callee = LLVMGetCalledValue(raw);
            if callee.is_null() || LLVMIsAInlineAsm(callee).is_null() {
                return None;
            }
            let mut length = 0usize;
            let ptr = LLVMGetInlineAsmAsmString(callee, &mut length);
            if ptr.is_null() {
                return None;
            }
            Some(std::slice::from_raw_parts(ptr.cast::<u8>(), length).to_vec())
        }
    }

    #[inline]
    fn inline_asm_string(self) -> Option<String> {
        String::from_utf8(self.inline_asm_bytes()?).ok()
    }

    #[inline]
    fn move_before(self, before: InstructionValue<'ctx>) {
        // SAFETY: 两条指令均有效且已插入基本块。
        unsafe {
            ir_utils::move_inst_before(self.as_value_ref().cast(), before.as_value_ref().cast())
        }
    }

    #[inline]
    fn move_to_first_insertion_pt(self, bb: BasicBlock<'ctx>) {
        // SAFETY: 指令与基本块均有效。
        unsafe {
            ir_utils::move_inst_to_first_insertion_pt(
                self.as_value_ref().cast(),
                bb.as_mut_ptr().cast(),
            )
        }
    }

    #[inline]
    unsafe fn demote_reg_to_stack(
        self,
        volatile_loads: bool,
        alloca_pt: Option<InstructionValue<'ctx>>,
    ) -> Option<PointerValue<'ctx>> {
        let alloca_pt = alloca_pt
            .map(|instruction| instruction.as_value_ref().cast())
            .unwrap_or(std::ptr::null_mut());
        // SAFETY: self 有效，alloca_pt 可为空。
        let alloca = unsafe {
            ir_utils::demote_reg_to_stack(self.as_value_ref().cast(), volatile_loads, alloca_pt)
        };
        if alloca.is_null() {
            None
        } else {
            // SAFETY: DemoteRegToStack 返回有效 AllocaInst*。
            Some(unsafe { PointerValue::new(alloca.cast()) })
        }
    }

    #[inline]
    unsafe fn demote_phi_to_stack(
        self,
        alloca_pt: Option<InstructionValue<'ctx>>,
    ) -> PointerValue<'ctx> {
        let alloca_pt = alloca_pt
            .map(|instruction| instruction.as_value_ref().cast())
            .unwrap_or(std::ptr::null_mut());
        // SAFETY: 调用方保证 self 是 PHI，alloca_pt 可为空。
        let alloca =
            unsafe { ir_utils::demote_phi_to_stack(self.as_value_ref().cast(), alloca_pt) };
        assert!(!alloca.is_null(), "DemotePHIToStack returned null");
        // SAFETY: DemotePHIToStack 返回有效 AllocaInst*。
        unsafe { PointerValue::new(alloca.cast()) }
    }

    #[inline]
    unsafe fn build_call_preserving_bundles(
        self,
        callee: FunctionValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
        name: &str,
    ) -> CallSiteValue<'ctx> {
        let original = self.as_value_ref();
        let block = self
            .get_parent()
            .expect("Call instruction must have a parent BasicBlock");
        let call_name = if callee.get_type().get_return_type().is_none() {
            CString::new("").expect("empty CString")
        } else {
            CString::new(name).expect("Call name must not contain interior null bytes")
        };

        // SAFETY: 调用方保证 self 位于基本块内，callee 与 args 均有效。
        unsafe {
            let function = LLVMGetBasicBlockParent(block.as_mut_ptr());
            let module = LLVMGetGlobalParent(function);
            let context = LLVMGetModuleContext(module);
            let builder = LLVMCreateBuilderInContext(context);
            LLVMPositionBuilderBefore(builder, original);

            let bundle_count = LLVMGetNumOperandBundles(original);
            let mut bundles: Vec<LLVMOperandBundleRef> = Vec::with_capacity(bundle_count as usize);
            for index in 0..bundle_count {
                bundles.push(LLVMGetOperandBundleAtIndex(original, index));
            }

            let mut raw_args: Vec<LLVMValueRef> =
                args.iter().map(BasicValueEnum::as_value_ref).collect();

            let call_value = LLVMBuildCallWithOperandBundles(
                builder,
                callee.get_type().as_type_ref(),
                callee.as_value_ref(),
                raw_args.as_mut_ptr(),
                raw_args.len() as u32,
                bundles.as_mut_ptr(),
                bundles.len() as u32,
                call_name.as_ptr(),
            );

            for bundle in bundles {
                LLVMDisposeOperandBundle(bundle);
            }
            LLVMDisposeBuilder(builder);

            LLVMSetInstructionCallConv(call_value, LLVMGetInstructionCallConv(original));
            CallSiteValue::new(call_value)
        }
    }

    #[inline]
    unsafe fn change_to_unreachable(self) -> u32 {
        // SAFETY: 调用方保证 self 有效；返回后后续指令失效。
        unsafe { ir_utils::change_to_unreachable(self.as_value_ref().cast()) }
    }

    #[inline]
    fn switch_add_case(self, on_val: IntValue<'ctx>, dest: BasicBlock<'ctx>) {
        assert_eq!(
            self.get_opcode(),
            InstructionOpcode::Switch,
            "switch_add_case called on non-Switch instruction"
        );
        // SAFETY: 已确认是 Switch，参数均有效。
        unsafe {
            LLVMAddCase(
                self.as_value_ref(),
                on_val.as_value_ref(),
                dest.as_mut_ptr(),
            )
        }
    }

    #[inline]
    fn switch_num_cases(self) -> u32 {
        if self.get_opcode() != InstructionOpcode::Switch {
            return 0;
        }
        self.num_successors().saturating_sub(1)
    }

    #[inline]
    fn switch_get_case(self, idx: u32) -> Option<(IntValue<'ctx>, BasicBlock<'ctx>)> {
        if self.get_opcode() != InstructionOpcode::Switch || idx >= self.switch_num_cases() {
            return None;
        }
        // SAFETY: 已确认是有效 Switch 且 idx 在范围内。
        let value = unsafe { ffi::ir_switch_case_value(self.as_value_ref().cast(), idx) };
        if value.is_null() {
            return None;
        }
        let dest = self.get_successor(idx + 1)?;
        // SAFETY: C++ helper 返回有效 ConstantInt*。
        Some((unsafe { IntValue::new(value.cast()) }, dest))
    }

    #[inline]
    fn switch_default_dest(self) -> Option<BasicBlock<'ctx>> {
        if self.get_opcode() != InstructionOpcode::Switch {
            return None;
        }
        // SAFETY: 已确认是 Switch。
        unsafe { BasicBlock::new(LLVMGetSwitchDefaultDest(self.as_value_ref())) }
    }

    #[inline]
    fn switch_set_default_dest(self, dest: BasicBlock<'ctx>) {
        assert_eq!(
            self.get_opcode(),
            InstructionOpcode::Switch,
            "switch_set_default_dest called on non-Switch instruction"
        );
        // Switch 的第 0 个后继即 default dest。
        self.set_successor(0, dest);
    }

    #[inline]
    fn num_successors(self) -> u32 {
        if !self.is_terminator() {
            return 0;
        }
        // SAFETY: self 是终结指令。
        unsafe { LLVMGetNumSuccessors(self.as_value_ref()) }
    }

    #[inline]
    fn get_successor(self, idx: u32) -> Option<BasicBlock<'ctx>> {
        if idx >= self.num_successors() {
            return None;
        }
        // SAFETY: idx 在范围内。
        unsafe { BasicBlock::new(LLVMGetSuccessor(self.as_value_ref(), idx)) }
    }

    #[inline]
    fn set_successor(self, idx: u32, dest: BasicBlock<'ctx>) {
        assert!(idx < self.num_successors(), "Successor index out of bounds");
        // SAFETY: idx 在范围内。
        unsafe { LLVMSetSuccessor(self.as_value_ref(), idx, dest.as_mut_ptr()) }
    }
}

/// 针对 [`inkwell::basic_block::BasicBlock`] 的扩展方法。
#[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
pub trait BasicBlockExt<'ctx> {
    /// 在 `split_pt` 处切分基本块，返回新基本块。
    fn split_at(self, split_pt: InstructionValue<'ctx>, name: &str) -> BasicBlock<'ctx>;

    /// 获取首个合法指令插入点。
    fn first_insertion_pt(self) -> Option<InstructionValue<'ctx>>;

    /// 获取首条非 PHI、非调试记录、非静态 alloca 的指令。
    fn first_non_phi_or_dbg_or_alloca(self) -> Option<InstructionValue<'ctx>>;
}

impl<'ctx> BasicBlockExt<'ctx> for BasicBlock<'ctx> {
    #[inline]
    fn split_at(self, split_pt: InstructionValue<'ctx>, name: &str) -> BasicBlock<'ctx> {
        assert_eq!(
            split_pt.get_parent(),
            Some(self),
            "split_pt must belong to the BasicBlock being split"
        );
        // SAFETY: split_pt 属于 self。
        let new_block = unsafe {
            ffi::ir_split_basic_block(
                self.as_mut_ptr().cast(),
                split_pt.as_value_ref().cast(),
                name,
            )
        };
        // SAFETY: LLVM splitBasicBlock 返回有效基本块。
        unsafe { BasicBlock::new(new_block.cast()).expect("splitBasicBlock returned null") }
    }

    #[inline]
    fn first_insertion_pt(self) -> Option<InstructionValue<'ctx>> {
        // SAFETY: self 有效。
        let target = unsafe { ffi::ir_first_insertion_pt(self.as_mut_ptr().cast()) };
        if target.is_null() {
            return None;
        }
        // SAFETY: helper 返回属于 self 的有效 Instruction*。
        Some(unsafe { InstructionValue::new(target.cast()) })
    }

    #[inline]
    fn first_non_phi_or_dbg_or_alloca(self) -> Option<InstructionValue<'ctx>> {
        // SAFETY: self 有效。
        let target = unsafe { ffi::ir_first_non_phi_or_dbg_or_alloca(self.as_mut_ptr().cast()) };
        if target.is_null() {
            return None;
        }
        // SAFETY: helper 返回属于 self 的有效 Instruction*。
        Some(unsafe { InstructionValue::new(target.cast()) })
    }
}

/// 针对 [`inkwell::values::GlobalValue`] 的扩展方法。
#[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
pub trait GlobalValueExt<'ctx> {
    /// 清空关联的 Comdat。
    fn clear_comdat(self);

    /// 设置 `dso_local` 标志。
    fn set_dso_local(self, dso_local: bool);
}

impl<'ctx> GlobalValueExt<'ctx> for GlobalValue<'ctx> {
    #[inline]
    fn clear_comdat(self) {
        // SAFETY: self 是有效全局值。
        unsafe { LLVMSetComdat(self.as_value_ref(), std::ptr::null_mut()) }
    }

    #[inline]
    fn set_dso_local(self, dso_local: bool) {
        // SAFETY: self 是有效全局值。
        unsafe { ir_utils::set_dso_local(self.as_value_ref().cast(), dso_local) }
    }
}

/// 针对 [`inkwell::values::FunctionValue`] 的扩展方法。
#[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
pub trait FunctionExt<'ctx> {
    /// 获取函数的首个 Use（等价于 `self.as_global_value().get_first_use()`）。
    fn get_first_use(self) -> Option<BasicValueUse<'ctx>>;

    /// 清空关联的 Comdat。
    fn clear_comdat(self);

    /// 设置 `dso_local` 标志。
    fn set_dso_local(self, dso_local: bool);

    /// 清空函数引用并删除函数体。
    ///
    /// # Safety
    ///
    /// 调用后，函数内所有基本块与指令均失效。
    unsafe fn delete_body(self);
}

impl<'ctx> FunctionExt<'ctx> for FunctionValue<'ctx> {
    #[inline]
    fn get_first_use(self) -> Option<BasicValueUse<'ctx>> {
        self.as_global_value().get_first_use()
    }

    #[inline]
    fn clear_comdat(self) {
        self.as_global_value().clear_comdat();
    }

    #[inline]
    fn set_dso_local(self, dso_local: bool) {
        // SAFETY: self 是有效函数值（GlobalValue）。
        unsafe { ir_utils::set_dso_local(self.as_value_ref().cast(), dso_local) }
    }

    #[inline]
    unsafe fn delete_body(self) {
        // SAFETY: 调用方保证不再访问函数体。
        unsafe { ir_utils::reset_function_body(self.as_value_ref().cast()) }
    }
}

/// 针对 [`inkwell::module::Module`] 的扩展方法。
#[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
pub trait ModuleExt<'ctx> {
    /// 获取模块数据布局下指定地址空间的指针宽度整数类型。
    fn intptr_type(&self, address_space: u32) -> IntType<'ctx>;
}

impl<'ctx> ModuleExt<'ctx> for Module<'ctx> {
    #[inline]
    fn intptr_type(&self, address_space: u32) -> IntType<'ctx> {
        // SAFETY: module 有效；`LLVMGetModuleDataLayout` 返回模块内部 DataLayout
        // 的借用句柄，不得调用 `LLVMDisposeTargetData`。
        unsafe {
            let raw_module = self.as_mut_ptr();
            let target_data = LLVMGetModuleDataLayout(raw_module);
            let context = LLVMGetModuleContext(raw_module);
            let raw_type = LLVMIntPtrTypeForASInContext(context, target_data, address_space);
            IntType::new(raw_type)
        }
    }
}
