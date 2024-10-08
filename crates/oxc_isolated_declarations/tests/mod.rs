mod deno;

use std::{fs, path::Path, sync::Arc};

use oxc_allocator::Allocator;
use oxc_codegen::CodeGenerator;
use oxc_isolated_declarations::IsolatedDeclarations;
use oxc_parser::Parser;
use oxc_span::SourceType;

fn transform(path: &Path, source_text: &str) -> String {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path).unwrap();
    let program = Parser::new(&allocator, source_text, source_type).parse().program;

    let ret = IsolatedDeclarations::new(&allocator).build(&program);
    let code = CodeGenerator::new().build(&ret.program).source_text;

    let mut snapshot = format!("==================== .D.TS ====================\n\n{code}\n\n");
    if !ret.errors.is_empty() {
        let source = Arc::new(source_text.to_string());
        let error_messages = ret
            .errors
            .iter()
            .map(|d| d.clone().with_source_code(Arc::clone(&source)))
            .map(|error| format!("{error:?}"))
            .collect::<Vec<_>>()
            .join("\n");

        snapshot.push_str(&format!(
            "==================== Errors ====================\n\n{error_messages}\n\n"
        ));
    }

    snapshot
}

#[test]
fn snapshots() {
    insta::glob!("fixtures/well-known-symbols.{ts,tsx}", |path| {
        let source_text = fs::read_to_string(path).unwrap();
        let snapshot = transform(path, &source_text);
        let name = path.file_stem().unwrap().to_str().unwrap();
        insta::with_settings!({ prepend_module_to_snapshot => false, snapshot_suffix => "", omit_expression => true }, {
            insta::assert_snapshot!(name, snapshot);
        });
    });
}
