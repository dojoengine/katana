pub const MAX_RECURSION_DEPTH: usize = 1000;

pub const DEFAULT_INVOCATION_MAX_STEPS: u32 = 10_000_000;
pub const DEFAULT_VALIDATION_MAX_STEPS: u32 = 1_000_000;

#[cfg(feature = "native")]
pub const DEFAULT_ENABLE_NATIVE_COMPILATION: bool = false;

#[derive(Debug, Clone, PartialEq, Eq)]
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
