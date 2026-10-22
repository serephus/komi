use std::env;
use std::net::SocketAddr;
use std::process::ExitCode;

use komi::cli;
use komi::config::Config;
use komi::{db, web};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("komi: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let cli =
        cli::parse(env::args().skip(1)).map_err(|error| format!("{error}\n\n{}", cli::USAGE))?;
    if cli.help {
        print!("{}", cli::USAGE);
        return Ok(());
    }
    if cli.version {
        println!("komi {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let loaded = Config::load(cli.config).map_err(|error| {
        format!("{error}\n\nrun `komi --help` for the configuration search order")
    })?;
    let source = loaded.source_label();
    let config = loaded.config;
    init_logging(&config.log)?;

    let database = db::Store::open(&config.database)
        .map_err(|error| format!("could not open {}: {error}", config.database.display()))?;

    tracing::info!(
        address = %config.bind,
        database = %config.database.display(),
        config = %source,
        site = %config.site_name,
        "komi is listening"
    );

    let bind = config.bind;
    let state = web::AppState::new(database, config);
    let app = web::router(state.clone());
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|error| format!("could not listen on {bind}: {error}"))?;

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        // Event streams are responses in flight; tell them to finish.
        state.shut_down();
    })
    .await
    .map_err(|error| format!("server stopped: {error}"))?;
    Ok(())
}

/// `RUST_LOG` wins if it is set, otherwise the config file's `log` is used.
fn init_logging(default_filter: &str) -> Result<(), String> {
    let filter = match EnvFilter::try_from_default_env() {
        Ok(filter) => filter,
        Err(_) => EnvFilter::try_new(default_filter)
            .map_err(|error| format!("invalid log filter “{default_filter}”: {error}"))?,
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();
    Ok(())
}

async fn shutdown_signal() {
    let interrupt = async {
        tokio::signal::ctrl_c().await.expect("ctrl-c handler");
    };
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("sigterm handler")
            .recv()
            .await;
    };
    tokio::select! {
        () = interrupt => {},
        _ = terminate => {},
    }
    tracing::info!("shutting down");
}
