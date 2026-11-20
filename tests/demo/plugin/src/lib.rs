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

        // 2. 测试 Module 级命令行管线解析 (-passes="demo-mod" / "demo-panic" / "demo-inkwell-mod")
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
                #[cfg(feature = "inkwell")]
                "demo-inkwell-mod" => {
                    mpm.add_pass(|m: &mut ModuleRef<'_>| {
                        // 测试 as_inkwell() 与 Deref
                        let fn_count = m.as_inkwell().get_functions().count();
                        eprintln!("[TRACE] InkwellModule::as_inkwell fn_count={}", fn_count);

                        // 测试 with_inkwell() 与模块级 IR 修改（注入全局变量）
                        m.with_inkwell(|iw_mod, ctx| {
                            let i32_ty = ctx.i32_type();
                            let global = iw_mod.add_global(i32_ty, None, "__inkwell_injected_global");
                            global.set_initializer(&i32_ty.const_int(1337, false));
                            global.set_linkage(inkwell::module::Linkage::Internal);
                        });
                        PreservedAnalyses::None
                    });
                    true
                }
                _ => false,
            }
        });

        // 3. 测试 Function 级命令行管线解析 (-passes="demo-fn<...>" / "demo-inkwell-fn")
        pb.register_function_pipeline_parsing_callback(|full_name, fpm| {
            #[cfg(feature = "inkwell")]
            if full_name == "demo-inkwell-fn" {
                fpm.add_pass(|func: &mut FunctionRef<'_>| {
                    if func.is_declaration() {
                        return PreservedAnalyses::All;
                    }
                    // 测试 as_inkwell()
                    let bb_count = func.as_inkwell().count_basic_blocks();
                    eprintln!(
                        "[TRACE] InkwellFunction::as_inkwell func={} bb_count={}",
                        func.name(),
                        bb_count
                    );

                    // 测试 with_inkwell() 与函数级 IR 修改（使用 Builder 插入指令）
                    func.with_inkwell(|iw_fn, ctx| {
                        if let (Some(entry_bb), Some(first_param)) =
                            (iw_fn.get_first_basic_block(), iw_fn.get_first_param())
                        {
                            let builder = ctx.create_builder();
                            if let Some(first_inst) = entry_bb.get_first_instruction() {
                                builder.position_before(&first_inst);
                            } else {
                                builder.position_at_end(entry_bb);
                            }
                            let rhs = ctx.i32_type().const_int(999, false);
                            let _ = builder
                                .build_int_add(
                                    first_param.into_int_value(),
                                    rhs,
                                    "inkwell_injected_add",
                                )
                                .expect("Failed to build add instruction via inkwell");
                        }
                    });
                    PreservedAnalyses::PreserveCFG
                });
                return true;
            }

            if let Some(("demo-fn", params)) = split_parametrized_pass_name(full_name) {
                let param_str = params.unwrap_or("default").to_string();
                fpm.add_pass(DemoFunctionPass { params: param_str });
                return true;
            }
            false
        });
    }
}
