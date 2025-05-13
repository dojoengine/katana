use std::str::FromStr;
use std::sync::{Arc, LazyLock};

use blockifier::execution::contract_class::{CompiledClassV1, RunnableCompiledClass};
use katana_primitives::class::{ClassHash, CompiledClass, ContractClass};
use quick_cache::sync::Cache;
use starknet_api::contract_class::SierraVersion;

use super::utils::to_class;

pub static COMPILED_CLASS_CACHE: LazyLock<ClassCache> =
    LazyLock::new(|| ClassCache::builder().build().unwrap());

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[cfg(feature = "native")]
    #[error(transparent)]
    FailedToCreateThreadPool(#[from] rayon::ThreadPoolBuildError),
}

#[derive(Debug, Clone)]
pub struct ClassCache {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    #[cfg(feature = "native")]
    pool: rayon::ThreadPool,
    cache: Cache<ClassHash, RunnableCompiledClass>,
}

/// Builder for configuring and creating a `ClassCache` instance.
///
/// This builder allows for customizing various aspects of the `ClassCache`,
/// such as the cache size and thread pool settings (when the "native" feature is enabled).
#[derive(Debug, Clone)]
pub struct ClassCacheBuilder {
    cache_size: usize,
    #[cfg(feature = "native")]
    thread_count: usize,
    #[cfg(feature = "native")]
    thread_name_pattern: String,
}

impl Default for ClassCacheBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ClassCacheBuilder {
    /// Creates a new `ClassCacheBuilder` with default settings.
    ///
    /// Default values:
    /// - Cache size: 100 entries
    /// - Thread count: 3 threads (when "native" feature is enabled)
    /// - Thread name pattern: "cache-native-compiler-{i}" (when "native" feature is enabled)
    pub fn new() -> Self {
        Self {
            cache_size: 100,
            #[cfg(feature = "native")]
            thread_count: 3,
            #[cfg(feature = "native")]
            thread_name_pattern: "cache-native-compiler-{i}".to_string(),
        }
    }

    /// Sets the maximum number of entries in the class cache.
    ///
    /// # Arguments
    ///
    /// * `size` - The maximum number of compiled classes to store in the cache.
    pub fn with_cache_size(mut self, size: usize) -> Self {
        self.cache_size = size;
        self
    }

    /// Sets the number of threads in the thread pool for native compilation.
    ///
    /// This method is only available when the "native" feature is enabled.
    ///
    /// # Arguments
    ///
    /// * `count` - The number of threads to use for native compilation.
    #[cfg(feature = "native")]
    pub fn with_thread_count(mut self, count: usize) -> Self {
        self.thread_count = count;
        self
    }

    /// Sets the thread name pattern for the native compilation thread pool.
    ///
    /// The pattern can include "{i}" which will be replaced with the thread index.
    /// This method is only available when the "native" feature is enabled.
    ///
    /// # Arguments
    ///
    /// * `pattern` - The pattern to use for thread names.
    #[cfg(feature = "native")]
    pub fn with_thread_name_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.thread_name_pattern = pattern.into();
        self
    }

    /// Builds a new `ClassCache` instance with the configured settings.
    ///
    /// # Returns
    ///
    /// A `Result` containing either the constructed `ClassCache` or an `Error`
    /// if the thread pool could not be created.
    pub fn build(self) -> Result<ClassCache, Error> {
        let cache = Cache::new(self.cache_size);

        #[cfg(feature = "native")]
        let thread_name_pattern = self.thread_name_pattern.clone();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.thread_count)
            .thread_name(move |i| thread_name_pattern.replace("{i}", &i.to_string()))
            .build()?;

        Ok(ClassCache {
            inner: Arc::new(Inner {
                cache,
                #[cfg(feature = "native")]
                pool,
            }),
        })
    }
}

impl ClassCache {
    pub fn new() -> Result<Self, Error> {
        Self::builder().build()
    }

    /// Returns a new `ClassCacheBuilder` for configuring a `ClassCache` instance.
    ///
    /// This method provides a builder pattern interface for creating a `ClassCache`
    /// with custom configuration options.
    pub fn builder() -> ClassCacheBuilder {
        ClassCacheBuilder::new()
    }

    pub fn get(&self, hash: &ClassHash) -> Option<RunnableCompiledClass> {
        self.inner.cache.get(hash)
    }

    pub fn insert(&self, hash: ClassHash, class: ContractClass) -> RunnableCompiledClass {
        match class {
            ContractClass::Legacy(..) => {
                let class = class.compile().unwrap();
                let class = to_class(class).unwrap();
                self.inner.cache.insert(hash, class.clone());
                class
            }

            #[allow(unused_variables)]
            ContractClass::Class(ref sierra) => {
                #[cfg(feature = "native")]
                use blockifier::execution::native::contract_class::NativeCompiledClassV1;
                #[cfg(feature = "native")]
                use cairo_native::executor::AotContractExecutor;
                #[cfg(feature = "native")]
                use cairo_native::OptLevel;

                #[cfg(feature = "native")]
                let program = sierra.extract_sierra_program().unwrap();
                #[cfg(feature = "native")]
                let entry_points = sierra.entry_points_by_type.clone();

                let CompiledClass::Class(casm) = class.compile().unwrap() else {
                    unreachable!("cant be legacy")
                };

                let version = SierraVersion::from_str(&casm.compiler_version).unwrap();
                let compiled = CompiledClassV1::try_from((casm, version.clone())).unwrap();

                #[cfg(feature = "native")]
                let inner = self.inner.clone();
                #[cfg(feature = "native")]
                let compiled_clone = compiled.clone();

                #[cfg(feature = "native")]
                self.inner.pool.spawn(move || {
                    tracing::trace!(target: "class_cache", class = format!("{hash:#x}"), "Compiling native class");

                    let executor =
                        AotContractExecutor::new(&program, &entry_points, version.into(), OptLevel::Default)
                            .unwrap();

                    let native = NativeCompiledClassV1::new(executor, compiled_clone);
                    inner.cache.insert(hash, RunnableCompiledClass::V1Native(native));

                    tracing::trace!(target: "class_cache", class = format!("{hash:#x}"), "Native class compiled")
                });

                let class = RunnableCompiledClass::V1(compiled);
                self.inner.cache.insert(hash, class.clone());

                class
            }
        }
    }
}
