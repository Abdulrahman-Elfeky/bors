use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use axum::routing::post;
use axum::Router;
use clap::Parser;
use sea_orm::{ConnectOptions, Database};
use tokio::task::LocalSet;
use tower::limit::ConcurrencyLimitLayer;

use bors::database::SeaORMClient;
use bors::github::server::{create_bors_process, github_webhook_handler, ServerState};
use bors::github::{GithubAppState, WebhookSecret};
use migration::{Migrator, MigratorTrait};

#[derive(clap::Parser)]
struct Opts {
    /// Secret used to authenticate webhooks.
    #[arg(long, env = "WEBHOOK_SECRET")]
    webhook_secret: String,

    /// Github App ID.
    #[arg(long, env = "APP_ID")]
    app_id: u64,

    /// Private key used to authenticate as a Github App.
    #[arg(long, env = "PRIVATE_KEY")]
    private_key: String,

    /// Database connection string.
    #[arg(long, env = "DATABASE")]
    db: String,
}

async fn server(state: ServerState) -> anyhow::Result<()> {
    let state = Arc::new(state);

    let app = Router::new()
        .route("/github", post(github_webhook_handler))
        .layer(ConcurrencyLimitLayer::new(100))
        .with_state(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;
    Ok(())
}

async fn initialize_db(connection_string: &str) -> anyhow::Result<SeaORMClient> {
    let db = Database::connect(connection_string).await?;
    Migrator::up(&db, None).await?;
    Ok(SeaORMClient::new(db))
}

fn try_main(opts: Opts) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Cannot build tokio runtime")?;

    let db = runtime
        .block_on(initialize_db(&opts.db))
        .context("Cannot initialize database")?;

    let state = runtime.block_on(GithubAppState::load(
        opts.app_id.into(),
        opts.private_key.into_bytes().into(),
        db,
    ))?;
    let (tx, gh_process) = create_bors_process(state);

    let state = ServerState::new(tx, WebhookSecret::new(opts.webhook_secret));
    let server_process = server(state);

    let fut = async move {
        tokio::select! {
            () = gh_process => {
                tracing::warn!("Github webhook process has ended");
                Ok(())
            },
            res = server_process => {
                tracing::warn!("Server has ended: {res:?}");
                res
            }
        }
    };

    runtime.block_on(async move {
        let set = LocalSet::new();
        set.run_until(fut).await.unwrap();
    });

    Ok(())
}

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    let opts = Opts::parse();
    if let Err(error) = try_main(opts) {
        eprintln!("Error: {error:?}");
        std::process::exit(1);
    }
}
