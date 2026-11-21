// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![cfg(feature = "inkwell")]

use inkwell::basic_block::BasicBlock;
use inkwell::context::Context;
use inkwell::memory_buffer::MemoryBuffer;
use inkwell::module::Module;
use inkwell::values::{AnyValueEnum, AsValueRef, BasicValue, InstructionOpcode, InstructionValue};
use llvm_pass::{
    AnyValueExt, BasicBlockExt, FunctionExt, GlobalValueExt, InstructionExt, ModuleExt,
};

#[test]
fn llvm_sys_reexport_is_usable() {
    // 重导出路径由 `inkwell` 特性提供。
    use llvm_pass::llvm_sys::prelude::LLVMValueRef;

    let _: Option<LLVMValueRef> = None;
}

fn parse<'ctx>(context: &'ctx Context, ir: &str) -> Module<'ctx> {
    let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), "test-ir");
    context
        .create_module_from_ir(buffer)
        .unwrap_or_else(|error| panic!("failed to parse test IR: {error}"))
}

fn find_instruction<'ctx>(
    function: inkwell::values::FunctionValue<'ctx>,
    opcode: InstructionOpcode,
) -> Option<InstructionValue<'ctx>> {
    let mut block = function.get_first_basic_block();
    while let Some(current_block) = block {
        let mut instruction = current_block.get_first_instruction();
        while let Some(current_instruction) = instruction {
            if current_instruction.get_opcode() == opcode {
                return Some(current_instruction);
            }
            instruction = current_instruction.get_next_instruction();
        }
        block = current_block.get_next_basic_block();
    }
    None
}

fn block_name(block: BasicBlock<'_>) -> String {
    block.get_name().to_string_lossy().into_owned()
}

#[test]
fn call_operand_layout_is_correct_for_call_invoke_and_callbr() {
    let context = Context::create();
    let module = parse(
        &context,
        r#"
            declare i32 @callee(i32, i32)
            declare i32 @personality(...)

            define i32 @caller() personality ptr @personality {
            entry:
                %call = call i32 @callee(i32 10, i32 20) [ "deopt"(i32 7), "funclet"(token none) ]
                %invoke = invoke i32 @callee(i32 30, i32 40) [ "deopt"(i32 8) ] to label %normal unwind label %lpad
            normal:
                ret i32 %invoke
            lpad:
                %lp = landingpad { ptr, i32 } cleanup
                ret i32 0
            }

            define void @callbr_caller() {
            entry:
                callbr void asm sideeffect "", "r,~{dirflag},~{fpsr},~{flags}"(i32 1) to label %normal [label %abnormal]
            normal:
                ret void
            abnormal:
                ret void
            }
        "#,
    );

    let caller = module.get_function("caller").expect("caller function");
    let call = find_instruction(caller, InstructionOpcode::Call).expect("call instruction");
    assert_eq!(call.get_num_operands(), 5);
    assert_eq!(call.call_arg_count(), Some(2));
    assert!(call.is_arg_operand(0));
    assert!(call.is_arg_operand(1));
    assert!(!call.is_arg_operand(2));
    assert!(call.is_bundle_operand(2));
    assert!(call.is_bundle_operand(3));
    assert!(!call.is_bundle_operand(4));
    assert!(call.is_callee_operand(4));
    assert!(call.has_token());
    assert!(!call.is_inline_asm_call());

    let invoke = find_instruction(caller, InstructionOpcode::Invoke).expect("invoke instruction");
    assert_eq!(invoke.get_num_operands(), 6);
    assert_eq!(invoke.call_arg_count(), Some(2));
    assert!(invoke.is_arg_operand(0));
    assert!(invoke.is_arg_operand(1));
    assert!(invoke.is_bundle_operand(2));
    // normal_dest / unwind_dest 不是 bundle 操作数。
    assert!(!invoke.is_bundle_operand(3));
    assert!(!invoke.is_bundle_operand(4));
    assert!(invoke.is_callee_operand(5));
    assert_eq!(invoke.num_successors(), 2);

    let callbr_function = module
        .get_function("callbr_caller")
        .expect("callbr_caller function");
    let callbr =
        find_instruction(callbr_function, InstructionOpcode::CallBr).expect("callbr instruction");
    assert_eq!(callbr.get_num_operands(), 4);
    assert_eq!(callbr.call_arg_count(), Some(1));
    assert!(callbr.is_arg_operand(0));
    // default / abnormal 两个基本块不是 bundle 操作数。
    assert!(!callbr.is_bundle_operand(1));
    assert!(!callbr.is_bundle_operand(2));
    assert!(callbr.is_callee_operand(3));
    assert_eq!(callbr.num_successors(), 2);
    assert!(callbr.is_inline_asm_call());
    assert_eq!(callbr.inline_asm_bytes().as_deref(), Some(&b""[..]));
}

