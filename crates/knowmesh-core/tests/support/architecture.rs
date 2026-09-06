use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    process::Command,
};

use serde::Deserialize;
use serde_json::Value;
use syn::{
    visit::{self, Visit},
    *,
};

#[derive(Debug)]
pub struct Violation {
    pub code: &'static str,
    pub path: String,
    pub detail: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {} ({})", self.path, self.code, self.detail)
    }
}

#[derive(Deserialize)]
struct Policy {
    composition_roots: Vec<String>,
    canonical_writer_users: Vec<String>,
    projection_users: Vec<String>,
    filesystem_writers: Vec<String>,
    sql_writers: Vec<String>,
    process_users: Vec<String>,
    filesystem_exceptions: BTreeMap<String, Vec<String>>,
}

fn test_only(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .parse_args::<syn::Path>()
                .is_ok_and(|path| path.is_ident("test"))
    })
}

fn imports(tree: &UseTree, prefix: &str, out: &mut BTreeMap<String, String>) {
    let join = |name: &str| {
        if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}::{name}")
        }
    };
    match tree {
        UseTree::Path(path) => imports(&path.tree, &join(&path.ident.to_string()), out),
        UseTree::Name(name) if name.ident == "self" => {
            out.insert(prefix.rsplit("::").next().unwrap().into(), prefix.into());
        }
        UseTree::Name(name) => {
            out.insert(name.ident.to_string(), join(&name.ident.to_string()));
        }
        UseTree::Rename(rename) => {
            out.insert(rename.rename.to_string(), join(&rename.ident.to_string()));
        }
        UseTree::Group(group) => {
            for tree in &group.items {
                imports(tree, prefix, out);
            }
        }
        UseTree::Glob(_) => {
            out.insert(format!("{prefix}::*"), format!("{prefix}::*"));
        }
    }
}

struct Guard<'a> {
    path: &'a str,
    policy: Policy,
    aliases: BTreeMap<String, String>,
    violations: Vec<Violation>,
}

impl Guard<'_> {
    fn reject(&mut self, code: &'static str, detail: impl Into<String>) {
        self.violations.push(Violation {
            code,
            path: self.path.into(),
            detail: detail.into(),
        });
    }
    fn resolved(&self, path: &syn::Path) -> String {
        let mut names = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string());
        let first = names.next().unwrap_or_default();
        let mut path = self.aliases.get(&first).cloned().unwrap_or(first);
        for name in names {
            path.push_str("::");
            path.push_str(&name);
        }
        path
    }
    fn check_path(&mut self, path: &str) {
        let sqlite = self.path.starts_with("crates/knowmesh-sqlite/");
        let core = self.path.starts_with("crates/knowmesh-core/");
        if path == "std::fs::*" {
            self.file_write(path);
        }
        if path.ends_with("transaction::*")
            && !self
                .policy
                .canonical_writer_users
                .iter()
                .any(|owner| owner == self.path)
        {
            self.reject("CANONICAL_WRITER_ACCESS", path);
        }
        if core
            && path.ends_with("ports::*")
            && !self
                .policy
                .projection_users
                .iter()
                .any(|owner| owner == self.path)
        {
            self.reject("PROJECTION_WRITE_CAPABILITY", path);
        }
        if ["rusqlite", "sqlx", "libsqlite3_sys"]
            .iter()
            .any(|root| path == *root || path.starts_with(&format!("{root}::")))
            && !sqlite
        {
            self.reject("DATABASE_DRIVER_ACCESS", path);
        }
        if (path == "knowmesh_sqlite" || path.starts_with("knowmesh_sqlite::"))
            && !self
                .policy
                .composition_roots
                .iter()
                .any(|owner| owner == self.path)
        {
            self.reject("DATABASE_ADAPTER_ACCESS", path);
        }
        if path.split("::").any(|part| part == "WorkspaceWriter")
            && !self
                .policy
                .canonical_writer_users
                .iter()
                .any(|owner| owner == self.path)
        {
            self.reject("CANONICAL_WRITER_ACCESS", path);
        }
        if core
            && path
                .split("::")
                .any(|part| ["ProjectionStore", "IndexStore", "ImpactStore"].contains(&part))
            && !self
                .policy
                .projection_users
                .iter()
                .any(|owner| owner == self.path)
        {
            self.reject("PROJECTION_WRITE_CAPABILITY", path);
        }
        if path.starts_with("std::process::Command")
            && !self
                .policy
                .process_users
                .iter()
                .any(|owner| owner == self.path)
        {
            self.reject("PROCESS_CAPABILITY", path);
        }
    }
    fn file_write(&mut self, operation: &str) {
        let allowed = self
            .policy
            .filesystem_writers
            .iter()
            .any(|owner| owner == self.path)
            || self
                .policy
                .filesystem_exceptions
                .get(self.path)
                .is_some_and(|operations| operations.iter().any(|allowed| allowed == operation));
        if !allowed {
            self.reject("UNREGISTERED_FILE_WRITE", operation);
        }
    }

    fn sql_write(&mut self, method: &str) {
        if self.path.starts_with("crates/knowmesh-sqlite/")
            && ["execute", "execute_batch", "pragma_update"].contains(&method)
            && !self
                .policy
                .sql_writers
                .iter()
                .any(|owner| owner == self.path)
        {
            self.reject("UNREGISTERED_SQL_WRITE", method);
        }
    }

    fn projection_write(&mut self) {
        if !self
            .policy
            .projection_users
            .iter()
            .chain(&self.policy.sql_writers)
            .any(|owner| owner == self.path)
        {
            self.reject("PROJECTION_WRITE_CAPABILITY", "reconcile");
        }
    }

    fn connection_type(&self, ty: &Type) -> bool {
        struct Finder<'a> {
            aliases: &'a BTreeMap<String, String>,
            found: bool,
        }
        impl<'ast> Visit<'ast> for Finder<'_> {
            fn visit_path(&mut self, path: &'ast syn::Path) {
                if path.segments.iter().any(|part| {
                    part.ident == "Connection"
                        || self
                            .aliases
                            .get(&part.ident.to_string())
                            .is_some_and(|name| name.ends_with("::Connection"))
                }) {
                    self.found = true;
                }
                visit::visit_path(self, path);
            }
        }
        let mut finder = Finder {
            aliases: &self.aliases,
            found: false,
        };
        finder.visit_type(ty);
        finder.found
    }

    fn check_return(&mut self, visibility: &Visibility, signature: &Signature) {
        if self.path.starts_with("crates/knowmesh-sqlite/")
            && matches!(visibility, Visibility::Public(_))
            && let ReturnType::Type(_, ty) = &signature.output
            && self.connection_type(ty)
        {
            self.reject("PUBLIC_MUTATOR_EXPOSURE", signature.ident.to_string());
        }
    }
}

