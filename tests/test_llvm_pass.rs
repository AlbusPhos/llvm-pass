// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use llvm_pass::{
    CallGraphAnalysis, DominatorTreeAnalysis, LLVM_VERSION_MAJOR, LoopAnalysis,
    PostDominatorTreeAnalysis, PreservedAnalyses, split_parametrized_pass_name,
};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// LLVM 主版本 → 可加载对应插件的 rustc 工具链。
const RUSTC_TOOLCHAINS: &[(&str, &str)] = &[
    ("18", "1.79.0"),
    ("19", "1.82.0"),
    ("20", "1.88.0"),
    ("21", "1.92"),
    ("22", "1.98.0"),
];

/// 选择并验证当前 LLVM 版本对应的 rustc 工具链。
///
fn resolve_rustc_toolchain() -> Option<String> {
    let toolchain = env::var("LLVM_PASS_RUSTC_TOOLCHAIN").ok().or_else(|| {
        RUSTC_TOOLCHAINS
            .iter()
            .find(|(llvm, _)| llvm.parse::<u32>() == Ok(LLVM_VERSION_MAJOR))
            .map(|(_, toolchain)| (*toolchain).to_string())
    })?;

    let output = Command::new("rustc")
        .arg(format!("+{toolchain}"))
        .arg("-V")
        .arg("-v")
        .output()
        .ok()?;

    if !output.status.success() {
        eprintln!("rustc toolchain `{toolchain}` is not installed");
        return None;
    }

    let version_info = String::from_utf8_lossy(&output.stdout);
    let expected = format!("LLVM version: {LLVM_VERSION_MAJOR}.");
    if !version_info.contains(&expected) {
        eprintln!(
            "rustc toolchain `{toolchain}` does not match LLVM {LLVM_VERSION_MAJOR}.x:\n{version_info}"
        );
        return None;
    }

    let help = Command::new("rustc")
        .arg(format!("+{toolchain}"))
        .arg("-Z")
        .arg("help")
        .env("RUSTC_BOOTSTRAP", "1")
        .output()
        .ok()?;
    let help_text = format!(
        "{}{}",
        String::from_utf8_lossy(&help.stdout),
        String::from_utf8_lossy(&help.stderr)
    );
    if !help.status.success() || !help_text.contains("llvm-plugins") {
        eprintln!("rustc toolchain `{toolchain}` does not support `-Z llvm-plugins`");
        return None;
    }

    Some(toolchain)
}

#[test]
fn test_split_parametrized_pass_name() {
    assert_eq!(split_parametrized_pass_name("bcf"), Some(("bcf", None)));
    assert_eq!(
        split_parametrized_pass_name("bcf<prob=80;loop=3>"),
        Some(("bcf", Some("prob=80;loop=3")))
    );
    assert_eq!(
        split_parametrized_pass_name("fla<>"),
        Some(("fla", Some("")))
    );

    assert_eq!(split_parametrized_pass_name(""), None);
    assert_eq!(split_parametrized_pass_name("<prob=80>"), None);
    assert_eq!(split_parametrized_pass_name("bcf<prob=80"), None);
    assert_eq!(split_parametrized_pass_name("bcf>prob=80"), None);
}

#[test]
fn test_preserved_analyses_masks_and_chaining() {
    // 内置常量语义
    assert!(PreservedAnalyses::All.is_all());
    assert!(!PreservedAnalyses::All.is_none());
    assert!(PreservedAnalyses::None.is_none());
    assert!(PreservedAnalyses::PreserveCFG.is_preserved::<DominatorTreeAnalysis>());
    assert!(PreservedAnalyses::PreserveCFG.is_preserved::<PostDominatorTreeAnalysis>());
    assert!(PreservedAnalyses::PreserveCFG.is_preserved::<LoopAnalysis>());
    assert!(!PreservedAnalyses::PreserveCFG.is_preserved::<CallGraphAnalysis>());
    assert!(!PreservedAnalyses::None.is_preserved::<DominatorTreeAnalysis>());

    // 链式声明只影响指定分析
    let pa = PreservedAnalyses::None
        .preserve::<CallGraphAnalysis>()
        .preserve_function_analyses_proxy();
    assert!(pa.is_preserved::<CallGraphAnalysis>());
    assert!(!pa.is_preserved::<DominatorTreeAnalysis>());
    assert!(!pa.is_all());
    assert!(!pa.is_none());

    // All 不受链式声明影响
    assert_eq!(
        PreservedAnalyses::All.preserve::<CallGraphAnalysis>(),
        PreservedAnalyses::All
    );
    assert_eq!(
        PreservedAnalyses::All.preserve_function_analyses_proxy(),
        PreservedAnalyses::All
    );
}

