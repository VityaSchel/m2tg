use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::config::Config;

const REGEX_METACHARACTERS: &[char] = &[
	'.', '+', '*', '?', '(', ')', '|', '[', ']', '{', '}', '^', '$', '\\',
];

pub fn registration_yaml(config: &Config, extra_localparts: &[&str]) -> String {
	let localpart = config.app_service_localpart();
	let server_name = escape_regex(config.server_name());
	let users: String = std::iter::once(localpart)
		.chain(extra_localparts.iter().copied())
		.map(|user| {
			let regex = format!("^@{}:{server_name}$", escape_regex(user));
			format!("    - exclusive: true\n      regex: {}\n", quoted(&regex))
		})
		.collect();
	let id = quoted(localpart);
	let url = quoted(&app_service_url(config));
	let as_token = quoted(&config.app_service_token);
	let hs_token = quoted(&config.homeserver_token);
	format!(
		"id: {id}\nurl: {url}\nas_token: {as_token}\nhs_token: {hs_token}\nsender_localpart: {id}\nrate_limited: false\nnamespaces:\n  users:\n{users}  aliases: []\n  rooms: []\n"
	)
}

fn app_service_url(config: &Config) -> String {
	let host = match config.host.parse::<IpAddr>() {
		Ok(IpAddr::V4(ip)) if ip.is_unspecified() => Ipv4Addr::LOCALHOST.to_string(),
		Ok(IpAddr::V6(ip)) if ip.is_unspecified() => format!("[{}]", Ipv6Addr::LOCALHOST),
		Ok(IpAddr::V6(ip)) => format!("[{ip}]"),
		_ => config.host.clone(),
	};
	format!("http://{host}:{}", config.port)
}

fn escape_regex(text: &str) -> String {
	text.chars().fold(String::new(), |mut escaped, character| {
		if REGEX_METACHARACTERS.contains(&character) {
			escaped.push('\\');
		}
		escaped.push(character);
		escaped
	})
}

fn quoted(value: &str) -> String {
	format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests;
