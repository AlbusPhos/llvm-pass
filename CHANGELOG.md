# Changelog

本文件记录本项目所有值得关注的变更。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [语义化版本 2.0.0](https://semver.org/lang/zh-CN/)。

<!-- 仅 version < 1.0.0 保留本段；1.x 项目删除。 -->
> **版本号说明（0.x 阶段）**：在 1.0.0 发布之前，次版本号（`0.MINOR.0`）的变更
> 也可能包含破坏性改动，请留意标注为 **[BREAKING]** 的条目。

---

## [Unreleased]

### Added

- 完成初始框架实现：通过 `cxx` 在 Rust 与 LLVM C++ Pass API 之间建立桥接，可用 Rust trait 或闭包编写 Module / Function Pass，并产出可供 `opt`、`clang`、`rustc` 加载的 `cdylib` 插件（同时保留 `rlib` 供库方式复用）。
- 支持 LLVM 18 ~ 22，对应 Cargo 特性 `llvm18` ~ `llvm22`（默认 `llvm22`）；编译期强制恰好启用一个版本特性，未启用或启用多个时给出明确错误。
- 提供 `ModulePass` / `FunctionPass` trait，并为满足 `FnMut(...) -> PreservedAnalyses + Send + Sync + 'static` 的闭包自动实现，可直接向 PassManager 传入闭包。
- 提供 `ModuleRef` / `FunctionRef` IR 视图，支持 `name()`、`is_declaration()`（函数视图）以及 `as_raw_ptr()` 裸指针访问。
- `ModulePassManager` 支持 `add_pass`、`add_function_pass`（经 `ModuleToFunctionPassAdaptor`）与 `add_function_pass_eagerly_invalidate`；`FunctionPassManager` 支持 `add_pass`。
- `PassBuilder` 支持注册 6 类管线扩展点回调：PipelineStart、PipelineEarlySimplification、OptimizerEarly、OptimizerLast、Peephole、ScalarOptimizerLate。
- 支持 Module / Function 级 `-passes=` 名称解析回调，可注册自定义 pass 名并解析 `name<key=value;...>` 形式的参数化名称。
- 新增 `split_parametrized_pass_name` 辅助函数，区分无参数、空参数（如 `fla<>`）与非法输入。
- 提供 `OptLevel`、`LtoPhase`、`PreservedAnalyses` 值类型：优化级别完整映射 O0 ~ Oz，分析保留范围支持 All / PreserveCFG / None。
- LLVM 20 及以上向扩展点回调传递真实 LTO 阶段；LLVM 18 / 19 统一传入 `LtoPhase::None`，表示该信息不可用。
- 新增 `export_llvm_plugin!` 宏，生成 `llvmGetPassPluginInfo` 入口并自动填充插件名称、版本、插件 API 版本与 PassBuilder 注册回调。
- 导出 `LLVM_VERSION_MAJOR` 与 `LLVM_PLUGIN_API_VERSION` 常量，并适配 LLVM 22 的 API v2 结构（新增 `pre_code_gen_callback` 字段）。
- 构建脚本自动查找与所选特性匹配的 `llvm-config-{version}`、Homebrew 常见路径或 `llvm-config`，支持通过 `LLVM_CONFIG_PATH` 显式指定，并在版本不匹配时给出明确错误。
- 构建脚本解析 `llvm-config --includedir` 与 `--cxxflags`，将 include 路径统一转为 `-isystem` 并去重，兼容不同发行版与 Homebrew 的头文件布局。
- C++ 侧通过编译期断言校验 LLVM 头文件主版本、插件 API 版本与 `PassPluginLibraryInfo` 字段布局，并适配 LLVM 22 的 `llvm/Plugins/PassPlugin.h` 头文件迁移。
- FFI 边界 panic 防护：捕获 Pass 与回调中的 Rust panic，输出 `[llvm-pass Fatal]` 诊断信息后安全终止宿主进程，避免跨语言栈展开。
- C++ 侧回调包装通过 `Arc` 与 `clone_cb` 实现可复制回调，满足 LLVM 对回调对象的复制要求。
- 集成测试覆盖 `opt` 自定义管线与参数化 pass、`default<O2>` 扩展点、panic 边界、`clang` / `clang++`（含 ThinLTO 阶段）以及 `rustc -Z llvm-plugins`。
- 提供 `tests/demo/plugin` 示例插件及 C、C++、LLVM IR、Rust 测试输入；rustc 集成测试按 LLVM 版本选择工具链，缺失或版本不匹配时自动跳过，并支持 `LLVM_PASS_RUSTC_TOOLCHAIN` 覆盖。
- rustdoc 示例覆盖 trait 实现、闭包 Pass 与插件导出；README 经 `#![doc = include_str!("../README.md")]` 纳入 crate 文档并由 doctest 验证。

