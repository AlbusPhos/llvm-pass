// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pass trait、PassManager 与 PassBuilder 的安全封装。

use std::pin::Pin;
use std::sync::Arc;

use crate::analysis::PreservedAnalyses;
use crate::ffi::{self, LtoPhase, OptLevel};
use crate::ir::{FunctionRef, ModuleRef};
use crate::{
    BoxedFunctionEpCallback, BoxedFunctionPass, BoxedFunctionPipelineParser, BoxedModuleEpCallback,
    BoxedModuleLtoEpCallback, BoxedModulePass, BoxedModulePipelineParser,
};

// ==================== Pass Trait 定义 ====================

/// 模块级 LLVM Pass 的 Rust 侧契约。
///
/// 任何满足 `FnMut(&mut ModuleRef<'_>) -> PreservedAnalyses + Send + Sync + 'static`
/// 的闭包都会自动实现该 trait，因此可以直接向 [`ModulePassManager::add_pass`]
/// 传入闭包。
///
/// # Examples
///
/// ```
/// use llvm_pass::{ModulePass, ModuleRef, PreservedAnalyses};
///
/// fn inspect(module: &mut ModuleRef<'_>) -> PreservedAnalyses {
///     let _name = module.name();
///     let _cg = module.call_graph();
///     PreservedAnalyses::All
/// }
///
/// let mut pass = inspect;
/// assert!(!ModulePass::name(&pass).is_empty());
/// ```
///
/// # Panics
///
/// 实现方不应 panic；跨越 FFI 边界的 panic 会被框架捕获并终止宿主进程。
pub trait ModulePass: Send + Sync + 'static {
    /// Pass 名称，用于 `opt -passes` 的管线打印；默认取当前类型的完整路径名。
    fn name(&self) -> &str {
        std::any::type_name::<Self>()
    }

    /// 对给定模块执行 Pass，返回分析结果的保留范围。
    fn run(&mut self, module: &mut ModuleRef<'_>) -> PreservedAnalyses;
}

/// 函数级 LLVM Pass 的 Rust 侧契约。
///
/// 任何满足 `FnMut(&mut FunctionRef<'_>) -> PreservedAnalyses + Send + Sync + 'static`
/// 的闭包都会自动实现该 trait，因此可以直接向 [`FunctionPassManager::add_pass`]
/// 传入闭包。
///
/// # Examples
///
/// ```
/// use llvm_pass::{DominatorTreeAnalysis, FunctionPass, FunctionRef, PreservedAnalyses};
///
/// fn inspect(func: &mut FunctionRef<'_>) -> PreservedAnalyses {
///     if func.is_declaration() {
///         PreservedAnalyses::All
///     } else {
///         let _name = func.name();
///         let _dt = func.dominator_tree();
///         PreservedAnalyses::PreserveCFG.preserve::<DominatorTreeAnalysis>()
///     }
/// }
///
/// let mut pass = inspect;
/// assert!(!FunctionPass::name(&pass).is_empty());
/// ```
///
/// # Panics
///
/// 实现方不应 panic；跨越 FFI 边界的 panic 会被框架捕获并终止宿主进程。
pub trait FunctionPass: Send + Sync + 'static {
    /// Pass 名称，用于 `opt -passes` 的管线打印；默认取当前类型的完整路径名。
    fn name(&self) -> &str {
        std::any::type_name::<Self>()
    }

    /// 对给定函数执行 Pass，返回分析结果的保留范围。
    fn run(&mut self, func: &mut FunctionRef<'_>) -> PreservedAnalyses;
}

