use katana_executor::BlockLimits;

/// Configurations related to block production.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SequencingConfig {
    /// The block production mode.
    pub mining: MiningMode,

    /// The maximum number of Cairo steps in a block.
    //
    /// The block will automatically be closed when the accumulated Cairo steps across all the
    /// transactions has reached this limit.
    ///
    /// NOTE: This only affect interval block production.
    ///
    /// See <https://docs.starknet.io/chain-info/#current_limits>.
    pub block_cairo_steps_limit: Option<u64>,
}

impl SequencingConfig {
    pub fn block_limits(&self) -> BlockLimits {
        BlockLimits { cairo_steps: self.block_cairo_steps_limit.unwrap_or(50_000_000) }
    }
}

/// The mode of block production.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum MiningMode {
    /// Blocks are produced instantly when a transaction is received.
    #[default]
    Instant,

    /// Blocks are produced at fixed intervals (in milliseconds).
    Interval(u64),

    /// No automatic block production.
    ///
    /// Blocks are only produced manually.
    Manual,
}
