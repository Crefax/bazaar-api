mod db;

pub use self::db::{
    ensure_indexes,
    insert_bazaar_data,
    get_latest_bazaar_data,
    get_bazaar_data_by_timeframe,
    get_bazaar_data_paginated,
    get_bazaar_data_aggregated,
    get_bazaar_data_smart,
    // Compression tracking functions
    get_compression_state,
    upsert_compression_state,
    log_compression,
    get_compression_logs,
    needs_compression,
    get_compression_stats
};

