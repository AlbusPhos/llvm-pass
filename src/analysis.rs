// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! 分析管理器视图、`PreservedAnalyses` 与内置分析实现。

use std::ffi::c_void;

use crate::ffi;
use crate::ir::{FunctionRef, ModuleRef};

/// `PreservedAnalyses` 内部使用的位掩码常量。
///
/// 必须与 `cxx/shim.cpp` 中的 `PA_*` 常量严格对应。
pub(crate) mod pa_mask {
    pub const NONE: u32 = 0;
    pub const ALL: u32 = 1 << 0;
    pub const CFG: u32 = 1 << 1;
    pub const DOMINATOR_TREE: u32 = 1 << 2;
    pub const POST_DOM_TREE: u32 = 1 << 3;
    pub const LOOP_INFO: u32 = 1 << 4;
    pub const CALL_GRAPH: u32 = 1 << 5;
    pub const FAM_PROXY: u32 = 1 << 6;
    pub const GLOBALS_AA: u32 = 1 << 7;
}

/// Pass 执行后对 LLVM 分析结果的保留集合。
///
/// 既可直接使用内置常量 [`PreservedAnalyses::All`]、[`PreservedAnalyses::PreserveCFG`]、
/// [`PreservedAnalyses::None`]，也可通过 [`PreservedAnalyses::preserve`] 链式声明保留特定分析：
///
/// ```
/// use llvm_pass::{
///     CallGraphAnalysis, DominatorTreeAnalysis, LoopAnalysis, PreservedAnalyses,
/// };
///
/// let pa = PreservedAnalyses::None
///     .preserve::<DominatorTreeAnalysis>()
///     .preserve::<LoopAnalysis>();
/// assert!(pa.is_preserved::<DominatorTreeAnalysis>());
/// assert!(!pa.is_preserved::<CallGraphAnalysis>());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreservedAnalyses(pub(crate) u32);

#[allow(non_upper_case_globals)]
impl PreservedAnalyses {
    /// 保留全部分析（Pass 未修改 IR）。
    pub const All: Self = Self(pa_mask::ALL);

    /// 仅保留控制流图相关分析（`CFGAnalyses`：含 `DominatorTree`、`PostDominatorTree`、`LoopInfo`）。
    pub const PreserveCFG: Self = Self(pa_mask::CFG);

    /// 不保留任何分析（Pass 可能任意修改 IR）。
    pub const None: Self = Self(pa_mask::NONE);

    /// 在当前保留集合基础上，额外声明保留分析 `A`。
    #[inline]
    #[must_use]
    pub const fn preserve<A: PreservableAnalysis>(self) -> Self {
        if (self.0 & pa_mask::ALL) != 0 {
            self
        } else {
            Self(self.0 | A::PRESERVE_BIT)
        }
    }

    /// 在模块 Pass 中声明保留 `FunctionAnalysisManagerModuleProxy`（避免清空内部函数分析缓存）。
    #[inline]
    #[must_use]
    pub const fn preserve_function_analyses_proxy(self) -> Self {
        if (self.0 & pa_mask::ALL) != 0 {
            self
        } else {
            Self(self.0 | pa_mask::FAM_PROXY)
        }
    }

    /// 判断是否保留了全部分析。
    #[inline]
    pub const fn is_all(self) -> bool {
        (self.0 & pa_mask::ALL) != 0
    }

    /// 判断是否不保留任何分析。
    #[inline]
    pub const fn is_none(self) -> bool {
        self.0 == pa_mask::NONE
    }

    /// 判断是否保留了指定的分析类型 `A`。
    #[inline]
    pub const fn is_preserved<A: PreservableAnalysis>(self) -> bool {
        (self.0 & pa_mask::ALL) != 0
            || (self.0 & A::PRESERVE_BIT) != 0
            || ((self.0 & pa_mask::CFG) != 0 && A::IS_CFG_ANALYSIS)
    }
}

