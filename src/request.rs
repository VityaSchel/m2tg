use std::fmt;
use std::time::Duration;

use reqwest::StatusCode;

const EXCERPT_LIMIT: usize = 200;

#[derive(Debug)]
pub enum RequestError {
	Transient {
		retry_after: Option<Duration>,
		message: String,
	},
	Permanent {
		message: String,
	},
}

impl RequestError {
	pub fn network(error: reqwest::Error) -> Self {
		Self::Transient {
			retry_after: None,
			message: format!("{:#}", anyhow::Error::from(error.without_url())),
		}
	}

	pub fn from_status(status: StatusCode, retry_after: Option<Duration>, message: String) -> Self {
		if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
			Self::Transient {
				retry_after,
				message,
			}
		} else {
			Self::Permanent { message }
		}
	}
}

impl fmt::Display for RequestError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Transient { message, .. } | Self::Permanent { message } => f.write_str(message),
		}
	}
}

pub fn excerpt(text: &str) -> String {
	let mut chars = text.chars().map(|c| if c.is_control() { ' ' } else { c });
	let mut out: String = chars.by_ref().take(EXCERPT_LIMIT).collect();
	if chars.next().is_some() {
		out.push('…');
	}
	out
}

#[cfg(test)]
mod tests;
