use std::time::Duration;

use anyhow::Context;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;

use crate::config::Config;
use crate::request::{self, RequestError};

#[derive(Deserialize)]
pub struct Transaction {
	#[serde(default)]
	pub events: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct Event {
	#[serde(rename = "type")]
	pub event_type: String,
	pub event_id: String,
	pub room_id: String,
	pub sender: String,
	pub state_key: Option<String>,
	#[serde(default)]
	pub content: Content,
}

#[derive(Deserialize, PartialEq, Eq, Clone, Copy, Debug)]
pub enum Msgtype {
	#[serde(rename = "m.text")]
	Text,
	#[serde(rename = "m.image")]
	Image,
	#[serde(rename = "m.video")]
	Video,
	#[serde(rename = "m.audio")]
	Audio,
	#[serde(rename = "m.file")]
	File,
	#[serde(rename = "m.emote")]
	Emote,
	#[serde(rename = "m.notice")]
	Notice,
	#[serde(other)]
	Other,
}

#[derive(Deserialize, Default)]
pub struct Content {
	pub msgtype: Option<Msgtype>,
	pub body: Option<String>,
	pub formatted_body: Option<String>,
	pub format: Option<String>,
	pub url: Option<String>,
	pub filename: Option<String>,
	pub info: Option<MediaInfo>,
	pub membership: Option<String>,
	pub replacement_room: Option<String>,
	#[serde(rename = "m.relates_to")]
	pub relates_to: Option<RelatesTo>,
}

impl Content {
	pub fn is_edit(&self) -> bool {
		self.relates_to
			.as_ref()
			.is_some_and(|relation| relation.rel_type.as_deref() == Some("m.replace"))
	}

	pub fn is_reply(&self) -> bool {
		self.relates_to
			.as_ref()
			.is_some_and(|relation| relation.in_reply_to.is_some())
	}
}

#[derive(Deserialize)]
pub struct MediaInfo {
	pub mimetype: Option<String>,
}

#[derive(Deserialize)]
pub struct RelatesTo {
	pub rel_type: Option<String>,
	#[serde(rename = "m.in_reply_to")]
	pub in_reply_to: Option<serde::de::IgnoredAny>,
}

pub struct Media {
	pub bytes: Bytes,
	pub content_type: String,
}

#[derive(Deserialize)]
struct ErrorResponse {
	retry_after_ms: Option<u64>,
}

pub fn strip_reply_fallback(body: &str) -> String {
	if !body.starts_with("> <") && !body.starts_with("> * <") {
		return body.to_string();
	}
	let lines: Vec<&str> = body.split('\n').collect();
	let mut i = 0;
	while i < lines.len() && lines[i].starts_with("> ") {
		i += 1;
	}
	if i < lines.len() && lines[i].is_empty() {
		i += 1;
	}
	lines[i..].join("\n")
}

pub async fn download_media(
	http: &Client,
	config: &Config,
	mxc: &str,
) -> Result<Media, RequestError> {
	let (server, media_id) = mxc
		.strip_prefix("mxc://")
		.and_then(|rest| rest.split_once('/'))
		.ok_or_else(|| RequestError::Permanent {
			message: format!("malformed mxc uri {mxc:?}"),
		})?;
	let url = format!(
		"{}/_matrix/client/v1/media/download/{}/{}",
		config.homeserver_url,
		urlencoding::encode(server),
		urlencoding::encode(media_id),
	);
	let res = http
		.get(url)
		.bearer_auth(&config.app_service_token)
		.send()
		.await
		.map_err(RequestError::network)?;
	let status = res.status();
	if !status.is_success() {
		let body = res.bytes().await.unwrap_or_default();
		let retry_after = serde_json::from_slice::<ErrorResponse>(&body)
			.ok()
			.and_then(|error| error.retry_after_ms)
			.map(Duration::from_millis);
		let text = String::from_utf8_lossy(&body);
		return Err(RequestError::from_status(
			status,
			retry_after,
			format!(
				"media download {mxc}: {} {}",
				status.as_u16(),
				request::excerpt(&text)
			),
		));
	}
	let content_type = res
		.headers()
		.get(reqwest::header::CONTENT_TYPE)
		.and_then(|v| v.to_str().ok())
		.unwrap_or("application/octet-stream")
		.to_string();
	let bytes = res.bytes().await.map_err(RequestError::network)?;
	Ok(Media {
		bytes,
		content_type,
	})
}

pub async fn join_room(http: &Client, config: &Config) -> anyhow::Result<()> {
	let room_id = &config.room_id;
	let url = format!(
		"{}/_matrix/client/v3/join/{}?user_id={}",
		config.homeserver_url,
		urlencoding::encode(room_id),
		urlencoding::encode(&config.app_service_user),
	);
	let res = http
		.post(url)
		.bearer_auth(&config.app_service_token)
		.json(&serde_json::json!({}))
		.send()
		.await
		.map_err(reqwest::Error::without_url)
		.context("join request failed")?;
	if !res.status().is_success() {
		let status = res.status();
		let text = res.text().await.unwrap_or_default();
		anyhow::bail!(
			"failed to join room {room_id}: {} {}",
			status.as_u16(),
			request::excerpt(&text)
		);
	}
	Ok(())
}

#[cfg(test)]
mod tests;
