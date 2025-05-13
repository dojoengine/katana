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

/// Builder for configuring and creating a `ClassCache` instance.
///
/// This builder allows for customizing various aspects of the `ClassCache`,
/// such as the cache size and thread pool settings (when the "native" feature is enabled).
pub struct ClassCacheBuilder {
    size: usize,
    #[cfg(feature = "native")]
    compile_native: bool,
    #[cfg(feature = "native")]
    thread_count: usize,
    #[cfg(feature = "native")]
    thread_name: Option<Box<dyn Fn(usize) -> String + Send + Sync + 'static>>,
}

///////////////////////////////////////////////////////////////
// ClassCacheBuilder implementations
///////////////////////////////////////////////////////////////

impl ClassCacheBuilder {
    /// Creates a new `ClassCacheBuilder` with default settings.
    pub fn new() -> Self {
        Self {
            size: 100,
            #[cfg(feature = "native")]
            compile_native: false,
            #[cfg(feature = "native")]
            thread_count: 3,
            #[cfg(feature = "native")]
            thread_name: None,
        }
    }

    /// Sets the maximum number of entries in the class cache. Default is 100.
    ///
    /// # Arguments
    ///
    /// * `size` - The maximum number of compiled classes to store in the cache.
    pub fn size(mut self, size: usize) -> Self {
        self.size = size;
        self
    }

    /// Enables or disables native compilation. Default is disabled.
    #[cfg(feature = "native")]
    pub fn compile_native(mut self, enable: bool) -> Self {
        self.compile_native = enable;
        self
    }

    /// Sets the number of threads in the thread pool for native compilation. Default is 3.
    ///
    /// If `count` is zero, the thread pool will choose the number of threads
    /// automatically. This is typically based on the number of logical CPUs
    /// available to the process. However, the exact behavior depends on the
    /// underlying Rayon's [`ThreadPool`](rayon::ThreadPool) implementation.
    ///
    /// If native compilation is not enabled via [`ClassCacheBuilder::compile_native`],
    /// configuring the thread pool is a no-op.
    #[cfg(feature = "native")]
    pub fn thread_count(mut self, count: usize) -> Self {
        self.thread_count = count;
        self
    }

    /// Sets the thread name for the native compilation thread pool.
    ///
    /// If native compilation is not enabled via [`ClassCacheBuilder::compile_native`],
    /// configuring the thread pool is a no-op.
    ///
    /// # Arguments
    ///
    /// * `name_fn` - A closure that takes a thread index and returns a name for the thread.
    #[cfg(feature = "native")]
    pub fn thread_name<F>(mut self, name_fn: F) -> Self
    where
        F: Fn(usize) -> String + Send + Sync + 'static,
    {
        self.thread_name = Some(Box::new(name_fn));
        self
    }

    /// Builds a new `ClassCache` instance with the configured settings.
    ///
    /// # Returns
    ///
    /// A `Result` containing either the constructed `ClassCache` or an `Error`
    /// if the thread pool could not be created.
    pub fn build(self) -> Result<ClassCache, Error> {
        let cache = Cache::new(self.size);

        #[cfg(feature = "native")]
        let pool = if self.compile_native {
            let builder = rayon::ThreadPoolBuilder::new().num_threads(self.thread_count);
            let default_thread_name = Box::new(|i| format!("cache-native-compiler-{i}")) as _;
            let thread_name = self.thread_name.unwrap_or(default_thread_name);
            Some(builder.thread_name(thread_name).build()?)
        } else {
            None
        };

        Ok(ClassCache {
            inner: Arc::new(Inner {
                cache,
                #[cfg(feature = "native")]
                pool,
            }),
        })
    }
}

impl std::fmt::Debug for ClassCacheBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        #[cfg(not(feature = "native"))]
        {
            f.debug_struct("ClassCacheBuilder").field("size", &self.size).finish()
        }

        #[cfg(feature = "native")]
        {
            f.debug_struct("ClassCacheBuilder")
                .field("size", &self.size)
                .field("compile_native", &self.compile_native)
                .field("thread_count", &self.thread_count)
                .field("thread_name", &"..")
                .finish()
        }
    }
}

impl Default for ClassCacheBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct ClassCache {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    #[cfg(feature = "native")]
    pool: Option<rayon::ThreadPool>,
    cache: Cache<ClassHash, RunnableCompiledClass>,
}

///////////////////////////////////////////////////////////////
// ClassCache implementations
///////////////////////////////////////////////////////////////

impl ClassCache {
    /// Creates a new [`ClassCache`] with default configurations.
    pub fn new() -> Result<Self, Error> {
        Self::builder().build()
    }

    /// Returns a new [`ClassCacheBuilder`] for configuring a `ClassCache` instance.
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
                if let Some(pool) = self.inner.pool.as_ref() {
                    let inner = self.inner.clone();
                    let compiled_clone = compiled.clone();

                    pool.spawn(move || {
                        tracing::trace!(target: "class_cache", class = format!("{hash:#x}"), "Compiling native class");

                        let executor =
                            AotContractExecutor::new(&program, &entry_points, version.into(), OptLevel::Default)
                                .unwrap();

                        let native = NativeCompiledClassV1::new(executor, compiled_clone);
                        inner.cache.insert(hash, RunnableCompiledClass::V1Native(native));

                        tracing::trace!(target: "class_cache", class = format!("{hash:#x}"), "Native class compiled")
                    });
                }

                let class = RunnableCompiledClass::V1(compiled);
                self.inner.cache.insert(hash, class.clone());

                class
            }
        }
    }
}
