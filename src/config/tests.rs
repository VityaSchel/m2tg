use std::collections::HashMap;
use std::path::PathBuf;

use super::{Config, credential_file_name};

struct TempDir(PathBuf);

impl TempDir {
	fn with_files(name: &str, files: &[(&str, &str)]) -> Self {
		let path = std::env::temp_dir().join(format!("m2tg-{}-{name}", std::process::id()));
		std::fs::create_dir_all(&path).unwrap();
		for (file, contents) in files {
			std::fs::write(path.join(file), contents).unwrap();
		}
		Self(path)
	}

	fn path(&self) -> &str {
		self.0.to_str().unwrap()
	}
}

impl Drop for TempDir {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.0);
	}
}

fn fake_env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
	let map: HashMap<String, String> = pairs
		.iter()
		.map(|(k, v)| (k.to_string(), v.to_string()))
		.collect();
	move |key| map.get(key).cloned()
}

fn valid_vars() -> Vec<(&'static str, &'static str)> {
	vec![
		("HOMESERVER_URL", "https://matrix.test"),
		("ROOM_ID", "!room:test"),
		("PORT", "8080"),
		("HOMESERVER_TOKEN", "hs"),
		("APP_SERVICE_TOKEN", "as"),
		("APP_SERVICE_USER", "@m2tg:test"),
		("TELEGRAM_CHAT_ID", "42"),
		("TELEGRAM_BOT_TOKEN", "123:abc"),
	]
}

fn with_var<'a>(key: &'static str, value: &'a str) -> Vec<(&'static str, &'a str)> {
	let mut vars = valid_vars();
	vars.retain(|(k, _)| *k != key);
	vars.push((key, value));
	vars
}

fn credential_files() -> Vec<(&'static str, &'static str)> {
	vec![
		("app-service-token", "file-as\n"),
		("homeserver-token", "file-hs\n"),
		("telegram-bot-token", "456:file\n"),
	]
}

fn load_error(vars: &[(&str, &str)]) -> String {
	Config::from_env_and_credentials(fake_env(vars))
		.err()
		.map(|e| e.to_string())
		.unwrap_or_default()
}

#[test]
fn derives_the_credential_file_name_from_the_variable() {
	assert_eq!(
		credential_file_name("APP_SERVICE_TOKEN"),
		"app-service-token"
	);
	assert_eq!(credential_file_name("HOMESERVER_TOKEN"), "homeserver-token");
	assert_eq!(
		credential_file_name("TELEGRAM_BOT_TOKEN"),
		"telegram-bot-token"
	);
}

#[test]
fn tokens_come_from_credential_files_when_the_directory_is_set() {
	let directory = TempDir::with_files("credentials", &credential_files());
	let vars = with_var("CREDENTIALS_DIRECTORY", directory.path());
	let config = Config::from_env_and_credentials(fake_env(&vars)).unwrap();
	assert_eq!(config.app_service_token, "file-as");
	assert_eq!(config.homeserver_token, "file-hs");
	assert_eq!(config.telegram_bot_token, "456:file");
	assert_eq!(config.telegram_chat_id, "42");
}

#[test]
fn credentials_do_not_fall_back_to_the_environment() {
	let directory = TempDir::with_files("missing", &credential_files()[..2]);
	let vars = with_var("CREDENTIALS_DIRECTORY", directory.path());
	let expected = format!(
		"reading credential {}/telegram-bot-token: is LoadCredential=telegram-bot-token missing from the unit?",
		directory.path()
	);
	assert_eq!(load_error(&vars), expected);
}

#[test]
fn rejects_an_empty_credential() {
	let mut files = credential_files();
	files[1].1 = "\n";
	let directory = TempDir::with_files("empty", &files);
	let vars = with_var("CREDENTIALS_DIRECTORY", directory.path());
	let expected = format!("{}/homeserver-token is empty", directory.path());
	assert_eq!(load_error(&vars), expected);
}

#[test]
fn tokens_come_from_the_environment_without_a_credentials_directory() {
	for vars in [valid_vars(), with_var("CREDENTIALS_DIRECTORY", "")] {
		let config = Config::from_env_and_credentials(fake_env(&vars)).unwrap();
		assert_eq!(config.app_service_token, "as");
		assert_eq!(config.homeserver_token, "hs");
		assert_eq!(config.telegram_bot_token, "123:abc");
	}
	assert!(load_error(&with_var("HOMESERVER_TOKEN", "")).contains("HOMESERVER_TOKEN"));
}

#[test]
fn defaults_and_normalization() {
	let mut vars = with_var("TELEGRAM_API_BASE", "http://127.0.0.1:9000/");
	vars.retain(|(k, _)| *k != "HOMESERVER_URL");
	vars.push(("HOMESERVER_URL", "https://matrix.test/"));
	let config = Config::from_lookup(fake_env(&vars)).unwrap();
	assert_eq!(config.homeserver_url, "https://matrix.test");
	assert_eq!(config.host, "127.0.0.1");
	assert_eq!(config.port, 8080);
	assert_eq!(config.telegram_api_base, "http://127.0.0.1:9000");

	let config = Config::from_lookup(fake_env(&with_var("TELEGRAM_API_BASE", ""))).unwrap();
	assert_eq!(config.telegram_api_base, "https://api.telegram.org");
}

#[test]
fn rejects_invalid_identifiers_and_missing_values() {
	let error = |vars: &[(&str, &str)]| {
		Config::from_lookup(fake_env(vars))
			.err()
			.map(|e| e.to_string())
			.unwrap_or_default()
	};
	assert!(error(&with_var("ROOM_ID", "#alias:test")).contains("ROOM_ID"));
	for user in ["m2tg", "m2tg:test", "@m2tg", "@:test", "@m2tg:"] {
		assert!(
			error(&with_var("APP_SERVICE_USER", user)).contains("APP_SERVICE_USER"),
			"{user}"
		);
	}
	assert!(error(&with_var("PORT", "")).contains("PORT"));
}

#[test]
fn splits_app_service_user_at_the_first_colon() {
	let config = Config::from_lookup(fake_env(&valid_vars())).unwrap();
	assert_eq!(config.app_service_localpart(), "m2tg");
	assert_eq!(config.server_name(), "test");

	let vars = with_var("APP_SERVICE_USER", "@m2tg-staging:example.org:8448");
	let config = Config::from_lookup(fake_env(&vars)).unwrap();
	assert_eq!(config.app_service_localpart(), "m2tg-staging");
	assert_eq!(config.server_name(), "example.org:8448");
}
