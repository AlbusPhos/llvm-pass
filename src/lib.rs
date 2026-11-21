// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod analysis;
#[cfg(feature = "inkwell")]
mod inkwell_ext;
mod ir;
pub mod ir_utils;
mod pass;

pub use analysis::*;
#[cfg(feature = "inkwell")]
pub use inkwell_ext::*;
pub use ir::*;
pub use pass::*;

// 编译期校验：必须且只能启用一个 LLVM 版本特性。
const _: () = {
    let count = cfg!(feature = "llvm18") as u32
        + cfg!(feature = "llvm19") as u32
        + cfg!(feature = "llvm20") as u32
        + cfg!(feature = "llvm21") as u32
        + cfg!(feature = "llvm22") as u32;
    assert!(
        count == 1,
        "llvm_pass requires exactly one LLVM version feature (llvm18..llvm22)"
    );
};

/// 当前启用的 LLVM 主版本号（18 ~ 22）。
#[cfg(feature = "llvm18")]
pub const LLVM_VERSION_MAJOR: u32 = 18;
/// 当前启用的 LLVM 主版本号（18 ~ 22）。
#[cfg(feature = "llvm19")]
pub const LLVM_VERSION_MAJOR: u32 = 19;
/// 当前启用的 LLVM 主版本号（18 ~ 22）。
#[cfg(feature = "llvm20")]
pub const LLVM_VERSION_MAJOR: u32 = 20;
/// 当前启用的 LLVM 主版本号（18 ~ 22）。
#[cfg(feature = "llvm21")]
pub const LLVM_VERSION_MAJOR: u32 = 21;
/// 当前启用的 LLVM 主版本号（18 ~ 22）。
#[cfg(feature = "llvm22")]
pub const LLVM_VERSION_MAJOR: u32 = 22;

use std::any::Any;
use std::ffi::c_char;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::Arc;