/// 可在 [`PreservedAnalyses::preserve`] 中声明保留的分析标记契约。
///
/// 注意：`PRESERVE_BIT` 必须与 `cxx/shim.cpp` 中 `map_preserved_analyses` 识别的位
/// 一一对应；当前仅内置分析满足该条件。下游若要保留自定义 LLVM 分析，需要同步
/// 扩展 C++ shim 并新增对应位。
pub trait PreservableAnalysis {
    /// 对应的内部保留位掩码。
    const PRESERVE_BIT: u32;
    /// 该分析是否属于 `CFGAnalyses` 集合（若为 `true`，则 `PreserveCFG` 会自动涵盖它）。
    const IS_CFG_ANALYSIS: bool = false;
}

/// 模块级分析（`ModuleAnalysisManager`）的统一契约。
pub trait ModuleAnalysis: PreservableAnalysis {
    /// 分析结果视图类型，生命周期 `'a` 绑定在 `ModuleRef` 的借用上。
    type Result<'a>;

    /// 获取或惰性计算该分析结果（对应 `MAM.getResult<PassT>(M)`）。
    fn get_result<'a>(module: &'a ModuleRef<'_>) -> Self::Result<'a>;

    /// 仅获取已缓存的分析结果，不触发计算（对应 `MAM.getCachedResult<PassT>(M)`）。
    fn get_cached_result<'a>(module: &'a ModuleRef<'_>) -> Option<Self::Result<'a>>;
}

/// 支持从函数级 Pass 通过 `ModuleAnalysisManagerFunctionProxy` 只读查询的模块级缓存分析契约。
///
/// 只应为"永不失效"的模块级分析实现本 trait（即其 `Result::invalidate` 始终返回
/// `false`，例如 [`GlobalsAA`]）。LLVM 会在调试构建中对可失效分析的跨层级缓存查询
/// 触发 `Cached result cannot be invalidated` 断言，因此 [`CallGraphAnalysis`] 等
/// 普通模块级分析不实现该 trait。
pub trait OuterModuleAnalysis: ModuleAnalysis {
    /// 从 `FunctionRef` 查询外层模块已缓存的分析结果。
    fn get_cached_from_function<'a>(func: &'a FunctionRef<'_>) -> Option<Self::Result<'a>>;
}

/// 函数级分析（`FunctionAnalysisManager`）的统一契约。
pub trait FunctionAnalysis: PreservableAnalysis {
    /// 分析结果视图类型，生命周期 `'a` 绑定在 `FunctionRef` 的借用上。
    type Result<'a>;

    /// 获取或惰性计算该分析结果（对应 `FAM.getResult<PassT>(F)`）。
    fn get_result<'a>(func: &'a FunctionRef<'_>) -> Self::Result<'a>;

    /// 仅获取已缓存的分析结果，不触发计算（对应 `FAM.getCachedResult<PassT>(F)`）。
    fn get_cached_result<'a>(func: &'a FunctionRef<'_>) -> Option<Self::Result<'a>>;
}

// ==================== 内置分析类型实现 ====================

/// 支配树分析标记类型（对应 `llvm::DominatorTreeAnalysis`）。
pub struct DominatorTreeAnalysis;

impl PreservableAnalysis for DominatorTreeAnalysis {
    const PRESERVE_BIT: u32 = pa_mask::DOMINATOR_TREE;
    const IS_CFG_ANALYSIS: bool = true;
}

impl FunctionAnalysis for DominatorTreeAnalysis {
    type Result<'a> = DominatorTree<'a>;

    #[inline]
    fn get_result<'a>(func: &'a FunctionRef<'_>) -> Self::Result<'a> {
        DominatorTree(ffi::fam_get_dominator_tree(&func.fam, &func.func))
    }

    #[inline]
    fn get_cached_result<'a>(func: &'a FunctionRef<'_>) -> Option<Self::Result<'a>> {
        let ptr = ffi::fam_get_cached_dominator_tree(&func.fam, &func.func);
        // SAFETY: 该指针由 LLVM AnalysisManager 返回，要么为空，要么指向在
        // `func` 借用期间保持有效的分析结果。
        unsafe { ptr.as_ref().map(DominatorTree) }
    }
}

