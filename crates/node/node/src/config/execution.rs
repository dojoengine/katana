pub use katana_node_defaults::execution::*;

#[derive(Debug, Clone)]
pub struct ExecutionConfig {
    pub invocation_max_steps: u32,
    pub validation_max_steps: u32,
    pub max_recursion_depth: usize,
    #[cfg(feature = "native")]
    pub compile_native: bool,
}

impl std::default::Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            max_recursion_depth: MAX_RECURSION_DEPTH,
            invocation_max_steps: DEFAULT_INVOCATION_MAX_STEPS,
            validation_max_steps: DEFAULT_VALIDATION_MAX_STEPS,
            #[cfg(feature = "native")]
            compile_native: DEFAULT_ENABLE_NATIVE_COMPILATION,
        }
    }
}