/// `cxx` 桥接层：声明 C++ 侧辅助函数与 Rust 侧回调包装类型。
///
/// 该模块由 `#[cxx::bridge]` 生成，通常无需直接使用；请优先使用 crate 根导出的
/// [`ModulePass`]、[`FunctionPass`]、[`PassBuilder`] 等安全封装。
///
/// 桥接函数的安全契约由本模块的 `# Safety` 文档与 C++ 侧实现共同约定；
/// `#[cxx::bridge]` 会重写这些声明，clippy 无法读取其中的文档，故在此豁免。
#[allow(clippy::missing_safety_doc)]
#[cxx::bridge(namespace = "llvm_pass")]
pub mod ffi {
    /// LLVM 优化级别，映射自 `llvm::OptimizationLevel`。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum OptLevel {
        /// 不优化（-O0）。
        O0,
        /// 轻度优化（-O1）。
        O1,
        /// 标准优化（-O2）。
        O2,
        /// 激进优化（-O3）。
        O3,
        /// 面向体积优化（-Os）。
        Os,
        /// 面向体积的更激进优化（-Oz）。
        Oz,
    }

    /// LTO 阶段，映射自 `llvm::ThinOrFullLTOPhase`。
    ///
    /// 注意：LLVM 18 / 19 的模块级扩展点回调不携带该参数，此时传入的
    /// 恒为 [`LtoPhase::None`]（表示"信息不可用"，而非"非 LTO"）。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum LtoPhase {
        /// 非 LTO 编译。
        None,
        /// ThinLTO 链接前阶段。
        ThinLTOPreLink,
        /// ThinLTO 链接后阶段。
        ThinLTOPostLink,
        /// FullLTO 链接前阶段。
        FullLTOPreLink,
        /// FullLTO 链接后阶段。
        FullLTOPostLink,
    }

    unsafe extern "C++" {
        include!("cxx/shim.h");

        #[namespace = "llvm"]
        type PassBuilder;
        #[namespace = "llvm"]
        type ModulePassManager;
        #[namespace = "llvm"]
        type FunctionPassManager;
        #[namespace = "llvm"]
        type ModuleAnalysisManager;
        #[namespace = "llvm"]
        type FunctionAnalysisManager;
        #[namespace = "llvm"]
        type Module;
        #[namespace = "llvm"]
        type Function;
        #[namespace = "llvm"]
        type BasicBlock;
        #[namespace = "llvm"]
        type Value;
        #[namespace = "llvm"]
        type Instruction;
        #[namespace = "llvm"]
        type DominatorTree;
        #[namespace = "llvm"]
        type PostDominatorTree;
        #[namespace = "llvm"]
        type LoopInfo;
        #[namespace = "llvm"]
        type CallGraph;
        #[namespace = "llvm"]
        type GlobalsAAResult;

        fn mpm_add_module_pass(mpm: Pin<&mut ModulePassManager>, pass: Box<BoxedModulePass>);
        fn mpm_add_function_pass(
            mpm: Pin<&mut ModulePassManager>,
            pass: Box<BoxedFunctionPass>,
            eagerly_invalidate: bool,
        );
        fn fpm_add_function_pass(fpm: Pin<&mut FunctionPassManager>, pass: Box<BoxedFunctionPass>);

        fn mpm_add_adce_pass(mpm: Pin<&mut ModulePassManager>);
        fn mpm_add_global_dce_pass(mpm: Pin<&mut ModulePassManager>);
        fn fpm_add_adce_pass(fpm: Pin<&mut FunctionPassManager>);
        fn fpm_add_simplify_cfg_pass(fpm: Pin<&mut FunctionPassManager>);
        fn fpm_add_promote_pass(fpm: Pin<&mut FunctionPassManager>);

        fn pb_register_pipeline_start_ep(pb: Pin<&mut PassBuilder>, cb: Box<BoxedModuleEpCallback>);
        fn pb_register_pipeline_early_simplification_ep(
            pb: Pin<&mut PassBuilder>,
            cb: Box<BoxedModuleLtoEpCallback>,
        );
        fn pb_register_optimizer_early_ep(
            pb: Pin<&mut PassBuilder>,
            cb: Box<BoxedModuleLtoEpCallback>,
        );
        fn pb_register_optimizer_last_ep(
            pb: Pin<&mut PassBuilder>,
            cb: Box<BoxedModuleLtoEpCallback>,
        );
        fn pb_register_peephole_ep(pb: Pin<&mut PassBuilder>, cb: Box<BoxedFunctionEpCallback>);
        fn pb_register_scalar_optimizer_late_ep(
            pb: Pin<&mut PassBuilder>,
            cb: Box<BoxedFunctionEpCallback>,
        );

        fn pb_register_module_pipeline_parser(
            pb: Pin<&mut PassBuilder>,
            cb: Box<BoxedModulePipelineParser>,
        );
        fn pb_register_function_pipeline_parser(
            pb: Pin<&mut PassBuilder>,
            cb: Box<BoxedFunctionPipelineParser>,
        );

        fn get_module_name(m: &Module) -> &str;
        fn get_function_name(f: &Function) -> &str;
        fn is_function_declaration(f: &Function) -> bool;
        fn module_first_function(m: Pin<&mut Module>) -> *mut Function;
        fn module_next_function(m: Pin<&mut Module>, f: Pin<&mut Function>) -> *mut Function;

        // C++-only IR helpers

        /// # Safety
        ///
        /// `inst` 必须是有效的 `llvm::Instruction*`。调用后，同一基本块中位于
        /// `inst` 及其后的所有指令均被销毁，不得再次访问。
        unsafe fn ir_change_to_unreachable(inst: *mut Instruction) -> u32;
        fn ir_delete_function_body(f: Pin<&mut Function>);
        /// # Safety
        ///
        /// `gv` 必须是有效的 `llvm::Value*`；非 `GlobalValue` 时为空操作。
        unsafe fn ir_set_dso_local(gv: *mut Value, dso_local: bool);
        /// # Safety
        ///
        /// `bb` 与 `split_pt` 必须有效，且 `split_pt` 属于 `bb`。
        unsafe fn ir_split_basic_block(
            bb: *mut BasicBlock,
            split_pt: *mut Instruction,
            name: &str,
        ) -> *mut BasicBlock;
        /// # Safety
        ///
        /// `bb` 必须是有效的 `llvm::BasicBlock*`。
        unsafe fn ir_first_insertion_pt(bb: *mut BasicBlock) -> *mut Instruction;
        /// # Safety
        ///
        /// `bb` 必须是有效的 `llvm::BasicBlock*`。
        unsafe fn ir_first_non_phi_or_dbg_or_alloca(bb: *mut BasicBlock) -> *mut Instruction;
        /// # Safety
        ///
        /// `inst` 与 `before` 必须是已插入基本块的有效指令。
        unsafe fn ir_move_inst_before(inst: *mut Instruction, before: *mut Instruction);
        /// # Safety
        ///
        /// `inst` 与 `bb` 必须有效。
        unsafe fn ir_move_inst_to_first_insertion_pt(inst: *mut Instruction, bb: *mut BasicBlock);
        /// # Safety
        ///
        /// `inst` 必须是有效的 `llvm::Instruction*`；`alloca_pt` 可为空。
        unsafe fn ir_demote_reg_to_stack(
            inst: *mut Instruction,
            volatile_loads: bool,
            alloca_pt: *mut Instruction,
        ) -> *mut Instruction;
        /// # Safety
        ///
        /// `phi` 必须是有效的 `llvm::PHINode*`；调用后原 `phi` 被销毁。
        unsafe fn ir_demote_phi_to_stack(
            phi: *mut Instruction,
            alloca_pt: *mut Instruction,
        ) -> *mut Instruction;
        /// # Safety
        ///
        /// `inst` 必须是有效的 `llvm::Instruction*`。
        unsafe fn ir_may_throw(inst: *const Instruction) -> bool;
        /// # Safety
        ///
        /// `inst` 必须是有效的 `llvm::Instruction*`。
        unsafe fn ir_may_have_side_effects(inst: *const Instruction) -> bool;
        /// # Safety
        ///
        /// `inst` 必须是有效的 `llvm::Instruction*`。
        unsafe fn ir_is_static_alloca(inst: *const Instruction) -> bool;
        /// # Safety
        ///
        /// `inst` 与 `bb` 必须有效。
        unsafe fn ir_is_used_outside_of_block(
            inst: *const Instruction,
            bb: *const BasicBlock,
        ) -> bool;
        /// # Safety
        ///
        /// `gep` 必须是有效的 `llvm::Instruction*`。
        unsafe fn ir_gep_index_is_struct(gep: *const Instruction, operand_no: u32) -> bool;
        /// # Safety
        ///
        /// `inst` 必须是有效的 `llvm::Instruction*`；非 `switch` 返回空指针。
        unsafe fn ir_switch_case_value(inst: *mut Instruction, idx: u32) -> *mut Value;

        // ModuleAnalysisManager
        fn mam_get_call_graph<'a>(mam: &'a ModuleAnalysisManager, m: &Module) -> &'a CallGraph;
        fn mam_get_cached_call_graph(mam: &ModuleAnalysisManager, m: &Module) -> *const CallGraph;
        fn mam_get_function_analysis_manager<'a>(
            mam: Pin<&'a mut ModuleAnalysisManager>,
            m: Pin<&mut Module>,
        ) -> Pin<&'a mut FunctionAnalysisManager>;
        fn mam_invalidate_module(
            mam: Pin<&mut ModuleAnalysisManager>,
            m: Pin<&mut Module>,
            preserved_mask: u32,
        );
        fn mam_get_globals_aa<'a>(
            mam: &'a ModuleAnalysisManager,
            m: &Module,
        ) -> &'a GlobalsAAResult;
        fn mam_get_cached_globals_aa(
            mam: &ModuleAnalysisManager,
            m: &Module,
        ) -> *const GlobalsAAResult;

        // FunctionAnalysisManager
        fn fam_get_dominator_tree<'a>(
            fam: &'a FunctionAnalysisManager,
            f: &Function,
        ) -> &'a DominatorTree;
        fn fam_get_cached_dominator_tree(
            fam: &FunctionAnalysisManager,
            f: &Function,
        ) -> *const DominatorTree;

        fn fam_get_post_dominator_tree<'a>(
            fam: &'a FunctionAnalysisManager,
            f: &Function,
        ) -> &'a PostDominatorTree;
        fn fam_get_cached_post_dominator_tree(
            fam: &FunctionAnalysisManager,
            f: &Function,
        ) -> *const PostDominatorTree;

        fn fam_get_loop_info<'a>(fam: &'a FunctionAnalysisManager, f: &Function) -> &'a LoopInfo;
        fn fam_get_cached_loop_info(fam: &FunctionAnalysisManager, f: &Function)
        -> *const LoopInfo;

        fn fam_get_cached_globals_aa(
            fam: &FunctionAnalysisManager,
            f: &Function,
        ) -> *const GlobalsAAResult;

        fn fam_invalidate_function(
            fam: Pin<&mut FunctionAnalysisManager>,
            f: Pin<&mut Function>,
            preserved_mask: u32,
        );

        // Analysis Result queries

        /// # Safety
        ///
        /// `a` 与 `b` 必须是指向 `dt` 所属函数的有效 `llvm::BasicBlock`，且在调用期间保持有效。
        unsafe fn dt_dominates_bb(
            dt: &DominatorTree,
            a: *const BasicBlock,
            b: *const BasicBlock,
        ) -> bool;
        /// # Safety
        ///
        /// `a` 与 `b` 必须是指向 `dt` 所属函数的有效 `llvm::BasicBlock`，且在调用期间保持有效。
        unsafe fn dt_strictly_dominates_bb(
            dt: &DominatorTree,
            a: *const BasicBlock,
            b: *const BasicBlock,
        ) -> bool;
        /// # Safety
        ///
        /// `bb` 必须是指向 `dt` 所属函数的有效 `llvm::BasicBlock`，且在调用期间保持有效。
        unsafe fn dt_is_reachable_from_entry(dt: &DominatorTree, bb: *const BasicBlock) -> bool;

        /// # Safety
        ///
        /// `a` 与 `b` 必须是指向 `pdt` 所属函数的有效 `llvm::BasicBlock`，且在调用期间保持有效。
        unsafe fn pdt_dominates_bb(
            pdt: &PostDominatorTree,
            a: *const BasicBlock,
            b: *const BasicBlock,
        ) -> bool;
        /// # Safety
        ///
        /// `a` 与 `b` 必须是指向 `pdt` 所属函数的有效 `llvm::BasicBlock`，且在调用期间保持有效。
        unsafe fn pdt_strictly_dominates_bb(
            pdt: &PostDominatorTree,
            a: *const BasicBlock,
            b: *const BasicBlock,
        ) -> bool;

        fn li_is_empty(li: &LoopInfo) -> bool;
        fn li_top_level_loops_count(li: &LoopInfo) -> usize;
        /// # Safety
        ///
        /// `bb` 必须是指向 `li` 所属函数的有效 `llvm::BasicBlock`，且在调用期间保持有效。
        unsafe fn li_get_loop_depth(li: &LoopInfo, bb: *const BasicBlock) -> u32;
        /// # Safety
        ///
        /// `bb` 必须是指向 `li` 所属函数的有效 `llvm::BasicBlock`，且在调用期间保持有效。
        unsafe fn li_is_loop_header(li: &LoopInfo, bb: *const BasicBlock) -> bool;
    }

    extern "Rust" {
        type BoxedModulePass;
        fn run(
            self: &mut BoxedModulePass,
            m: Pin<&mut Module>,
            mam: Pin<&mut ModuleAnalysisManager>,
        ) -> u32;
        fn name(self: &BoxedModulePass) -> &str;

        type BoxedFunctionPass;
        fn run(
            self: &mut BoxedFunctionPass,
            f: Pin<&mut Function>,
            fam: Pin<&mut FunctionAnalysisManager>,
        ) -> u32;
        fn name(self: &BoxedFunctionPass) -> &str;

        type BoxedModuleEpCallback;
        fn call(self: &BoxedModuleEpCallback, mpm: Pin<&mut ModulePassManager>, level: OptLevel);
        fn clone_cb(self: &BoxedModuleEpCallback) -> Box<BoxedModuleEpCallback>;

        type BoxedModuleLtoEpCallback;
        fn call(
            self: &BoxedModuleLtoEpCallback,
            mpm: Pin<&mut ModulePassManager>,
            level: OptLevel,
            phase: LtoPhase,
        );
        fn clone_cb(self: &BoxedModuleLtoEpCallback) -> Box<BoxedModuleLtoEpCallback>;

        type BoxedFunctionEpCallback;
        fn call(
            self: &BoxedFunctionEpCallback,
            fpm: Pin<&mut FunctionPassManager>,
            level: OptLevel,
        );
        fn clone_cb(self: &BoxedFunctionEpCallback) -> Box<BoxedFunctionEpCallback>;

        type BoxedModulePipelineParser;
        fn parse(
            self: &BoxedModulePipelineParser,
            name: &str,
            mpm: Pin<&mut ModulePassManager>,
        ) -> bool;
        fn clone_cb(self: &BoxedModulePipelineParser) -> Box<BoxedModulePipelineParser>;

        type BoxedFunctionPipelineParser;
        fn parse(
            self: &BoxedFunctionPipelineParser,
            name: &str,
            fpm: Pin<&mut FunctionPassManager>,
        ) -> bool;
        fn clone_cb(self: &BoxedFunctionPipelineParser) -> Box<BoxedFunctionPipelineParser>;
    }
}

