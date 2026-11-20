// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! LLVM IR 上下文借用视图（`ModuleRef` / `FunctionRef` / inkwell 桥接）。

use std::ffi::c_void;
#[cfg(feature = "inkwell")]
use std::mem::ManuallyDrop;
#[cfg(feature = "inkwell")]
use std::ops::Deref;
use std::pin::Pin;

use crate::analysis::{
    CallGraph, CallGraphAnalysis, DominatorTree, DominatorTreeAnalysis, FunctionAnalysis,
    GlobalsAA, GlobalsAAResult, LoopAnalysis, LoopInfo, ModuleAnalysis, OuterModuleAnalysis,
    PostDominatorTree, PostDominatorTreeAnalysis, PreservedAnalyses,
};
use crate::ffi;

/// [`inkwell::module::Module`] 的非拥有型借用视图，由 [`ModuleRef::as_inkwell`] 返回。
///
/// `inkwell::module::Module` 默认拥有底层 `LLVMModuleRef` 的所有权，在 `Drop` 时会调用
/// `LLVMDisposeModule` 释放模块。而在 Pass 插件中，模块所有权归属于宿主编译器（如 `opt` / `clang`）。
/// 本类型通过内部 [`ManuallyDrop`] 屏蔽析构逻辑，并通过 [`Deref`] 暴露
/// `inkwell::module::Module` 的全部方法，同时防止调用方移动或销毁底层模块。
#[cfg(feature = "inkwell")]
#[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
#[derive(Debug)]
pub struct InkwellModule<'a>(ManuallyDrop<inkwell::module::Module<'a>>);

#[cfg(feature = "inkwell")]
impl<'a> Deref for InkwellModule<'a> {
    type Target = inkwell::module::Module<'a>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// LLVM `Module` 及其关联 `ModuleAnalysisManager` 的上下文借用视图，作为 [`ModulePass::run`](crate::ModulePass::run) 的参数传入。
pub struct ModuleRef<'a> {
    pub(crate) module: Pin<&'a mut ffi::Module>,
    pub(crate) mam: Pin<&'a mut ffi::ModuleAnalysisManager>,
}

impl<'a> ModuleRef<'a> {
    /// 返回模块的 LLVM 名称。
    #[inline]
    pub fn name(&self) -> &str {
        ffi::get_module_name(&self.module)
    }

    /// 返回底层 `llvm::Module` 的裸指针，便于直接调用 LLVM C/C++ API。
    ///
    /// # Safety
    ///
    /// 该指针仅在 `self` 的借用有效期内有效。调用方不得移动或释放底层对象，
    /// 也不得在借用结束后继续使用此指针。通过指针修改 IR 时，Pass 返回的
    /// [`PreservedAnalyses`] 必须如实反映修改范围。
    #[inline]
    pub fn as_raw_ptr(&mut self) -> *mut c_void {
        // SAFETY: `self.module` 是对已 pin 对象的可变借用；这里只取出指针而不移动目标，
        // 调用方在借用有效期内使用该指针满足 Pin 的约束。
        unsafe {
            Pin::into_inner_unchecked(self.module.as_mut()) as *mut ffi::Module as *mut c_void
        }
    }

    /// 返回底层 `llvm::ModuleAnalysisManager` 的裸指针。
    #[inline]
    pub fn analysis_manager_raw_ptr(&mut self) -> *mut c_void {
        // SAFETY: `self.mam` 是对已 pin 对象的可变借用；这里只取出指针而不移动目标。
        unsafe {
            Pin::into_inner_unchecked(self.mam.as_mut()) as *mut ffi::ModuleAnalysisManager
                as *mut c_void
        }
    }

