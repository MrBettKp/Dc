use solana_transaction_status::UiTransactionTokenBalance;
use solana_transaction_status::TransactionTokenBalance;
use crate::transfer::TokenTransferInfo;
use std::collections::HashMap;

// USDC mint addresses for different networks
const USDC_MAINNET: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const USDC_DEVNET: &str = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU"; // For testing

pub fn is_usdc_mint(mint: &str) -> bool {
    mint == USDC_MAINNET || mint == USDC_DEVNET
}

pub fn parse_token_transfers(
    meta: &solana_transaction_status::UiTransactionStatusMeta,
) -> Option<Vec<TokenTransferInfo>> {
    let pre_balances = meta.pre_token_balances.as_ref()?;
    let post_balances = meta.post_token_balances.as_ref()?;

    // Create maps for easier lookup
    let mut pre_balance_map: HashMap<usize, &UiTransactionTokenBalance> = HashMap::new();
    let mut post_balance_map: HashMap<usize, &UiTransactionTokenBalance> = HashMap::new();

    for balance in pre_balances {
        pre_balance_map.insert(balance.account_index as usize, balance);
    }

    for balance in post_balances {
        post_balance_map.insert(balance.account_index as usize, balance);
    }

    let mut transfers = Vec::new();

    // Find all accounts that had balance changes
    let mut all_accounts: std::collections::HashSet<usize> = std::collections::HashSet::new();
    
    for balance in pre_balances {
        all_accounts.insert(balance.account_index as usize);
    }
    for balance in post_balances {
        all_accounts.insert(balance.account_index as usize);
    }

    // Group accounts by mint
    let mut mint_accounts: HashMap<String, Vec<usize>> = HashMap::new();
    
    for &account_index in &all_accounts {
        let mint = if let Some(pre) = pre_balance_map.get(&account_index) {
            pre.mint.clone()
        } else if let Some(post) = post_balance_map.get(&account_index) {
            post.mint.clone()
        } else {
            continue;
        };
        
        mint_accounts.entry(mint).or_insert_with(Vec::new).push(account_index);
    }

    // Process each mint group to find transfers
    for (mint, accounts) in mint_accounts {
        // Only process if it's USDC
        if !is_usdc_mint(&mint) {
            continue;
        }

        // Calculate balance changes for each account
        let mut balance_changes: Vec<(usize, i64, String)> = Vec::new();
        
        for &account_index in &accounts {
            let pre_amount = if let Some(pre) = pre_balance_map.get(&account_index) {
                parse_token_amount(&pre.ui_token_amount.amount)
            } else {
                0
            };
            
            let post_amount = if let Some(post) = post_balance_map.get(&account_index) {
                parse_token_amount(&post.ui_token_amount.amount)
            } else {
                0
            };
            
            let change = post_amount as i64 - pre_amount as i64;
            
            if change != 0 {
                let owner = if let Some(post) = post_balance_map.get(&account_index) {
                    post.owner.clone().unwrap_or_default()
                } else if let Some(pre) = pre_balance_map.get(&account_index) {
                    pre.owner.clone().unwrap_or_default()
                } else {
                    String::new()
                };
                
                balance_changes.push((account_index, change, owner));
            }
        }

        // Match decreases with increases to form transfers
        let mut decreases: Vec<_> = balance_changes.iter()
            .filter(|(_, change, _)| *change < 0)
            .collect();
        let mut increases: Vec<_> = balance_changes.iter()
            .filter(|(_, change, _)| *change > 0)
            .collect();

        // Try to match transfers
        while let Some(decrease) = decreases.pop() {
            let decrease_amount = (-decrease.1) as u64;
            
            // Find matching increase
            if let Some(increase_pos) = increases.iter().position(|(_, change, _)| *change as u64 == decrease_amount) {
                let increase = increases.remove(increase_pos);
                
                transfers.push(TokenTransferInfo {
                    mint: mint.clone(),
                    amount: decrease_amount,
                    from_owner: decrease.2.clone(),
                    to_owner: increase.2.clone(),
                });
            }
        }
    }

    if transfers.is_empty() {
        None
    } else {
        Some(transfers)
    }
}

fn parse_token_amount(amount_str: &str) -> u64 {
    amount_str.parse::<u64>().unwrap_or(0)
}