// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#pragma once
#include <memory>
#include <string>
#include <cstdint>
#include <cstddef>
#include "rust/cxx.h"
#include "llvm/Passes/PassBuilder.h"

#include "llvm/IR/PassManager.h"
#include "llvm/IR/Module.h"
#include "llvm/IR/Function.h"

namespace llvm_pass {

// 前向声明 Rust 侧导出的不透明包装类型
struct BoxedModulePass;
struct BoxedFunctionPass;
struct BoxedModuleEpCallback;
struct BoxedModuleLtoEpCallback;
struct BoxedFunctionEpCallback;
struct BoxedModulePipelineParser;
struct BoxedFunctionPipelineParser;

// === PassManager 操作接口 ===
void mpm_add_module_pass(llvm::ModulePassManager &MPM, rust::Box<BoxedModulePass> pass);
void mpm_add_function_pass(llvm::ModulePassManager &MPM, rust::Box<BoxedFunctionPass> pass, bool eagerly_invalidate);
void fpm_add_function_pass(llvm::FunctionPassManager &FPM, rust::Box<BoxedFunctionPass> pass);

// === PassBuilder 扩展点 (Extension Points) 注册接口 ===
void pb_register_pipeline_start_ep(llvm::PassBuilder &PB, rust::Box<BoxedModuleEpCallback> cb);
void pb_register_pipeline_early_simplification_ep(llvm::PassBuilder &PB, rust::Box<BoxedModuleLtoEpCallback> cb);
void pb_register_optimizer_early_ep(llvm::PassBuilder &PB, rust::Box<BoxedModuleLtoEpCallback> cb);
void pb_register_optimizer_last_ep(llvm::PassBuilder &PB, rust::Box<BoxedModuleLtoEpCallback> cb);
void pb_register_peephole_ep(llvm::PassBuilder &PB, rust::Box<BoxedFunctionEpCallback> cb);
void pb_register_scalar_optimizer_late_ep(llvm::PassBuilder &PB, rust::Box<BoxedFunctionEpCallback> cb);

// === PassBuilder 命令行 Pipeline 解析注册接口 (用于 opt -passes="...") ===
void pb_register_module_pipeline_parser(llvm::PassBuilder &PB, rust::Box<BoxedModulePipelineParser> cb);
void pb_register_function_pipeline_parser(llvm::PassBuilder &PB, rust::Box<BoxedFunctionPipelineParser> cb);

// === IR 基础信息辅助函数 ===
rust::Str get_module_name(const llvm::Module &M);
rust::Str get_function_name(const llvm::Function &F);
bool is_function_declaration(const llvm::Function &F);

} // namespace llvm_pass