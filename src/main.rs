use std::borrow::BorrowMut;

use anyhow::anyhow;
use borsh::BorshDeserialize;
use solana_client::rpc_config::RpcSendTransactionConfig;
use solana_program::instruction::{Instruction, AccountMeta};
use clap::Parser;
use solana_cli_config::{Config, CONFIG_FILE};
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::signature::{Signer, read_keypair_file, Keypair};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::transaction::Transaction;
use vaults::state::Vault;
use solana_program::pubkey;
use solana_program::pubkey::Pubkey;

// Command-line args to parameterize the interest payment client
#[derive(Parser)]
struct Args {
    /// Use your own RPC endpoint by passing into the -u flag or default Solana RPC endpoints.
    #[clap(short, long)]
    url: Option<String>,

    // Pass in the vault to which the interest is to be paid into the -v flag
    #[clap(short, long)]
    vault: Pubkey,

    // Pass the duration in days on how frequently should the call be made.
    // Defaults to 30 days
    #[clap(short, long, default_value = "30")]
    duration: u16,

    /// Defaults to your Solana CLI config file. You can optionally include your keypair path.
    keypair_path: Option<String>,
}

pub const ANCHOR_DISCRIMINATOR_SIZE: usize = 8;

pub const VAULTS_PROGRAM_ID: Pubkey = pubkey!("5j3KuMK2u7KFtoEwiLTexUeooHq5NPQX96rYp5dhuze9");

pub fn get_keypair_from_path(path: &str) -> anyhow::Result<Keypair> {
    read_keypair_file(&*shellexpand::tilde(path)).map_err(|e| anyhow!(e.to_string()))
}

pub fn get_network(network_str: &str) -> &str {
    match network_str {
        "devnet" | "dev" | "d" => "https://api.devnet.solana.com",
        "mainnet" | "main" | "m" | "mainnet-beta" => "https://api.mainnet-beta.solana.com",
        "localnet" | "localhost" | "l" => "http://localhost:8899",
        _ => network_str
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let config = match CONFIG_FILE.as_ref() {
        Some(config_file) => Config::load(config_file).unwrap_or_else(|_| {
            println!("Failed to load config file: {}", config_file);
            Config::default()
        }),
        None => Config::default()
    };

    let interest_payer_keypair = get_keypair_from_path(&args.keypair_path.unwrap_or(config.keypair_path))?;
    let network_url = &get_network(&args.url.unwrap_or(config.json_rpc_url)).to_string();

    let vault = &args.vault;
    let payment_duration = &args.duration;

    println!("Payer key: {}", interest_payer_keypair.pubkey().to_string());
    println!("Vault: {}", vault.to_string());
    println!("Runs every {} days", payment_duration);

    loop {
        let rpc_client = RpcClient::new(network_url.to_string());
        let vault_account = rpc_client.get_account(vault).await?;

        let vault_deserialized = Vault::deserialize(vault_account.data.split_at(ANCHOR_DISCRIMINATOR_SIZE).1.borrow_mut())?;

        let token_vault_ac = vault_deserialized.token_vault_ac;

        let token = vault_deserialized.token;

        let token_payer_ac = spl_associated_token_account::get_associated_token_address(&interest_payer_keypair.pubkey(), &token);

        // 8 byte discriminator for instruction TopupInterest
        let instruction_discriminator: &[u8] = &[196, 215, 224, 233, 237, 212, 2, 56];

        let pay_interest_ix = Instruction::new_with_bytes(
            VAULTS_PROGRAM_ID, 
            instruction_discriminator,
            vec![
                AccountMeta::new(VAULTS_PROGRAM_ID, false),
                AccountMeta::new(interest_payer_keypair.pubkey(), true),
                AccountMeta::new_readonly(token, false),
                AccountMeta::new(*vault, false),
                AccountMeta::new(token_payer_ac, false),
                AccountMeta::new(token_vault_ac, false),
                AccountMeta::new_readonly(anchor_spl::token::ID, false),
                AccountMeta::new_readonly(anchor_lang::system_program::ID, false)   
            ]
        );

        let recent_blockhash = rpc_client.get_latest_blockhash().await.unwrap();

        let mut transaction = Transaction::new_signed_with_payer(
            &[pay_interest_ix],
            Some(&interest_payer_keypair.pubkey()),
            &[&interest_payer_keypair],
            recent_blockhash
        );

        transaction.sign(&[&interest_payer_keypair], recent_blockhash);
        
        let result = rpc_client.send_and_confirm_transaction_with_spinner_and_config(
            &transaction,
            CommitmentConfig::processed(),
            RpcSendTransactionConfig {
                skip_preflight: true,
                preflight_commitment: None,
                max_retries: None,
                encoding: None,
                min_context_slot: None
            }
        ).await;

        match  result {
            Ok(signature) => {
                println!("Transaction sig: {}", signature);
            }
            Err(err) => {
                println!("Transaction failed. Error: {}", err);
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(
            *payment_duration as u64 * 86400_u64 * 1_000
        ))
        .await;
    }
}
