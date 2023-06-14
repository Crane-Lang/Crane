use std::process::Command;

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::passes::PassManager;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetTriple,
};
use inkwell::types::BasicType;
use inkwell::values::{
    BasicMetadataValueEnum, BasicValue, CallSiteValue, FunctionValue, GlobalValue,
};
use inkwell::{AddressSpace, OptimizationLevel};
use thin_vec::ThinVec;

use crate::ast::{Expr, ExprKind, FnParam, Item, ItemKind, Literal, LiteralKind, StmtKind};

pub struct NativeBackend {
    context: Context,
}

impl NativeBackend {
    pub fn new() -> Self {
        Self {
            context: Context::create(),
        }
    }

    pub fn compile(&self, program: Vec<Item>) {
        Target::initialize_aarch64(&InitializationConfig::default());

        let opt = OptimizationLevel::Default;
        let reloc = RelocMode::Default;
        let model = CodeModel::Default;

        let target = Target::from_name("aarch64").expect("Failed to parse target");

        let target = dbg!(target);

        let target_machine = target
            .create_target_machine(
                &TargetTriple::create("aarch64-apple-darwin"),
                "apple-m2",
                "",
                opt,
                reloc,
                model,
            )
            .unwrap();

        let target_machine = dbg!(target_machine);

        let module = self.context.create_module("main");
        let builder = self.context.create_builder();

        let fpm = PassManager::create(&module);

        fpm.add_instruction_combining_pass();

        fpm.initialize();

        let i8_type = self.context.i8_type();
        let i32_type = self.context.i32_type();
        let fn_type = i32_type.fn_type(
            &[i8_type
                .ptr_type(AddressSpace::default())
                .as_basic_type_enum()
                .into()],
            false,
        );

        let puts = module.add_function("puts", fn_type, Some(Linkage::External));

        dbg!(puts);

        // HACK: Register `print` and `println` functions.
        for fn_name in ["print", "println"] {
            let fn_type = self.context.void_type().fn_type(
                &[i8_type
                    .ptr_type(AddressSpace::default())
                    .as_basic_type_enum()
                    .into()],
                false,
            );

            let fn_value = module.add_function(&fn_name, fn_type, None);

            let value_param = fn_value.get_first_param().unwrap();

            let entry = self.context.append_basic_block(fn_value, "entry");

            builder.position_at_end(entry);

            if let Some(callee) = module.get_function(&"puts") {
                builder.build_call(callee, &[value_param.into()], "tmp");
            } else {
                eprintln!("Function '{}' not found.", "puts");
            }

            builder.build_return(None);

            if fn_value.verify(true) {
                fpm.run_on(&fn_value);

                println!("{} is verified!", fn_name);
            } else {
                println!("{} is not verified :(", fn_name);
            }

            dbg!(fn_value);
        }

        for item in program
            // HACK: Reverse the items so we define the helper functions before `main`.
            // This should be replaced with a call graph.
            .into_iter()
            .rev()
        {
            match item.kind {
                ItemKind::Fn(fun) => {
                    let params = fun
                        .params
                        .iter()
                        .map(|_param| {
                            i8_type
                                .ptr_type(AddressSpace::default())
                                .as_basic_type_enum()
                                .into()
                        })
                        .collect::<Vec<_>>();

                    let fn_type = self.context.void_type().fn_type(&params, false);

                    let fn_value = module.add_function(&item.name.to_string(), fn_type, None);

                    let entry = self.context.append_basic_block(fn_value, "entry");

                    builder.position_at_end(entry);

                    for stmt in fun.body {
                        match stmt.kind {
                            StmtKind::Expr(expr) => Self::compile_expr(
                                &self.context,
                                &builder,
                                &module,
                                &fun.params,
                                &fn_value,
                                expr,
                            ),
                            StmtKind::Item(item) => todo!(),
                        }
                    }

                    builder.build_return(None);

                    dbg!(fn_value);

                    if fn_value.verify(true) {
                        fpm.run_on(&fn_value);

                        println!("{} is verified!", item.name);
                    } else {
                        println!("{} is not verified :(", item.name);
                    }

                    fn_value.print_to_stderr();
                }
            }
        }

        dbg!(module.get_functions().count());

        module
            .print_to_file("build/main.ll")
            .expect("Failed to emit main.ll");

        let buffer = target_machine
            .write_to_memory_buffer(&module, FileType::Object)
            .expect("Failed to write to buffer");

        use std::io::Write;

        let mut outfile = std::fs::File::create("build/main.o").unwrap();

        outfile.write_all(buffer.as_slice()).unwrap();

        let bitcode = module.write_bitcode_to_memory();

        outfile.write_all(bitcode.as_slice()).unwrap();

        let exit_status = Command::new("clang")
            .args(["-o", "build/main", "build/main.o"])
            .status()
            .expect("Failed to build with clang");

        println!("clang exited with {}", exit_status);
    }