/// `llvm::DominatorTree` 的只读借用视图。
#[derive(Clone, Copy)]
pub struct DominatorTree<'a>(&'a ffi::DominatorTree);

impl<'a> DominatorTree<'a> {
    /// 返回底层 `const llvm::DominatorTree*` 裸指针。
    #[inline]
    pub fn as_raw_ptr(&self) -> *const c_void {
        self.0 as *const ffi::DominatorTree as *const c_void
    }

    /// 判断基本块 `a` 是否支配基本块 `b`（含 `a == b`）。
    #[cfg(feature = "inkwell")]
    #[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
    #[inline]
    pub fn dominates(
        &self,
        a: inkwell::basic_block::BasicBlock<'_>,
        b: inkwell::basic_block::BasicBlock<'_>,
    ) -> bool {
        // SAFETY: `a`、`b` 均为调用方持有的有效 inkwell 基本块，且在其生命周期内，
        // 底层 BasicBlock 指针有效。
        unsafe { ffi::dt_dominates_bb(self.0, a.as_mut_ptr().cast(), b.as_mut_ptr().cast()) }
    }

    /// 判断基本块 `a` 是否严格支配基本块 `b`（`a != b`）。
    #[cfg(feature = "inkwell")]
    #[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
    #[inline]
    pub fn strictly_dominates(
        &self,
        a: inkwell::basic_block::BasicBlock<'_>,
        b: inkwell::basic_block::BasicBlock<'_>,
    ) -> bool {
        // SAFETY: 同上，两个基本块指针在调用期间均有效。
        unsafe {
            ffi::dt_strictly_dominates_bb(self.0, a.as_mut_ptr().cast(), b.as_mut_ptr().cast())
        }
    }

    /// 判断基本块 `bb` 是否可从函数入口块到达。
    #[cfg(feature = "inkwell")]
    #[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
    #[inline]
    pub fn is_reachable_from_entry(&self, bb: inkwell::basic_block::BasicBlock<'_>) -> bool {
        // SAFETY: `bb` 为调用方持有的有效 inkwell 基本块，调用期间指针有效。
        unsafe { ffi::dt_is_reachable_from_entry(self.0, bb.as_mut_ptr().cast()) }
    }
}

/// 后支配树分析标记类型（对应 `llvm::PostDominatorTreeAnalysis`）。
pub struct PostDominatorTreeAnalysis;

impl PreservableAnalysis for PostDominatorTreeAnalysis {
    const PRESERVE_BIT: u32 = pa_mask::POST_DOM_TREE;
    const IS_CFG_ANALYSIS: bool = true;
}

impl FunctionAnalysis for PostDominatorTreeAnalysis {
    type Result<'a> = PostDominatorTree<'a>;

    #[inline]
    fn get_result<'a>(func: &'a FunctionRef<'_>) -> Self::Result<'a> {
        PostDominatorTree(ffi::fam_get_post_dominator_tree(&func.fam, &func.func))
    }

    #[inline]
    fn get_cached_result<'a>(func: &'a FunctionRef<'_>) -> Option<Self::Result<'a>> {
        let ptr = ffi::fam_get_cached_post_dominator_tree(&func.fam, &func.func);
        // SAFETY: 该指针由 LLVM AnalysisManager 返回，要么为空，要么指向在
        // `func` 借用期间保持有效的分析结果。
        unsafe { ptr.as_ref().map(PostDominatorTree) }
    }
}

/// `llvm::PostDominatorTree` 的只读借用视图。
#[derive(Clone, Copy)]
pub struct PostDominatorTree<'a>(&'a ffi::PostDominatorTree);

impl<'a> PostDominatorTree<'a> {
    /// 返回底层 `const llvm::PostDominatorTree*` 裸指针。
    #[inline]
    pub fn as_raw_ptr(&self) -> *const c_void {
        self.0 as *const ffi::PostDominatorTree as *const c_void
    }

