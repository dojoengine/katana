use std::sync::{Arc, OnceLock};

use blockifier::execution::contract_class::{CompiledClassV1, RunnableCompiledClass};
use katana_primitives::class::{ClassHash, CompiledClass, ContractClass};
use quick_cache::sync::Cache;

use super::utils::to_class;

static COMPILED_CLASS_CACHE: OnceLock<ClassCache> = OnceLock::new();

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("global class cache already initialized.")]
    AlreadyInitialized,

    #[error("global class cache not initialized.")]
    NotInitialized,

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

    /// Builds a new `ClassCache` instance and sets it as the global cache.
    ///
    /// This builds and initializes a global `ClassCache` that can be accessed via
    /// [`ClassCache::global`].
    ///
    /// ## Errors
    ///
    /// Returns an error if the global cache has already been initialized.
    pub fn build_global(self) -> Result<ClassCache, Error> {
        let cache = self.build()?;
        COMPILED_CLASS_CACHE.set(cache.clone()).map_err(|_| Error::AlreadyInitialized)?;
        Ok(cache)
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

/// Cache for compiled contract classes.
///
/// ## Cairo Native
///
/// When native compilation is enabled, every (non-legacy) class that gets inserted into the cache
/// will trigger an asynchronous compilation process for compiling the class into native
/// code using `cairo-native`. Once the compilation is done, the current cache entry for the class
/// will be replaced with the native-compiled variant. This process won't block the cache
/// operations.
#[derive(Debug, Clone)]
pub struct ClassCache {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    /// Threadpool for compiling the contract classes into native machine code.
    ///
    /// The threadpool would ONLY ever be present if native compilation is enabled when
    /// building the cache via [`ClassCacheBuilder::compile_native`].
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

    /// Returns a reference to the global cache instance.
    ///
    /// This method will return an error if the global cache has not been initialized via
    /// [`ClassCacheBuilder::build_global`] first.
    pub fn try_global() -> Result<ClassCache, Error> {
        COMPILED_CLASS_CACHE.get().cloned().ok_or(Error::NotInitialized)
    }

    /// Returns a reference to the global cache instance.
    ///
    /// # Panics
    ///
    /// Panics if the global cache has not been initialized.
    pub fn global() -> ClassCache {
        Self::try_global().expect("global class cache not initialized").clone()
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

                let version = class.sierra_version();

                let CompiledClass::Class(casm) = class.compile().unwrap() else {
                    unreachable!("cant be legacy")
                };

                let compiled = CompiledClassV1::try_from((casm, version.clone())).unwrap();

                #[cfg(feature = "native")]
                if let Some(pool) = self.inner.pool.as_ref() {
                    let inner = self.inner.clone();
                    let compiled_clone = compiled.clone();

                    pool.spawn(move || {
                        let span = tracing::trace_span!(target: "class_cache", "compile_native_class", class = format!("{hash:#x}"));
                        let _span = span.enter();

                        let executor =
                            AotContractExecutor::new(&program, &entry_points, version.into(), OptLevel::Default)
                                .inspect_err(|error| tracing::error!(target: "class_cache", %error, "Failed to compile native class"))
                                .unwrap();

                        let native = NativeCompiledClassV1::new(executor, compiled_clone);
                        inner.cache.insert(hash, RunnableCompiledClass::V1Native(native));
                    });
                }

                let class = RunnableCompiledClass::V1(compiled);
                self.inner.cache.insert(hash, class.clone());

                class
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use katana_contracts::contracts;
    use katana_primitives::felt;

    use super::{ClassCache, ClassCacheBuilder, Error};

    #[test]
    fn independent_cache() {
        let cache1 = ClassCacheBuilder::new().build().expect("Failed to build cache 1");
        let cache2 = ClassCacheBuilder::new().build().expect("Failed to build cache 2");

        let class_hash1 = felt!("0x1");
        let class_hash2 = felt!("0x2");

        cache1.insert(class_hash1, contracts::Account::CLASS.clone());
        cache1.insert(class_hash2, contracts::UniversalDeployer::CLASS.clone());

        assert!(cache1.get(&class_hash1).is_some());
        assert!(cache1.get(&class_hash2).is_some());
        assert!(cache2.get(&class_hash1).is_none());
        assert!(cache2.get(&class_hash2).is_none());
    }

    #[test]
    fn global_cache() {
        // Can't get global without initializing it first
        let error = ClassCache::try_global().unwrap_err();
        assert_matches!(error, Error::NotInitialized, "Global cache not initialized");

        let cache1 = ClassCacheBuilder::new().build_global().expect("failed to build global cache");

        let error = ClassCacheBuilder::new().build_global().unwrap_err();
        assert_matches!(error, Error::AlreadyInitialized, "Global cache already initialized");

        // Check that calling ClassCache::global() returns the same instance as cache1
        let cache2 = ClassCache::global();

        let class_hash1 = felt!("0x1");
        let class_hash2 = felt!("0x2");

        cache1.insert(class_hash1, contracts::Account::CLASS.clone());
        cache1.insert(class_hash2, contracts::UniversalDeployer::CLASS.clone());

        assert!(cache1.get(&class_hash1).is_some());
        assert!(cache1.get(&class_hash2).is_some());
        assert!(cache2.get(&class_hash1).is_some());
        assert!(cache2.get(&class_hash2).is_some());
    }
}