impl<'ast> Visit<'ast> for Guard<'_> {
    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        self.check_return(&item.vis, &item.sig);
        visit::visit_item_fn(self, item);
    }
    fn visit_impl_item_fn(&mut self, item: &'ast ImplItemFn) {
        self.check_return(&item.vis, &item.sig);
        visit::visit_impl_item_fn(self, item);
    }
    fn visit_item(&mut self, item: &'ast Item) {
        let attrs = match item {
            Item::Mod(item) => &item.attrs,
            Item::Fn(item) => &item.attrs,
            Item::Use(item) => &item.attrs,
            _ => {
                visit::visit_item(self, item);
                return;
            }
        };
        if !test_only(attrs) {
            visit::visit_item(self, item);
        }
    }
    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        let mut names = BTreeMap::new();
        imports(&item.tree, "", &mut names);
        for (alias, path) in names {
            self.check_path(&path);
            self.aliases.insert(alias, path);
        }
    }
    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.check_path(&self.resolved(path));
        visit::visit_path(self, path);
    }
    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Expr::Path(path) = call.func.as_ref() {
            let path = self.resolved(&path.path);
            if path.rsplit("::").next() == Some("reconcile") {
                self.projection_write();
            }
            if path.starts_with("rusqlite::") {
                self.sql_write(path.rsplit("::").next().unwrap());
            }
            let file_function = path.strip_prefix("std::fs::").is_some_and(|name| {
                [
                    "write",
                    "copy",
                    "rename",
                    "create_dir",
                    "create_dir_all",
                    "remove_file",
                    "remove_dir",
                    "remove_dir_all",
                    "hard_link",
                    "set_permissions",
                    "File::create",
                    "File::create_new",
                    "File::options",
                    "OpenOptions::new",
                    "OpenOptions::default",
                ]
                .contains(&name)
            }) || path.starts_with("tempfile::NamedTempFile::")
                || path == "std::io::copy"
                || path.starts_with("std::os::unix::fs::symlink")
                || path.starts_with("std::os::windows::fs::symlink");
            if file_function {
                self.file_write(&path);
            }
        }
        visit::visit_expr_call(self, call);
    }
    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        let method = call.method.to_string();
        if [
            "write_all",
            "persist",
            "persist_noclobber",
            "set_len",
            "set_permissions",
        ]
        .contains(&method.as_str())
        {
            self.file_write(&method);
        }
        self.sql_write(&method);
        if method == "reconcile" {
            self.projection_write();
        }
        visit::visit_expr_method_call(self, call);
    }
    fn visit_item_struct(&mut self, item: &'ast ItemStruct) {
        if item.ident == "WorkspaceWriter" && matches!(item.vis, Visibility::Public(_)) {
            self.reject("PUBLIC_MUTATOR_EXPOSURE", "WorkspaceWriter");
        }
        for field in &item.fields {
            if self.path.starts_with("crates/knowmesh-sqlite/")
                && self.connection_type(&field.ty)
                && matches!(field.vis, Visibility::Public(_))
            {
                self.reject("PUBLIC_MUTATOR_EXPOSURE", "connection");
            }
        }
        visit::visit_item_struct(self, item);
    }
}

