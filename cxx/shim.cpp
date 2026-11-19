// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#include "shim.h"
#include "llvm-pass/src/lib.rs.h"

#include "llvm/Config/llvm-config.h"

// `PassPlugin.h` 在 LLVM 22 才迁移到 `llvm/Plugins/` 目录
#if LLVM_VERSION_MAJOR >= 22
#include "llvm/Plugins/PassPlugin.h"
#else
#include "llvm/Passes/PassPlugin.h"
#endif

using namespace llvm;

namespace llvm_pass {

// 校验 C++ 头文件版本与 Cargo 特性选择的版本一致。
#ifdef LLVM_PASS_EXPECTED_LLVM_MAJOR
static_assert(
    LLVM_VERSION_MAJOR == LLVM_PASS_EXPECTED_LLVM_MAJOR,
    "LLVM header version (LLVM_VERSION_MAJOR) does not match the enabled Cargo feature!");
#endif

// 校验 LLVM 18 ~ 22 的 PassPluginLibraryInfo 布局与 API 版本预期一致。
#if LLVM_VERSION_MAJOR >= 22
static_assert(LLVM_PLUGIN_API_VERSION == 2,
              "Expected LLVM_PLUGIN_API_VERSION == 2 for LLVM 22+");
static_assert(
    offsetof(llvm::PassPluginLibraryInfo, PreCodeGenCallback) +
            sizeof(llvm::PassPluginLibraryInfo::PreCodeGenCallback) ==
        sizeof(llvm::PassPluginLibraryInfo),
    "Unexpected PassPluginLibraryInfo layout on LLVM 22+");
#else
static_assert(LLVM_PLUGIN_API_VERSION == 1,
              "Expected LLVM_PLUGIN_API_VERSION == 1 for LLVM 18-21");
static_assert(
    offsetof(llvm::PassPluginLibraryInfo, RegisterPassBuilderCallbacks) +
            sizeof(llvm::PassPluginLibraryInfo::RegisterPassBuilderCallbacks) ==
        sizeof(llvm::PassPluginLibraryInfo),
    "Unexpected PassPluginLibraryInfo layout on LLVM 18-21");
#endif

static OptLevel map_opt_level(OptimizationLevel Level) {
    if (Level == OptimizationLevel::O0) return OptLevel::O0;
    if (Level == OptimizationLevel::O1) return OptLevel::O1;
    if (Level == OptimizationLevel::O2) return OptLevel::O2;
    if (Level == OptimizationLevel::O3) return OptLevel::O3;
    if (Level == OptimizationLevel::Os) return OptLevel::Os;
    if (Level == OptimizationLevel::Oz) return OptLevel::Oz;
    return OptLevel::O2;
}

// LLVM 18 / 19 的模块级扩展点回调不携带 LTO 阶段，此函数仅在 20+ 使用。
#if LLVM_VERSION_MAJOR >= 20
static LtoPhase map_lto_phase(ThinOrFullLTOPhase Phase) {
    switch (Phase) {
        case ThinOrFullLTOPhase::None: return LtoPhase::None;
        case ThinOrFullLTOPhase::ThinLTOPreLink: return LtoPhase::ThinLTOPreLink;
        case ThinOrFullLTOPhase::ThinLTOPostLink: return LtoPhase::ThinLTOPostLink;
        case ThinOrFullLTOPhase::FullLTOPreLink: return LtoPhase::FullLTOPreLink;
        case ThinOrFullLTOPhase::FullLTOPostLink: return LtoPhase::FullLTOPostLink;
    }
    return LtoPhase::None;
}
#endif

static llvm::PreservedAnalyses map_preserved_analyses(PreservedAnalysesKind kind) {
    switch (kind) {
        case PreservedAnalysesKind::All:
            return llvm::PreservedAnalyses::all();
        case PreservedAnalysesKind::PreserveCFG: {
            llvm::PreservedAnalyses PA;
            PA.preserveSet<CFGAnalyses>();
            return PA;
        }
        case PreservedAnalysesKind::None:
        default:
            return llvm::PreservedAnalyses::none();
    }
}


template <typename T>
struct CopyableCallback {
    rust::Box<T> inner;