    fn compile_expr<'ctx>(
        context: &'ctx Context,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        fn_params: &ThinVec<FnParam>,
        fn_value: &FunctionValue<'ctx>,
        expr: Expr,
    ) {
        match expr.kind {
            ExprKind::Literal(literal) => match literal.kind {
                LiteralKind::String => {
                    Self::compile_string_literal(&context, &builder, &module, literal);
                }
            },
            ExprKind::Variable { name } => todo!(),
            ExprKind::Call { fun, args } => {
                Self::compile_fn_call(
                    context,
                    builder,
                    module,
                    fn_value,
                    fn_params,
                    fun.clone(),
                    args,
                )
                .expect(&format!("Failed to compile function call: {:?}", fun));
            }
        }
    }

    fn compile_string_literal<'ctx>(
        context: &'ctx Context,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        literal: Literal,
    ) -> GlobalValue<'ctx> {
        // Unquote the string literal.
        let value = {
            let mut chars = literal.value.chars();
            chars.next();
            chars.next_back();
            chars.as_str()
        };

        let value = value.as_bytes();

        let i8_type = context.i8_type();
        let i8_array_type = i8_type.array_type(value.len() as u32 + 1);

        let string = context.const_string(value, true);

        let global = module.add_global(i8_array_type, None, "string_lit");
        global.set_linkage(Linkage::Internal);
        global.set_constant(true);
        global.set_initializer(&string);

        global
    }

    fn compile_fn_call<'ctx>(
        context: &'ctx Context,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        caller: &FunctionValue<'ctx>,
        caller_params: &ThinVec<FnParam>,
        fun: Box<Expr>,
        args: ThinVec<Box<Expr>>,
    ) -> Result<CallSiteValue<'ctx>, String> {
        let callee_name = match fun.kind {
            ExprKind::Variable { name } => name,
            _ => todo!(),
        };

        if let Some(callee) = module.get_function(&callee_name.to_string()) {
            let args: Vec<BasicMetadataValueEnum> = args
                .into_iter()
                .map(|arg| match arg.kind {
                    ExprKind::Literal(literal) => match literal.kind {
                        LiteralKind::String => {
                            Self::compile_string_literal(&context, &builder, &module, literal)
                                .as_basic_value_enum()
                                .into()
                        }
                    },
                    ExprKind::Variable { name } => {
                        let (param_index, _) = caller_params
                            .into_iter()
                            .enumerate()
                            .find(|(_, param)| param.name == name)
                            .expect(&format!("Param '{}' not found.", name));

                        caller
                            .get_nth_param(param_index as u32)
                            .expect("Param not found")
                            .as_basic_value_enum()
                            .into()
                    }
                    ExprKind::Call { fun, args } => Self::compile_fn_call(
                        &context,
                        &builder,
                        &module,
                        caller,
                        caller_params,
                        fun,
                        args,
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .unwrap_left()
                    .into(),
                })
                .collect::<Vec<_>>();

            Ok(builder.build_call(callee, args.as_slice(), "tmp"))
        } else {
            eprintln!("Function '{}' not found.", callee_name);
            Err(format!("Function '{}' not found.", callee_name))
        }
    }
}
