use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{Config, EnvLayers};

struct TempFile(PathBuf);

impl TempFile {
	fn new(name: &str, contents: &str) -> Self {
		let path = std::env::temp_dir().join(format!("m2tg-{}-{name}", std::process::id()));
		std::fs::write(&path, contents).unwrap();
		Self(path)
	}
}

impl Drop for TempFile {
	fn drop(&mut self) {
		let _ = std::fs::remove_file(&self.0);
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

fn with_var(key: &'static str, value: &'static str) -> Vec<(&'static str, &'static str)> {
	let mut vars = valid_vars();
	vars.retain(|(k, _)| *k != key);
	vars.push((key, value));
	vars
}

#[test]
fn explicit_env_file_overrides_process_env() {
	let file = TempFile::new("explicit.env", "PORT=1111\nHOST=\"0.0.0.0\"\n");
	let missing_default = Path::new("/nonexistent/m2tg/.env");
	let layers = EnvLayers::resolve(
		Some(&file.0),
		missing_default,
		fake_env(&[("PORT", "2222"), ("ROOM_ID", "!env:test")]),
	)
	.unwrap();
	assert_eq!(layers.get("PORT").as_deref(), Some("1111"));
	assert_eq!(layers.get("HOST").as_deref(), Some("0.0.0.0"));
	assert_eq!(layers.get("ROOM_ID").as_deref(), Some("!env:test"));
	assert_eq!(layers.get("MISSING"), None);
}

#[test]
fn empty_values_are_treated_as_unset() {
	let file = TempFile::new("empty.env", "PORT=\nHOST=\"\"\nROOM_ID=!file:test\n");
	let missing_default = Path::new("/nonexistent/m2tg/.env");
	let env = || fake_env(&[("PORT", "2222"), ("ROOM_ID", "")]);
	let explicit = EnvLayers::resolve(Some(&file.0), missing_default, env()).unwrap();
	assert_eq!(explicit.get("PORT").as_deref(), Some("2222"));
	assert_eq!(explicit.get("HOST"), None);
	assert_eq!(explicit.get("ROOM_ID").as_deref(), Some("!file:test"));
	let default = EnvLayers::resolve(None, &file.0, env()).unwrap();
	assert_eq!(default.get("PORT").as_deref(), Some("2222"));
	assert_eq!(default.get("ROOM_ID").as_deref(), Some("!file:test"));
}

#[test]
fn default_env_file_yields_to_process_env() {
	let file = TempFile::new("default.env", "PORT=1111\nHOST=0.0.0.0\n");
	let layers = EnvLayers::resolve(None, &file.0, fake_env(&[("PORT", "2222")])).unwrap();
	assert_eq!(layers.get("PORT").as_deref(), Some("2222"));
	assert_eq!(layers.get("HOST").as_deref(), Some("0.0.0.0"));
}

#[test]
fn missing_default_env_file_uses_process_env_only() {
	let layers = EnvLayers::resolve(
		None,
		Path::new("/nonexistent/m2tg/.env"),
		fake_env(&[("PORT", "2222")]),
	)
	.unwrap();
	assert_eq!(layers.get("PORT").as_deref(), Some("2222"));
}

#[test]
fn missing_explicit_env_file_is_an_error() {
	let result = EnvLayers::resolve(
		Some(Path::new("/nonexistent/m2tg/explicit.env")),
		Path::new("/nonexistent/m2tg/.env"),
		fake_env(&[]),
	);
	assert!(result.is_err());
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
