// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

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
use std::ffi::{c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::Arc;

/// `cxx` 桥接层：声明 C++ 侧辅助函数与 Rust 侧回调包装类型。
///
/// 该模块由 `#[cxx::bridge]` 生成，通常无需直接使用；请优先使用 crate 根导出的
/// [`ModulePass`]、[`FunctionPass`]、[`PassBuilder`] 等安全封装。
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

    /// Pass 执行后对 LLVM 分析结果的保留范围。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum PreservedAnalysesKind {
        /// 保留全部分析（Pass 未修改 IR）。
        All,
        /// 仅保留控制流图相关分析。
        PreserveCFG,
        /// 不保留任何分析（Pass 可能任意修改 IR）。
        None,
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
        type Module;
        #[namespace = "llvm"]
        type Function;

        fn mpm_add_module_pass(mpm: Pin<&mut ModulePassManager>, pass: Box<BoxedModulePass>);
        fn mpm_add_function_pass(
            mpm: Pin<&mut ModulePassManager>,
            pass: Box<BoxedFunctionPass>,
            eagerly_invalidate: bool,
        );
        fn fpm_add_function_pass(fpm: Pin<&mut FunctionPassManager>, pass: Box<BoxedFunctionPass>);

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
    }

    extern "Rust" {
        type BoxedModulePass;
        fn run(self: &mut BoxedModulePass, m: Pin<&mut Module>) -> PreservedAnalysesKind;
        fn name(self: &BoxedModulePass) -> &str;

        type BoxedFunctionPass;
        fn run(self: &mut BoxedFunctionPass, f: Pin<&mut Function>) -> PreservedAnalysesKind;
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

/// 重导出常用值类型：`OptLevel` 与 `LtoPhase` 用于回调参数，
/// `PreservedAnalyses`（即 `ffi::PreservedAnalysesKind`）用于 Pass 返回值。
pub use ffi::{LtoPhase, OptLevel, PreservedAnalysesKind as PreservedAnalyses};

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
/// use llvm_pass::{FunctionPass, FunctionRef, PreservedAnalyses};
///
/// fn inspect(func: &mut FunctionRef<'_>) -> PreservedAnalyses {
///     if func.is_declaration() {
///         PreservedAnalyses::All
///     } else {
///         let _name = func.name();
///         PreservedAnalyses::PreserveCFG
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

/// LLVM `Module` 的借用视图，作为 [`ModulePass::run`] 的参数传入。
pub struct ModuleRef<'a>(Pin<&'a mut ffi::Module>);

impl<'a> ModuleRef<'a> {
    /// 返回模块的 LLVM 名称。
    #[inline]
    pub fn name(&self) -> &str {
        ffi::get_module_name(&self.0)
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
        // SAFETY: `self.0` 是对已 pin 对象的可变借用；这里只取出指针而不移动目标，
        // 调用方在借用有效期内使用该指针满足 Pin 的约束。
        unsafe { Pin::into_inner_unchecked(self.0.as_mut()) as *mut ffi::Module as *mut c_void }
    }
}

/// LLVM `Function` 的借用视图，作为 [`FunctionPass::run`] 的参数传入。
pub struct FunctionRef<'a>(Pin<&'a mut ffi::Function>);

impl<'a> FunctionRef<'a> {
    /// 返回函数的 LLVM 名称。
    #[inline]
    pub fn name(&self) -> &str {
        ffi::get_function_name(&self.0)
    }

    /// 判断该函数是否为声明（无函数体）。
    ///
    /// 外部声明通常没有可优化的指令，Pass 可视情况跳过。
    #[inline]
    pub fn is_declaration(&self) -> bool {
        ffi::is_function_declaration(&self.0)
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
        // SAFETY: `self.0` 是对已 pin 对象的可变借用；这里只取出指针而不移动目标，
        // 调用方在借用有效期内使用该指针满足 Pin 的约束。
        unsafe { Pin::into_inner_unchecked(self.0.as_mut()) as *mut ffi::Function as *mut c_void }
    }
}

/// `llvm::ModulePassManager` 的借用封装，用于挂载模块级 Pass。
///
/// 实例由 [`PassBuilder`] 的扩展点回调或管线解析回调提供。
pub struct ModulePassManager<'a>(Pin<&'a mut ffi::ModulePassManager>);

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
}

/// `llvm::FunctionPassManager` 的借用封装，用于挂载函数级 Pass。
///
/// 实例由 [`PassBuilder`] 的函数级扩展点回调或管线解析回调提供。
pub struct FunctionPassManager<'a>(Pin<&'a mut ffi::FunctionPassManager>);

impl<'a> FunctionPassManager<'a> {
    /// 向函数级管线追加一个 [`FunctionPass`]。
    pub fn add_pass<P: FunctionPass>(&mut self, pass: P) {
        ffi::fpm_add_function_pass(self.0.as_mut(), Box::new(BoxedFunctionPass(Box::new(pass))));
    }
}

/// `llvm::PassBuilder` 的借用封装，是注册 Pass 与扩展点回调的入口。
///
/// 通过 [`export_llvm_plugin!`] 生成的插件入口获得实例。所有回调都运行在 Rust 侧，
/// 其 panic 会被 FFI 边界捕获并终止宿主进程，因此不应依赖 panic 做控制流。
pub struct PassBuilder<'a>(Pin<&'a mut ffi::PassBuilder>);

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
pub struct BoxedModulePass(Box<dyn ModulePass>);
impl BoxedModulePass {
    fn run(&mut self, m: Pin<&mut ffi::Module>) -> PreservedAnalyses {
        match catch_unwind(AssertUnwindSafe(|| {
            let mut module_ref = ModuleRef(m);
            self.0.run(&mut module_ref)
        })) {
            Ok(pa) => pa,
            Err(payload) => handle_ffi_panic(self.0.name(), payload),
        }
    }

    fn name(&self) -> &str {
        self.0.name()
    }
}

/// `cxx` 桥接使用的函数级 Pass 包装类型，通常不直接构造。
pub struct BoxedFunctionPass(Box<dyn FunctionPass>);
impl BoxedFunctionPass {
    fn run(&mut self, f: Pin<&mut ffi::Function>) -> PreservedAnalyses {
        match catch_unwind(AssertUnwindSafe(|| {
            let mut func_ref = FunctionRef(f);
            self.0.run(&mut func_ref)
        })) {
            Ok(pa) => pa,
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
pub struct BoxedModuleEpCallback(Arc<ModuleEpCallbackFn>);
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
pub struct BoxedModuleLtoEpCallback(Arc<ModuleLtoEpCallbackFn>);
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
pub struct BoxedFunctionEpCallback(Arc<FunctionEpCallbackFn>);
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
pub struct BoxedModulePipelineParser(Arc<ModulePipelineParserFn>);
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
pub struct BoxedFunctionPipelineParser(Arc<FunctionPipelineParserFn>);
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
/// 宏会在调用处定义一个 `#[no_mangle] extern "C"` 函数，返回符合当前启用的
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
