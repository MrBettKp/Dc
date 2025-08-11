use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Utc};
use clap::Parser;
use solana_client::rpc_client::RpcClient;
use solana_client::rpc_config::{RpcTransactionConfig};
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::Signature,
};
use solana_transaction_status::UiTransactionEncoding;
use std::str::FromStr;

mod transfer;
mod utils;

use transfer::{UsdcTransfer, TransferDirection};
use utils::{parse_token_transfers, is_usdc_mint};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Wallet address to index
    #[arg(short, long)]
    wallet: String,

    /// RPC endpoint URL
    #[arg(short, long, default_value = "https://api.mainnet-beta.solana.com")]
    rpc_url: String,

    /// Hours to look back (default: 24)
    #[arg(long, default_value_t = 24)]
    hours: u64,

    /// Run as a service (keep running and re-index every hour)
    #[arg(long, default_value_t = false)]
    service: bool,
}

pub struct SolanaIndexer {
    client: RpcClient,
    wallet_pubkey: Pubkey,
}

impl SolanaIndexer {
    pub fn new(rpc_url: &str, wallet_address: &str) -> Result<Self> {
        let client = RpcClient::new_with_commitment(
            rpc_url.to_string(),
            CommitmentConfig::confirmed(),
        );
        
        let wallet_pubkey = Pubkey::from_str(wallet_address)
            .map_err(|_| anyhow!("Invalid wallet address: {}", wallet_address))?;

        Ok(Self {
            client,
            wallet_pubkey,
        })
    }

