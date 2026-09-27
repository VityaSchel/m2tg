use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Json, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use reqwest::Client;
use serde_json::json;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;

use crate::config::Config;
use crate::matrix::{self, Content, Event, Msgtype, Transaction};
use crate::outbox::{self, Job, Origin};
use crate::{format, telegram};

const BODY_LIMIT: usize = 16 * 1024 * 1024;
const RECENT_IDS_CAPACITY: usize = 4096;
const MAX_HTML_BYTES: usize = 32768;
const JOIN_RETRY_DELAYS_SECS: [u64; 4] = [0, 2, 10, 60];

struct AppState {
	config: Arc<Config>,
	outbox: outbox::Sender,
	join_requests: mpsc::UnboundedSender<()>,
	recent_ids: Mutex<RecentIds>,
	encrypted_warned: AtomicBool,
}

#[derive(Default)]
struct RecentIds {
	order: VecDeque<String>,
	set: HashSet<String>,
}

impl RecentIds {
	fn contains(&self, id: &str) -> bool {
		self.set.contains(id)
	}

	fn insert(&mut self, id: String) {
		if self.set.contains(&id) {
			return;
		}
		if self.order.len() >= RECENT_IDS_CAPACITY {
			if let Some(oldest) = self.order.pop_front() {
				self.set.remove(&oldest);
			}
		}
		self.set.insert(id.clone());
		self.order.push_back(id);
	}
}

pub fn router(config: Arc<Config>, http: Client) -> (Router, JoinHandle<()>) {
	let (outbox, worker) = outbox::spawn(config.clone(), http.clone());
	let (join_requests, requested_joins) = mpsc::unbounded_channel();
	tokio::spawn(join_on_request(config.clone(), http, requested_joins));
	let state = Arc::new(AppState {
		config,
		outbox,
		join_requests,
		recent_ids: Mutex::new(RecentIds::default()),
		encrypted_warned: AtomicBool::new(false),
	});
	let protected = Router::new()
		.route("/_matrix/app/v1/transactions/{txn_id}", put(transactions))
		.route("/_matrix/app/v1/ping", post(ping))
		.layer(middleware::from_fn_with_state(state.clone(), auth))
		.layer(DefaultBodyLimit::max(BODY_LIMIT));
	let router = Router::new()
		.merge(protected)
		.route("/health", get(health))
		.fallback(not_found)
		.with_state(state);
	(router, worker)
}

pub fn spawn_room_join(config: Arc<Config>, http: Client) {
	tokio::spawn(async move { join_with_retries(&config, &http).await });
}

async fn join_on_request(
	config: Arc<Config>,
	http: Client,
	mut requests: mpsc::UnboundedReceiver<()>,
) {
	while requests.recv().await.is_some() {
		join_with_retries(&config, &http).await;
	}
}

async fn join_with_retries(config: &Config, http: &Client) {
	for delay in JOIN_RETRY_DELAYS_SECS.map(Duration::from_secs) {
		tokio::time::sleep(delay).await;
		match matrix::join_room(http, config).await {
			Ok(()) => {
				tracing::info!("joined {}", config.room_id);
				return;
			}
			Err(e) => tracing::warn!("joining {} failed: {e:#}", config.room_id),
		}
	}
	tracing::error!(
		"giving up joining {}; inviting {} retries",
		config.room_id,
		config.app_service_user
	);
}

async fn health() -> Json<serde_json::Value> {
	Json(json!({ "ok": true }))
}

async fn ping() -> Json<serde_json::Value> {
	Json(json!({}))
}

async fn not_found() -> Response {
	(
		StatusCode::NOT_FOUND,
		Json(json!({ "errcode": "M_UNRECOGNIZED" })),
	)
		.into_response()
}

fn query_token_matches(req: &Request, token: &str) -> Option<bool> {
	let query = req.uri().query()?;
	let value = query
		.split('&')
		.find_map(|pair| pair.strip_prefix("access_token="))?;
	Some(urlencoding::decode(value).is_ok_and(|decoded| tokens_equal(&decoded, token)))
}

fn header_token_matches(req: &Request, token: &str) -> Option<bool> {
	let value = req.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
	Some(
		value
			.strip_prefix("Bearer ")
			.is_some_and(|given| tokens_equal(given, token)),
	)
}

// black_box keeps the optimizer from turning the fold into an early exit,
// so the running time does not reveal how many leading bytes matched.
fn tokens_equal(given: &str, expected: &str) -> bool {
	let difference = given
		.bytes()
		.zip(expected.bytes())
		.fold(0u8, |acc, (a, b)| std::hint::black_box(acc | (a ^ b)));
	difference == 0 && given.len() == expected.len()
}

fn authorized(token: &str, req: &Request) -> bool {
	match (
		query_token_matches(req, token),
		header_token_matches(req, token),
	) {
		(Some(query_ok), Some(header_ok)) => query_ok && header_ok,
		(Some(ok), None) | (None, Some(ok)) => ok,
		(None, None) => false,
	}
}

