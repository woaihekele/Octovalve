#[tokio::main]
async fn main() -> anyhow::Result<()> {
    acp_codex::run_stdio().await
}
