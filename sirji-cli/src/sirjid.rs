//! `sirjid` — the daemon for one `$SIRJI_HOME`.
//!
//! Takes no arguments. An instance *is* its home directory, so two sirjis on one
//! machine are two directories and nothing else: no ports to allocate, no flags
//! to keep straight.

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let home = sirji::keystore::home()?;
    if !sirji::Network::path_in(&home).exists() {
        anyhow::bail!(
            "no sirji at {} — run `sirji init` first",
            home.display()
        );
    }
    println!("sirji home {}", home.display());

    let daemon = sirji::Daemon::start(home).await?;
    daemon.run().await
}
