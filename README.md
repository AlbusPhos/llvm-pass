# llvm-pass

> A framework for writing LLVM pass plugins in Rust via cxx FFI (LLVM 18-22)

[![CI](https://img.shields.io/github/actions/workflow/status/AlbusPhos/llvm-pass/ci.yml?label=CI&logo=githubactions&logoColor=white)](https://github.com/AlbusPhos/llvm-pass/actions/workflows/ci.yml)
[![MSRV](https://img.shields.io/badge/MSRV-1.88-blue)](Cargo.toml)
[![Type](https://img.shields.io/badge/Type-Library-informational)](#快速开始-quick-start)
[![License](https://img.shields.io/crates/l/llvm-pass)](#开源协议-license)

---

本项目是一个面向 LLVM 18 ~ 22 的 Rust 插件开发框架。它通过 [`cxx`](https://cxx.rs/) 在 Rust 与 LLVM C++ API 之间建立一层窄桥接，使开发者可以用安全的 Rust trait 和闭包编写 Module/Function Pass，并注册到 LLVM `PassBuilder` 的各类扩展点与自定义 `-passes=` 解析流程。库同时产出 `rlib` 与 `cdylib`，既可作为依赖被插件工程引用，也可直接编译为 `opt` / `clang` / `rustc` 可加载的 Pass 插件。

LLVM 版本通过 Cargo 特性选择（`llvm18` ~ `llvm22`），默认启用 `llvm22`。构建脚本会自动查找对应版本的 `llvm-config-{version}`，也可通过环境变量 `LLVM_CONFIG_PATH` 显式指定。此外，可通过可选的 `inkwell` 特性将 `ModuleRef` / `FunctionRef` 无缝转换为 [`inkwell`](https://github.com/TheDan64/inkwell) 的安全 IR 视图。

## 快速开始 (Quick Start)

### 作为库使用 (Library)

```toml
[dependencies]
# 默认启用 llvm22
llvm-pass = "0.1"
# 或显式选择其他 LLVM 版本（必须与宿主工具链一致）
# llvm-pass = { version = "0.1", default-features = false, features = ["llvm18"] }
# 可选：启用 inkwell 互操作层（LLVM 版本会自动匹配，并重导出 llvm_pass::inkwell）
# llvm-pass = { version = "0.1", features = ["inkwell"] }
# llvm-pass = { version = "0.1", default-features = false, features = ["llvm21", "inkwell"] }
```

插件工程需将自身声明为 `cdylib`：

```toml
[lib]
crate-type = ["cdylib"]
```

基本用法（示例代码可编译并通过 doctest 验证；完整可运行工程见 `tests/demo/plugin`）：

```rust
use llvm_pass::*;

struct MyModulePass;

impl ModulePass for MyModulePass {
    fn run(&mut self, module: &mut ModuleRef<'_>) -> PreservedAnalyses {
        eprintln!("visiting module: {}", module.name());
        PreservedAnalyses::All
    }
}

export_llvm_plugin! {
    name: "MyPlugin",
    version: "0.1.0",
    register: |pb: &mut PassBuilder| {
        pb.register_pipeline_start_ep_callback(|mpm, level| {
            eprintln!("pipeline start: {level:?}");
            mpm.add_pass(MyModulePass);
        });
        pb.register_module_pipeline_parsing_callback(|name, mpm| {
            if name == "my-pass" {
                mpm.add_pass(MyModulePass);
                true
            } else {
                false
            }
        });
    }
}
```

### 配合 `inkwell` 操作 IR（可选）

启用 `inkwell` 特性后，无需在 `Cargo.toml` 中额外引入 `inkwell`，即可通过 `with_inkwell` 或 `as_inkwell` 安全地读写 LLVM IR（底层自动使用 `ManuallyDrop` 保护宿主编译器的模块所有权）：

```rust
use llvm_pass::*;

#[cfg(feature = "inkwell")]
fn instrument_function(func: &mut FunctionRef<'_>) -> PreservedAnalyses {
    if func.is_declaration() {
        return PreservedAnalyses::All;
    }
    func.with_inkwell(|iw_fn, ctx| {
        if let Some(entry_bb) = iw_fn.get_first_basic_block() {
            let builder = ctx.create_builder();
            builder.position_at_end(entry_bb);
            // 使用 inkwell Builder 与重导出的 llvm_pass::inkwell 类型操作 IR...
        }
    });
    PreservedAnalyses::PreserveCFG
}
```

构建并加载插件：

```bash
cargo build --release
# 以下命令中的 <ver> 必须与启用的 llvm* 特性一致（默认 22）
# 通过 opt 自定义管线加载
opt-<ver> -load-pass-plugin=./target/release/libmy_plugin.so \
  -passes='my-pass' -disable-output input.ll
# 通过 clang 在默认管线扩展点加载
clang-<ver> -fpass-plugin=./target/release/libmy_plugin.so \
  -O2 -S -emit-llvm -o input.ll input.c
# 通过 rustc（nightly）加载；工具链内部 LLVM 版本必须与插件一致
rustc +nightly-2026-06-15 -Z llvm-plugins=./target/release/libmy_plugin.so \
  -C opt-level=2 --emit=llvm-ir src.rs
```

### 使用分析管理器 (Analysis Manager)

`ModuleRef` / `FunctionRef` 已内聚对应的 `ModuleAnalysisManager` / `FunctionAnalysisManager`，可直接惰性计算或读取缓存分析：

```rust
use llvm_pass::*;

fn inspect(module: &mut ModuleRef<'_>) -> PreservedAnalyses {
    // 模块级分析：惰性计算 CallGraph
    let _cg = module.call_graph();

    // 在模块 Pass 中遍历函数，并查询函数级分析与跨层级模块分析
    module.for_each_function(|func| {
        if !func.is_declaration() {
            let dt = func.dominator_tree();
            let li = func.loop_info();
            // GlobalsAA 是 LLVM 的"永不失效"分析，可从函数级跨层级读取
            let _gaa = func.cached_module_analysis::<GlobalsAA>();
            let _has_loops = !li.is_empty();
            assert!(!dt.as_raw_ptr().is_null());
        }
    });

    PreservedAnalyses::All
}
```

> 启用 `inkwell` 特性后，可用 `DominatorTree::dominates`、`LoopInfo::loop_depth` 等方法直接查询 `inkwell` 基本块，详见 `tests/demo/plugin`。

内置分析类型与便捷入口：

| 标记类型                    | 层级             | 便捷入口                           | 结果视图            |
|:----------------------------|:-----------------|:-----------------------------------|:--------------------|
| `DominatorTreeAnalysis`     | 函数             | `FunctionRef::dominator_tree`      | `DominatorTree`     |
| `PostDominatorTreeAnalysis` | 函数             | `FunctionRef::post_dominator_tree` | `PostDominatorTree` |
| `LoopAnalysis`              | 函数             | `FunctionRef::loop_info`           | `LoopInfo`          |
| `CallGraphAnalysis`         | 模块             | `ModuleRef::call_graph`            | `CallGraph`         |
| `GlobalsAA`                 | 模块（可跨层级） | `ModuleRef::globals_aa`            | `GlobalsAAResult`   |

- `analysis::<A>()` 惰性计算或复用缓存；`cached_analysis::<A>()` 只读取已缓存结果。
- `invalidate_analyses(pa)` 立即失效指定范围，借用检查器保证不会与存活的分析结果引用重叠。
- `PreservedAnalyses` 支持链式保留声明：

```rust
use llvm_pass::*;

let pa = PreservedAnalyses::PreserveCFG
    .preserve::<DominatorTreeAnalysis>()
    .preserve::<LoopAnalysis>();
assert!(pa.is_preserved::<LoopAnalysis>());
```

- `CallGraphAnalysis` 等可失效的模块级分析只能通过 `ModuleRef` 查询；跨层级入口 `FunctionRef::cached_module_analysis` 仅适用于 LLVM 的"永不失效"模块分析（内置实现为 `GlobalsAA`），否则 LLVM 调试构建会触发 `Cached result cannot be invalidated` 断言。

## LLVM 版本特性 (LLVM Version Features)

| Feature          | 对应 LLVM | `PassPluginLibraryInfo` API |
|:-----------------|:----------|:----------------------------|
| `llvm18`         | 18        | v1                          |
| `llvm19`         | 19        | v1                          |
| `llvm20`         | 20        | v1                          |
| `llvm21`         | 21        | v1                          |
| `llvm22`（默认） | 22        | v2                          |

- 每次构建必须且只能启用一个 `llvm*` 特性；同时启用多个或全部不启用都会得到明确的编译错误。
- 插件工程与其依赖必须选择相同的 LLVM 版本，否则无法被宿主加载。
- LLVM 18 / 19 的模块级扩展点回调不提供 LTO 阶段，此时 `LtoPhase` 恒为 `None`（表示信息不可用）。
- 可通过 `llvm_pass::LLVM_VERSION_MAJOR` 获取当前绑定的主版本号。
- 启用可选的 `inkwell` 特性时，所选 `llvm*` 特性会自动转发至 `inkwell`（启用对应的 `no-llvm-linking` 模式以避免符号冲突），并在根模块重导出 `llvm_pass::inkwell`；请勿在下游额外声明不同 LLVM 版本的 `inkwell` 依赖，以免 Cargo 特性合并后触发冲突。

```bash
# 默认 LLVM 22
cargo build --release
# 切换到 LLVM 18
cargo build --release --no-default-features --features llvm18
# 非标准安装路径
LLVM_CONFIG_PATH=/opt/llvm-19/bin/llvm-config \
  cargo build --release --no-default-features --features llvm19
# 启用 inkwell 时，llvm-sys 还会独立查找 LLVM。
# 非标准安装路径建议同时设置对应的 LLVM_SYS_*_PREFIX：
# LLVM 18 -> 181, 19 -> 191, 20 -> 201, 21 -> 211, 22 -> 221
LLVM_CONFIG_PATH=/opt/llvm-19/bin/llvm-config \
LLVM_SYS_191_PREFIX=/opt/llvm-19 \
  cargo build --release --no-default-features --features llvm19,inkwell
```

## API 预览 (API Overview)

### 核心组件速查

| 核心入口 / 类型                                  | 角色定位         | 职责说明                                                                             |
|:-------------------------------------------------|:-----------------|:-------------------------------------------------------------------------------------|
| `export_llvm_plugin!`                            | **插件入口**     | 生成 `llvmGetPassPluginInfo`，供 `opt` / `clang` 加载                                |
| `ModulePass` / `FunctionPass`                    | **扩展契约**     | 实现此 trait 定义自定义 Pass，返回的 `PreservedAnalyses` 决定分析失效范围            |
| `ModuleRef` / `FunctionRef`                      | **IR + AM 视图** | 读取模块/函数名与裸指针，内聚 `ModuleAnalysisManager` / `FunctionAnalysisManager`    |
| `ModuleRef::analysis` / `cached_analysis`        | **分析入口**     | 惰性计算或读取缓存的模块/函数级分析；`for_each_function` 支持模块内遍历函数          |
| `DominatorTree( Analysis )` / `LoopInfo`         | **函数级分析**   | 支配树、后支配树、循环信息及其基本块查询（配合 `inkwell` 使用）                      |
| `CallGraph( Analysis )` / `GlobalsAA`            | **模块级分析**   | 调用图（仅模块级）与永不失效的全局别名分析（支持从函数级跨层级查询）                 |
| `InkwellModule` / `as_inkwell` / `with_inkwell`  | **Inkwell 桥接** | 启用 `inkwell` 特性后以 `inkwell` API 安全读写 IR（同时重导出 `llvm_pass::inkwell`） |
| `PassBuilder`                                    | **注册中心**     | 注册 6 类管线扩展点与 Module/Function 级自定义 pass 名解析                           |
| `ModulePassManager` / `FunctionPassManager`      | **Pass 容器**    | 向对应层级管线挂载 Pass                                                              |
| `PreservedAnalyses` / `OptLevel` / `LtoPhase`    | **值类型**       | 保留集合（`All` / `PreserveCFG` / `None` / `preserve::<A>()`）、优化级别与 LTO 阶段  |
| `LLVM_VERSION_MAJOR` / `LLVM_PLUGIN_API_VERSION` | **版本信息**     | 当前绑定的 LLVM 主版本号与插件 ABI 版本                                              |
| `split_parametrized_pass_name`                   | **辅助函数**     | 解析 `name<key=value;...>` 形式的参数化 pass 名                                      |

## 贡献 (Contributing)

欢迎提交 Issue 报告问题或发起 Pull Request 贡献代码！

- 提交 PR 前请确保在本地通过以下检查：

```bash
cargo fmt --all -- --check
# 注意：LLVM 版本特性互斥，--all-features 会失败；请逐版本检查。
cargo clippy --all-targets --no-default-features --features llvm22 -- -D warnings
cargo test --no-default-features --features llvm22
# 同时验证可选的 inkwell 集成
cargo clippy --all-targets --no-default-features --features llvm22,inkwell -- -D warnings
cargo test --no-default-features --features llvm22,inkwell
# 安装了其他 LLVM 版本时，替换为 llvm18 / llvm19 / llvm20 / llvm21
```

## 变更日志 (Changelog)

版本格式：主版本号.次版本号.修订号（遵循 SemVer）。各版本的详细变更记录请查阅 [CHANGELOG.md](CHANGELOG.md)。

## 开源协议 (License)

本项目基于 [Mozilla Public License 2.0](./LICENSE) 协议开源。除非另有明确声明，您提交的任何贡献默认按同等协议条款授权。
