mod peer;
mod server;
mod signaling;

use clap::Parser;
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

const DEFAULT_SIGNAL_URL: &str = "wss://cloud.blit.sh";
const DEFAULT_URL_TEMPLATE: &str = "https://blit.sh/#{secret}";

#[derive(Parser)]
#[command(name = "blitz", version, about = "Share a terminal session via WebRTC")]
struct Cli {
    /// Passphrase for the session (default: random UUID)
    #[arg(long)]
    passphrase: Option<String>,

    /// Signaling service URL
    #[arg(long, default_value = DEFAULT_SIGNAL_URL)]
    signal_url: String,

    /// URL template to display (use {secret} as placeholder)
    #[arg(long, default_value = DEFAULT_URL_TEMPLATE)]
    url: String,

    /// Connect to an existing blit-server socket instead of starting an embedded one
    #[arg(long)]
    socket: Option<String>,

    /// Don't print the sharing URL
    #[arg(long)]
    quiet: bool,
}

fn derive_signing_key(passphrase: &str) -> SigningKey {
    let mut hasher = Sha256::new();
    hasher.update(passphrase.as_bytes());
    let seed: [u8; 32] = hasher.finalize().into();
    SigningKey::from_bytes(&seed)
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let passphrase = cli
        .passphrase
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let signing_key = derive_signing_key(&passphrase);
    let public_key_hex = hex_encode(signing_key.verifying_key().as_bytes());

    let sock_path = match &cli.socket {
        Some(path) => path.clone(),
        None => {
            let path = server::start_embedded(&passphrase).await;
            eprintln!("embedded server listening on {path}");
            path
        }
    };

    if !cli.quiet {
        let url = cli.url.replace("{secret}", &passphrase);
        println!("{url}");
    }

    let (sig_tx, mut sig_rx) = mpsc::unbounded_channel::<signaling::Event>();
    let signal_url = format!(
        "{}/channel/{}/producer",
        cli.signal_url.trim_end_matches('/'),
        public_key_hex,
    );

    tokio::spawn(signaling::connect(signal_url, signing_key.clone(), sig_tx));

    let peers: Arc<tokio::sync::Mutex<HashMap<String, tokio::task::JoinHandle<()>>>> =
        Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    while let Some(event) = sig_rx.recv().await {
        match event {
            signaling::Event::Registered { session_id } => {
                eprintln!("registered with signaling server (session {session_id})");
            }
            signaling::Event::PeerJoined { session_id, .. } => {
                eprintln!("consumer joined: {session_id}");
                let peer_id = session_id.clone();
                let sock = sock_path.clone();
                let signing_key = signing_key.clone();
                let signal_url_base = cli.signal_url.clone();
                let pubkey = public_key_hex.clone();
                let handle = tokio::spawn(async move {
                    if let Err(e) =
                        peer::handle_peer(peer_id.clone(), sock, signing_key, signal_url_base, pubkey).await
                    {
                        eprintln!("peer {peer_id} error: {e}");
                    }
                });
                peers.lock().await.insert(session_id, handle);
            }
            signaling::Event::PeerLeft { session_id, .. } => {
                eprintln!("consumer left: {session_id}");
                if let Some(handle) = peers.lock().await.remove(&session_id) {
                    handle.abort();
                }
            }
            signaling::Event::Signal {
                from,
                data,
            } => {
                eprintln!("received signal from {from}");
                // Signal data is handled within the peer task via its own signaling connection.
                // For the initial implementation, the main signaling connection forwards
                // offer/answer/candidate messages to the appropriate peer.
                let _ = (from, data);
            }
            signaling::Event::Error { message } => {
                eprintln!("signaling error: {message}");
            }
        }
    }
}
