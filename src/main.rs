use std::io::IsTerminal;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use m2tg::appservice;
use m2tg::config::Config;
use m2tg::registration::registration_yaml;
use tracing_subscriber::EnvFilter;

const OUTBOX_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

fn install_crypto_provider() -> anyhow::Result<()> {
	rustls::crypto::ring::default_provider()
		.install_default()
		.map_err(|_| anyhow::anyhow!("failed to install rustls ring crypto provider"))
}

fn bot_id(token: &str) -> &str {
	token.split_once(':').map_or("unknown", |(id, _)| id)
}

async fn shutdown_signal() {
	let interrupt = async {
		if let Err(e) = tokio::signal::ctrl_c().await {
			tracing::error!("cannot listen for ctrl-c: {e}");
			std::future::pending::<()>().await;
		}
	};
	#[cfg(unix)]
	let terminate = async {
		use tokio::signal::unix::{SignalKind, signal};
		match signal(SignalKind::terminate()) {
			Ok(mut terminate) => {
				terminate.recv().await;
			}
			Err(e) => {
				tracing::error!("cannot listen for SIGTERM: {e}");
				std::future::pending::<()>().await;
			}
		}
	};
	#[cfg(not(unix))]
	let terminate = std::future::pending::<()>();
	tokio::select! {
		() = interrupt => {}
		() = terminate => {}
	}
	tracing::info!("shutting down");
}

fn main() -> anyhow::Result<()> {
	match std::env::args().nth(1).as_deref() {
		None => serve(),
		Some("registration") => {
			print!("{}", registration_yaml(&Config::from_env()?, &[]));
			Ok(())
		}
		Some(other) => anyhow::bail!("unknown command {other:?}; usage: m2tg [registration]"),
	}
}

#[tokio::main]
async fn serve() -> anyhow::Result<()> {
	tracing_subscriber::fmt()
		.with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
		.with_ansi(std::io::stdout().is_terminal())
		.init();

	install_crypto_provider()?;

	let config = Arc::new(Config::from_env()?);
	let http = reqwest::Client::builder()
		.connect_timeout(Duration::from_secs(10))
		.timeout(Duration::from_secs(120))
		.build()
		.context("building HTTP client")?;

	let listener = tokio::net::TcpListener::bind((config.host.as_str(), config.port))
		.await
		.with_context(|| format!("binding {}:{}", config.host, config.port))?;
	tracing::info!(
		address = %listener.local_addr()?,
		room_id = %config.room_id,
		app_service_user = %config.app_service_user,
		telegram_chat_id = %config.telegram_chat_id,
		bot_id = %bot_id(&config.telegram_bot_token),
		"m2tg listening"
	);

	let (app, outbox) = appservice::router(config.clone(), http.clone());
	appservice::spawn_room_join(config, http);

	axum::serve(listener, app)
		.with_graceful_shutdown(shutdown_signal())
		.await
		.context("server error")?;

	match tokio::time::timeout(OUTBOX_DRAIN_TIMEOUT, outbox).await {
		Ok(Ok(())) => {}
		Ok(Err(e)) => tracing::error!("outbox task failed: {e}"),
		Err(_) => tracing::warn!("outbox did not drain within {OUTBOX_DRAIN_TIMEOUT:?}"),
	}
	Ok(())
}
