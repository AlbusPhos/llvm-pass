use llvm_pass::*;

struct DemoModulePass;

impl ModulePass for DemoModulePass {
    fn name(&self) -> &str {
        "DemoModulePass"
    }

    fn run(&mut self, module: &mut ModuleRef<'_>) -> PreservedAnalyses {
        assert!(
            !module.as_raw_ptr().is_null(),
            "Module raw ptr must not be null"
        );
        eprintln!("[TRACE] ModulePass::run on module={}", module.name());
        PreservedAnalyses::All
    }
}

struct DemoFunctionPass {
    params: String,
}

impl FunctionPass for DemoFunctionPass {
    fn name(&self) -> &str {
        "DemoFunctionPass"
    }

    fn run(&mut self, func: &mut FunctionRef<'_>) -> PreservedAnalyses {
        assert!(
            !func.as_raw_ptr().is_null(),
            "Function raw ptr must not be null"
        );
        if func.is_declaration() {
            eprintln!("[TRACE] FunctionPass::skip_decl func={}", func.name());
            return PreservedAnalyses::All;
        }
        eprintln!(
            "[TRACE] FunctionPass::run func={} params={}",
            func.name(),
            self.params
        );
        PreservedAnalyses::PreserveCFG
    }
}

export_llvm_plugin! {
    name: "DemoPassPlugin",
    version: "0.1.0",
    register: |pb: &mut PassBuilder| {
        // 1. 测试各个编译管线扩展点 (Extension Points)
        pb.register_pipeline_start_ep_callback(|mpm, level| {
            eprintln!("[TRACE] EP::PipelineStart level={:?}", level);
            mpm.add_pass(DemoModulePass);
        });

        pb.register_pipeline_early_simplification_ep_callback(|_mpm, level, phase| {
            eprintln!("[TRACE] EP::EarlySimplification level={:?} phase={:?}", level, phase);
        });

        pb.register_optimizer_early_ep_callback(|mpm, level, phase| {
            eprintln!("[TRACE] EP::OptimizerEarly level={:?} phase={:?}", level, phase);
            if level != OptLevel::O0 {
                // 测试 EagerlyInvalidate 挂载与闭包 FunctionPass
                mpm.add_function_pass_eagerly_invalidate(|func: &mut FunctionRef<'_>| {
                    if !func.is_declaration() {
                        eprintln!("[TRACE] ClosureFunctionPass::run func={}", func.name());
                    }
                    PreservedAnalyses::All
                });
            }
        });

        pb.register_optimizer_last_ep_callback(|_mpm, level, phase| {
            eprintln!("[TRACE] EP::OptimizerLast level={:?} phase={:?}", level, phase);
        });

        pb.register_peephole_ep_callback(|_fpm, level| {
            eprintln!("[TRACE] EP::Peephole level={:?}", level);
        });

        pb.register_scalar_optimizer_late_ep_callback(|_fpm, level| {
            eprintln!("[TRACE] EP::ScalarOptimizerLate level={:?}", level);
        });

        // 2. 测试 Module 级命令行管线解析 (-passes="demo-mod" / "demo-panic")
        pb.register_module_pipeline_parsing_callback(|name, mpm| {
            match name {
                "demo-mod" => {
                    mpm.add_pass(DemoModulePass);
                    true
                }
                "demo-mod-closure" => {
                    mpm.add_pass(|m: &mut ModuleRef<'_>| {
                        eprintln!("[TRACE] ClosureModulePass::run module={}", m.name());
                        PreservedAnalyses::None
                    });
                    true
                }
                "demo-panic" => {
                    mpm.add_pass(|_m: &mut ModuleRef<'_>| -> PreservedAnalyses {
                        panic!("simulated panic inside Rust pass");
                    });
                    true
                }
                _ => false,
            }
        });

        // 3. 测试 Function 级带参数命令行管线解析 (-passes="demo-fn<prob=80;loop=3>")
        pb.register_function_pipeline_parsing_callback(|full_name, fpm| {
            if let Some(("demo-fn", params)) = split_parametrized_pass_name(full_name) {
                let param_str = params.unwrap_or("default").to_string();
                fpm.add_pass(DemoFunctionPass { params: param_str });
                return true;
            }
            false
        });
    }
}
