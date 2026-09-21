use muxlane_relay::Relay;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .try_init()
        .ok();
    let bind = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:9843".into());
    Relay::from_env()?.serve(&bind).await
}