    /// 获取或惰性计算指定类型的模块级分析结果。
    #[inline]
    pub fn analysis<A: ModuleAnalysis>(&self) -> A::Result<'_> {
        A::get_result(self)
    }

    /// 仅获取已缓存的模块级分析结果（若未缓存则返回 `None`，不会触发计算）。
    #[inline]
    pub fn cached_analysis<A: ModuleAnalysis>(&self) -> Option<A::Result<'_>> {
        A::get_cached_result(self)
    }

    /// 获取或计算当前模块的调用图（[`CallGraph`]）。
    #[inline]
    pub fn call_graph(&self) -> CallGraph<'_> {
        self.analysis::<CallGraphAnalysis>()
    }

    /// 获取或计算当前模块的全局别名分析（[`GlobalsAAResult`]）。
    #[inline]
    pub fn globals_aa(&self) -> GlobalsAAResult<'_> {
        self.analysis::<GlobalsAA>()
    }

    /// 通过 `FunctionAnalysisManagerModuleProxy` 遍历模块中的所有函数，
    /// 并以完整的 [`FunctionRef`]（内含 `FunctionAnalysisManager`）调用闭包。
    ///
    /// 这样即使在 `ModulePass` 中，也能直接调用 `func.dominator_tree()`、
    /// `func.loop_info()` 等函数级分析。
    ///
    /// 闭包内不得增删模块中的函数（会破坏底层迭代器）；若修改了函数 IR，
    /// 应通过 [`FunctionRef::invalidate_analyses`] 使对应分析失效。
    pub fn for_each_function<F>(&mut self, mut f: F)
    where
        F: FnMut(&mut FunctionRef<'_>),
    {
        let mut fam =
            ffi::mam_get_function_analysis_manager(self.mam.as_mut(), self.module.as_mut());
        let mut cur = ffi::module_first_function(self.module.as_mut());
        while !cur.is_null() {
            // SAFETY: `cur` 指向模块链表中的有效 Function；在调用用户闭包前先取出下一个
            // 节点指针，保证即使闭包修改当前函数也不影响迭代。底层对象由模块持有且在
            // `for_each_function` 期间不会移动。
            let next = unsafe {
                ffi::module_next_function(self.module.as_mut(), Pin::new_unchecked(&mut *cur))
            };
            // SAFETY: `cur` 非空且指向有效的 llvm::Function；借用在本次循环迭代内有效。
            let mut func_ref = unsafe {
                FunctionRef {
                    func: Pin::new_unchecked(&mut *cur),
                    fam: fam.as_mut(),
                }
            };
            f(&mut func_ref);
            cur = next;
        }
    }

    /// 根据给定的保留集合立即使当前模块的缓存分析失效。
    ///
    /// 注意：该方法需要独占借用 `&mut self`，编译器会在编译期保证调用此方法时
    /// 没有任何存活的分析结果引用。
    #[inline]
    pub fn invalidate_analyses(&mut self, preserved: PreservedAnalyses) {
        ffi::mam_invalidate_module(self.mam.as_mut(), self.module.as_mut(), preserved.0);
    }

    /// 将当前模块转换为 [`InkwellModule`] 视图，以便调用 `inkwell` API。
    ///
    /// 返回的视图与从中派生的所有 `inkwell` 值（如 `ContextRef`、`FunctionValue`、`Builder` 等）
    /// 的生命周期均绑定在当前 `&self` 借用上，无法逃逸出 [`ModulePass::run`](crate::ModulePass::run)。
    #[cfg(feature = "inkwell")]
    #[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
    #[inline]
    pub fn as_inkwell(&self) -> InkwellModule<'_> {
        let raw_ptr = (&*self.module) as *const ffi::Module as *mut c_void;
        // SAFETY:
        // 1. `raw_ptr` 指向有效的 `llvm::Module`，在 LLVM 中与 `LLVMModuleRef` 布局等价。
        // 2. 使用 `ManuallyDrop` 包装，防止 `inkwell::module::Module::drop` 调用 `LLVMDisposeModule`
        //    释放由宿主编译器持有的模块。
        // 3. 返回值生命周期绑定至 `&self`，保证借用期间独占且不会逃逸出 Pass 执行周期。
        let module = unsafe { inkwell::module::Module::new(raw_ptr.cast()) };
        InkwellModule(ManuallyDrop::new(module))
    }

    /// 在闭包中以 `inkwell` 的 [`Module`](inkwell::module::Module) 与 [`ContextRef`](inkwell::context::ContextRef) 操作当前模块。
    #[cfg(feature = "inkwell")]
    #[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
    #[inline]
    pub fn with_inkwell<'ctx, R>(
        &'ctx self,
        f: impl FnOnce(&inkwell::module::Module<'ctx>, inkwell::context::ContextRef<'ctx>) -> R,
    ) -> R {
        let module = self.as_inkwell();
        let ctx = module.get_context();
        f(&module, ctx)
    }
}

/// LLVM `Function` 及其关联 `FunctionAnalysisManager` 的上下文借用视图，作为 [`FunctionPass::run`](crate::FunctionPass::run) 的参数传入。
pub struct FunctionRef<'a> {
    pub(crate) func: Pin<&'a mut ffi::Function>,
    pub(crate) fam: Pin<&'a mut ffi::FunctionAnalysisManager>,
}

impl<'a> FunctionRef<'a> {
    /// 返回函数的 LLVM 名称。
    #[inline]
    pub fn name(&self) -> &str {
        ffi::get_function_name(&self.func)
    }

    /// 判断该函数是否为声明（无函数体）。
    ///
    /// 外部声明通常没有可优化的指令，也不应在其上计算支配树或循环分析，Pass 可直接跳过。
    #[inline]
    pub fn is_declaration(&self) -> bool {
        ffi::is_function_declaration(&self.func)
    }

    /// 返回底层 `llvm::Function` 的裸指针，便于直接调用 LLVM C/C++ API。
    ///
    /// # Safety
    ///
    /// 该指针仅在 `self` 的借用有效期内有效。调用方不得移动或释放底层对象，
    /// 也不得在借用结束后继续使用此指针。通过指针修改 IR 时，Pass 返回的
    /// [`PreservedAnalyses`] 必须如实反映修改范围。
    #[inline]
    pub fn as_raw_ptr(&mut self) -> *mut c_void {
        // SAFETY: `self.func` 是对已 pin 对象的可变借用；这里只取出指针而不移动目标，
        // 调用方在借用有效期内使用该指针满足 Pin 的约束。
        unsafe {
            Pin::into_inner_unchecked(self.func.as_mut()) as *mut ffi::Function as *mut c_void
        }
    }

