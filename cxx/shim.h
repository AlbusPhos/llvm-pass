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
#include "llvm/IR/BasicBlock.h"
#include "llvm/IR/Instructions.h"
#include "llvm/IR/Dominators.h"
#include "llvm/Analysis/PostDominators.h"
#include "llvm/Analysis/LoopInfo.h"
#include "llvm/Analysis/CallGraph.h"
#include "llvm/Analysis/GlobalsModRef.h"

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

// === 原生 LLVM Pass 挂载接口 ===
void mpm_add_adce_pass(llvm::ModulePassManager &MPM);
void mpm_add_global_dce_pass(llvm::ModulePassManager &MPM);
void fpm_add_adce_pass(llvm::FunctionPassManager &FPM);
void fpm_add_simplify_cfg_pass(llvm::FunctionPassManager &FPM);
void fpm_add_promote_pass(llvm::FunctionPassManager &FPM);

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

// === IR 基础信息与遍历辅助函数 ===
rust::Str get_module_name(const llvm::Module &M);
rust::Str get_function_name(const llvm::Function &F);
bool is_function_declaration(const llvm::Function &F);
llvm::Function *module_first_function(llvm::Module &M);
llvm::Function *module_next_function(llvm::Module &M, llvm::Function &F);

// === C++ 专属 IR 变换与查询辅助函数 ===
uint32_t ir_change_to_unreachable(llvm::Instruction *I);
void ir_delete_function_body(llvm::Function &F);
void ir_set_dso_local(llvm::Value *GV, bool dso_local);

llvm::BasicBlock *ir_split_basic_block(
    llvm::BasicBlock *BB, llvm::Instruction *SplitPt, rust::Str Name);
llvm::Instruction *ir_first_insertion_pt(llvm::BasicBlock *BB);
llvm::Instruction *ir_first_non_phi_or_dbg_or_alloca(llvm::BasicBlock *BB);
void ir_move_inst_before(llvm::Instruction *I, llvm::Instruction *Before);
void ir_move_inst_to_first_insertion_pt(llvm::Instruction *I, llvm::BasicBlock *BB);

llvm::Instruction *ir_demote_reg_to_stack(
    llvm::Instruction *I, bool volatile_loads, llvm::Instruction *alloca_pt);
llvm::Instruction *ir_demote_phi_to_stack(
    llvm::Instruction *Phi, llvm::Instruction *alloca_pt);

bool ir_may_throw(const llvm::Instruction *I);
bool ir_may_have_side_effects(const llvm::Instruction *I);
bool ir_is_static_alloca(const llvm::Instruction *I);
bool ir_is_used_outside_of_block(const llvm::Instruction *I, const llvm::BasicBlock *BB);
bool ir_gep_index_is_struct(const llvm::Instruction *GEP, uint32_t operand_no);

// LLVM 18-21 需要从操作数取 Switch case value；LLVM 22 的 case value 不在操作数中，
// 由该辅助函数统一屏蔽版本差异。
llvm::Value *ir_switch_case_value(llvm::Instruction *I, uint32_t idx);

// === ModuleAnalysisManager 查询与失效接口 ===
const llvm::CallGraph &mam_get_call_graph(
    const llvm::ModuleAnalysisManager &MAM, const llvm::Module &M);
const llvm::CallGraph *mam_get_cached_call_graph(
    const llvm::ModuleAnalysisManager &MAM, const llvm::Module &M);
llvm::FunctionAnalysisManager &mam_get_function_analysis_manager(
    llvm::ModuleAnalysisManager &MAM, llvm::Module &M);
void mam_invalidate_module(
    llvm::ModuleAnalysisManager &MAM, llvm::Module &M, uint32_t preserved_mask);

// `GlobalsAA` 是 LLVM 明确定义的"永不失效"模块级分析，可安全地跨层级读取。
const llvm::GlobalsAAResult &mam_get_globals_aa(
    const llvm::ModuleAnalysisManager &MAM, const llvm::Module &M);
const llvm::GlobalsAAResult *mam_get_cached_globals_aa(
    const llvm::ModuleAnalysisManager &MAM, const llvm::Module &M);

// === FunctionAnalysisManager 查询与失效接口 ===
const llvm::DominatorTree &fam_get_dominator_tree(
    const llvm::FunctionAnalysisManager &FAM, const llvm::Function &F);
const llvm::DominatorTree *fam_get_cached_dominator_tree(
    const llvm::FunctionAnalysisManager &FAM, const llvm::Function &F);

const llvm::PostDominatorTree &fam_get_post_dominator_tree(
    const llvm::FunctionAnalysisManager &FAM, const llvm::Function &F);
const llvm::PostDominatorTree *fam_get_cached_post_dominator_tree(
    const llvm::FunctionAnalysisManager &FAM, const llvm::Function &F);

const llvm::LoopInfo &fam_get_loop_info(
    const llvm::FunctionAnalysisManager &FAM, const llvm::Function &F);
const llvm::LoopInfo *fam_get_cached_loop_info(
    const llvm::FunctionAnalysisManager &FAM, const llvm::Function &F);

// 跨层级查询外层模块中永不失效的 `GlobalsAA` 分析结果。
const llvm::GlobalsAAResult *fam_get_cached_globals_aa(
    const llvm::FunctionAnalysisManager &FAM, const llvm::Function &F);

void fam_invalidate_function(
    llvm::FunctionAnalysisManager &FAM, llvm::Function &F, uint32_t preserved_mask);

// === 分析结果查询辅助函数（支持 BasicBlock 裸指针以对接 inkwell） ===
bool dt_dominates_bb(
    const llvm::DominatorTree &DT, const llvm::BasicBlock *A, const llvm::BasicBlock *B);
bool dt_strictly_dominates_bb(
    const llvm::DominatorTree &DT, const llvm::BasicBlock *A, const llvm::BasicBlock *B);
bool dt_is_reachable_from_entry(
    const llvm::DominatorTree &DT, const llvm::BasicBlock *BB);

bool pdt_dominates_bb(
    const llvm::PostDominatorTree &PDT, const llvm::BasicBlock *A, const llvm::BasicBlock *B);
bool pdt_strictly_dominates_bb(
    const llvm::PostDominatorTree &PDT, const llvm::BasicBlock *A, const llvm::BasicBlock *B);

bool li_is_empty(const llvm::LoopInfo &LI);
size_t li_top_level_loops_count(const llvm::LoopInfo &LI);
uint32_t li_get_loop_depth(const llvm::LoopInfo &LI, const llvm::BasicBlock *BB);
bool li_is_loop_header(const llvm::LoopInfo &LI, const llvm::BasicBlock *BB);

} // namespace llvm_pass