async fn auth(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
	if authorized(&state.config.homeserver_token, &req) {
		next.run(req).await
	} else {
		(
			StatusCode::FORBIDDEN,
			Json(json!({ "errcode": "M_FORBIDDEN", "error": "Bad hs_token" })),
		)
			.into_response()
	}
}

async fn transactions(
	State(state): State<Arc<AppState>>,
	Json(body): Json<Transaction>,
) -> Response {
	let mut recent_ids = state.recent_ids.lock().await;
	for raw in body.events {
		let event: Event = match serde_json::from_value(raw) {
			Ok(event) => event,
			Err(e) => {
				tracing::warn!("skipping malformed event: {e}");
				continue;
			}
		};
		let is_message = event.event_type == "m.room.message";
		if event.room_id != state.config.room_id
			|| (is_message && recent_ids.contains(&event.event_id))
		{
			continue;
		}
		if let Some(job) = handle_event(&state, &event) {
			if state.outbox.send(job).await.is_err() {
				return (
					StatusCode::INTERNAL_SERVER_ERROR,
					Json(json!({ "errcode": "M_UNKNOWN", "error": "bridge is shutting down" })),
				)
					.into_response();
			}
		}
		if is_message {
			recent_ids.insert(event.event_id);
		}
	}
	Json(json!({})).into_response()
}

fn handle_event(state: &AppState, event: &Event) -> Option<Job> {
	match event.event_type.as_str() {
		"m.room.member" => {
			handle_membership(state, event);
			None
		}
		"m.room.tombstone" if event.state_key.as_deref() == Some("") => {
			let replacement = event.content.replacement_room.as_deref();
			tracing::error!(
				"room {} was upgraded to {}; update ROOM_ID to keep bridging",
				event.room_id,
				replacement.unwrap_or("an unknown room"),
			);
			None
		}
		"m.room.encrypted" => {
			if !state.encrypted_warned.swap(true, Ordering::Relaxed) {
				tracing::warn!(
					"room {} has encrypted events, which cannot be bridged",
					event.room_id
				);
			}
			None
		}
		"m.room.message"
			if event.sender != state.config.app_service_user && !event.content.is_edit() =>
		{
			let origin = Origin {
				sender: event.sender.clone(),
				origin_server_ts: event.origin_server_ts,
			};
			message_job(&event.content, origin)
		}
		_ => None,
	}
}

fn handle_membership(state: &AppState, event: &Event) {
	if event.state_key.as_deref() != Some(state.config.app_service_user.as_str()) {
		return;
	}
	match event.content.membership.as_deref() {
		Some("invite") => {
			let _ = state.join_requests.send(());
		}
		Some(membership @ ("leave" | "ban")) => tracing::error!(
			"bridge user {} was removed from {} ({membership}); messages will not be bridged",
			state.config.app_service_user,
			event.room_id,
		),
		_ => {}
	}
}

fn message_job(content: &Content, origin: Origin) -> Option<Job> {
	let body = content.body.as_deref().unwrap_or_default();
	let text = if content.is_reply() {
		matrix::strip_reply_fallback(body)
	} else {
		body.to_string()
	};
	match (content.msgtype, content.url.as_deref()) {
		(
			Some(msgtype @ (Msgtype::Image | Msgtype::Video | Msgtype::Audio | Msgtype::File)),
			Some(mxc),
		) => Some(media_job(content, origin, msgtype, mxc, text)),
		_ => {
			let html = telegram_html(content, &text)?;
			Some(Job::Text {
				origin,
				html,
				plain: text,
				is_reply: content.is_reply(),
			})
		}
	}
}

fn media_job(content: &Content, origin: Origin, msgtype: Msgtype, mxc: &str, text: String) -> Job {
	let original_name = content
		.filename
		.as_deref()
		.filter(|name| !name.is_empty())
		.or(content.body.as_deref())
		.unwrap_or_default();
	let caption_html = if text.is_empty() || text == original_name {
		None
	} else {
		telegram_html(content, &text)
	};
	let (caption_html, caption_plain) = match caption_html {
		Some(html) => (html, text),
		None => (String::new(), String::new()),
	};
	Job::Media {
		origin,
		mxc: mxc.to_string(),
		msgtype,
		mimetype: content.info.as_ref().and_then(|info| info.mimetype.clone()),
		filename: original_name.chars().filter(|c| !c.is_control()).collect(),
		caption_html,
		caption_plain,
	}
}

fn telegram_html(content: &Content, plain: &str) -> Option<String> {
	let html = format::render_text(content, plain);
	if html.len() <= MAX_HTML_BYTES && telegram::has_visible_text(&html) {
		return Some(html);
	}
	let escaped = format::escape_html(plain);
	telegram::has_visible_text(&escaped).then_some(escaped)
}

#[cfg(test)]
mod tests;