impl<F> ModulePass for F
where
    F: FnMut(&mut ModuleRef<'_>) -> PreservedAnalyses + Send + Sync + 'static,
{
    fn run(&mut self, module: &mut ModuleRef<'_>) -> PreservedAnalyses {
        self(module)
    }
}

impl<F> FunctionPass for F
where
    F: FnMut(&mut FunctionRef<'_>) -> PreservedAnalyses + Send + Sync + 'static,
{
    fn run(&mut self, func: &mut FunctionRef<'_>) -> PreservedAnalyses {
        self(func)
    }
}

/// 拆分形如 `name<key=value;...>` 的参数化 pass 名称。
///
/// 返回 `(基础名称, 参数字符串)`：无 `<...>` 时参数部分为 `None`；
/// 存在尖括号但为空（如 `fla<>`）时参数部分为 `Some("")`。
/// 空字符串、缺少闭合 `>`、以 `<` 开头或含有游离 `>` 的输入返回 `None`。
///
/// # Examples
///
/// ```
/// use llvm_pass::split_parametrized_pass_name;
///
/// assert_eq!(split_parametrized_pass_name("bcf"), Some(("bcf", None)));
/// assert_eq!(
///     split_parametrized_pass_name("bcf<prob=80;loop=3>"),
///     Some(("bcf", Some("prob=80;loop=3")))
/// );
/// assert_eq!(split_parametrized_pass_name("fla<>"), Some(("fla", Some(""))));
/// assert_eq!(split_parametrized_pass_name("<prob=80>"), None);
/// ```
#[inline]
pub fn split_parametrized_pass_name(full_name: &str) -> Option<(&str, Option<&str>)> {
    if let Some((base, rest)) = full_name.split_once('<') {
        if base.is_empty() {
            return None;
        }
        let params = rest.strip_suffix('>')?;
        Some((base, Some(params)))
    } else {
        if full_name.is_empty() || full_name.contains('>') {
            return None;
        }
        Some((full_name, None))
    }
}

/// `llvm::ModulePassManager` 的借用封装，用于挂载模块级 Pass。
///
/// 实例由 [`PassBuilder`] 的扩展点回调或管线解析回调提供。
pub struct ModulePassManager<'a>(pub(crate) Pin<&'a mut ffi::ModulePassManager>);

impl<'a> ModulePassManager<'a> {
    /// 向模块级管线追加一个 [`ModulePass`]。
    pub fn add_pass<P: ModulePass>(&mut self, pass: P) {
        ffi::mpm_add_module_pass(self.0.as_mut(), Box::new(BoxedModulePass(Box::new(pass))));
    }

    /// 通过 `ModuleToFunctionPassAdaptor` 向模块级管线追加一个 [`FunctionPass`]。
    ///
    /// 等价于 [`Self::add_function_pass_eagerly_invalidate`] 的非急切失效版本。
    pub fn add_function_pass<P: FunctionPass>(&mut self, pass: P) {
        ffi::mpm_add_function_pass(
            self.0.as_mut(),
            Box::new(BoxedFunctionPass(Box::new(pass))),
            false,
        );
    }

    /// 追加 [`FunctionPass`] 并要求适配器在每个函数执行后立即让分析结果失效。
    ///
    /// 适用于 Pass 会大幅改写函数、导致缓存分析不可信的场景。
    pub fn add_function_pass_eagerly_invalidate<P: FunctionPass>(&mut self, pass: P) {
        ffi::mpm_add_function_pass(
            self.0.as_mut(),
            Box::new(BoxedFunctionPass(Box::new(pass))),
            true,
        );
    }

    /// 追加原生 LLVM `ADCEPass`（通过 `ModuleToFunctionPassAdaptor`）。
    ///
    /// 这是原生 C++ Pass，不经过 Rust [`ModulePass`] / [`FunctionPass`] 包装；
    /// 如需在自定义管线中等价于 `adce`，可直接调用本方法。
    pub fn add_adce(&mut self) {
        ffi::mpm_add_adce_pass(self.0.as_mut());
    }

    /// 追加原生 LLVM `GlobalDCEPass`。
    pub fn add_global_dce(&mut self) {
        ffi::mpm_add_global_dce_pass(self.0.as_mut());
    }
}

/// `llvm::FunctionPassManager` 的借用封装，用于挂载函数级 Pass。
///
/// 实例由 [`PassBuilder`] 的函数级扩展点回调或管线解析回调提供。
pub struct FunctionPassManager<'a>(pub(crate) Pin<&'a mut ffi::FunctionPassManager>);

impl<'a> FunctionPassManager<'a> {
    /// 向函数级管线追加一个 [`FunctionPass`]。
    pub fn add_pass<P: FunctionPass>(&mut self, pass: P) {
        ffi::fpm_add_function_pass(self.0.as_mut(), Box::new(BoxedFunctionPass(Box::new(pass))));
    }

    /// 追加原生 LLVM `ADCEPass`。
    pub fn add_adce(&mut self) {
        ffi::fpm_add_adce_pass(self.0.as_mut());
    }

    /// 追加原生 LLVM `SimplifyCFGPass`。
    pub fn add_simplify_cfg(&mut self) {
        ffi::fpm_add_simplify_cfg_pass(self.0.as_mut());
    }

    /// 追加原生 LLVM `PromotePass`（`mem2reg`）。
    pub fn add_promote(&mut self) {
        ffi::fpm_add_promote_pass(self.0.as_mut());
    }
}

/// `llvm::PassBuilder` 的借用封装，是注册 Pass 与扩展点回调的入口。
///
/// 通过 [`export_llvm_plugin!`](crate::export_llvm_plugin) 生成的插件入口获得实例。所有回调都运行在 Rust 侧，
/// 其 panic 会被 FFI 边界捕获并终止宿主进程，因此不应依赖 panic 做控制流。
pub struct PassBuilder<'a>(pub(crate) Pin<&'a mut ffi::PassBuilder>);