#[test]
fn build_call_preserving_bundles_keeps_bundle_operands() {
    let context = Context::create();
    let module = parse(
        &context,
        r#"
            declare void @callee(i32)
            declare void @new_target(i32)

            define void @caller(i32 %value) {
            entry:
                call void @callee(i32 %value) [ "deopt"(i32 7) ]
                ret void
            }
        "#,
    );

    let caller = module.get_function("caller").expect("caller function");
    let original = find_instruction(caller, InstructionOpcode::Call).expect("original call");
    let new_target = module
        .get_function("new_target")
        .expect("new_target function");
    let argument = context
        .i32_type()
        .const_int(42, false)
        .as_basic_value_enum();
    let new_call = unsafe {
        original.build_call_preserving_bundles(new_target, &[argument], "preserved_call")
    };
    assert_eq!(
        new_call.get_called_fn_value(),
        Some(new_target),
        "new call should target new_target"
    );
    let new_instruction = unsafe { InstructionValue::new(new_call.as_value_ref()) };
    assert_eq!(new_instruction.call_arg_count(), Some(1));
    assert!(new_instruction.is_arg_operand(0));
    assert!(new_instruction.is_bundle_operand(1));
    assert!(new_instruction.is_callee_operand(2));
    let verification = module.verify();
    assert!(
        verification.is_ok(),
        "module verification failed: {verification:?}"
    );
}

#[test]
fn switch_cases_work_across_llvm_versions() {
    let context = Context::create();
    let module = parse(
        &context,
        r#"
            define i32 @switcher(i32 %value) {
            entry:
                switch i32 %value, label %default [ i32 1, label %one
                                                    i32 2, label %two
                                                    i32 7, label %seven ]
            one:
                ret i32 1
            two:
                ret i32 2
            seven:
                ret i32 7
            default:
                ret i32 0
            }
        "#,
    );

    let function = module.get_function("switcher").expect("switcher function");
    let switch = find_instruction(function, InstructionOpcode::Switch).expect("switch instruction");
    assert_eq!(switch.switch_num_cases(), 3);
    assert_eq!(block_name(switch.switch_default_dest().unwrap()), "default");

    let expected = [(1u64, "one"), (2, "two"), (7, "seven")];
    for (index, (value, name)) in expected.into_iter().enumerate() {
        let (case_value, destination) = switch
            .switch_get_case(index as u32)
            .unwrap_or_else(|| panic!("missing switch case {index}"));
        assert_eq!(case_value.get_zero_extended_constant(), Some(value));
        assert_eq!(block_name(destination), name);
    }
    assert!(switch.switch_get_case(3).is_none());

    // 后加 case：LLVMAddCase 应同步更新后继与 case 存储。
    let i32_type = context.i32_type();
    let new_block = context.append_basic_block(function, "nine");
    let builder = context.create_builder();
    builder.position_at_end(new_block);
    builder
        .build_return(Some(&i32_type.const_int(9, false)))
        .unwrap();
    switch.switch_add_case(i32_type.const_int(9, false), new_block);
    assert_eq!(switch.switch_num_cases(), 4);
    let (value, destination) = switch.switch_get_case(3).expect("new switch case");
    assert_eq!(value.get_zero_extended_constant(), Some(9));
    assert_eq!(block_name(destination), "nine");

    let one = switch.switch_get_case(0).unwrap().1;
    switch.switch_set_default_dest(one);
    assert_eq!(block_name(switch.switch_default_dest().unwrap()), "one");

    assert!(module.verify().is_ok());
}

#[test]
fn token_typed_instructions_are_detected_without_type_lookup_panic() {
    let context = Context::create();
    let module = parse(
        &context,
        r#"
            declare void @may_throw()
            declare i32 @__CxxFrameHandler3(...)

            define void @token_user() personality ptr @__CxxFrameHandler3 {
            entry:
                invoke void @may_throw() to label %normal unwind label %cs_block
            cs_block:
                %cs = catchswitch within none [label %handler] unwind to caller
            handler:
                %cp = catchpad within %cs [ptr null, i32 0, ptr null]
                catchret from %cp to label %exit
            normal:
                br label %exit
            exit:
                ret void
            }
        "#,
    );

    let function = module.get_function("token_user").expect("token_user");
    let catch_switch = find_instruction(function, InstructionOpcode::CatchSwitch)
        .expect("catchswitch instruction");
    let catch_pad =
        find_instruction(function, InstructionOpcode::CatchPad).expect("catchpad instruction");

    assert!(catch_switch.is_token_type());
    assert!(catch_switch.has_token());
    assert!(catch_pad.is_token_type());
    assert!(catch_pad.has_token());

    // 这些查询都不应走 `InstructionValue::get_type()` / `AnyTypeEnum::new()`。
    let handler = catch_pad.get_parent().expect("catchpad parent");
    assert!(handler.first_insertion_pt().is_some());
    assert!(handler.first_non_phi_or_dbg_or_alloca().is_some());
    assert!(module.verify().is_ok());
}