/// 重导出常用值类型：`OptLevel` 与 `LtoPhase` 用于回调参数。
pub use ffi::{LtoPhase, OptLevel};

/// 重导出 `inkwell` crate，便于下游无需额外声明依赖即可使用匹配的 LLVM 版本类型。
#[cfg(feature = "inkwell")]
#[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
pub use inkwell;

/// 重导出 `inkwell::llvm_sys`，便于下游直接调用当前 LLVM 版本对应的底层 C API。
///
/// 仅在启用 `inkwell` 特性时可用。请勿在下游再声明不同 LLVM 版本的 `llvm-sys`
/// 依赖；应通过本重导出使用与 `llvm-pass`、`inkwell` 完全匹配的版本。
#[cfg(feature = "inkwell")]
#[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
pub use inkwell::llvm_sys;

#[cold]
#[inline(never)]
fn handle_ffi_panic(context: &str, payload: Box<dyn Any + Send>) -> ! {
    let msg = if let Some(s) = payload.downcast_ref::<&str>() {
        *s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.as_str()
    } else {
        "<non-string panic payload>"
    };
    eprintln!(
        "[llvm-pass Fatal] Panic caught across FFI boundary in `{}`: {}\nAborting safely.",
        context, msg
    );
    std::process::abort();
}

#[inline]
fn abort_on_panic<R>(context: &str, f: impl FnOnce() -> R) -> R {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(res) => res,
        Err(payload) => handle_ffi_panic(context, payload),
    }
}

