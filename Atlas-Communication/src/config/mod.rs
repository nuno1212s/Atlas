use getset::CopyGetters;

#[derive(CopyGetters)]
pub struct ClientPoolConfig {
    #[get_copy = "pub"]
    batch_limit: usize,
    #[get_copy = "pub"]
    per_client_bound: usize,
    #[get_copy = "pub"]
    clients_per_pool: usize,
    #[get_copy = "pub"]
    batch_timeout_micros: u64,
    #[get_copy = "pub"]
    batch_sleep_micros: u64,
    #[get_copy = "pub"]
    channel_size: usize,
}

impl ClientPoolConfig {
    pub fn new(
        batch_limit: usize,
        per_client_bound: usize,
        clients_per_pool: usize,
        batch_timeout_micros: u64,
        batch_sleep_micros: u64,
        channel_size: usize,
    ) -> Self {
        Self {
            batch_limit,
            per_client_bound,
            clients_per_pool,
            batch_timeout_micros,
            batch_sleep_micros,
            channel_size,
        }
    }
}

impl Default for ClientPoolConfig {
    fn default() -> Self {
        Self {
            batch_limit: 1024,
            per_client_bound: 1024,
            clients_per_pool: 1024,
            batch_timeout_micros: 500,
            batch_sleep_micros: 250,
            channel_size: 1024,
        }
    }
}

#[derive(CopyGetters, Debug)]
pub struct UnpooledConnection {
    #[get_copy = "pub"]
    channel_size: usize,
}

impl UnpooledConnection {
    pub fn new(channel_size: usize) -> Self {
        Self { channel_size }
    }
}

impl Default for UnpooledConnection {
    fn default() -> Self {
        Self { channel_size: 2048 }
    }
}
