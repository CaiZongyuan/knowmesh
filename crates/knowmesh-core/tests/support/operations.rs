use std::collections::HashSet;

use syn::{Expr, ImplItem, Item, Lit, PatWild, Stmt, Type, visit::Visit};

pub fn check_cli(source: &str, registered: &HashSet<&str>) -> Vec<String> {
    let syntax = match syn::parse_file(source) {
        Ok(syntax) => syntax,
        Err(error) => return vec![format!("Cannot parse CLI operation mapping: {error}")],
    };
    let methods: Vec<_> = syntax
        .items
        .iter()
        .filter_map(|item| {
            let Item::Impl(item) = item else {
                return None;
            };
            let Type::Path(path) = item.self_ty.as_ref() else {
                return None;
            };
            path.path.is_ident("Command").then_some(item)
        })
        .flat_map(|item| &item.items)
        .filter_map(|item| {
            let ImplItem::Fn(method) = item else {
                return None;
            };
            (method.sig.ident == "operation_name").then_some(method)
        })
        .collect();
    if methods.len() != 1 {
        return vec!["Expected one Command::operation_name mapping.".into()];
    }
    let [Stmt::Expr(Expr::Match(mapping), None)] = methods[0].block.stmts.as_slice() else {
        return vec!["The operation mapping must be an explicit match on self.".into()];
    };
    if !matches!(mapping.expr.as_ref(), Expr::Path(path) if path.path.is_ident("self"))
        || mapping.arms.is_empty()
    {
        return vec!["The operation mapping must exhaustively match self.".into()];
    }
    let mut errors = Vec::new();
    for arm in &mapping.arms {
        let mut wildcard = Wildcard(false);
        wildcard.visit_pat(&arm.pat);
        if wildcard.0 {
            errors.push("Operation mappings cannot hide handlers behind wildcard patterns.".into());
        }
        let Expr::Lit(literal) = arm.body.as_ref() else {
            errors.push("Operation names must be registered string literals.".into());
            continue;
        };
        let Lit::Str(name) = &literal.lit else {
            errors.push("Operation names must be string literals.".into());
            continue;
        };
        if !registered.contains(name.value().as_str()) {
            errors.push(format!("Unregistered public CLI handler: {}", name.value()));
        }
    }
    errors
}

struct Wildcard(bool);

impl<'ast> Visit<'ast> for Wildcard {
    fn visit_pat_wild(&mut self, _: &'ast PatWild) {
        self.0 = true;
    }
}