/// `cxx` 桥接使用的模块级 Pass 包装类型，通常不直接构造。
pub struct BoxedModulePass(pub(crate) Box<dyn ModulePass>);

impl BoxedModulePass {
    fn run(&mut self, m: Pin<&mut ffi::Module>, mam: Pin<&mut ffi::ModuleAnalysisManager>) -> u32 {
        match catch_unwind(AssertUnwindSafe(|| {
            let mut module_ref = ModuleRef { module: m, mam };
            self.0.run(&mut module_ref).0
        })) {
            Ok(mask) => mask,
            Err(payload) => handle_ffi_panic(self.0.name(), payload),
        }
    }

    fn name(&self) -> &str {
        self.0.name()
    }
}

/// `cxx` 桥接使用的函数级 Pass 包装类型，通常不直接构造。
pub struct BoxedFunctionPass(pub(crate) Box<dyn FunctionPass>);

impl BoxedFunctionPass {
    fn run(
        &mut self,
        f: Pin<&mut ffi::Function>,
        fam: Pin<&mut ffi::FunctionAnalysisManager>,
    ) -> u32 {
        match catch_unwind(AssertUnwindSafe(|| {
            let mut func_ref = FunctionRef { func: f, fam };
            self.0.run(&mut func_ref).0
        })) {
            Ok(mask) => mask,
            Err(payload) => handle_ffi_panic(self.0.name(), payload),
        }
    }