static PLUGIN_DYLIB_PATH: OnceLock<PathBuf> = OnceLock::new();

fn get_plugin_dylib() -> &'static Path {
    PLUGIN_DYLIB_PATH.get_or_init(|| {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let plugin_dir = manifest_dir.join("tests/demo/plugin");
        // 按 LLVM 版本隔离构建目录，避免切换 feature 时复用旧版本缓存。
        let target_dir =
            manifest_dir.join(format!("target/test-plugin-build-llvm{LLVM_VERSION_MAJOR}"));
        let mut features = format!("llvm{LLVM_VERSION_MAJOR}");
        if cfg!(feature = "inkwell") {
            features.push_str(",inkwell");
        }
        let status = Command::new(env!("CARGO"))
            .arg("build")
            .arg("--manifest-path")
            .arg(plugin_dir.join("Cargo.toml"))
            .arg("--no-default-features")
            .arg("--features")
            .arg(&features)
            .arg("--target-dir")
            .arg(&target_dir)
            .status()
            .expect("Failed to invoke cargo to build demo_plugin");
        assert!(status.success(), "Failed to compile tests/demo/plugin");
        let dylib_name = format!(
            "{}demo_plugin.{}",
            env::consts::DLL_PREFIX,
            env::consts::DLL_EXTENSION
        );
        let dylib_path = target_dir.join("debug").join(dylib_name);
        assert!(
            dylib_path.exists(),
            "Plugin dylib not found at {:?}",
            dylib_path
        );
        dylib_path
    })
}

fn llvm_tool(base_tool: &str) -> PathBuf {
    let versioned_tool = format!("{}-{}", base_tool, LLVM_VERSION_MAJOR);
    PathBuf::from(versioned_tool)
}

fn demo_src(file_name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/demo/src")
        .join(file_name)
}

#[test]
fn test_opt_custom_pipeline_and_params() {
    let plugin = get_plugin_dylib();
    let ir_file = demo_src("c.ll");
    let opt_bin = llvm_tool("opt");

    let output = Command::new(&opt_bin)
        .arg(format!("-load-pass-plugin={}", plugin.display()))
        .arg("-passes=demo-mod,demo-mod-closure,function(demo-fn<prob=80;loop=3>,demo-fn)")
        .arg("-disable-output")
        .arg(&ir_file)
        .output()
        .unwrap_or_else(|e| panic!("Failed to execute {:?}: {}", opt_bin, e));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "`opt-{LLVM_VERSION_MAJOR}` failed:\n{}",
        stderr
    );

    assert!(stderr.contains("[TRACE] ModulePass::run on module="));
    assert!(stderr.contains("[TRACE] ClosureModulePass::run module="));

    assert!(!stderr.contains("FunctionPass::run func=external_decl"));

    assert!(stderr.contains("[TRACE] FunctionPass::run func=target_func_a params=prob=80;loop=3"));
    assert!(stderr.contains("[TRACE] FunctionPass::run func=target_func_b params=prob=80;loop=3"));
    assert!(stderr.contains("[TRACE] FunctionPass::run func=target_func_a params=default"));
    assert!(stderr.contains("[TRACE] FunctionPass::run func=target_func_b params=default"));
}

