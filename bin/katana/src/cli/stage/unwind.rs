use anyhow::Result;
use clap::Args;
use katana_primitives::block::BlockNumber;
use katana_provider::api::stage::StageCheckpointProvider;
use katana_provider::providers::db::DbProvider;
use katana_stage::Stage;

use crate::cli::db::open_db_rw;

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq))]
pub struct UnwindArgs {
    /// The stage ID to unwind
    #[arg(value_name = "STAGE_ID")]
    stage_id: String,

    /// The stage ID to unwind to
    #[arg(value_name = "UNWIND_TO")]
    unwind_to: BlockNumber,

    /// Path to the database directory.
    #[arg(short, long)]
    path: String,
}

impl UnwindArgs {
    pub async fn execute(self) -> Result<()> {
        use katana_stage::StateTrie;

        let provider = DbProvider::new(open_db_rw(&self.path)?);
        let mut stage = StateTrie::new(&provider);

        stage.unwind(self.unwind_to).await?;
        provider.set_checkpoint(stage.id(), self.unwind_to)?;

        Ok(())
    }
}
