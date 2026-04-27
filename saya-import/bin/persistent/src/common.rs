pub const SAYA_DB_PATH: &str = "saya.db";

// All time values are in seconds
const SNOS_PROOF_GENERATION_TIME: u32 = 15 * 60;
const LAYOUT_BRIDGE_PROOF_GENERATION_TIME: u32 = 30 * 60;
const PIE_GENERATION_TIME: u32 = 60;

// Number of stages in the process
pub const NUMBER_OF_STAGES: usize = 3;

/// Calculates the number of workers required for each stage of the pipeline.
///
/// The distribution is based on the relative proof generation times of each stage,
/// ensuring a proportional allocation of workers.
///
/// # Parameters
/// - `num_blocks_in_pipeline`: The total number of blocks that need processing in the pipeline.
///
/// # Returns
/// An array of `NUMBER_OF_STAGES` elements where each entry represents the number
/// of workers allocated to that stage.
///
/// # Logic
/// - The total proof generation time is calculated as the sum of all individual stage times.
/// - Each stage is assigned a proportion of workers based on its fraction of the total time.
/// - The number of workers per stage is computed using `ceil` to avoid fractional assignments
///   and ensuring that each step has at least one worker.
pub fn calculate_workers_per_stage(num_blocks_in_pipeline: usize) -> [usize; NUMBER_OF_STAGES] {
    let total_time =
        SNOS_PROOF_GENERATION_TIME + LAYOUT_BRIDGE_PROOF_GENERATION_TIME + PIE_GENERATION_TIME;
    let mut workers_count: [usize; NUMBER_OF_STAGES] = [0; NUMBER_OF_STAGES];
    for (i, workers) in workers_count.iter_mut().enumerate() {
        let weight = match i {
            0 => SNOS_PROOF_GENERATION_TIME as f64 / total_time as f64,
            1 => LAYOUT_BRIDGE_PROOF_GENERATION_TIME as f64 / total_time as f64,
            2 => PIE_GENERATION_TIME as f64 / total_time as f64,
            _ => 0.0,
        };
        *workers = (num_blocks_in_pipeline as f64 * weight).ceil() as usize;
    }

    workers_count
}
#[test]
fn test_split_workers() {
    let num_blocks_in_pipeline = 110;
    let workers = calculate_workers_per_stage(num_blocks_in_pipeline);
    assert_eq!(workers, [36, 72, 3]);
}
