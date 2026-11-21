// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::HashSet;
use std::env;
use std::path::PathBuf;
use std::process::Command;

/// 读取 Cargo 传入的 LLVM 版本特性，并校验恰好启用一个。
fn selected_llvm_version() -> u32 {
    let candidates = [
        (18, env::var_os("CARGO_FEATURE_LLVM18").is_some()),
        (19, env::var_os("CARGO_FEATURE_LLVM19").is_some()),
        (20, env::var_os("CARGO_FEATURE_LLVM20").is_some()),
        (21, env::var_os("CARGO_FEATURE_LLVM21").is_some()),
        (22, env::var_os("CARGO_FEATURE_LLVM22").is_some()),
    ];

    let enabled: Vec<u32> = candidates
        .into_iter()
        .filter_map(|(ver, on)| on.then_some(ver))
        .collect();

    match enabled.as_slice() {
        [ver] => *ver,
        [] => panic!(
            "未启用任何 LLVM 版本特性。请从 llvm18、llvm19、llvm20、llvm21、llvm22 中选择一个。"
        ),
        _ => {
            panic!("同时启用了多个 LLVM 版本特性 ({enabled:?})。每次构建只能启用一个 llvm* 特性。")
        }
    }
}

/// 解析 `llvm-config --version` 输出的主版本号，兼容 `18.1.8` 与
/// `LLVM version 18.1.8` 两种形式。
fn parse_major_version(version: &str) -> Option<u32> {
    let version = version.trim();
    let version = version.strip_prefix("LLVM version ").unwrap_or(version);
    version.split('.').next()?.parse().ok()
}

fn llvm_config_version(bin: &PathBuf) -> Option<u32> {
    let output = Command::new(bin).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_major_version(&String::from_utf8_lossy(&output.stdout))
}

/// 查找与目标主版本匹配的 `llvm-config` 可执行文件。
fn find_llvm_config(major_ver: u32) -> PathBuf {
    println!("cargo:rerun-if-env-changed=LLVM_CONFIG_PATH");

    if let Some(custom_path) = env::var_os("LLVM_CONFIG_PATH") {
        let bin = PathBuf::from(custom_path);
        match llvm_config_version(&bin) {
            Some(ver) if ver == major_ver => return bin,
            Some(ver) => panic!(
                "LLVM_CONFIG_PATH={:?} 指向 LLVM {ver}，与启用的 llvm{major_ver} 特性不匹配。",
                bin
            ),
            None => panic!(
                "无法执行 LLVM_CONFIG_PATH={:?} 或 `--version` 调用失败。",
                bin
            ),
        }
    }

    let candidates = [
        PathBuf::from(format!("llvm-config-{major_ver}")),
        PathBuf::from(format!(
            "/opt/homebrew/opt/llvm@{major_ver}/bin/llvm-config"
        )),
        PathBuf::from(format!("/usr/local/opt/llvm@{major_ver}/bin/llvm-config")),
        PathBuf::from("llvm-config"),
    ];

    for bin in candidates {
        match llvm_config_version(&bin) {
            Some(ver) if ver == major_ver => return bin,
            Some(ver) => println!(
                "cargo:warning=跳过 {:?}：版本为 {ver}，期望 {major_ver}.x",
                bin
            ),
            None => {}
        }
    }

    panic!(
        "未找到可用的 LLVM {major_ver} `llvm-config`。已尝试 `llvm-config-{major_ver}`、\
         Homebrew 常见路径与 `llvm-config`。可通过环境变量 `LLVM_CONFIG_PATH` 显式指定。"
    );
}

fn run_llvm_config(bin: &PathBuf, arg: &str) -> String {
    let output = Command::new(bin)
        .arg(arg)
        .output()
        .unwrap_or_else(|e| panic!("failed to run `{:?} {}`: {e}", bin, arg));
    assert!(
        output.status.success(),
        "`{:?} {}` failed with status {}",
        bin,
        arg,
        output.status
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn main() {
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cxx/shim.cpp");
    println!("cargo:rerun-if-changed=cxx/shim.h");

    #[cfg(target_os = "macos")]
    println!("cargo:rustc-cdylib-link-arg=-Wl,-undefined,dynamic_lookup");
    #[cfg(target_os = "linux")]
    println!("cargo:rustc-cdylib-link-arg=-Wl,--unresolved-symbols=ignore-all");

    let llvm_ver = selected_llvm_version();
    let llvm_config = find_llvm_config(llvm_ver);
    let libdir = run_llvm_config(&llvm_config, "--libdir");

    // 集成测试会直接调用 LLVM C API（IR 扩展测试）。库本身保持
    // `no-llvm-linking`，仅给测试可执行文件链接共享 libLLVM，避免把
    // LLVM 符号强行链接进插件 cdylib。该步骤是 best-effort：某些 LLVM
    // 发行版只提供静态库，不应因此影响正常库构建。
    if env::var("CARGO_CFG_TARGET_FAMILY").as_deref() == Ok("unix") {
        println!("cargo:rustc-link-arg-tests=-L{libdir}");
        println!("cargo:rustc-link-arg-tests=-Wl,-rpath,{libdir}");
        match Command::new(&llvm_config)
            .args(["--link-shared", "--libs", "core"])
            .output()
        {
            Ok(output) if output.status.success() => {
                for flag in String::from_utf8_lossy(&output.stdout).split_whitespace() {
                    println!("cargo:rustc-link-arg-tests={flag}");
                }
            }
            Ok(_) => println!(
                "cargo:warning=`{:?} --link-shared --libs` failed; IR extension integration tests may need manual LLVM linking",
                llvm_config
            ),
            Err(error) => {
                println!("cargo:warning=failed to query LLVM shared libraries for tests: {error}")
            }
        }
    }

    let includedir = run_llvm_config(&llvm_config, "--includedir");
    let cflags = run_llvm_config(&llvm_config, "--cxxflags");

    let mut builder = cxx_build::bridge("src/lib.rs");
    builder.file("cxx/shim.cpp").include(".").include("cxx");

    // 显式使用 `--includedir`，覆盖不同发行版/Homebrew 的头文件布局差异。
    let mut added_includes = HashSet::new();
    if !includedir.is_empty() {
        builder.flag("-isystem").flag(&includedir);
        added_includes.insert(includedir);
    }

    // 解析 `--cxxflags`：include 路径统一转为 `-isystem`，其余标志原样保留。
    let mut iter = cflags.split_whitespace().peekable();
    while let Some(flag) = iter.next() {
        if flag == "-I" || flag == "-isystem" {
            if let Some(path) = iter.next()
                && added_includes.insert(path.to_string())
            {
                builder.flag("-isystem").flag(path);
            }
        } else if let Some(path) = flag.strip_prefix("-I") {
            if added_includes.insert(path.to_string()) {
                builder.flag("-isystem").flag(path);
            }
        } else if let Some(path) = flag.strip_prefix("-isystem") {
            if !path.is_empty() && added_includes.insert(path.to_string()) {
                builder.flag("-isystem").flag(path);
            }
        } else {
            builder.flag(flag);
        }
    }

    // 向 C++ 侧注入预期主版本号，供 shim.cpp 做编译期一致性校验。
    builder.define(
        "LLVM_PASS_EXPECTED_LLVM_MAJOR",
        Some(llvm_ver.to_string().as_str()),
    );

    builder.compile("llvm_pass");
}