impl<'a> PassBuilder<'a> {
    /// 注册管线起始扩展点回调（`registerPipelineStartEPCallback`）。
    ///
    /// 回调在默认管线构建之初触发，可挂载模块级 Pass。
    pub fn register_pipeline_start_ep_callback<F>(&mut self, callback: F)
    where
        F: Fn(&mut ModulePassManager<'_>, OptLevel) + Send + Sync + 'static,
    {
        ffi::pb_register_pipeline_start_ep(
            self.0.as_mut(),
            Box::new(BoxedModuleEpCallback(Arc::new(callback))),
        );
    }

    /// 注册早期简化扩展点回调（`registerPipelineEarlySimplificationEPCallback`）。
    ///
    /// 回调同时收到优化级别与 LTO 阶段。
    pub fn register_pipeline_early_simplification_ep_callback<F>(&mut self, callback: F)
    where
        F: Fn(&mut ModulePassManager<'_>, OptLevel, LtoPhase) + Send + Sync + 'static,
    {
        ffi::pb_register_pipeline_early_simplification_ep(
            self.0.as_mut(),
            Box::new(BoxedModuleLtoEpCallback(Arc::new(callback))),
        );
    }

    /// 注册优化器早期扩展点回调（`registerOptimizerEarlyEPCallback`）。
    ///
    /// 回调同时收到优化级别与 LTO 阶段，常用于在主要优化前插入函数级 Pass。
    pub fn register_optimizer_early_ep_callback<F>(&mut self, callback: F)
    where
        F: Fn(&mut ModulePassManager<'_>, OptLevel, LtoPhase) + Send + Sync + 'static,
    {
        ffi::pb_register_optimizer_early_ep(
            self.0.as_mut(),
            Box::new(BoxedModuleLtoEpCallback(Arc::new(callback))),
        );
    }

    /// 注册优化器末尾扩展点回调（`registerOptimizerLastEPCallback`）。
    ///
    /// 回调同时收到优化级别与 LTO 阶段，在所有常规优化之后触发。
    pub fn register_optimizer_last_ep_callback<F>(&mut self, callback: F)
    where
        F: Fn(&mut ModulePassManager<'_>, OptLevel, LtoPhase) + Send + Sync + 'static,
    {
        ffi::pb_register_optimizer_last_ep(
            self.0.as_mut(),
            Box::new(BoxedModuleLtoEpCallback(Arc::new(callback))),
        );
    }

    /// 注册窥孔优化扩展点回调（`registerPeepholeEPCallback`）。
    ///
    /// 回调挂载函数级 Pass。
    pub fn register_peephole_ep_callback<F>(&mut self, callback: F)
    where
        F: Fn(&mut FunctionPassManager<'_>, OptLevel) + Send + Sync + 'static,
    {
        ffi::pb_register_peephole_ep(
            self.0.as_mut(),
            Box::new(BoxedFunctionEpCallback(Arc::new(callback))),
        );
    }

    /// 注册标量优化后期扩展点回调（`registerScalarOptimizerLateEPCallback`）。
    ///
    /// 回调挂载函数级 Pass。
    pub fn register_scalar_optimizer_late_ep_callback<F>(&mut self, callback: F)
    where
        F: Fn(&mut FunctionPassManager<'_>, OptLevel) + Send + Sync + 'static,
    {
        ffi::pb_register_scalar_optimizer_late_ep(
            self.0.as_mut(),
            Box::new(BoxedFunctionEpCallback(Arc::new(callback))),
        );
    }

    /// 注册模块级 `-passes=` 名称解析回调。
    ///
    /// 回调返回 `true` 表示已识别该名称并向管线挂载了 Pass；返回 `false`
    /// 表示不处理，由 LLVM 或其他插件继续解析。可配合
    /// [`split_parametrized_pass_name`] 支持带参数的名称。
    pub fn register_module_pipeline_parsing_callback<F>(&mut self, callback: F)
    where
        F: Fn(&str, &mut ModulePassManager<'_>) -> bool + Send + Sync + 'static,
    {
        ffi::pb_register_module_pipeline_parser(
            self.0.as_mut(),
            Box::new(BoxedModulePipelineParser(Arc::new(callback))),
        );
    }

    /// 注册函数级 `-passes=` 名称解析回调。
    ///
    /// 语义与 [`Self::register_module_pipeline_parsing_callback`] 相同，
    /// 但回调接收 [`FunctionPassManager`]。
    pub fn register_function_pipeline_parsing_callback<F>(&mut self, callback: F)
    where
        F: Fn(&str, &mut FunctionPassManager<'_>) -> bool + Send + Sync + 'static,
    {
        ffi::pb_register_function_pipeline_parser(
            self.0.as_mut(),
            Box::new(BoxedFunctionPipelineParser(Arc::new(callback))),
        );
    }
}