    pub async fn backfill_usdc_transfers(&self, hours_back: u64) -> Result<Vec<UsdcTransfer>> {
        println!("🔍 Starting USDC transfer indexing for wallet: {}", self.wallet_pubkey);
        println!("📅 Looking back {} hours", hours_back);

        let mut all_transfers = Vec::new();
        let mut before_signature: Option<Signature> = None;
        let limit = 1000; // Maximum allowed by Solana RPC
        let target_time = Utc::now() - Duration::hours(hours_back as i64);

        loop {
            println!("📡 Fetching transaction batch...");
            
            let signatures = self.client.get_signatures_for_address_with_config(
                &self.wallet_pubkey,
                solana_client::rpc_client::GetConfirmedSignaturesForAddress2Config {
                    limit: Some(limit),
                    before: before_signature,
                    until: None,
                    commitment: Some(CommitmentConfig::confirmed()),
                },
            )?;

            if signatures.is_empty() {
                println!("✅ No more transactions found");
                break;
            }

            println!("🔄 Processing {} signatures...", signatures.len());
            let mut batch_transfers = Vec::new();
            let mut oldest_time = Utc::now();

            for sig_info in &signatures {
                // Check if we've gone back far enough
                if let Some(block_time) = sig_info.block_time {
                    let tx_time = DateTime::from_timestamp(block_time, 0)
                        .unwrap_or(Utc::now());
                    
                    oldest_time = oldest_time.min(tx_time);
                    
                    if tx_time < target_time {
                        println!("⏰ Reached target time: {}", target_time);
                        break;
                    }
                }

                if let Some(err) = &sig_info.err {
                    println!("⚠️ Skipping failed transaction: {:?}", err);
                    continue;
                }

                let signature = Signature::from_str(&sig_info.signature)?;
                
                match self.process_transaction(signature).await {
                    Ok(transfers) => {
                        batch_transfers.extend(transfers);
                    }
                    Err(e) => {
                        println!("⚠️ Error processing transaction {}: {}", sig_info.signature, e);
                        continue;
                    }
                }
            }

            all_transfers.extend(batch_transfers);
            
            // Check if we should continue
            if oldest_time < target_time {
                println!("✅ Reached target time window");
                break;
            }

            // Set up for next batch
            before_signature = signatures.last().map(|s| Signature::from_str(&s.signature).unwrap());
            
            if signatures.len() < limit {
                println!("✅ Fetched all available transactions");
                break;
            }

            // Small delay to avoid rate limiting
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        // Filter transfers to only include those within the time window
        let filtered_transfers: Vec<UsdcTransfer> = all_transfers
            .into_iter()
            .filter(|transfer| transfer.timestamp >= target_time)
            .collect();

        println!("🎯 Found {} USDC transfers in the last {} hours", filtered_transfers.len(), hours_back);
        Ok(filtered_transfers)
    }

    async fn process_transaction(&self, signature: Signature) -> Result<Vec<UsdcTransfer>> {
        let transaction = self.client.get_transaction_with_config(
            &signature,
            RpcTransactionConfig {
                encoding: Some(UiTransactionEncoding::Json),
                commitment: Some(CommitmentConfig::confirmed()),
                max_supported_transaction_version: Some(0),
            },
        )?;

        let mut transfers = Vec::new();

        if let Some(meta) = &transaction.transaction.meta {
            if let Some(block_time) = transaction.block_time {
                let timestamp = DateTime::from_timestamp(block_time, 0)
                    .unwrap_or(Utc::now());

                // Parse token transfers from transaction
                if let Some(token_transfers) = parse_token_transfers(meta) {
                    for transfer in token_transfers {
                        // Check if it's a USDC transfer involving our wallet
                        if is_usdc_mint(&transfer.mint) {
                            let from_pubkey = Pubkey::from_str(&transfer.from_owner)?;
                            let to_pubkey = Pubkey::from_str(&transfer.to_owner)?;

                            let direction = if from_pubkey == self.wallet_pubkey {
                                Some(TransferDirection::Sent)
                            } else if to_pubkey == self.wallet_pubkey {
                                Some(TransferDirection::Received)
                            } else {
                                None
                            };

                            if let Some(dir) = direction {
                                transfers.push(UsdcTransfer {
                                    signature: signature.to_string(),
                                    timestamp,
                                    amount: transfer.amount,
                                    direction: dir,
                                    from: transfer.from_owner,
                                    to: transfer.to_owner,
                                });
                            }
                        }
                    }
                }
            }
        }

        Ok(transfers)
    }
}

async fn run_indexer_once(args: &Args) -> Result<()> {
    let indexer = SolanaIndexer::new(&args.rpc_url, &args.wallet)?;
    let transfers = indexer.backfill_usdc_transfers(args.hours).await?;

    // Display results
    display_results(&transfers).await?;
    Ok(())
}

async fn display_results(transfers: &[UsdcTransfer]) -> Result<()> {
    if transfers.is_empty() {
        println!("\n📭 No USDC transfers found in the specified time period.");
    } else {
        println!("\n📊 USDC Transfer Summary:");
        println!("========================");
        
        let mut total_sent = 0u64;
        let mut total_received = 0u64;
        
        for transfer in transfers {
            let direction_symbol = match transfer.direction {
                TransferDirection::Sent => "📤",
                TransferDirection::Received => "📥",
            };
            
            let amount_usdc = transfer.amount as f64 / 1_000_000.0; // USDC has 6 decimals
            
            match transfer.direction {
                TransferDirection::Sent => total_sent += transfer.amount,
                TransferDirection::Received => total_received += transfer.amount,
            }
            
            println!(
                "{} {} | {} USDC | {} | {}",
                direction_symbol,
                transfer.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
                amount_usdc,
                match transfer.direction {
                    TransferDirection::Sent => format!("To: {}", &transfer.to[..8]),
                    TransferDirection::Received => format!("From: {}", &transfer.from[..8]),
                },
                transfer.signature
            );
        }
        
        println!("\n📈 Summary:");
        println!("📥 Total Received: {} USDC", total_received as f64 / 1_000_000.0);
        println!("📤 Total Sent: {} USDC", total_sent as f64 / 1_000_000.0);
        println!("💹 Net Change: {} USDC", 
            (total_received as i64 - total_sent as i64) as f64 / 1_000_000.0
        );
        
        // Export to JSON
        let json_output = serde_json::to_string_pretty(&transfers)?;
        std::fs::write("usdc_transfers.json", json_output)?;
        println!("\n💾 Results saved to: usdc_transfers.json");
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Set up panic handler for better debugging
    std::panic::set_hook(Box::new(|panic_info| {
        eprintln!("🚨 PANIC: {}", panic_info);
        if let Some(location) = panic_info.location() {
            eprintln!("📍 Location: {}:{}", location.file(), location.line());
        }
    }));

    println!("🚀 Solana USDC Indexer Starting...");
    
    let args = match Args::try_parse() {
        Ok(args) => {
            println!("✅ Arguments parsed successfully");
            args
        }
        Err(e) => {
            eprintln!("❌ Failed to parse arguments: {}", e);
            // If argument parsing fails, run with default values
            Args {
                wallet: "7cMEhpt9y3inBNVv8fNnuaEbx7hKHZnLvR1KWKKxuDDU".to_string(),
                rpc_url: "https://api.mainnet-beta.solana.com".to_string(),
                hours: 24,
                service: false,
            }
        }
    };
    
    println!("💰 Target wallet: {}", args.wallet);
    println!("🌐 RPC endpoint: {}", args.rpc_url);
    println!("⏰ Hours to index: {}", args.hours);
    
    if args.service {
        println!("🔄 Running as a service - will re-index every hour");
        loop {
            match run_indexer_once(&args).await {
                Ok(()) => println!("✅ Indexing cycle completed successfully at {}", Utc::now().format("%Y-%m-%d %H:%M:%S UTC")),
                Err(e) => {
                    eprintln!("❌ Indexing cycle failed: {}", e);
                    eprintln!("🔄 Will retry in next cycle...");
                }
            }
            
            println!("😴 Sleeping for 1 hour before next indexing cycle...");
            tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
        }
    } else {
        // Run once and keep alive for hosting platforms
        println!("🎯 Running single indexing cycle...");
        
        match run_indexer_once(&args).await {
            Ok(()) => {
                println!("🏁 Indexing completed successfully!");
            }
            Err(e) => {
                eprintln!("❌ Indexing failed: {}", e);
                eprintln!("📋 This might be due to:");
                eprintln!("  • Network connectivity issues");
                eprintln!("  • RPC rate limiting");
                eprintln!("  • Invalid wallet address");
                eprintln!("  • Solana RPC endpoint issues");
            }
        }
        
        println!("🔄 Keeping service alive for hosting platform...");
        println!("📝 To run as a continuous service, use --service flag");
        
        // Keep the service alive with more frequent heartbeats
        let mut counter = 0;
        loop {
            counter += 1;
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await; // Sleep 1 minute
            println!("💓 Service heartbeat #{} - {}", counter, Utc::now().format("%Y-%m-%d %H:%M:%S UTC"));
            
            // Every 10 minutes, show memory info
            if counter % 10 == 0 {
                println!("📊 Service has been alive for {} minutes", counter);
            }
        }
    }
}