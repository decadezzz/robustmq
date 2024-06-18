pub mod client_keep_alive;
pub mod connection;
pub mod heartbeat_cache;
pub mod metadata_cache;
pub mod session;
pub mod system_topic;
pub mod topic;
pub mod qos_manager;

pub const HEART_CONNECT_SHARD_HASH_NUM: u64 = 20;
