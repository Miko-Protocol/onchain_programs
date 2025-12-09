use std::env;
use std::fs;
use std::path::Path;
use solana_sdk::signature::{Keypair, Signer};

fn main() {
    // Get the program keypair path from environment or use default
    let keypair_path = env::var("SMART_DIAL_PROGRAM_KEYPAIR")
        .unwrap_or_else(|_| "../../keypairs/smart-dial-program-keypair.json".to_string());
    
    // Read the keypair file
    let keypair_path = Path::new(&keypair_path);
    if keypair_path.exists() {
        let keypair_data = fs::read_to_string(keypair_path)
            .expect("Failed to read keypair file");
        let keypair_bytes: Vec<u8> = serde_json::from_str(&keypair_data)
            .expect("Failed to parse keypair JSON");
        let keypair = Keypair::try_from(&keypair_bytes[..])
            .expect("Failed to create keypair from bytes");
        
        // Write the program ID to a file that will be included
        let out_dir = env::var("OUT_DIR").unwrap();
        let dest_path = Path::new(&out_dir).join("program_id.rs");
        fs::write(
            dest_path,
            format!(
                r#"declare_id!("{}");"#,
                keypair.pubkey().to_string()
            ),
        ).expect("Failed to write program ID");
    } else {
        // Fallback for when keypair doesn't exist (like in IDL generation)
        let out_dir = env::var("OUT_DIR").unwrap();
        let dest_path = Path::new(&out_dir).join("program_id.rs");
        fs::write(
            dest_path,
            r#"declare_id!("11111111111111111111111111111111");"#,
        ).expect("Failed to write placeholder ID");
    }
    
    // Tell Cargo to rerun if keypair changes
    println!("cargo:rerun-if-changed={}", keypair_path.display());
}