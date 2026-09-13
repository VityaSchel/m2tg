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
const CREDENTIALS_DIRECTORY: &str = "CREDENTIALS_DIRECTORY";
const CREDENTIAL_VARIABLES: [&str; 3] = [
	"APP_SERVICE_TOKEN",
	"HOMESERVER_TOKEN",
	"TELEGRAM_BOT_TOKEN",
];

impl Config {
	pub fn from_env() -> anyhow::Result<Self> {
		Self::from_env_and_credentials(|key| std::env::var(key).ok())
	}

	fn from_env_and_credentials(env: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
		let credentials = env(CREDENTIALS_DIRECTORY)
			.filter(|directory| !directory.is_empty())
			.map(|directory| read_credentials(Path::new(&directory)))
			.transpose()?;
		Self::from_lookup(|key| match &credentials {
			Some(credentials) if CREDENTIAL_VARIABLES.contains(&key) => {
				credentials.get(key).cloned()
			}
			_ => env(key),
		})
	}

	pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
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

fn read_credentials(directory: &Path) -> anyhow::Result<HashMap<&'static str, String>> {
	CREDENTIAL_VARIABLES
		.into_iter()
		.map(|variable| Ok((variable, read_credential(directory, variable)?)))
		.collect()
}

fn read_credential(directory: &Path, variable: &str) -> anyhow::Result<String> {
	let name = credential_file_name(variable);
	let path = directory.join(&name);
	let value = std::fs::read_to_string(&path).with_context(|| {
		format!(
			"reading credential {}: is LoadCredential={name} missing from the unit?",
			path.display()
		)
	})?;
	let value = value.trim_end_matches('\n');
	anyhow::ensure!(!value.is_empty(), "{} is empty", path.display());
	Ok(value.to_string())
}

fn credential_file_name(variable: &str) -> String {
	variable.to_ascii_lowercase().replace('_', "-")
}

#[cfg(test)]
mod tests;
