const SNOS_PROOF_GENERATION_TIME: u32 = 20 * 60; // 20 minutes
const LAYOUT_BRIDGE_PROOF_GENERATION_TIME: u32 = 36 * 60; // 36 minutes
const PIE_GENERATION_TIME: u32 = 60; // 1 minute
const NUMBER_OF_STAGES: usize = 3;

pub fn calculate_workers_per_stage(num_blocks_in_pipeline: usize) -> [usize; NUMBER_OF_STAGES] {
    let total_time =
        SNOS_PROOF_GENERATION_TIME + LAYOUT_BRIDGE_PROOF_GENERATION_TIME + PIE_GENERATION_TIME;
    let mut workers_count: [usize; NUMBER_OF_STAGES] = [0; NUMBER_OF_STAGES];
    for i in 0..NUMBER_OF_STAGES {
        let weight = match i {
            0 => SNOS_PROOF_GENERATION_TIME as f64 / total_time as f64,
            1 => LAYOUT_BRIDGE_PROOF_GENERATION_TIME as f64 / total_time as f64,
            2 => PIE_GENERATION_TIME as f64 / total_time as f64,
            _ => 0.0,
        };
        let workers = (num_blocks_in_pipeline as f64 * weight).ceil() as usize;
        workers_count[i] = workers;
    }
    workers_count
}
#[test]
fn test_split_workers() {
    let num_blocks_in_pipeline = 110;
    let workers = calculate_workers_per_stage(num_blocks_in_pipeline);
    assert_eq!(workers, [39, 70, 2]);
    println!("{:?}", workers);
}
