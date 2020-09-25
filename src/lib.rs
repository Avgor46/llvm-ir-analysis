//! This crate provides various analyses of LLVM IR, such as control-flow
//! graphs, dominator trees, control dependence graphs, etc.
//!
//! For a more thorough introduction to the crate and how to get started,
//! see the [crate's README](https://github.com/cdisselkoen/llvm-ir-analysis/blob/master/README.md).

mod call_graph;
mod control_dep_graph;
mod control_flow_graph;
mod dominator_tree;
mod functions_by_type;

pub use crate::call_graph::CallGraph;
pub use crate::control_dep_graph::ControlDependenceGraph;
pub use crate::control_flow_graph::{CFGNode, ControlFlowGraph};
pub use crate::dominator_tree::{DominatorTree, PostDominatorTree};
pub use crate::functions_by_type::FunctionsByType;
use llvm_ir::{Function, Module};
use log::debug;
use std::cell::{Ref, RefCell};
use std::collections::HashMap;
use std::hash::Hash;

/// Computes (and caches the results of) various analyses on a given `Module` or set of `Module`s.
pub struct Analysis<'m> {
    /// Reference to the `llvm-ir` `Module`s
    modules: Vec<&'m Module>,
    /// Call graph
    call_graph: SimpleCache<CallGraph<'m>>,
    /// `FunctionsByType`, which allows you to iterate over functions by type
    functions_by_type: SimpleCache<FunctionsByType<'m>>,
    /// Map from function name to the `ControlFlowGraph` for that function
    control_flow_graphs: MappingCache<&'m str, ControlFlowGraph<'m>>,
    /// Map from function name to the `DominatorTree` for that function
    dominator_trees: MappingCache<&'m str, DominatorTree<'m>>,
    /// Map from function name to the `PostDominatorTree` for that function
    postdominator_trees: MappingCache<&'m str, PostDominatorTree<'m>>,
    /// Map from function name to the `ControlDependenceGraph` for that function
    control_dep_graphs: MappingCache<&'m str, ControlDependenceGraph<'m>>,
}

impl<'m> Analysis<'m> {
    /// Create a new `Analysis` for the given `Module`.
    ///
    /// This method itself is cheap; individual analyses will be computed lazily
    /// on demand.
    pub fn new(module: &'m Module) -> Self {
        Self {
            modules: vec![module],
            call_graph: SimpleCache::new(),
            functions_by_type: SimpleCache::new(),
            control_flow_graphs: MappingCache::new(),
            dominator_trees: MappingCache::new(),
            postdominator_trees: MappingCache::new(),
            control_dep_graphs: MappingCache::new(),
        }
    }

    /// Create a new `Analysis` for the given set of `Module`s.
    ///
    /// This method itself is cheap; individual analyses will be computed lazily
    /// on demand.
    pub fn new_multi_module(modules: impl IntoIterator<Item = &'m Module>) -> Self {
        Self {
            modules: modules.into_iter().collect(),
            call_graph: SimpleCache::new(),
            functions_by_type: SimpleCache::new(),
            control_flow_graphs: MappingCache::new(),
            dominator_trees: MappingCache::new(),
            postdominator_trees: MappingCache::new(),
            control_dep_graphs: MappingCache::new(),
        }
    }

