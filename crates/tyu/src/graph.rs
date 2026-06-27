//! Module dependency graph resolution.
//!
//! Parses `.mod` files to extract `import` declarations, resolves module
//! file paths using the same search order as `langc`, and produces a
//! topologically-sorted build order.

use crate::error::TyuError;
use frontend::parse::Parser;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

/// A resolved module node in the dependency graph.
#[derive(Clone, Debug)]
pub struct ModuleNode {
    pub name: String,
    /// Absolute path to the source `.mod` file.
    pub path: PathBuf,
    /// Whether this module is a library (no `main` word).
    pub is_lib: bool,
    /// Resolved file paths of modules that this module directly imports
    /// (only those that were found on disk — platform imports excluded).
    pub dep_paths: Vec<PathBuf>,
}

/// Resolve the full dependency graph starting from `main_path`.
///
/// Returns modules in **build order** (dependencies first), with the root
/// module last.  Diamond dependencies are resolved once.
pub fn resolve_graph(
    main_path: &Path,
    include_dirs: &[PathBuf],
    sysroot: Option<&Path>,
) -> Result<Vec<ModuleNode>, TyuError> {
    let main_abs = if main_path.is_absolute() {
        main_path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| TyuError::Graph(format!("current_dir: {}", e)))?
            .join(main_path)
    };

    let base_dir = main_abs
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    // Build search directories in langc order.
    let mut search_dirs: Vec<PathBuf> = Vec::new();
    search_dirs.push(base_dir.clone());
    search_dirs.extend(include_dirs.iter().cloned());
    if let Some(sr) = sysroot {
        search_dirs.push(sr.to_path_buf());
    }

    // Adjacency: module name -> set of dependency module names.
    let mut deps: HashMap<String, Vec<String>> = HashMap::new();
    // Module name -> path.
    let mut mod_paths: HashMap<String, PathBuf> = HashMap::new();

    // BFS/stack-based traversal to discover all modules.
    let mut pending: VecDeque<(PathBuf, PathBuf)> = VecDeque::new();
    pending.push_back((main_abs.clone(), base_dir));

    while let Some((mod_path, containing_dir)) = pending.pop_front() {
        let src_bytes = fs::read(&mod_path)
            .map_err(|e| TyuError::Graph(format!("reading '{}': {}", mod_path.display(), e)))?;
        // Box the ModuleAst to avoid ~156KB stack frame from FixedVec inline storage.
        let module = Box::new(Parser::new(&src_bytes).parse_module_ast().map_err(|e| {
            TyuError::Graph(format!(
                "parsing '{}': error {}",
                mod_path.display(),
                e.code()
            ))
        })?);

        let mod_name =
            String::from_utf8_lossy(&src_bytes[module.name.start..module.name.end]).to_string();

        // Skip if already processed.
        if deps.contains_key(&mod_name) {
            continue;
        }

        deps.entry(mod_name.clone()).or_insert_with(Vec::new);
        mod_paths
            .entry(mod_name.clone())
            .or_insert_with(|| mod_path.clone());

        // Process each import.
        for import in module.imports.iter() {
            let import_name =
                String::from_utf8_lossy(&src_bytes[import.module.start..import.module.end])
                    .to_string();

            // Record edge: mod_name -> import_name.
            deps.get_mut(&mod_name).unwrap().push(import_name.clone());

            // Try to find the import's file if not yet discovered.
            if !mod_paths.contains_key(&import_name) {
                let import_path = resolve_module_file(&import_name, &containing_dir, &search_dirs);
                match import_path {
                    Some(p) => {
                        let parent = p
                            .parent()
                            .map(|p| p.to_path_buf())
                            .unwrap_or_else(|| containing_dir.clone());
                        pending.push_back((p, parent));
                    }
                    None => {
                        // Platform modules (import platform/...) that are not
                        // found on disk are resolved by the sysroot at build
                        // time — skip silently (GR-5).
                        if import_name.starts_with("platform/") {
                            continue;
                        }
                        // Non-platform modules that are neither .mod nor .def
                        // are genuine missing-user-module errors (GR-6).
                        return Err(TyuError::Graph(format!(
                            "module '{}' imported by '{}' not found",
                            import_name, mod_name,
                        ))
                        .into());
                    }
                }
            }
        }
    }

    // Topological sort (Kahn's algorithm).
    // in_degree[node] = number of dependencies (things it imports) within our graph.
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    for (name, _) in &deps {
        in_degree.insert(name.as_str(), 0);
    }
    for (name, edges) in &deps {
        for dep in edges {
            if deps.contains_key(dep.as_str()) {
                // `name` depends on `dep` → name's in_degree increases.
                *in_degree.get_mut(name.as_str()).unwrap() += 1;
            }
        }
    }

    // dependents[module] = list of modules that import this module
    // (reverse edges for efficient decrement during Kahn's).
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
    for (name, edges) in &deps {
        for dep in edges {
            if deps.contains_key(dep.as_str()) {
                dependents
                    .entry(dep.as_str())
                    .or_insert_with(Vec::new)
                    .push(name.as_str());
            }
        }
    }

    // Start with modules that have no dependencies (in_degree == 0).
    let mut zero_in: VecDeque<&str> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(name, _)| *name)
        .collect();

    let mut order: Vec<ModuleNode> = Vec::new();
    let mut sorted_set: HashSet<&str> = HashSet::new();

    while let Some(name) = zero_in.pop_front() {
        if sorted_set.contains(name) {
            continue;
        }
        sorted_set.insert(name);

        if let Some(path) = mod_paths.get(name) {
            let is_root = path == &main_abs;
            let is_lib = !is_root;

            let dep_paths: Vec<PathBuf> = deps
                .get(name)
                .map(|dep_names| {
                    dep_names
                        .iter()
                        .filter_map(|dn| mod_paths.get(dn.as_str()))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();

            order.push(ModuleNode {
                name: name.to_string(),
                path: path.clone(),
                is_lib,
                dep_paths,
            });
        }

        // Decrement in-degree of modules that depend on `name`.
        if let Some(deps_of) = dependents.get(name) {
            for &dep_name in deps_of {
                if let Some(deg) = in_degree.get_mut(dep_name) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        zero_in.push_back(dep_name);
                    }
                }
            }
        }
    }

    if sorted_set.len() != deps.len() {
        return Err(
            TyuError::Graph("circular dependency detected in module graph".to_string()).into(),
        );
    }

    Ok(order)
}

/// Resolve a module name to a file path by searching in order.
/// Returns the first `.mod` file found.  Tries `.mod` first, then `.def`.
fn resolve_module_file(
    module_name: &str,
    containing_dir: &Path,
    search_dirs: &[PathBuf],
) -> Option<PathBuf> {
    // First search the containing directory of the importing module.
    for candidate in &[containing_dir.to_path_buf()] {
        let p = candidate.join(format!("{}.mod", module_name));
        if p.is_file() {
            return Some(p);
        }
        let p = candidate.join(format!("{}.def", module_name));
        if p.is_file() {
            return Some(p);
        }
    }

    // Then search include dirs + sysroot.
    for dir in search_dirs {
        let p = dir.join(format!("{}.mod", module_name));
        if p.is_file() {
            return Some(p);
        }
        let p = dir.join(format!("{}.def", module_name));
        if p.is_file() {
            return Some(p);
        }
    }

    None
}