#[test]
fn inline_asm_and_any_value_conversion() {
    let context = Context::create();
    let module = parse(
        &context,
        r#"
            define i32 @callee(i32 %value) {
            entry:
                ret i32 %value
            }

            define void @asm_caller() {
            entry:
                %result = call i32 asm sideeffect "add $1, $0", "=r,r"(i32 5)
                ret void
            }

            define i32 @value_caller(i32 %value) {
            entry:
                %result = call i32 @callee(i32 %value)
                ret i32 %result
            }
        "#,
    );

    let asm_function = module.get_function("asm_caller").expect("asm_caller");
    let asm_call = find_instruction(asm_function, InstructionOpcode::Call).expect("asm call");
    assert!(asm_call.is_inline_asm_call());
    assert_eq!(asm_call.inline_asm_string().as_deref(), Some("add $1, $0"));
    assert!(!asm_call.has_token());

    let value_function = module.get_function("value_caller").expect("value_caller");
    let value_call = find_instruction(value_function, InstructionOpcode::Call).expect("value call");
    let any_value = unsafe { AnyValueEnum::new(value_call.as_value_ref()) };
    assert_eq!(
        any_value.as_instruction_value(),
        Some(value_call),
        "non-void call must be recoverable as an InstructionValue"
    );

    // 通过 use-chain 获取调用结果的使用者，也必须能还原为指令。
    let call_result = unsafe { inkwell::values::IntValue::new(value_call.as_value_ref()) };
    let first_use = call_result.get_first_use().expect("call result use");
    assert!(first_use.get_user().as_instruction_value().is_some());
}

#[test]
fn token_and_gep_queries_do_not_panic() {
    let context = Context::create();
    let module = parse(
        &context,
        r#"
            %struct.S = type { i32, [4 x i8] }

            define void @gep_user(ptr %pointer) {
            entry:
                %gep = getelementptr %struct.S, ptr %pointer, i32 0, i32 1, i32 2
                ret void
            }

            define void @alloca_user() {
            entry:
                %slot = alloca i32, align 4
                ret void
            }
        "#,
    );

    let gep_function = module.get_function("gep_user").expect("gep_user");
    let gep = find_instruction(gep_function, InstructionOpcode::GetElementPtr)
        .expect("getelementptr instruction");
    assert!(!gep.gep_index_is_struct(1));
    assert!(gep.gep_index_is_struct(2));
    assert!(!gep.gep_index_is_struct(3));
    assert!(!gep.gep_index_is_struct(0));

    let alloca_function = module.get_function("alloca_user").expect("alloca_user");
    let alloca = find_instruction(alloca_function, InstructionOpcode::Alloca).expect("alloca");
    assert!(alloca.is_static_alloca());
    assert!(!alloca.is_token_type());

    let branch = alloca_function
        .get_first_basic_block()
        .unwrap()
        .get_terminator()
        .expect("alloca_user terminator");
    assert!(!branch.has_token());
}

#[test]
fn basic_block_helpers_and_data_layout() {
    let context = Context::create();
    let module = parse(
        &context,
        r#"
            target datalayout = "e-p:32:32"

            define void @operations(i1 %condition) {
            entry:
                %slot = alloca i32, align 4
                %sum = add i32 1, 2
                br i1 %condition, label %left, label %right
            left:
                ret void
            right:
                ret void
            }
        "#,
    );

    let function = module.get_function("operations").expect("operations");
    let entry = function.get_first_basic_block().expect("entry block");
    let insertion = entry.first_insertion_pt().expect("insertion point");
    let first_non_phi = entry
        .first_non_phi_or_dbg_or_alloca()
        .expect("first non-phi instruction");
    assert_eq!(
        insertion.get_opcode(),
        InstructionOpcode::Alloca,
        "getFirstInsertionPt does not skip static allocas"
    );
    assert_eq!(
        first_non_phi.get_opcode(),
        InstructionOpcode::Add,
        "getFirstNonPHIOrDbgOrAlloca should skip the static alloca"
    );

    let branch = entry.get_terminator().expect("entry terminator");
    let split = entry.split_at(branch, "split");
    assert_eq!(block_name(split), "split");
    assert_eq!(
        entry.get_terminator().unwrap().get_opcode(),
        InstructionOpcode::Br
    );

    assert_eq!(module.intptr_type(0).get_bit_width(), 32);

    let module64 = parse(
        &context,
        r#"
            target datalayout = "e-p:64:64"

            define void @f64() {
            entry:
                ret void
            }
        "#,
    );
    assert_eq!(module64.intptr_type(0).get_bit_width(), 64);
}

