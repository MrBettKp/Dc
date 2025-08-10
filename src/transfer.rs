use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransferDirection {
    Sent,
    Received,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsdcTransfer {
    pub signature: String,
    pub timestamp: DateTime<Utc>,
    pub amount: u64, // Raw amount (multiply by 10^-6 for USDC)
    pub direction: TransferDirection,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone)]
pub struct TokenTransferInfo {
    pub mint: String,
    pub amount: u64,
    pub from_owner: String,
    pub to_owner: String,
}