    explicit CopyableCallback(rust::Box<T> b) noexcept : inner(std::move(b)) {}
    CopyableCallback(const CopyableCallback &other) : inner(other.inner->clone_cb()) {}
    CopyableCallback(CopyableCallback &&) noexcept = default;
    CopyableCallback &operator=(const CopyableCallback &other) {
        if (this != &other) {
            inner = other.inner->clone_cb();
        }
        return *this;
    }
    CopyableCallback &operator=(CopyableCallback &&) noexcept = default;

    const T *operator->() const noexcept { return inner.operator->(); }
};


struct RustModulePassProxy : public PassInfoMixin<RustModulePassProxy> {
    rust::Box<BoxedModulePass> rust_pass;

    explicit RustModulePassProxy(rust::Box<BoxedModulePass> pass) noexcept
        : rust_pass(std::move(pass)) {}

    llvm::PreservedAnalyses run(Module &M, ModuleAnalysisManager &) {
        PreservedAnalysesKind kind = rust_pass->run(M);
        return map_preserved_analyses(kind);
    }

    void printPipeline(raw_ostream &OS, function_ref<StringRef(StringRef)>) {
        rust::Str name = rust_pass->name();
        OS << StringRef(name.data(), name.size());
    }

    static bool isRequired() { return true; }
};

struct RustFunctionPassProxy : public PassInfoMixin<RustFunctionPassProxy> {
    rust::Box<BoxedFunctionPass> rust_pass;

    explicit RustFunctionPassProxy(rust::Box<BoxedFunctionPass> pass) noexcept
        : rust_pass(std::move(pass)) {}

    llvm::PreservedAnalyses run(Function &F, FunctionAnalysisManager &) {
        PreservedAnalysesKind kind = rust_pass->run(F);
        return map_preserved_analyses(kind);
    }

    void printPipeline(raw_ostream &OS, function_ref<StringRef(StringRef)>) {
        rust::Str name = rust_pass->name();
        OS << StringRef(name.data(), name.size());
    }

