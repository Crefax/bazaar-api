use serde::{Deserialize, Serialize};

// ApiKeyQuery
#[derive(Debug, Deserialize, Serialize)]
pub struct ApiKeyQuery {
    pub key: String,
}