    fn name(&self) -> &str {
        self.0.name()
    }
}

type ModuleEpCallbackFn = dyn Fn(&mut ModulePassManager<'_>, OptLevel) + Send + Sync;
type ModuleLtoEpCallbackFn = dyn Fn(&mut ModulePassManager<'_>, OptLevel, LtoPhase) + Send + Sync;
type FunctionEpCallbackFn = dyn Fn(&mut FunctionPassManager<'_>, OptLevel) + Send + Sync;
type ModulePipelineParserFn = dyn Fn(&str, &mut ModulePassManager<'_>) -> bool + Send + Sync;
type FunctionPipelineParserFn = dyn Fn(&str, &mut FunctionPassManager<'_>) -> bool + Send + Sync;

/// `cxx` 桥接使用的模块级扩展点回调包装类型，通常不直接构造。
pub struct BoxedModuleEpCallback(pub(crate) Arc<ModuleEpCallbackFn>);

impl BoxedModuleEpCallback {
    fn call(&self, mpm: Pin<&mut ffi::ModulePassManager>, level: OptLevel) {
        abort_on_panic("ModuleEpCallback", || {
            let mut mgr = ModulePassManager(mpm);
            self.0(&mut mgr, level);
        });
    }

    fn clone_cb(&self) -> Box<Self> {
        Box::new(Self(Arc::clone(&self.0)))
    }
}

/// `cxx` 桥接使用的带 LTO 阶段模块级扩展点回调包装类型，通常不直接构造。
pub struct BoxedModuleLtoEpCallback(pub(crate) Arc<ModuleLtoEpCallbackFn>);

impl BoxedModuleLtoEpCallback {
    fn call(&self, mpm: Pin<&mut ffi::ModulePassManager>, level: OptLevel, phase: LtoPhase) {
        abort_on_panic("ModuleLtoEpCallback", || {
            let mut mgr = ModulePassManager(mpm);
            self.0(&mut mgr, level, phase);
        });
    }

    fn clone_cb(&self) -> Box<Self> {
        Box::new(Self(Arc::clone(&self.0)))
    }
}

/// `cxx` 桥接使用的函数级扩展点回调包装类型，通常不直接构造。
pub struct BoxedFunctionEpCallback(pub(crate) Arc<FunctionEpCallbackFn>);

impl BoxedFunctionEpCallback {
    fn call(&self, fpm: Pin<&mut ffi::FunctionPassManager>, level: OptLevel) {
        abort_on_panic("FunctionEpCallback", || {
            let mut mgr = FunctionPassManager(fpm);
            self.0(&mut mgr, level);
        });
    }

    fn clone_cb(&self) -> Box<Self> {
        Box::new(Self(Arc::clone(&self.0)))
    }
}

/// `cxx` 桥接使用的模块级管线解析回调包装类型，通常不直接构造。
pub struct BoxedModulePipelineParser(pub(crate) Arc<ModulePipelineParserFn>);

impl BoxedModulePipelineParser {
    fn parse(&self, name: &str, mpm: Pin<&mut ffi::ModulePassManager>) -> bool {
        abort_on_panic("ModulePipelineParser", || {
            let mut mgr = ModulePassManager(mpm);
            self.0(name, &mut mgr)
        })
    }

    fn clone_cb(&self) -> Box<Self> {
        Box::new(Self(Arc::clone(&self.0)))
    }
}

/// `cxx` 桥接使用的函数级管线解析回调包装类型，通常不直接构造。
pub struct BoxedFunctionPipelineParser(pub(crate) Arc<FunctionPipelineParserFn>);

impl BoxedFunctionPipelineParser {
    fn parse(&self, name: &str, fpm: Pin<&mut ffi::FunctionPassManager>) -> bool {
        abort_on_panic("FunctionPipelineParser", || {
            let mut mgr = FunctionPassManager(fpm);
            self.0(name, &mut mgr)
        })
    }

    fn clone_cb(&self) -> Box<Self> {
        Box::new(Self(Arc::clone(&self.0)))
    }
}

/// 本框架实现的 LLVM Pass Plugin API 版本号。
///
/// LLVM 18 ~ 21 为 `1`，LLVM 22 为 `2`（新增 `PreCodeGenCallback` 字段）。
#[cfg(not(feature = "llvm22"))]
pub const LLVM_PLUGIN_API_VERSION: u32 = 1;
/// 本框架实现的 LLVM Pass Plugin API 版本号。
///
/// LLVM 18 ~ 21 为 `1`，LLVM 22 为 `2`（新增 `PreCodeGenCallback` 字段）。
#[cfg(feature = "llvm22")]
pub const LLVM_PLUGIN_API_VERSION: u32 = 2;

/// 与 `llvm::PassPluginLibraryInfo` 对应的 C ABI 结构体。
///
/// 字段布局随 LLVM 版本变化：LLVM 18 ~ 21 为 4 个字段，LLVM 22 新增
/// `pre_code_gen_callback`。由 [`export_llvm_plugin!`] 填充并返回给 LLVM
/// 插件加载器，通常不直接构造。
#[repr(C)]
pub struct PassPluginLibraryInfo {
    /// 插件 API 版本，固定为 [`LLVM_PLUGIN_API_VERSION`]。
    pub api_version: u32,
    /// 以 NUL 结尾的插件名称。
    pub plugin_name: *const c_char,
    /// 以 NUL 结尾的插件版本。
    pub plugin_version: *const c_char,
    /// 由 LLVM 调用的注册回调，接收 `llvm::PassBuilder` 指针。
    pub register_pass_builder_callbacks: Option<unsafe extern "C" fn(pb: *mut ffi::PassBuilder)>,
    /// 预代码生成回调地址（仅 LLVM 22+）；本框架不使用，固定为空指针。
    #[cfg(feature = "llvm22")]
    pub pre_code_gen_callback: usize,
}

/// 构造 [`PassPluginLibraryInfo`]，屏蔽不同 LLVM 版本的字段差异。
///
/// [`export_llvm_plugin!`] 在下游 crate 中展开，而下游 crate 不定义本 crate 的
/// `llvm*` 特性；因此结构体字面量必须收敛到这里，避免宏内的 `#[cfg]` 判断
/// 依赖下游的 feature 环境。
#[doc(hidden)]
#[inline]
pub const fn __internal_build_plugin_info(
    plugin_name: *const c_char,
    plugin_version: *const c_char,
    register_cb: unsafe extern "C" fn(pb: *mut ffi::PassBuilder),
) -> PassPluginLibraryInfo {
    PassPluginLibraryInfo {
        api_version: LLVM_PLUGIN_API_VERSION,
        plugin_name,
        plugin_version,
        register_pass_builder_callbacks: Some(register_cb),
        #[cfg(feature = "llvm22")]
        pre_code_gen_callback: 0,
    }
}

/// 从 LLVM 注册回调的原始指针构造 [`PassBuilder`] 并执行注册逻辑。
///
/// # Safety
///
/// `pb` 必须是由 LLVM 插件加载器在 `register_pass_builder_callbacks` 回调中传入的
/// 有效、非空指针，并且在 `f` 执行期间保持有效且不被其他线程修改。
#[doc(hidden)]
pub unsafe fn __internal_with_pass_builder<F>(pb: *mut ffi::PassBuilder, f: F)
where
    F: FnOnce(&mut PassBuilder<'_>),
{
    abort_on_panic("llvmGetPassPluginInfo::register", || {
        // SAFETY: 调用方保证 pb 非空、有效，且在回调期间独占可写。
        if let Some(pb_ref) = unsafe { pb.as_mut() } {
            // SAFETY: PassBuilder 由 LLVM 持有，在注册回调期间不会被移动。
            let pinned = unsafe { Pin::new_unchecked(pb_ref) };
            let mut safe_pb = PassBuilder(pinned);
            f(&mut safe_pb);
        }
    });
}

/// 生成 LLVM Pass 插件的 C ABI 入口 `llvmGetPassPluginInfo`。
///
/// 宏会在调用处定义一个 `#[unsafe(no_mangle)] extern "C"` 函数，返回符合当前启用的
/// LLVM 版本（18 ~ 22）`PassPluginLibraryInfo` 布局的结构体，供 `opt` /
/// `clang` / `rustc` 在加载插件时调用。插件工程需将 crate 类型设为 `cdylib`，
/// 并确保 `llvm-pass` 恰好启用一个 `llvm*` 特性。
///
/// # Examples
///
/// ```rust
/// use llvm_pass::*;
///
/// export_llvm_plugin! {
///     name: "MyPlugin",
///     version: "0.1.0",
///     register: |pb: &mut PassBuilder| {
///         pb.register_pipeline_start_ep_callback(|mpm, _level| {
///             mpm.add_pass(|m: &mut ModuleRef<'_>| {
///                 eprintln!("module = {}", m.name());
///                 PreservedAnalyses::All
///             });
///         });
///     }
/// }
/// ```
#[macro_export]
macro_rules! export_llvm_plugin {
    (
        name: $name:literal,
        version: $version:literal,
        register: $register_fn:expr
    ) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn llvmGetPassPluginInfo() -> $crate::PassPluginLibraryInfo {
            unsafe extern "C" fn __register_trampoline(pb: *mut $crate::ffi::PassBuilder) {
                // SAFETY: pb 由 LLVM 插件加载器在注册回调中传入，满足
                // __internal_with_pass_builder 的 # Safety 前置条件。
                unsafe {
                    $crate::__internal_with_pass_builder(pb, $register_fn);
                }
            }
            $crate::__internal_build_plugin_info(
                concat!($name, "\0").as_ptr() as *const ::std::ffi::c_char,
                concat!($version, "\0").as_ptr() as *const ::std::ffi::c_char,
                __register_trampoline,
            )
        }
    };
}