    static bool isRequired() { return true; }
};


void mpm_add_module_pass(ModulePassManager &MPM, rust::Box<BoxedModulePass> pass) {
    MPM.addPass(RustModulePassProxy(std::move(pass)));
}

void mpm_add_function_pass(ModulePassManager &MPM, rust::Box<BoxedFunctionPass> pass, bool eagerly_invalidate) {
    MPM.addPass(createModuleToFunctionPassAdaptor(
        RustFunctionPassProxy(std::move(pass)), eagerly_invalidate));
}

void fpm_add_function_pass(FunctionPassManager &FPM, rust::Box<BoxedFunctionPass> pass) {
    FPM.addPass(RustFunctionPassProxy(std::move(pass)));
}


void pb_register_pipeline_start_ep(PassBuilder &PB, rust::Box<BoxedModuleEpCallback> cb) {
    CopyableCallback<BoxedModuleEpCallback> wrapper(std::move(cb));
    PB.registerPipelineStartEPCallback(
        [wrapper](ModulePassManager &MPM, OptimizationLevel Level) {
            wrapper->call(MPM, map_opt_level(Level));
        });
}

void pb_register_pipeline_early_simplification_ep(PassBuilder &PB, rust::Box<BoxedModuleLtoEpCallback> cb) {
    CopyableCallback<BoxedModuleLtoEpCallback> wrapper(std::move(cb));
#if LLVM_VERSION_MAJOR >= 20
    PB.registerPipelineEarlySimplificationEPCallback(
        [wrapper](ModulePassManager &MPM, OptimizationLevel Level, ThinOrFullLTOPhase Phase) {
            wrapper->call(MPM, map_opt_level(Level), map_lto_phase(Phase));
        });
#else
    PB.registerPipelineEarlySimplificationEPCallback(
        [wrapper](ModulePassManager &MPM, OptimizationLevel Level) {
            wrapper->call(MPM, map_opt_level(Level), LtoPhase::None);
        });
#endif
}

void pb_register_optimizer_early_ep(PassBuilder &PB, rust::Box<BoxedModuleLtoEpCallback> cb) {
    CopyableCallback<BoxedModuleLtoEpCallback> wrapper(std::move(cb));
#if LLVM_VERSION_MAJOR >= 20
    PB.registerOptimizerEarlyEPCallback(
        [wrapper](ModulePassManager &MPM, OptimizationLevel Level, ThinOrFullLTOPhase Phase) {
            wrapper->call(MPM, map_opt_level(Level), map_lto_phase(Phase));
        });
#else
    PB.registerOptimizerEarlyEPCallback(
        [wrapper](ModulePassManager &MPM, OptimizationLevel Level) {
            wrapper->call(MPM, map_opt_level(Level), LtoPhase::None);
        });
#endif
}

void pb_register_optimizer_last_ep(PassBuilder &PB, rust::Box<BoxedModuleLtoEpCallback> cb) {
    CopyableCallback<BoxedModuleLtoEpCallback> wrapper(std::move(cb));
#if LLVM_VERSION_MAJOR >= 20
    PB.registerOptimizerLastEPCallback(
        [wrapper](ModulePassManager &MPM, OptimizationLevel Level, ThinOrFullLTOPhase Phase) {
            wrapper->call(MPM, map_opt_level(Level), map_lto_phase(Phase));
        });
#else
    PB.registerOptimizerLastEPCallback(
        [wrapper](ModulePassManager &MPM, OptimizationLevel Level) {
            wrapper->call(MPM, map_opt_level(Level), LtoPhase::None);
        });
#endif
}

void pb_register_peephole_ep(PassBuilder &PB, rust::Box<BoxedFunctionEpCallback> cb) {
    CopyableCallback<BoxedFunctionEpCallback> wrapper(std::move(cb));
    PB.registerPeepholeEPCallback(
        [wrapper](FunctionPassManager &FPM, OptimizationLevel Level) {
            wrapper->call(FPM, map_opt_level(Level));
        });
}

void pb_register_scalar_optimizer_late_ep(PassBuilder &PB, rust::Box<BoxedFunctionEpCallback> cb) {
    CopyableCallback<BoxedFunctionEpCallback> wrapper(std::move(cb));
    PB.registerScalarOptimizerLateEPCallback(
        [wrapper](FunctionPassManager &FPM, OptimizationLevel Level) {
            wrapper->call(FPM, map_opt_level(Level));
        });
}

void pb_register_module_pipeline_parser(PassBuilder &PB, rust::Box<BoxedModulePipelineParser> cb) {
    CopyableCallback<BoxedModulePipelineParser> wrapper(std::move(cb));
    PB.registerPipelineParsingCallback(
        [wrapper](StringRef Name, ModulePassManager &MPM, ArrayRef<PassBuilder::PipelineElement>) -> bool {
            rust::Str name_str(Name.data(), Name.size());
            return wrapper->parse(name_str, MPM);
        });
}

void pb_register_function_pipeline_parser(PassBuilder &PB, rust::Box<BoxedFunctionPipelineParser> cb) {
    CopyableCallback<BoxedFunctionPipelineParser> wrapper(std::move(cb));
    PB.registerPipelineParsingCallback(
        [wrapper](StringRef Name, FunctionPassManager &FPM, ArrayRef<PassBuilder::PipelineElement>) -> bool {
            rust::Str name_str(Name.data(), Name.size());
            return wrapper->parse(name_str, FPM);
        });
}


rust::Str get_module_name(const Module &M) {
    StringRef name = M.getName();
    return rust::Str(name.data(), name.size());
}

rust::Str get_function_name(const Function &F) {
    StringRef name = F.getName();
    return rust::Str(name.data(), name.size());
}

bool is_function_declaration(const Function &F) {
    return F.isDeclaration();
}

} // namespace llvm_pass