    /// Iterate over the analyzed `Module`(s).
    fn modules<'s>(&'s self) -> impl Iterator<Item = &'m Module> + 's {
        self.modules.iter().copied()
    }

    /// Get the `CallGraph` for the `Module`(s).
    pub fn call_graph(&self) -> Ref<CallGraph<'m>> {
        self.call_graph.get_or_insert_with(|| {
            let functions_by_type = self.functions_by_type();
            debug!("computing call graph");
            CallGraph::new(self.modules(), &functions_by_type)
        })
    }

    /// Get the `FunctionsByType` for the `Module`(s).
    pub fn functions_by_type(&self) -> Ref<FunctionsByType<'m>> {
        self.functions_by_type.get_or_insert_with(|| {
            debug!("computing functions-by-type");
            FunctionsByType::new(self.modules())
        })
    }

    /// Get the `ControlFlowGraph` for the function with the given name.
    ///
    /// Panics if no function of that name exists in the `Module`(s)
    /// which the `Analysis` was created with.
    pub fn control_flow_graph(&self, func_name: &'m str) -> Ref<ControlFlowGraph<'m>> {
        self.control_flow_graphs.get_or_insert_with(&func_name, || {
            let (func, _) = self.get_func_by_name(func_name)
                .unwrap_or_else(|| panic!("Function named {:?} not found in the Module(s)", func_name));
            debug!("computing control flow graph for {}", func_name);
            ControlFlowGraph::new(func)
        })
    }

    /// Get the `DominatorTree` for the function with the given name.
    ///
    /// Panics if no function of that name exists in the `Module`(s)
    /// which the `Analysis` was created with.
    pub fn dominator_tree(&self, func_name: &'m str) -> Ref<DominatorTree<'m>> {
        self.dominator_trees.get_or_insert_with(&func_name, || {
            let cfg = self.control_flow_graph(func_name);
            debug!("computing dominator tree for {}", func_name);
            DominatorTree::new(&cfg)
        })
    }

    /// Get the `PostDominatorTree` for the function with the given name.
    ///
    /// Panics if no function of that name exists in the `Module`(s)
    /// which the `Analysis` was created with.
    pub fn postdominator_tree(&self, func_name: &'m str) -> Ref<PostDominatorTree<'m>> {
        self.postdominator_trees.get_or_insert_with(&func_name, || {
            let cfg = self.control_flow_graph(func_name);
            debug!("computing postdominator tree for {}", func_name);
            PostDominatorTree::new(&cfg)
        })
    }

    /// Get the `ControlDependenceGraph` for the function with the given name.
    ///
    /// Panics if no function of that name exists in the `Module`(s)
    /// which the `Analysis` was created with.
    pub fn control_dependence_graph(&self, func_name: &'m str) -> Ref<ControlDependenceGraph<'m>> {
        self.control_dep_graphs.get_or_insert_with(&func_name, || {
            let cfg = self.control_flow_graph(func_name);
            let postdomtree = self.postdominator_tree(func_name);
            debug!("computing control dependence graph for {}", func_name);
            ControlDependenceGraph::new(&cfg, &postdomtree)
        })
    }

    /// Get the `Function` with the given name from the analyzed `Module`(s).
    ///
    /// Returns both the `Function` and the `Module` it was found in, or `None`
    /// if no function was found with that name.
    pub fn get_func_by_name(&self, func_name: &str) -> Option<(&'m Function, &'m Module)> {
        let mut retval = None;
        for &module in &self.modules {
            if let Some(func) = module.get_func_by_name(func_name) {
                match retval {
                    None => retval = Some((func, module)),
                    Some((_, retmod)) => panic!("Multiple functions found with name {:?}: one in module {:?}, another in module {:?}", func_name, &retmod.name, &module.name),
                }
            }
        }
        retval
    }
}

struct SimpleCache<T> {
    /// `None` if not computed yet
    data: RefCell<Option<T>>,
}

impl<T> SimpleCache<T> {
    fn new() -> Self {
        Self {
            data: RefCell::new(None),
        }
    }

    /// Get the cached value, or if no value is cached, compute the value using
    /// the given closure, then cache that result and return it
    fn get_or_insert_with(&self, f: impl FnOnce() -> T) -> Ref<T> {
        // borrow mutably only if it's empty. else don't even try to borrow mutably
        let need_mutable_borrow = self.data.borrow().is_none();
        if need_mutable_borrow {
            let old_val = self.data.borrow_mut().replace(f());
            debug_assert!(old_val.is_none());
        }
        // now, either way, it's populated, so we borrow immutably and return.
        // future users can also borrow immutably using this function (even
        // while this borrow is still outstanding), since it won't try to borrow
        // mutably in the future.
        Ref::map(self.data.borrow(), |o| {
            o.as_ref().expect("should be populated now")
        })
    }
}

struct MappingCache<K, V> {
    /// The hashmap starts empty and is populated on demand
    map: RefCell<HashMap<K, V>>,
}

impl<K: Eq + Hash + Clone, V> MappingCache<K, V> {
    fn new() -> Self {
        Self {
            map: RefCell::new(HashMap::new()),
        }
    }

    /// Get the cached value for the given key, or if no value is cached for that
    /// key, compute the value using the given closure, then cache that result
    /// and return it
    fn get_or_insert_with(&self, key: &K, f: impl FnOnce() -> V) -> Ref<V> {
        // borrow mutably only if the entry is missing.
        // else don't even try to borrow mutably
        let need_mutable_borrow = !self.map.borrow().contains_key(key);
        if need_mutable_borrow {
            let old_val = self.map.borrow_mut().insert(key.clone(), f());
            debug_assert!(old_val.is_none());
        }
        // now, either way, the entry is populated, so we borrow immutably and
        // return. future users can also borrow immutably using this function
        // (even while this borrow is still outstanding), since it won't try to
        // borrow mutably in the future.
        Ref::map(self.map.borrow(), |map| {
            map.get(&key).expect("should be populated now")
        })
    }
}
