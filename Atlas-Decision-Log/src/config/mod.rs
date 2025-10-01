pub struct DecLogConfig {
    // The amount of ongoing decisions to keep in memory
    // By default (if the ordering protocol overtakes this, it will overflow this capacity)
    pub default_ongoing_capacity: usize,
}