    /// 返回底层 `llvm::FunctionAnalysisManager` 的裸指针。
    #[inline]
    pub fn analysis_manager_raw_ptr(&mut self) -> *mut c_void {
        // SAFETY: `self.fam` 是对已 pin 对象的可变借用；这里只取出指针而不移动目标。
        unsafe {
            Pin::into_inner_unchecked(self.fam.as_mut()) as *mut ffi::FunctionAnalysisManager
                as *mut c_void
        }
    }

    /// 获取或惰性计算指定类型的函数级分析结果。
    ///
    /// 由于采用共享借用 `&self`（底层 `AnalysisManager` 节点地址稳定），
    /// 可同时获取多个不同分析（如 `DominatorTree` 与 `LoopInfo`）并与 `with_inkwell` 混用。
    #[inline]
    pub fn analysis<A: FunctionAnalysis>(&self) -> A::Result<'_> {
        A::get_result(self)
    }

    /// 仅获取已缓存的函数级分析结果（若未缓存则返回 `None`，不会触发计算）。
    #[inline]
    pub fn cached_analysis<A: FunctionAnalysis>(&self) -> Option<A::Result<'_>> {
        A::get_cached_result(self)
    }

    /// 跨层级获取外层模块已缓存的分析结果（通过 `ModuleAnalysisManagerFunctionProxy`）。
    ///
    /// 根据 LLVM 并发与确定性规范，函数级 Pass 不允许触发外层模块分析的重新计算，因此仅提供缓存查询。
    #[inline]
    pub fn cached_module_analysis<A: OuterModuleAnalysis>(&self) -> Option<A::Result<'_>> {
        A::get_cached_from_function(self)
    }

    /// 获取或计算当前函数的支配树（[`DominatorTree`]）。
    #[inline]
    pub fn dominator_tree(&self) -> DominatorTree<'_> {
        self.analysis::<DominatorTreeAnalysis>()
    }

    /// 获取或计算当前函数的后支配树（[`PostDominatorTree`]）。
    #[inline]
    pub fn post_dominator_tree(&self) -> PostDominatorTree<'_> {
        self.analysis::<PostDominatorTreeAnalysis>()
    }

    /// 获取或计算当前函数的循环信息（[`LoopInfo`]）。
    #[inline]
    pub fn loop_info(&self) -> LoopInfo<'_> {
        self.analysis::<LoopAnalysis>()
    }

    /// 根据给定的保留集合立即使当前函数的缓存分析失效。
    ///
    /// 注意：该方法需要独占借用 `&mut self`，编译器会在编译期保证调用此方法时
    /// 没有任何存活的 `DominatorTree` / `LoopInfo` 等分析引用，防止悬垂指针。
    #[inline]
    pub fn invalidate_analyses(&mut self, preserved: PreservedAnalyses) {
        ffi::fam_invalidate_function(self.fam.as_mut(), self.func.as_mut(), preserved.0);
    }

    /// 将当前函数转换为 [`inkwell::values::FunctionValue`]，以便调用 `inkwell` API。
    ///
    /// 返回的 `FunctionValue` 及其关联 `ContextRef` 的生命周期绑定在当前 `&self` 借用上，
    /// 无法逃逸出 [`FunctionPass::run`](crate::FunctionPass::run)。
    #[cfg(feature = "inkwell")]
    #[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
    #[inline]
    pub fn as_inkwell(&self) -> inkwell::values::FunctionValue<'_> {
        let raw_ptr = (&*self.func) as *const ffi::Function as *mut c_void;
        // SAFETY:
        // 1. `raw_ptr` 返回非空且有效的 `llvm::Function*`，与 `LLVMValueRef` 等价。
        // 2. `FunctionValue` 为 `Copy` 视图类型，无析构副作用，生命周期受 `&self` 约束。
        unsafe {
            inkwell::values::FunctionValue::new(raw_ptr.cast())
                .expect("FunctionRef must hold a valid non-null LLVM Function pointer")
        }
    }

    /// 在闭包中以 `inkwell` 的 [`FunctionValue`](inkwell::values::FunctionValue) 与 [`ContextRef`](inkwell::context::ContextRef) 操作当前函数。
    #[cfg(feature = "inkwell")]
    #[cfg_attr(docsrs, doc(cfg(feature = "inkwell")))]
    #[inline]
    pub fn with_inkwell<'ctx, R>(
        &'ctx self,
        f: impl FnOnce(inkwell::values::FunctionValue<'ctx>, inkwell::context::ContextRef<'ctx>) -> R,
    ) -> R {
        let func = self.as_inkwell();
        let ctx = func.get_type().get_context();
        f(func, ctx)
    }
}
