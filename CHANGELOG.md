# Changelog

本文件记录本项目所有值得关注的变更。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [语义化版本 2.0.0](https://semver.org/lang/zh-CN/)。

<!-- 仅 version < 1.0.0 保留本段；1.x 项目删除。 -->
> **版本号说明（0.x 阶段）**：在 1.0.0 发布之前，次版本号（`0.MINOR.0`）的变更
> 也可能包含破坏性改动，请留意标注为 **[BREAKING]** 的条目。

---

## [0.3.0] - 2026-11-21

### Added

- 重导出 `llvm_sys`（启用 `inkwell` 时），下游无需额外声明 `llvm-sys` 依赖即可使用匹配的 LLVM C API。
- 新增 `ir_utils` 与 `inkwell_ext` 模块：`ir_utils` 提供必须复用 LLVM C++ 实现的裸指针辅助函数；`inkwell_ext` 提供 `AnyValueExt`、`InstructionExt`、`BasicBlockExt`、`GlobalValueExt`、`FunctionExt`、`ModuleExt` 等扩展 Trait。
- 新增原生 LLVM Pass 挂载接口：`ModulePassManager::add_adce`、`add_global_dce`，以及 `FunctionPassManager::add_adce`、`add_simplify_cfg`、`add_promote`。
- 新增 `tests/test_inkwell_ext.rs`，覆盖 Call/Invoke/CallBr 操作数布局、LLVM 22 switch case、token、operand bundle、内联汇编、GEP、DataLayout、PHI 降级等跨版本行为。

### Fixed

- 修正 `CallBase` 操作数分类：`InvokeInst` / `CallBrInst` 的 normal/unwind/default/indirect 目标不是 bundle 操作数。
- 统一封装 LLVM 18–22 的 `SwitchInst` case value 差异，避免 LLVM 22 上按旧操作数布局读取 case value。

## [0.2.0] - 2026-11-20

### Added

- `ModuleRef` / `FunctionRef` 内聚 `ModuleAnalysisManager` / `FunctionAnalysisManager`，新增 `analysis` / `cached_analysis`、`ModuleRef::for_each_function`（经 FAM 代理遍历函数）以及 `invalidate_analyses`。
- 新增内置分析标记：`DominatorTreeAnalysis`、`PostDominatorTreeAnalysis`、`LoopAnalysis`、`CallGraphAnalysis`、`GlobalsAA`，以及对应结果视图 `DominatorTree` / `PostDominatorTree` / `LoopInfo` / `CallGraph` / `GlobalsAAResult`。
- 新增 `ModuleRef::call_graph` / `globals_aa` 与 `FunctionRef::dominator_tree` / `post_dominator_tree` / `loop_info` 便捷入口。
- 新增跨层级缓存查询 `FunctionRef::cached_module_analysis`（仅支持永不失效的模块级分析，内置实现为 `GlobalsAA`）。
- `PreservedAnalyses` 新增 `preserve::<A>()`、`preserve_function_analyses_proxy()`、`is_all` / `is_none` / `is_preserved::<A>()` 链式 API。
- 演示插件与集成测试覆盖模块/函数级分析的惰性计算、缓存与失效，跨层级 `GlobalsAA` 查询，以及配合 `inkwell` 的基本块查询。
- 新增 GitHub Actions CI：按 LLVM 18 ~ 22 矩阵运行 clippy 与测试（含 `inkwell` 组合）、MSRV 1.88 检查，以及 docs.rs 配置模拟。
- CI 增加 `cargo hack` 特性组合检查：每个 LLVM 版本验证基础特性与 `inkwell` 组合均可编译。
- 新增基于 `v*.*.*` tag 的发布工作流：校验 tag 与 `Cargo.toml` 版本一致后运行测试、发布 crates.io 并创建 GitHub Release。

### Changed

- [BREAKING] `PreservedAnalyses` 由 `ffi::PreservedAnalysesKind` 枚举别名改为位掩码结构体；`All` / `PreserveCFG` / `None` 常量用法保持兼容，`ffi::PreservedAnalysesKind` 不再导出。
- `Cargo.toml` 声明 `rust-version = "1.88"`，与 CI 的 MSRV 检查保持一致。
- `ModuleRef::as_inkwell` / `with_inkwell` 与 `FunctionRef::as_inkwell` / `with_inkwell` 由 `&mut self` 改为 `&self`，便于与共享借用的分析结果混用；原调用写法无需修改。
- 内部实现按职责拆分为 `analysis`、`ir`、`pass` 子模块，`#[cxx::bridge]`、FFI 胶水与插件入口保留在 crate 根；公开类型仍从 crate 根扁平导出，无 API 变更。
- README 同步补充分析管理器使用方式与 API 预览。

---

## [0.1.2] - 2026-11-20

### Added

- 新增可选 `inkwell` 特性（默认不启用）：启用后重导出与当前 LLVM 版本匹配的 `inkwell`，并提供 `InkwellModule`、`ModuleRef::as_inkwell` / `with_inkwell`、`FunctionRef::as_inkwell` / `with_inkwell`，可在 Pass 中以 `inkwell` API 读写 IR。
- 新增 `inkwell` 互操作集成测试：通过 `opt` 加载 demo 插件，验证模块/函数级 IR 读取与注入（global 与 add 指令）。

### Changed

- `llvm18` ~ `llvm22` 特性会将所选 LLVM 版本转发给可选的 `inkwell` 依赖；启用 `inkwell` 时使用 `no-llvm-linking` / `no-libffi-linking`，由宿主提供 LLVM 符号。
- docs.rs 文档构建配置扩展为 `llvm21,inkwell`。

---

## [0.1.1] - 2026-11-19

### Added

- 在 crate 根启用 `#![cfg_attr(docsrs, feature(doc_cfg))]`，为 docs.rs 文档构建启用 feature 标注支持。

### Changed

- 调整 docs.rs 构建配置：关闭默认特性并固定使用 `llvm21` 特性构建文档，避免默认的 `llvm22` 在文档构建环境不可用。
- README 的依赖示例改用 crates.io 版本依赖 `llvm-pass = "0.1"`，不再引导用户使用 git 依赖。

---

## [0.1.0] - 2026-11-19

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