#[test]
fn test_analysis_invalidation() {
    let plugin = get_plugin_dylib();
    let ir_file = demo_src("c.ll");
    let opt_bin = llvm_tool("opt");

    let output = Command::new(&opt_bin)
        .arg(format!("-load-pass-plugin={}", plugin.display()))
        .arg("-passes=demo-invalidate,function(demo-invalidate-fn)")
        .arg("-disable-output")
        .arg(&ir_file)
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "Failed to execute {:?} for invalidation test: {}",
                opt_bin, e
            )
        });

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "`opt-{LLVM_VERSION_MAJOR}` invalidation pipeline failed:\n{}",
        stderr
    );
    assert!(stderr.contains("[TRACE] ModuleAnalysisInvalidation::run module="));
    assert!(stderr.contains("[TRACE] FunctionAnalysisInvalidation::run func=target_func_a"));
    assert!(stderr.contains("[TRACE] FunctionAnalysisInvalidation::run func=target_func_b"));
}

#[test]
fn test_opt_default_o2_extension_points() {
    let plugin = get_plugin_dylib();
    let ir_file = demo_src("c.ll");
    let opt_bin = llvm_tool("opt");

    let output = Command::new(&opt_bin)
        .arg(format!("-load-pass-plugin={}", plugin.display()))
        .arg("-passes=default<O2>")
        .arg("-disable-output")
        .arg(&ir_file)
        .output()
        .unwrap_or_else(|e| panic!("Failed to execute {:?} with default<O2>: {}", opt_bin, e));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "`opt-{LLVM_VERSION_MAJOR} -passes=default<O2>` failed:\n{}",
        stderr
    );

    assert!(stderr.contains("[TRACE] EP::PipelineStart level=O2"));
    assert!(stderr.contains("[TRACE] EP::EarlySimplification level=O2 phase=None"));
    assert!(stderr.contains("[TRACE] EP::OptimizerEarly level=O2 phase=None"));
    assert!(stderr.contains("[TRACE] EP::OptimizerLast level=O2 phase=None"));
    assert!(stderr.contains("[TRACE] EP::Peephole level=O2"));
    assert!(stderr.contains("[TRACE] EP::ScalarOptimizerLate level=O2"));

    assert!(stderr.contains("[TRACE] ClosureFunctionPass::run func=target_func_a"));
}

#[test]
fn test_ffi_panic_boundary_aborts_safely() {
    let plugin = get_plugin_dylib();
    let ir_file = demo_src("c.ll");
    let opt_bin = llvm_tool("opt");

    let output = Command::new(&opt_bin)
        .arg(format!("-load-pass-plugin={}", plugin.display()))
        .arg("-passes=demo-panic")
        .arg("-disable-output")
        .arg(&ir_file)
        .output()
        .unwrap_or_else(|e| panic!("Failed to execute {:?} for panic test: {}", opt_bin, e));

    assert!(
        !output.status.success(),
        "Process should have aborted on panic, but exited successfully"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[llvm-pass Fatal] Panic caught across FFI boundary"),
        "Missing fatal panic guard banner in stderr:\n{}",
        stderr
    );
    assert!(
        stderr.contains("simulated panic inside Rust pass"),
        "Missing panic payload message in stderr:\n{}",
        stderr
    );
}

#[test]
fn test_clang_c_and_cpp_with_lto_phases() {
    let plugin = get_plugin_dylib();
    let c_file = demo_src("a.c");
    let cpp_file = demo_src("b.cpp");
    let clang_bin = llvm_tool("clang");
    let clangxx_bin = llvm_tool("clang++");

    let out_c = Command::new(&clang_bin)
        .arg(format!("-fpass-plugin={}", plugin.display()))
        .arg("-O1")
        .arg("-emit-llvm")
        .arg("-S")
        .arg("-o")
        .arg("-")
        .arg(&c_file)
        .output()
        .unwrap_or_else(|e| panic!("Failed to execute {:?}: {}", clang_bin, e));

    let stderr_c = String::from_utf8_lossy(&out_c.stderr);
    assert!(
        out_c.status.success(),
        "`clang-{LLVM_VERSION_MAJOR}` failed:\n{}",
        stderr_c
    );
    assert!(stderr_c.contains("[TRACE] EP::PipelineStart level=O1"));
    assert!(stderr_c.contains("[TRACE] ClosureFunctionPass::run func=compute_c"));

    let out_cpp = Command::new(&clangxx_bin)
        .arg(format!("-fpass-plugin={}", plugin.display()))
        .arg("-Oz")
        .arg("-flto=thin")
        .arg("-c")
        .arg("-o")
        .arg(env::temp_dir().join(format!("b_thinlto_llvm{LLVM_VERSION_MAJOR}.bc")))
        .arg(&cpp_file)
        .output()
        .unwrap_or_else(|e| panic!("Failed to execute {:?}: {}", clangxx_bin, e));

    let stderr_cpp = String::from_utf8_lossy(&out_cpp.stderr);
    assert!(
        out_cpp.status.success(),
        "`clang++-{LLVM_VERSION_MAJOR}` failed:\n{}",
        stderr_cpp
    );
    // LLVM 18 / 19 的扩展点回调不携带 LTO 阶段，框架统一传 None。
    if LLVM_VERSION_MAJOR >= 20 {
        assert!(stderr_cpp.contains("[TRACE] EP::OptimizerEarly level=Oz phase=ThinLTOPreLink"));
    } else {
        assert!(stderr_cpp.contains("[TRACE] EP::OptimizerEarly level=Oz phase=None"));
    }
    assert!(stderr_cpp.contains("[TRACE] ClosureFunctionPass::run func=compute_cpp"));
}

