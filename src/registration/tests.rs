use super::registration_yaml;
use crate::config::Config;

fn config(app_service_user: &str, host: &str) -> Config {
	Config {
		homeserver_url: "https://matrix.example.org".into(),
		room_id: "!room:example.org".into(),
		host: host.into(),
		port: 8080,
		homeserver_token: "hs-token".into(),
		app_service_token: "as-token".into(),
		app_service_user: app_service_user.into(),
		telegram_chat_id: "42".into(),
		telegram_bot_token: "123:abc".into(),
		telegram_api_base: "https://api.telegram.org".into(),
	}
}

fn url_line(host: &str) -> String {
	registration_yaml(&config("@m2tg:example.org", host), &[])
		.lines()
		.find(|line| line.starts_with("url: "))
		.unwrap()
		.to_string()
}

fn regex_lines(yaml: &str) -> Vec<&str> {
	yaml.lines()
		.filter_map(|line| line.trim_start().strip_prefix("regex: "))
		.collect()
}

#[test]
fn renders_the_registration_for_a_typical_config() {
	let yaml = registration_yaml(&config("@m2tg:example.org", "127.0.0.1"), &[]);
	assert_eq!(
		yaml,
		"id: 'm2tg'
url: 'http://127.0.0.1:8080'
as_token: 'as-token'
hs_token: 'hs-token'
sender_localpart: 'm2tg'
rate_limited: false
namespaces:
  users:
    - exclusive: true
      regex: '^@m2tg:example\\.org$'
  aliases: []
  rooms: []
"
	);
}

#[test]
fn escapes_regex_metacharacters_and_keeps_the_port_colon() {
	let yaml = registration_yaml(&config("@a.b+c:example.org:8448", "127.0.0.1"), &[]);
	assert_eq!(regex_lines(&yaml), [r"'^@a\.b\+c:example\.org:8448$'"]);
	assert!(yaml.contains("id: 'a.b+c'\n"));
}

#[test]
fn adds_a_namespace_entry_per_extra_localpart() {
	let yaml = registration_yaml(
		&config("@m2tg-staging:example.org", "127.0.0.1"),
		&["m2tg-staging-sender"],
	);
	assert_eq!(
		regex_lines(&yaml),
		[
			r"'^@m2tg-staging:example\.org$'",
			r"'^@m2tg-staging-sender:example\.org$'",
		]
	);
	assert_eq!(yaml.matches("- exclusive: true").count(), 2);
	assert!(yaml.contains("sender_localpart: 'm2tg-staging'\n"));
}

#[test]
fn points_url_at_a_reachable_address() {
	assert_eq!(url_line("0.0.0.0"), "url: 'http://127.0.0.1:8080'");
	assert_eq!(url_line("::"), "url: 'http://[::1]:8080'");
	assert_eq!(url_line("::1"), "url: 'http://[::1]:8080'");
	assert_eq!(url_line("10.0.0.2"), "url: 'http://10.0.0.2:8080'");
	assert_eq!(
		url_line("m2tg.internal"),
		"url: 'http://m2tg.internal:8080'"
	);
}

#[test]
fn quotes_values_for_yaml() {
	let mut config = config("@m2tg:example.org", "127.0.0.1");
	config.app_service_token = "it's: #1".into();
	let yaml = registration_yaml(&config, &[]);
	assert!(yaml.contains("as_token: 'it''s: #1'\n"));
}
