// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#include "shim.h"
#include "llvm-pass/src/lib.rs.h"

#include "llvm/Config/llvm-config.h"
#include <iterator>

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

// 与 Rust 侧 `PreservedAnalyses` 位掩码常量严格对应
static constexpr uint32_t PA_NONE           = 0;
static constexpr uint32_t PA_ALL            = 1u << 0;
static constexpr uint32_t PA_CFG            = 1u << 1;
static constexpr uint32_t PA_DOMINATOR_TREE = 1u << 2;
static constexpr uint32_t PA_POST_DOM_TREE  = 1u << 3;
static constexpr uint32_t PA_LOOP_INFO      = 1u << 4;
static constexpr uint32_t PA_CALL_GRAPH     = 1u << 5;
static constexpr uint32_t PA_FAM_PROXY      = 1u << 6;
static constexpr uint32_t PA_GLOBALS_AA     = 1u << 7;

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

static llvm::PreservedAnalyses map_preserved_analyses(uint32_t mask) {
    if (mask & PA_ALL) {
        return llvm::PreservedAnalyses::all();
    }
    if (mask == PA_NONE) {
        return llvm::PreservedAnalyses::none();
    }

    llvm::PreservedAnalyses PA;
    if (mask & PA_CFG) {
        PA.preserveSet<CFGAnalyses>();
    }
    if (mask & PA_DOMINATOR_TREE) {
        PA.preserve<DominatorTreeAnalysis>();
    }
    if (mask & PA_POST_DOM_TREE) {
        PA.preserve<PostDominatorTreeAnalysis>();
    }
    if (mask & PA_LOOP_INFO) {
        PA.preserve<LoopAnalysis>();
    }
    if (mask & PA_CALL_GRAPH) {
        PA.preserve<CallGraphAnalysis>();
    }
    if (mask & PA_FAM_PROXY) {
        PA.preserve<FunctionAnalysisManagerModuleProxy>();
    }
    if (mask & PA_GLOBALS_AA) {
        PA.preserve<GlobalsAA>();
    }
    return PA;
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

    llvm::PreservedAnalyses run(Module &M, ModuleAnalysisManager &MAM) {
        uint32_t mask = rust_pass->run(M, MAM);
        return map_preserved_analyses(mask);
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

    llvm::PreservedAnalyses run(Function &F, FunctionAnalysisManager &FAM) {
        uint32_t mask = rust_pass->run(F, FAM);
        return map_preserved_analyses(mask);
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

Function *module_first_function(Module &M) {
    return M.empty() ? nullptr : &*M.begin();
}

Function *module_next_function(Module &M, Function &F) {
    auto It = F.getIterator();
    ++It;
    return It == M.end() ? nullptr : &*It;
}

// === ModuleAnalysisManager 实现 ===

const CallGraph &mam_get_call_graph(const ModuleAnalysisManager &MAM, const Module &M) {
    auto &MutMAM = const_cast<ModuleAnalysisManager &>(MAM);
    auto &MutM = const_cast<Module &>(M);
    return MutMAM.getResult<CallGraphAnalysis>(MutM);
}

const CallGraph *mam_get_cached_call_graph(const ModuleAnalysisManager &MAM, const Module &M) {
    auto &MutM = const_cast<Module &>(M);
    return MAM.getCachedResult<CallGraphAnalysis>(MutM);
}

FunctionAnalysisManager &mam_get_function_analysis_manager(
    ModuleAnalysisManager &MAM, Module &M) {
    return MAM.getResult<FunctionAnalysisManagerModuleProxy>(M).getManager();
}

void mam_invalidate_module(
    ModuleAnalysisManager &MAM, Module &M, uint32_t preserved_mask) {
    MAM.invalidate(M, map_preserved_analyses(preserved_mask));
}

const GlobalsAAResult &mam_get_globals_aa(const ModuleAnalysisManager &MAM, const Module &M) {
    auto &MutMAM = const_cast<ModuleAnalysisManager &>(MAM);
    auto &MutM = const_cast<Module &>(M);
    return MutMAM.getResult<GlobalsAA>(MutM);
}

const GlobalsAAResult *mam_get_cached_globals_aa(const ModuleAnalysisManager &MAM, const Module &M) {
    auto &MutM = const_cast<Module &>(M);
    return MAM.getCachedResult<GlobalsAA>(MutM);
}

// === FunctionAnalysisManager 实现 ===

const DominatorTree &fam_get_dominator_tree(
    const FunctionAnalysisManager &FAM, const Function &F) {
    // SAFETY: FAM 与 F 在 PassManager::run 中原本即为非 const 对象；
    // AnalysisManager 内部使用 std::list<pair<AnalysisKey*, unique_ptr<ResultConceptT>>> 存储结果，
    // getResult 仅在未命中时追加节点，绝不会使已有分析引用失效。
    auto &MutFAM = const_cast<FunctionAnalysisManager &>(FAM);
    auto &MutF = const_cast<Function &>(F);
    return MutFAM.getResult<DominatorTreeAnalysis>(MutF);
}

const DominatorTree *fam_get_cached_dominator_tree(
    const FunctionAnalysisManager &FAM, const Function &F) {
    auto &MutF = const_cast<Function &>(F);
    return FAM.getCachedResult<DominatorTreeAnalysis>(MutF);
}

const PostDominatorTree &fam_get_post_dominator_tree(
    const FunctionAnalysisManager &FAM, const Function &F) {
    auto &MutFAM = const_cast<FunctionAnalysisManager &>(FAM);
    auto &MutF = const_cast<Function &>(F);
    return MutFAM.getResult<PostDominatorTreeAnalysis>(MutF);
}

const PostDominatorTree *fam_get_cached_post_dominator_tree(
    const FunctionAnalysisManager &FAM, const Function &F) {
    auto &MutF = const_cast<Function &>(F);
    return FAM.getCachedResult<PostDominatorTreeAnalysis>(MutF);
}

const LoopInfo &fam_get_loop_info(
    const FunctionAnalysisManager &FAM, const Function &F) {
    auto &MutFAM = const_cast<FunctionAnalysisManager &>(FAM);
    auto &MutF = const_cast<Function &>(F);
    return MutFAM.getResult<LoopAnalysis>(MutF);
}

const LoopInfo *fam_get_cached_loop_info(
    const FunctionAnalysisManager &FAM, const Function &F) {
    auto &MutF = const_cast<Function &>(F);
    return FAM.getCachedResult<LoopAnalysis>(MutF);
}

const GlobalsAAResult *fam_get_cached_globals_aa(
    const FunctionAnalysisManager &FAM, const Function &F) {
    const Module *M = F.getParent();
    if (!M) return nullptr;
    auto &MutFAM = const_cast<FunctionAnalysisManager &>(FAM);
    auto &MutF = const_cast<Function &>(F);
    const auto *Proxy =
        MutFAM.getCachedResult<ModuleAnalysisManagerFunctionProxy>(MutF);
    if (!Proxy) {
        const auto &ComputedProxy =
            MutFAM.getResult<ModuleAnalysisManagerFunctionProxy>(MutF);
        return ComputedProxy.getCachedResult<GlobalsAA>(*const_cast<Module *>(M));
    }
    return Proxy->getCachedResult<GlobalsAA>(*const_cast<Module *>(M));
}

void fam_invalidate_function(
    FunctionAnalysisManager &FAM, Function &F, uint32_t preserved_mask) {
    FAM.invalidate(F, map_preserved_analyses(preserved_mask));
}

// === 分析结果查询辅助函数 ===

bool dt_dominates_bb(const DominatorTree &DT, const BasicBlock *A, const BasicBlock *B) {
    return DT.dominates(A, B);
}

bool dt_strictly_dominates_bb(const DominatorTree &DT, const BasicBlock *A, const BasicBlock *B) {
    return DT.properlyDominates(A, B);
}

bool dt_is_reachable_from_entry(const DominatorTree &DT, const BasicBlock *BB) {
    return DT.isReachableFromEntry(BB);
}

bool pdt_dominates_bb(const PostDominatorTree &PDT, const BasicBlock *A, const BasicBlock *B) {
    return PDT.dominates(A, B);
}

bool pdt_strictly_dominates_bb(const PostDominatorTree &PDT, const BasicBlock *A, const BasicBlock *B) {
    return PDT.properlyDominates(A, B);
}

bool li_is_empty(const LoopInfo &LI) {
    return LI.empty();
}

size_t li_top_level_loops_count(const LoopInfo &LI) {
    return static_cast<size_t>(std::distance(LI.begin(), LI.end()));
}

uint32_t li_get_loop_depth(const LoopInfo &LI, const BasicBlock *BB) {
    return LI.getLoopDepth(BB);
}

bool li_is_loop_header(const LoopInfo &LI, const BasicBlock *BB) {
    return LI.isLoopHeader(BB);
}

} // namespace llvm_pass
