use glance_github_graph::api::run_api_server;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    run_api_server().await
} 