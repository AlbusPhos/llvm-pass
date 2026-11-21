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
        assert!(
            !module.analysis_manager_raw_ptr().is_null(),
            "ModuleAnalysisManager raw ptr must not be null"
        );
        // 测试模块级分析 CallGraph 的惰性计算与缓存读取
        let cg = module.call_graph();
        assert!(!cg.as_raw_ptr().is_null());
        assert!(module.cached_analysis::<CallGraphAnalysis>().is_some());

        // GlobalsAA 是 LLVM 的"永不失效"模块分析，可从函数级跨层级查询
        let gaa = module.globals_aa();
        assert!(!gaa.as_raw_ptr().is_null());
        assert!(module.cached_analysis::<GlobalsAA>().is_some());

        // 测试在 ModulePass 中遍历函数并跨层级查询分析
        let mut analyzed = 0usize;
        module.for_each_function(|func| {
            if !func.is_declaration() {
                let _dt = func.dominator_tree();
                let _li = func.loop_info();
                assert!(
                    func.cached_module_analysis::<GlobalsAA>().is_some(),
                    "GlobalsAA should be cached in the outer ModuleAnalysisManager"
                );
                analyzed += 1;
            }
        });
        assert!(analyzed > 0, "expected at least one function definition");

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
        assert!(
            !func.analysis_manager_raw_ptr().is_null(),
            "FunctionAnalysisManager raw ptr must not be null"
        );
        if func.is_declaration() {
            eprintln!("[TRACE] FunctionPass::skip_decl func={}", func.name());
            return PreservedAnalyses::All;
        }

        // 同时获取多个函数级分析（共享借用互不冲突）
        let dt = func.dominator_tree();
        let pdt = func.post_dominator_tree();
        let li = func.loop_info();
        assert!(!dt.as_raw_ptr().is_null());
        assert!(!pdt.as_raw_ptr().is_null());
        assert!(!li.as_raw_ptr().is_null());

        eprintln!(
            "[TRACE] FunctionPass::run func={} params={} loops={}",
            func.name(),
            self.params,
            li.top_level_loops_count()
        );

        PreservedAnalyses::PreserveCFG
            .preserve::<DominatorTreeAnalysis>()
            .preserve::<LoopAnalysis>()
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

        // 2. 测试 Module 级命令行管线解析
        pb.register_module_pipeline_parsing_callback(|name, mpm| {
            match name {
                "demo-mod" => {
                    mpm.add_pass(DemoModulePass);
                    true
                }
                "demo-mod-closure" => {
                    mpm.add_pass(|m: &mut ModuleRef<'_>| {
                        eprintln!("[TRACE] ClosureModulePass::run module={}", m.name());
                        // 测试在 ModulePass 中通过 InnerAnalysisManagerProxy 遍历函数并查询函数级分析
                        m.for_each_function(|func| {
                            if !func.is_declaration() {
                                let _dt = func.dominator_tree();
                            }
                        });
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
                "demo-invalidate" => {
                    mpm.add_pass(|m: &mut ModuleRef<'_>| {
                        let _cg = m.call_graph();
                        assert!(
                            m.cached_analysis::<CallGraphAnalysis>().is_some(),
                            "CallGraph should be cached before invalidation"
                        );
                        m.invalidate_analyses(PreservedAnalyses::None);
                        assert!(
                            m.cached_analysis::<CallGraphAnalysis>().is_none(),
                            "CallGraph should be invalidated"
                        );
                        eprintln!("[TRACE] ModuleAnalysisInvalidation::run module={}", m.name());
                        PreservedAnalyses::None
                    });
                    true
                }
                "demo-native-module" => {
                    mpm.add_adce();
                    mpm.add_global_dce();
                    true
                }
                #[cfg(feature = "inkwell")]
                "demo-inkwell-mod" => {
                    mpm.add_pass(|m: &mut ModuleRef<'_>| {
                        let fn_count = m.as_inkwell().get_functions().count();
                        eprintln!("[TRACE] InkwellModule::as_inkwell fn_count={}", fn_count);
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

        // 3. 测试 Function 级命令行管线解析
        pb.register_function_pipeline_parsing_callback(|full_name, fpm| {
            if full_name == "demo-invalidate-fn" {
                fpm.add_pass(|func: &mut FunctionRef<'_>| {
                    if func.is_declaration() {
                        return PreservedAnalyses::All;
                    }
                    let _dt = func.dominator_tree();
                    assert!(
                        func.cached_analysis::<DominatorTreeAnalysis>().is_some(),
                        "DominatorTree should be cached before invalidation"
                    );
                    func.invalidate_analyses(PreservedAnalyses::None);
                    assert!(
                        func.cached_analysis::<DominatorTreeAnalysis>().is_none(),
                        "DominatorTree should be invalidated"
                    );
                    eprintln!(
                        "[TRACE] FunctionAnalysisInvalidation::run func={}",
                        func.name()
                    );
                    PreservedAnalyses::None
                });
                return true;
            }

            if full_name == "demo-native-fn" {
                fpm.add_adce();
                fpm.add_simplify_cfg();
                fpm.add_promote();
                return true;
            }

            #[cfg(feature = "inkwell")]
            if full_name == "demo-inkwell-fn" {
                fpm.add_pass(|func: &mut FunctionRef<'_>| {
                    if func.is_declaration() {
                        return PreservedAnalyses::All;
                    }
                    // 同时持有 DominatorTree、LoopInfo 并配合 inkwell 操作基本块
                    let dt = func.dominator_tree();
                    let li = func.loop_info();
                    let bb_count = func.as_inkwell().count_basic_blocks();
                    eprintln!(
                        "[TRACE] InkwellFunction::as_inkwell func={} bb_count={}",
                        func.name(),
                        bb_count
                    );
                    func.with_inkwell(|iw_fn, ctx| {
                        if let (Some(entry_bb), Some(first_param)) =
                            (iw_fn.get_first_basic_block(), iw_fn.get_first_param())
                        {
                            assert!(dt.is_reachable_from_entry(entry_bb));
                            assert!(dt.dominates(entry_bb, entry_bb));
                            let _depth = li.loop_depth(entry_bb);

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
