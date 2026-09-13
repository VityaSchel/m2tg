use std::time::Duration;

use bytes::Bytes;
use reqwest::multipart::{Form, Part};
use reqwest::{Body, Client, StatusCode};
use serde::Deserialize;
use serde_json::json;

use crate::config::Config;
use crate::matrix::Msgtype;
use crate::request::{self, RequestError};

const MESSAGE_LIMIT: usize = 4096;
const CAPTION_LIMIT: usize = 1024;
const PHOTO_SIZE_LIMIT: usize = 10 * 1024 * 1024;
const ELLIPSIS: char = '…';
const FALLBACK_MIME: &str = "application/octet-stream";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
	Photo,
	Animation,
	Video,
	Voice,
	Audio,
	Document,
}

impl MediaKind {
	pub fn classify(msgtype: Msgtype, content_type: &str, size: usize) -> Self {
		if msgtype == Msgtype::File {
			return Self::Document;
		}
		let fits_photo = size <= PHOTO_SIZE_LIMIT;
		match mime_essence(content_type).split_once('/') {
			Some(("image", "gif")) => Self::Animation,
			Some(("image", "jpeg" | "png" | "webp")) if fits_photo => Self::Photo,
			Some(("image", _)) => Self::Document,
			Some(("video", _)) => Self::Video,
			Some(("audio", "ogg")) => Self::Voice,
			Some(("audio", _)) => Self::Audio,
			_ => match msgtype {
				Msgtype::Image if fits_photo => Self::Photo,
				Msgtype::Video => Self::Video,
				Msgtype::Audio => Self::Audio,
				_ => Self::Document,
			},
		}
	}

	pub fn method(self) -> &'static str {
		match self {
			Self::Photo => "sendPhoto",
			Self::Animation => "sendAnimation",
			Self::Video => "sendVideo",
			Self::Voice => "sendVoice",
			Self::Audio => "sendAudio",
			Self::Document => "sendDocument",
		}
	}

	pub fn field(self) -> &'static str {
		match self {
			Self::Photo => "photo",
			Self::Animation => "animation",
			Self::Video => "video",
			Self::Voice => "voice",
			Self::Audio => "audio",
			Self::Document => "document",
		}
	}

	pub fn groupable(self) -> bool {
		matches!(self, Self::Photo | Self::Video)
	}
}

pub fn mime_essence(content_type: &str) -> String {
	content_type
		.split(';')
		.next()
		.unwrap_or_default()
		.trim()
		.to_ascii_lowercase()
}

pub struct OutMedia {
	pub bytes: Bytes,
	pub content_type: String,
	pub filename: String,
	pub caption_html: String,
	pub caption_plain: String,
	pub kind: MediaKind,
}

enum Token<'a> {
	Tag(&'a str),
	Visible(&'a str),
}

fn tokens(html: &str) -> impl Iterator<Item = Token<'_>> {
	let mut rest = html;
	std::iter::from_fn(move || {
		let first = rest.chars().next()?;
		let len = match first {
			'<' => rest.find('>').map_or(rest.len(), |end| end + 1),
			'&' => entity_len(rest),
			other => other.len_utf8(),
		};
		let (token, tail) = rest.split_at(len);
		rest = tail;
		Some(if first == '<' {
			Token::Tag(token)
		} else {
			Token::Visible(token)
		})
	})
}

fn entity_len(text: &str) -> usize {
	text[1..]
		.char_indices()
		.take(10)
		.find(|&(_, c)| c == ';')
		.map_or(1, |(end, _)| end + 2)
}

fn visible_len(html: &str) -> usize {
	tokens(html)
		.filter(|token| matches!(token, Token::Visible(_)))
		.count()
}

pub fn has_visible_text(html: &str) -> bool {
	tokens(html).any(|token| match token {
		Token::Visible(text) => !text.chars().all(char::is_whitespace),
		Token::Tag(_) => false,
	})
}

fn tag_name(tag: &str) -> &str {
	tag.trim_start_matches('<')
		.trim_start_matches('/')
		.trim_end_matches('>')
		.trim_end_matches('/')
		.split([' ', '\t', '\n'])
		.next()
		.unwrap_or("")
}

fn truncate(html: &str, limit: usize) -> String {
	if visible_len(html) <= limit {
		return html.to_string();
	}
	let budget = limit.saturating_sub(1);
	let mut out = String::with_capacity(html.len().min(limit * 4));
	let mut open: Vec<&str> = Vec::new();
	let mut visible = 0;
	for token in tokens(html) {
		if visible >= budget {
			break;
		}
		match token {
			Token::Tag(tag) if tag.starts_with("</") => {
				if let Some(pos) = open.iter().rposition(|name| *name == tag_name(tag)) {
					open.remove(pos);
				}
				out.push_str(tag);
			}
			Token::Tag(tag) => {
				if !tag.ends_with("/>") {
					open.push(tag_name(tag));
				}
				out.push_str(tag);
			}
			Token::Visible(text) => {
				out.push_str(text);
				visible += 1;
			}
		}
	}
	out.push(ELLIPSIS);
	for name in open.iter().rev() {
		out.push_str("</");
		out.push_str(name);
		out.push('>');
	}
	out
}

fn truncate_plain(text: &str, limit: usize) -> String {
	if text.chars().count() <= limit {
		return text.to_string();
	}
	let mut out: String = text.chars().take(limit.saturating_sub(1)).collect();
	out.push(ELLIPSIS);
	out
}