    /// 判断基本块 `a` 是否后支配基本块 `b`（含 `a == b`）。
    #[cfg(feature = "inkwell")]
    #[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
    #[inline]
    pub fn dominates(
        &self,
        a: inkwell::basic_block::BasicBlock<'_>,
        b: inkwell::basic_block::BasicBlock<'_>,
    ) -> bool {
        // SAFETY: `a`、`b` 均为调用方持有的有效 inkwell 基本块，且在其生命周期内，
        // 底层 BasicBlock 指针有效。
        unsafe { ffi::pdt_dominates_bb(self.0, a.as_mut_ptr().cast(), b.as_mut_ptr().cast()) }
    }

    /// 判断基本块 `a` 是否严格后支配基本块 `b`（`a != b`）。
    #[cfg(feature = "inkwell")]
    #[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
    #[inline]
    pub fn strictly_dominates(
        &self,
        a: inkwell::basic_block::BasicBlock<'_>,
        b: inkwell::basic_block::BasicBlock<'_>,
    ) -> bool {
        // SAFETY: 同上，两个基本块指针在调用期间均有效。
        unsafe {
            ffi::pdt_strictly_dominates_bb(self.0, a.as_mut_ptr().cast(), b.as_mut_ptr().cast())
        }
    }
}

/// 循环信息分析标记类型（对应 `llvm::LoopAnalysis`）。
pub struct LoopAnalysis;

impl PreservableAnalysis for LoopAnalysis {
    const PRESERVE_BIT: u32 = pa_mask::LOOP_INFO;
    const IS_CFG_ANALYSIS: bool = true;
}

impl FunctionAnalysis for LoopAnalysis {
    type Result<'a> = LoopInfo<'a>;

    #[inline]
    fn get_result<'a>(func: &'a FunctionRef<'_>) -> Self::Result<'a> {
        LoopInfo(ffi::fam_get_loop_info(&func.fam, &func.func))
    }

    #[inline]
    fn get_cached_result<'a>(func: &'a FunctionRef<'_>) -> Option<Self::Result<'a>> {
        let ptr = ffi::fam_get_cached_loop_info(&func.fam, &func.func);
        // SAFETY: 该指针由 LLVM AnalysisManager 返回，要么为空，要么指向在
        // `func` 借用期间保持有效的分析结果。
        unsafe { ptr.as_ref().map(LoopInfo) }
    }
}

/// `llvm::LoopInfo` 的只读借用视图。
#[derive(Clone, Copy)]
pub struct LoopInfo<'a>(&'a ffi::LoopInfo);

impl<'a> LoopInfo<'a> {
    /// 返回底层 `const llvm::LoopInfo*` 裸指针。
    #[inline]
    pub fn as_raw_ptr(&self) -> *const c_void {
        self.0 as *const ffi::LoopInfo as *const c_void
    }

    /// 判断当前函数中是否不包含任何循环。
    #[inline]
    pub fn is_empty(&self) -> bool {
        ffi::li_is_empty(self.0)
    }

    /// 返回当前函数中顶层循环（Top-Level Loops）的数量。
    #[inline]
    pub fn top_level_loops_count(&self) -> usize {
        ffi::li_top_level_loops_count(self.0)
    }

    /// 返回基本块 `bb` 所在的循环嵌套深度（`0` 表示不在循环内）。
    #[cfg(feature = "inkwell")]
    #[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
    #[inline]
    pub fn loop_depth(&self, bb: inkwell::basic_block::BasicBlock<'_>) -> u32 {
        // SAFETY: `bb` 为调用方持有的有效 inkwell 基本块，调用期间指针有效。
        unsafe { ffi::li_get_loop_depth(self.0, bb.as_mut_ptr().cast()) }
    }

    /// 判断基本块 `bb` 是否为某个循环的头节点（Loop Header）。
    #[cfg(feature = "inkwell")]
    #[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
    #[inline]
    pub fn is_loop_header(&self, bb: inkwell::basic_block::BasicBlock<'_>) -> bool {
        // SAFETY: `bb` 为调用方持有的有效 inkwell 基本块，调用期间指针有效。
        unsafe { ffi::li_is_loop_header(self.0, bb.as_mut_ptr().cast()) }
    }
}