#[test]
fn test_rustc_llvm_plugin_integration() {
    let Some(toolchain) = resolve_rustc_toolchain() else {
        eprintln!(
            "Skipping rustc integration test for LLVM {LLVM_VERSION_MAJOR}: \
             no usable toolchain (install one or set LLVM_PASS_RUSTC_TOOLCHAIN)"
        );
        return;
    };

    let plugin = get_plugin_dylib();
    let rs_file = demo_src("d.rs");
    let out_dir = env::temp_dir();

    let output = Command::new("rustc")
        .arg(format!("+{}", toolchain))
        .env("RUSTC_BOOTSTRAP", "1")
        .arg("-Z")
        .arg(format!("llvm-plugins={}", plugin.display()))
        .arg("-C")
        .arg("opt-level=2")
        .arg("--emit=llvm-ir")
        .arg("--out-dir")
        .arg(&out_dir)
        .arg(&rs_file)
        .output()
        .expect("Failed to execute `rustc`");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "`rustc` failed:\n{}", stderr);
    assert!(stderr.contains("[TRACE] EP::PipelineStart level=O2"));
    assert!(stderr.contains("[TRACE] ClosureFunctionPass::run func=compute_rust"));
}

#[cfg(feature = "inkwell")]
#[test]
fn test_opt_inkwell_ir_inspection_and_mutation() {
    let plugin = get_plugin_dylib();
    let ir_file = demo_src("c.ll");
    let opt_bin = llvm_tool("opt");

    let output = Command::new(&opt_bin)
        .arg(format!("-load-pass-plugin={}", plugin.display()))
        .arg("-passes=demo-inkwell-mod,function(demo-inkwell-fn)")
        .arg("-S")
        .arg(&ir_file)
        .output()
        .unwrap_or_else(|e| panic!("Failed to execute {:?} for inkwell test: {}", opt_bin, e));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "`opt-{LLVM_VERSION_MAJOR}` inkwell pipeline failed:\n{}",
        stderr
    );

    // 1. 验证 stderr 中的 Inkwell IR 读取统计信息（c.ll 中共有 3 个函数：1 个声明 + 2 个定义）
    assert!(stderr.contains("[TRACE] InkwellModule::as_inkwell fn_count=3"));
    assert!(stderr.contains("[TRACE] InkwellFunction::as_inkwell func=target_func_a bb_count=1"));
    assert!(stderr.contains("[TRACE] InkwellFunction::as_inkwell func=target_func_b bb_count=1"));

    // 2. 验证 stdout 输出的 LLVM IR 文本中包含 Inkwell 注入的全局变量与指令
    assert!(
        stdout.contains("@__inkwell_injected_global = internal global i32 1337"),
        "Missing injected global variable in output IR:\n{}",
        stdout
    );
    assert!(
        stdout.contains("%inkwell_injected_add = add i32 %x, 999"),
        "Missing injected add instruction in target_func_a:\n{}",
        stdout
    );
    assert!(
        stdout.contains("%inkwell_injected_add = add i32 %y, 999"),
        "Missing injected add instruction in target_func_b:\n{}",
        stdout
    );
}
