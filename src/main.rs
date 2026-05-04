mod http;
mod keys;
mod narinfo;
mod p2p;
mod peer_protocol;
mod store;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "nix-p2p-cache",
    version,
    about = "P2P LAN substituter for /nix/store"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    Run {
        #[arg(long, default_value = "5555")]
        port: u16,
        #[arg(long, default_value = "::")]
        bind: String,
        #[arg(long, default_value = store::DEFAULT_DB_PATH)]
        db: PathBuf,
        #[arg(long, default_value = narinfo::STORE_DIR)]
        store_dir: PathBuf,
    },
    Keygen {
        #[arg(long)]
        out: PathBuf,
    },
    DerivePubkey,
    Setup {
        #[arg(long, default_value = "5555")]
        port: u16,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Run {
            port,
            bind,
            db,
            store_dir,
        } => run(port, bind, db, store_dir).await,
        Cmd::Keygen { out } => {
            let (sec, pubp) = keys::write_key_files(&out)?;
            println!("wrote {} and {}", sec.display(), pubp.display());
            Ok(())
        }
        Cmd::DerivePubkey => {
            println!("{}", keys::public_key_line());
            Ok(())
        }
        Cmd::Setup { port } => {
            println!(
                "# Add to /etc/nix/nix.conf:\nsubstituters = http://127.0.0.1:{port} https://cache.nixos.org\ntrusted-public-keys = {} cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY=",
                keys::public_key_line()
            );
            Ok(())
        }
    }
}

async fn run(port: u16, bind: String, db: PathBuf, store_dir: PathBuf) -> Result<()> {
    tracing::info!("starting nix-p2p-cache");
    let key = Arc::new(keys::LocalKey::shared());
    let store = Arc::new(store::LocalStore::new(db, store_dir));
    let (p2p_handle, p2p_fut) = p2p::start(store.clone(), port);
    let state = http::AppState {
        store,
        key,
        p2p: p2p_handle,
    };
    let addr: SocketAddr = if bind.contains(':') {
        format!("[{bind}]:{port}").parse()?
    } else {
        format!("{bind}:{port}").parse()?
    };
    tokio::try_join!(p2p_fut, http::serve(state, addr))?;
    Ok(())
}