#[test]
fn unreachable_and_delete_body_helpers() {
    let context = Context::create();
    let module = parse(
        &context,
        r#"
            declare void @side_effect()

            define void @unreachable_user() {
            entry:
                call void @side_effect()
                ret void
            }

            define void @body_user() {
            entry:
                call void @side_effect()
                ret void
            }
        "#,
    );

    let unreachable_function = module
        .get_function("unreachable_user")
        .expect("unreachable_user");
    let call = find_instruction(unreachable_function, InstructionOpcode::Call).expect("call");
    let removed = unsafe { call.change_to_unreachable() };
    assert!(removed > 0);
    let entry = unreachable_function.get_first_basic_block().unwrap();
    assert_eq!(
        entry.get_terminator().unwrap().get_opcode(),
        InstructionOpcode::Unreachable
    );

    let body_function = module.get_function("body_user").expect("body_user");
    assert_eq!(body_function.count_basic_blocks(), 1);
    unsafe { body_function.delete_body() };
    assert_eq!(body_function.count_basic_blocks(), 0);
    assert!(body_function.as_global_value().is_declaration());
}

#[test]
fn instruction_mutation_helpers() {
    let context = Context::create();
    let module = parse(
        &context,
        r#"
            @slot = global i32 0
            declare void @may_throw()

            define i32 @mutations(i32 %left, i32 %right) {
            entry:
                %sum = add i32 %left, %right
                store i32 %sum, ptr @slot
                call void @may_throw()
                br label %next
            next:
                ret i32 %sum
            }
        "#,
    );

    let function = module.get_function("mutations").expect("mutations");
    let entry = function.get_first_basic_block().expect("entry");
    let add = find_instruction(function, InstructionOpcode::Add).expect("add");
    let store = find_instruction(function, InstructionOpcode::Store).expect("store");
    let call = find_instruction(function, InstructionOpcode::Call).expect("call");

    assert!(add.is_used_outside_of_block(entry));
    assert!(!add.may_have_side_effects());
    assert!(store.may_have_side_effects());
    assert!(call.may_throw());

    store.move_before(call);
    call.move_to_first_insertion_pt(entry);
    assert_eq!(entry.get_first_instruction(), Some(call));

    let alloca = unsafe { add.demote_reg_to_stack(false, None) }
        .expect("DemoteRegToStack should return an alloca");
    assert!(!alloca.as_value_ref().is_null());
    let verification = module.verify();
    assert!(
        verification.is_ok(),
        "module verification failed after demotion: {verification:?}"
    );
}

#[test]
fn phi_demotion_is_cross_version_valid() {
    let context = Context::create();
    let module = parse(
        &context,
        r#"
            define i32 @phi_user(i1 %condition, i32 %left_value, i32 %right_value) {
            entry:
                br i1 %condition, label %left, label %right
            left:
                br label %merge
            right:
                br label %merge
            merge:
                %result = phi i32 [ %left_value, %left ], [ %right_value, %right ]
                ret i32 %result
            }
        "#,
    );

    let function = module.get_function("phi_user").expect("phi_user");
    let phi = find_instruction(function, InstructionOpcode::Phi).expect("phi instruction");
    let alloca = unsafe { phi.demote_phi_to_stack(None) };
    assert!(!alloca.as_value_ref().is_null());
    assert!(module.verify().is_ok());
}

#[test]
fn comdat_clear_and_function_body_use_chain() {
    let context = Context::create();
    let module = parse(
        &context,
        r#"
            $payload = comdat any
            @payload = global i32 0, comdat

            define void @target() {
            entry:
                ret void
            }

            define void @caller() {
            entry:
                call void @target()
                ret void
            }
        "#,
    );

    let global = module.get_global("payload").expect("payload global");
    assert!(global.get_comdat().is_some());
    global.clear_comdat();
    assert!(global.get_comdat().is_none());

    let target = module.get_function("target").expect("target");
    assert!(target.get_first_use().is_some());
    target.set_dso_local(true);
}