/// 调用图分析标记类型（对应 `llvm::CallGraphAnalysis`）。
pub struct CallGraphAnalysis;

impl PreservableAnalysis for CallGraphAnalysis {
    const PRESERVE_BIT: u32 = pa_mask::CALL_GRAPH;
}

impl ModuleAnalysis for CallGraphAnalysis {
    type Result<'a> = CallGraph<'a>;

    #[inline]
    fn get_result<'a>(module: &'a ModuleRef<'_>) -> Self::Result<'a> {
        CallGraph(ffi::mam_get_call_graph(&module.mam, &module.module))
    }

    #[inline]
    fn get_cached_result<'a>(module: &'a ModuleRef<'_>) -> Option<Self::Result<'a>> {
        let ptr = ffi::mam_get_cached_call_graph(&module.mam, &module.module);
        // SAFETY: 该指针由 LLVM AnalysisManager 返回，要么为空，要么指向在
        // `module` 借用期间保持有效的分析结果。
        unsafe { ptr.as_ref().map(CallGraph) }
    }
}

/// `llvm::CallGraph` 的只读借用视图。
#[derive(Clone, Copy)]
pub struct CallGraph<'a>(&'a ffi::CallGraph);

impl<'a> CallGraph<'a> {
    /// 返回底层 `const llvm::CallGraph*` 裸指针。
    #[inline]
    pub fn as_raw_ptr(&self) -> *const c_void {
        self.0 as *const ffi::CallGraph as *const c_void
    }
}

/// 全局别名分析（`llvm::GlobalsAA`）的标记类型。
///
/// `GlobalsAA` 是 LLVM 明确定义的"永不失效"模块级分析。只有这类分析才能从
/// 函数级 Pass 通过 `ModuleAnalysisManagerFunctionProxy` 跨层级读取；
/// 可失效的模块级分析（如 [`CallGraphAnalysis`]）在 LLVM 调试构建中会触发
/// `Cached result cannot be invalidated` 断言。
pub struct GlobalsAA;

impl PreservableAnalysis for GlobalsAA {
    const PRESERVE_BIT: u32 = pa_mask::GLOBALS_AA;
}

impl ModuleAnalysis for GlobalsAA {
    type Result<'a> = GlobalsAAResult<'a>;

    #[inline]
    fn get_result<'a>(module: &'a ModuleRef<'_>) -> Self::Result<'a> {
        GlobalsAAResult(ffi::mam_get_globals_aa(&module.mam, &module.module))
    }

    #[inline]
    fn get_cached_result<'a>(module: &'a ModuleRef<'_>) -> Option<Self::Result<'a>> {
        let ptr = ffi::mam_get_cached_globals_aa(&module.mam, &module.module);
        // SAFETY: 该指针由 LLVM AnalysisManager 返回，要么为空，要么指向在
        // `module` 借用期间保持有效的分析结果。
        unsafe { ptr.as_ref().map(GlobalsAAResult) }
    }
}

impl OuterModuleAnalysis for GlobalsAA {
    #[inline]
    fn get_cached_from_function<'a>(func: &'a FunctionRef<'_>) -> Option<Self::Result<'a>> {
        let ptr = ffi::fam_get_cached_globals_aa(&func.fam, &func.func);
        // SAFETY: 该指针由 LLVM AnalysisManager 返回，要么为空，要么指向在
        // `func` 借用期间保持有效的分析结果。
        unsafe { ptr.as_ref().map(GlobalsAAResult) }
    }
}

/// `llvm::GlobalsAAResult` 的只读借用视图。
#[derive(Clone, Copy)]
pub struct GlobalsAAResult<'a>(&'a ffi::GlobalsAAResult);

impl<'a> GlobalsAAResult<'a> {
    /// 返回底层 `const llvm::GlobalsAAResult*` 裸指针。
    #[inline]
    pub fn as_raw_ptr(&self) -> *const c_void {
        self.0 as *const ffi::GlobalsAAResult as *const c_void
    }
}
