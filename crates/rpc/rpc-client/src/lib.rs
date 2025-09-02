pub mod starknet;

#[derive(Debug, thiserror::Error)]
pub enum Error<T> {
    /// API-specific error returned by the server.
    #[error(transparent)]
    Api(T),

    /// Transport or other client-level error.
    #[error(transparent)]
    Client(jsonrpsee::core::client::Error),
}

impl<T> From<jsonrpsee::core::client::Error> for Error<T>
where
    T: std::error::Error + 'static,
    T: From<jsonrpsee::types::ErrorObjectOwned>,
{
    fn from(err: jsonrpsee::core::client::Error) -> Self {
        match err {
            jsonrpsee::core::client::Error::Call(err_obj) => Error::Api(T::from(err_obj)),
            _ => Error::Client(err),
        }
    }
}
