use inkwell::context::Context;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetTriple,
};
use inkwell::OptimizationLevel;

use crate::ast::{Item, ItemKind};

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

        for item in program {
            match item.kind {
                ItemKind::Fn(fun) => {
                    let fn_type = self.context.void_type().fn_type(&[], false);

                    let fn_value = module.add_function(&item.name.to_string(), fn_type, None);

                    dbg!(fn_value);
                }
            }
        }

        dbg!(module.get_functions().count());

        let buffer = target_machine
            .write_to_memory_buffer(&module, FileType::Object)
            .expect("Failed to write to buffer");

        use std::io::Write;

        let mut outfile = std::fs::File::create("build/main.o").unwrap();

        outfile.write_all(buffer.as_slice()).unwrap();

        let bitcode = module.write_bitcode_to_memory();

        outfile.write_all(bitcode.as_slice()).unwrap();
    }
}