pub fn check_source(path: &str, source: &str) -> Vec<Violation> {
    let syntax = syn::parse_file(source).unwrap_or_else(|error| panic!("{path}: {error}"));
    let mut guard = Guard {
        path,
        policy: serde_json::from_str(include_str!("architecture-policy.json")).unwrap(),
        aliases: BTreeMap::new(),
        violations: vec![],
    };
    for item in &syntax.items {
        if let Item::Use(item) = item
            && !test_only(&item.attrs)
        {
            imports(&item.tree, "", &mut guard.aliases);
        }
    }
    guard.visit_file(&syntax);
    guard.violations
}

pub fn check_dependencies(metadata: &Value) -> Vec<Violation> {
    let mut violations = Vec::new();
    for package in metadata["packages"].as_array().unwrap() {
        let name = package["name"].as_str().unwrap();
        for dependency in package["dependencies"].as_array().unwrap() {
            let target = dependency["name"].as_str().unwrap();
            let forbidden = match name {
                "knowmesh-core" => [
                    "knowmesh",
                    "knowmesh-sqlite",
                    "axum",
                    "clap",
                    "rusqlite",
                    "sqlx",
                    "libsqlite3-sys",
                ]
                .contains(&target),
                "knowmesh-sqlite" => ["knowmesh", "axum", "clap"].contains(&target),
                "knowmesh" => ["rusqlite", "sqlx", "libsqlite3-sys"].contains(&target),
                _ => false,
            };
            if forbidden {
                violations.push(Violation {
                    code: "DEPENDENCY_DIRECTION",
                    path: name.into(),
                    detail: target.into(),
                });
            }
        }
    }
    violations
}

pub fn check_tree(package: &str, source: &Path) -> Vec<Violation> {
    let root = if source.join("lib.rs").exists() {
        source.join("lib.rs")
    } else {
        source.join("main.rs")
    };
    let mut violations = Vec::new();
    walk(
        package,
        source,
        &root,
        source,
        &mut BTreeSet::new(),
        &mut violations,
    );
    violations
}

fn walk(
    package: &str,
    root: &Path,
    file: &Path,
    module_dir: &Path,
    visited: &mut BTreeSet<std::path::PathBuf>,
    violations: &mut Vec<Violation>,
) {
    if !visited.insert(file.to_owned()) {
        return;
    }
    let source =
        fs::read_to_string(file).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
    let relative = file
        .strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    violations.extend(check_source(
        &format!("crates/{package}/src/{relative}"),
        &source,
    ));
    let syntax = syn::parse_file(&source).unwrap();
    walk_modules(
        package,
        root,
        file.parent().unwrap(),
        module_dir,
        &syntax.items,
        visited,
        violations,
    );
}

fn walk_modules(
    package: &str,
    root: &Path,
    file_dir: &Path,
    module_dir: &Path,
    items: &[Item],
    visited: &mut BTreeSet<std::path::PathBuf>,
    violations: &mut Vec<Violation>,
) {
    for item in items {
        let Item::Mod(module) = item else {
            continue;
        };
        if test_only(&module.attrs) {
            continue;
        }
        let name = module.ident.to_string();
        if let Some((_, items)) = &module.content {
            walk_modules(
                package,
                root,
                file_dir,
                &module_dir.join(&name),
                items,
                visited,
                violations,
            );
            continue;
        }
        let explicit = module.attrs.iter().find_map(|attr| {
            if !attr.path().is_ident("path") {
                return None;
            }
            if let Meta::NameValue(meta) = &attr.meta
                && let Expr::Lit(value) = &meta.value
                && let Lit::Str(path) = &value.lit
            {
                Some(file_dir.join(path.value()))
            } else {
                None
            }
        });
        let file = explicit.unwrap_or_else(|| {
            let flat = module_dir.join(format!("{name}.rs"));
            if flat.exists() {
                flat
            } else {
                module_dir.join(&name).join("mod.rs")
            }
        });
        let child_dir = if file.file_name().unwrap() == "mod.rs" {
            file.parent().unwrap().to_owned()
        } else {
            file.with_extension("")
        };
        walk(package, root, &file, &child_dir, visited, violations);
    }
}

pub fn check_workspace(root: &Path) -> Vec<Violation> {
    let output = Command::new(env!("CARGO"))
        .current_dir(root)
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--locked",
            "--offline",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut violations = check_dependencies(&metadata);
    for package in ["knowmesh-core", "knowmesh-sqlite", "knowmesh"] {
        violations.extend(check_tree(
            package,
            &root.join("crates").join(package).join("src"),
        ));
    }
    violations
}