fn api(config: &Config, method: &str) -> String {
	format!(
		"{}/bot{}/{}",
		config.telegram_api_base, config.telegram_bot_token, method
	)
}

fn media_part(media: &OutMedia) -> Part {
	let mime = if Part::text("").mime_str(&media.content_type).is_ok() {
		media.content_type.as_str()
	} else {
		FALLBACK_MIME
	};
	Part::stream_with_length(Body::from(media.bytes.clone()), media.bytes.len() as u64)
		.file_name(media.filename.clone())
		.mime_str(mime)
		.expect("mime validated")
}

#[derive(Deserialize)]
struct ApiResponse {
	ok: bool,
	error_code: Option<u16>,
	description: Option<String>,
	parameters: Option<ResponseParameters>,
}

#[derive(Deserialize)]
struct ResponseParameters {
	retry_after: Option<u64>,
}

async fn check(res: reqwest::Response, method: &str) -> Result<(), RequestError> {
	let status = res.status();
	let body = res.bytes().await.map_err(RequestError::network)?;
	let response = serde_json::from_slice::<ApiResponse>(&body).ok();
	if status.is_success() && response.as_ref().is_none_or(|r| r.ok) {
		return Ok(());
	}
	let code = response
		.as_ref()
		.and_then(|r| r.error_code)
		.and_then(|code| StatusCode::from_u16(code).ok())
		.filter(|_| status.is_success())
		.unwrap_or(status);
	let retry_after = response
		.as_ref()
		.and_then(|r| r.parameters.as_ref())
		.and_then(|p| p.retry_after)
		.map(Duration::from_secs);
	let description = response
		.and_then(|r| r.description)
		.unwrap_or_else(|| String::from_utf8_lossy(&body).into_owned());
	Err(RequestError::from_status(
		code,
		retry_after,
		format!(
			"telegram {method} {}: {}",
			code.as_u16(),
			request::excerpt(&description)
		),
	))
}

async fn post_json(
	http: &Client,
	config: &Config,
	method: &str,
	body: serde_json::Value,
) -> Result<(), RequestError> {
	let res = http
		.post(api(config, method))
		.json(&body)
		.send()
		.await
		.map_err(RequestError::network)?;
	check(res, method).await
}

async fn post_form(
	http: &Client,
	config: &Config,
	method: &str,
	form: Form,
) -> Result<(), RequestError> {
	let res = http
		.post(api(config, method))
		.multipart(form)
		.send()
		.await
		.map_err(RequestError::network)?;
	check(res, method).await
}

pub async fn send_message(http: &Client, config: &Config, html: &str) -> Result<(), RequestError> {
	let body = json!({
		"chat_id": config.telegram_chat_id,
		"text": truncate(html, MESSAGE_LIMIT),
		"parse_mode": "HTML",
		"link_preview_options": { "is_disabled": true },
	});
	post_json(http, config, "sendMessage", body).await
}

pub async fn send_plain_message(
	http: &Client,
	config: &Config,
	plain: &str,
) -> Result<(), RequestError> {
	let body = json!({
		"chat_id": config.telegram_chat_id,
		"text": truncate_plain(plain, MESSAGE_LIMIT),
		"link_preview_options": { "is_disabled": true },
	});
	post_json(http, config, "sendMessage", body).await
}

pub async fn send_media(
	http: &Client,
	config: &Config,
	media: &OutMedia,
) -> Result<(), RequestError> {
	let mut form = Form::new()
		.text("chat_id", config.telegram_chat_id.clone())
		.part(media.kind.field(), media_part(media));
	if !media.caption_html.is_empty() {
		form = form
			.text("caption", truncate(&media.caption_html, CAPTION_LIMIT))
			.text("parse_mode", "HTML");
	}
	post_form(http, config, media.kind.method(), form).await
}

pub async fn send_media_as_document(
	http: &Client,
	config: &Config,
	media: &OutMedia,
) -> Result<(), RequestError> {
	let document = MediaKind::Document;
	let mut form = Form::new()
		.text("chat_id", config.telegram_chat_id.clone())
		.part(document.field(), media_part(media));
	if !media.caption_plain.is_empty() {
		form = form.text(
			"caption",
			truncate_plain(&media.caption_plain, CAPTION_LIMIT),
		);
	}
	post_form(http, config, document.method(), form).await
}

pub async fn send_media_group(
	http: &Client,
	config: &Config,
	items: &[OutMedia],
) -> Result<(), RequestError> {
	let mut form = Form::new().text("chat_id", config.telegram_chat_id.clone());
	let mut entries = Vec::with_capacity(items.len());
	for (i, item) in items.iter().enumerate() {
		let key = format!("file{i}");
		let mut entry = json!({
			"type": item.kind.field(),
			"media": format!("attach://{key}"),
		});
		if !item.caption_html.is_empty() {
			entry["caption"] = json!(truncate(&item.caption_html, CAPTION_LIMIT));
			entry["parse_mode"] = json!("HTML");
		}
		entries.push(entry);
		form = form.part(key, media_part(item));
	}
	form = form.text("media", serde_json::Value::Array(entries).to_string());
	post_form(http, config, "sendMediaGroup", form).await
}

#[cfg(test)]
mod tests;
