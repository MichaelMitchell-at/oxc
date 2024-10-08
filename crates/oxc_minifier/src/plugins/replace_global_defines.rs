use std::sync::Arc;

use oxc_allocator::Allocator;
use oxc_ast::{ast::*, visit::walk_mut, AstBuilder, VisitMut};
use oxc_diagnostics::OxcDiagnostic;
use oxc_parser::Parser;
use oxc_span::{CompactStr, SourceType};
use oxc_syntax::identifier::is_identifier_name;

/// Configuration for [ReplaceGlobalDefines].
///
/// Due to the usage of an arena allocator, the constructor will parse once for grammatical errors,
/// and does not save the constructed expression.
///
/// The data is stored in an `Arc` so this can be shared across threads.
#[derive(Debug, Clone)]
pub struct ReplaceGlobalDefinesConfig(Arc<ReplaceGlobalDefinesConfigImpl>);

#[derive(Debug)]
struct ReplaceGlobalDefinesConfigImpl {
    identifier_defines: Vec<(/* key */ CompactStr, /* value */ CompactStr)>,
    dot_defines: Vec<DotDefine>,
}

#[derive(Debug)]
pub struct DotDefine {
    /// Member expression parts
    pub parts: Vec<CompactStr>,
    pub value: CompactStr,
}

impl DotDefine {
    fn new(parts: Vec<CompactStr>, value: CompactStr) -> Self {
        Self { parts, value }
    }
}

enum IdentifierType {
    Identifier,
    DotDefines(Vec<CompactStr>),
}

impl ReplaceGlobalDefinesConfig {
    /// # Errors
    ///
    /// * key is not an identifier
    /// * value has a syntax error
    pub fn new<S: AsRef<str>>(defines: &[(S, S)]) -> Result<Self, Vec<OxcDiagnostic>> {
        let allocator = Allocator::default();
        let mut identifier_defines = vec![];
        let mut dot_defines = vec![];
        for (key, value) in defines {
            let key = key.as_ref();

            let value = value.as_ref();
            Self::check_value(&allocator, value)?;

            match Self::check_key(key)? {
                IdentifierType::Identifier => {
                    identifier_defines.push((CompactStr::new(key), CompactStr::new(value)));
                }
                IdentifierType::DotDefines(parts) => {
                    dot_defines.push(DotDefine::new(parts, CompactStr::new(value)));
                }
            }
        }

        Ok(Self(Arc::new(ReplaceGlobalDefinesConfigImpl { identifier_defines, dot_defines })))
    }

    fn check_key(key: &str) -> Result<IdentifierType, Vec<OxcDiagnostic>> {
        let parts: Vec<&str> = key.split('.').collect();

        assert!(!parts.is_empty());

        if parts.len() == 1 {
            if !is_identifier_name(parts[0]) {
                return Err(vec![OxcDiagnostic::error(format!("`{key}` is not an identifier."))]);
            }
            return Ok(IdentifierType::Identifier);
        }

        for part in &parts {
            if !is_identifier_name(part) {
                return Err(vec![OxcDiagnostic::error(format!("`{key}` is not an identifier."))]);
            }
        }

        Ok(IdentifierType::DotDefines(parts.iter().map(|s| CompactStr::new(s)).collect()))
    }

    fn check_value(allocator: &Allocator, source_text: &str) -> Result<(), Vec<OxcDiagnostic>> {
        Parser::new(allocator, source_text, SourceType::default()).parse_expression()?;
        Ok(())
    }
}

/// Replace Global Defines.
///
/// References:
///
/// * <https://esbuild.github.io/api/#define>
/// * <https://github.com/terser/terser?tab=readme-ov-file#conditional-compilation>
/// * <https://github.com/evanw/esbuild/blob/9c13ae1f06dfa909eb4a53882e3b7e4216a503fe/internal/config/globals.go#L852-L1014>
pub struct ReplaceGlobalDefines<'a> {
    ast: AstBuilder<'a>,
    config: ReplaceGlobalDefinesConfig,
}

impl<'a> VisitMut<'a> for ReplaceGlobalDefines<'a> {
    fn visit_expression(&mut self, expr: &mut Expression<'a>) {
        self.replace_identifier_defines(expr);
        self.replace_dot_defines(expr);
        walk_mut::walk_expression(self, expr);
    }
}

impl<'a> ReplaceGlobalDefines<'a> {
    pub fn new(allocator: &'a Allocator, config: ReplaceGlobalDefinesConfig) -> Self {
        Self { ast: AstBuilder::new(allocator), config }
    }

    pub fn build(&mut self, program: &mut Program<'a>) {
        self.visit_program(program);
    }

    // Construct a new expression because we don't have ast clone right now.
    fn parse_value(&self, source_text: &str) -> Expression<'a> {
        // Allocate the string lazily because replacement happens rarely.
        let source_text = self.ast.allocator.alloc(source_text.to_string());
        // Unwrapping here, it should already be checked by [ReplaceGlobalDefinesConfig::new].
        Parser::new(self.ast.allocator, source_text, SourceType::default())
            .parse_expression()
            .unwrap()
    }

    fn replace_identifier_defines(&self, expr: &mut Expression<'a>) {
        if let Expression::Identifier(ident) = expr {
            for (key, value) in &self.config.0.identifier_defines {
                if ident.name.as_str() == key {
                    let value = self.parse_value(value);
                    *expr = value;
                    break;
                }
            }
        }
    }

    fn replace_dot_defines(&mut self, expr: &mut Expression<'a>) {
        if let Expression::StaticMemberExpression(member) = expr {
            for dot_define in &self.config.0.dot_defines {
                if Self::is_dot_define(dot_define, member) {
                    let value = self.parse_value(&dot_define.value);
                    *expr = value;
                    break;
                }
            }
        }
    }

    pub fn is_dot_define(dot_define: &DotDefine, member: &StaticMemberExpression<'a>) -> bool {
        debug_assert!(dot_define.parts.len() > 1);

        let mut current_part_member_expression = Some(member);
        let mut cur_part_name = &member.property.name;

        for (i, part) in dot_define.parts.iter().enumerate().rev() {
            if cur_part_name.as_str() != part {
                return false;
            }

            if i == 0 {
                break;
            }

            current_part_member_expression = if let Some(member) = current_part_member_expression {
                match &member.object {
                    Expression::StaticMemberExpression(member) => {
                        cur_part_name = &member.property.name;
                        Some(member)
                    }
                    Expression::Identifier(ident) => {
                        cur_part_name = &ident.name;
                        None
                    }
                    _ => None,
                }
            } else {
                return false;
            };
        }

        true
    }
}
