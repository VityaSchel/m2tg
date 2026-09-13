use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;

pub struct Config {
	pub homeserver_url: String,
	pub room_id: String,
	pub host: String,
	pub port: u16,
	pub homeserver_token: String,
	pub app_service_token: String,
	pub app_service_user: String,
	pub telegram_chat_id: String,
	pub telegram_bot_token: String,
	pub telegram_api_base: String,
}

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_TELEGRAM_API_BASE: &str = "https://api.telegram.org";
const DEFAULT_ENV_FILE: &str = ".env";

impl Config {
	pub fn from_env() -> anyhow::Result<Self> {
		let process_env = |key: &str| std::env::var(key).ok();
		let env_file = process_env("ENV_FILE").filter(|path| !path.is_empty());
		let layers = EnvLayers::resolve(
			env_file.as_deref().map(Path::new),
			Path::new(DEFAULT_ENV_FILE),
			process_env,
		)?;
		Self::from_lookup(|key| layers.get(key))
	}

	fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
		let optional = |key: &str| lookup(key).filter(|value| !value.is_empty());
		let required = |key: &str| {
			optional(key).with_context(|| format!("{key} environment variable is required"))
		};

		let room_id = required("ROOM_ID")?;
		anyhow::ensure!(
			room_id.starts_with('!'),
			"ROOM_ID must be an internal room id starting with '!', got {room_id:?}"
		);
		let app_service_user = required("APP_SERVICE_USER")?;
		anyhow::ensure!(
			split_user_id(&app_service_user).is_some(),
			"APP_SERVICE_USER must be a user id like @localpart:server, got {app_service_user:?}"
		);

		Ok(Self {
			homeserver_url: required("HOMESERVER_URL")?
				.trim_end_matches('/')
				.to_string(),
			room_id,
			host: optional("HOST").unwrap_or_else(|| DEFAULT_HOST.to_string()),
			port: required("PORT")?
				.parse()
				.context("PORT must be a number between 0 and 65535")?,
			homeserver_token: required("HOMESERVER_TOKEN")?,
			app_service_token: required("APP_SERVICE_TOKEN")?,
			app_service_user,
			telegram_chat_id: required("TELEGRAM_CHAT_ID")?,
			telegram_bot_token: required("TELEGRAM_BOT_TOKEN")?,
			telegram_api_base: optional("TELEGRAM_API_BASE")
				.map(|base| base.trim_end_matches('/').to_string())
				.filter(|base| !base.is_empty())
				.unwrap_or_else(|| DEFAULT_TELEGRAM_API_BASE.to_string()),
		})
	}

	pub fn app_service_localpart(&self) -> &str {
		split_user_id(&self.app_service_user).map_or("", |(localpart, _)| localpart)
	}

	pub fn server_name(&self) -> &str {
		split_user_id(&self.app_service_user).map_or("", |(_, server_name)| server_name)
	}
}

fn split_user_id(user_id: &str) -> Option<(&str, &str)> {
	let (localpart, server_name) = user_id.strip_prefix('@')?.split_once(':')?;
	(!localpart.is_empty() && !server_name.is_empty()).then_some((localpart, server_name))
}

struct EnvLayers<E> {
	file: HashMap<String, String>,
	file_overrides_env: bool,
	env: E,
}

impl<E: Fn(&str) -> Option<String>> EnvLayers<E> {
	fn resolve(explicit_file: Option<&Path>, default_file: &Path, env: E) -> anyhow::Result<Self> {
		let (file, file_overrides_env) = match explicit_file {
			Some(path) => (read_env_file(path)?, true),
			None if default_file.is_file() => (read_env_file(default_file)?, false),
			None => (HashMap::new(), false),
		};
		Ok(Self {
			file,
			file_overrides_env,
			env,
		})
	}

	fn get(&self, key: &str) -> Option<String> {
		let from_file = || {
			self.file
				.get(key)
				.filter(|value| !value.is_empty())
				.cloned()
		};
		let from_env = || (self.env)(key).filter(|value| !value.is_empty());
		if self.file_overrides_env {
			from_file().or_else(from_env)
		} else {
			from_env().or_else(from_file)
		}
	}
}

fn read_env_file(path: &Path) -> anyhow::Result<HashMap<String, String>> {
	let describe = || format!("reading env file {}", path.display());
	dotenvy::from_path_iter(path)
		.with_context(describe)?
		.map(|entry| entry.with_context(describe))
		.collect()
}

#[cfg(test)]
mod tests;
