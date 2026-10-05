mod appservice;
mod captions;
mod failures;
mod media;
mod mock;
mod text;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderValue, Method, Request, StatusCode};
use m2tg::config::Config;
use mock::{Mock, TelegramCall};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tower::ServiceExt;

const ROOM_ID: &str = "!room:test";
const BRIDGE_USER: &str = "@m2tg:test";
const ALICE: &str = "@alice:test";
const BOB: &str = "@bob:test";
const HS_TOKEN: &str = "hs-secret";
const AS_TOKEN: &str = "as-secret";
const CHAT_ID: &str = "42";
const MEDIA_SERVER: &str = "media.test";
const LAST_MESSAGE: &str = "last message";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nnot really a png";
const GIF: &[u8] = b"GIF89a not really a gif";

struct TestBridge {
	mock: Mock,
	app: Router,
	outbox: JoinHandle<()>,
}

impl TestBridge {
	async fn start() -> Self {
		let _ = rustls::crypto::ring::default_provider().install_default();
		let mock = Mock::start().await;
		let config = Arc::new(Config {
			homeserver_url: mock.base.clone(),
			room_id: ROOM_ID.into(),
			host: "127.0.0.1".into(),
			port: 0,
			homeserver_token: HS_TOKEN.into(),
			app_service_token: AS_TOKEN.into(),
			app_service_user: BRIDGE_USER.into(),
			telegram_chat_id: CHAT_ID.into(),
			telegram_bot_token: "123:TESTTOKEN".into(),
			telegram_api_base: mock.base.clone(),
		});
		let (app, outbox) = m2tg::appservice::router(config, reqwest::Client::new());
		Self { mock, app, outbox }
	}
}

fn json_request(method: Method, uri: &str, body: Value) -> Request<Body> {
	Request::builder()
		.method(method)
		.uri(uri)
		.header(CONTENT_TYPE, "application/json")
		.body(Body::from(body.to_string()))
		.unwrap()
}

fn with_bearer(mut request: Request<Body>, token: &str) -> Request<Body> {
	let value = HeaderValue::from_str(&format!("Bearer {token}")).unwrap();
	request.headers_mut().insert(AUTHORIZATION, value);
	request
}

fn transaction_uri(txn_id: &str) -> String {
	format!("/_matrix/app/v1/transactions/{txn_id}")
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
	let response = app.clone().oneshot(request).await.unwrap();
	let status = response.status();
	let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
	(
		status,
		serde_json::from_slice(&bytes).unwrap_or(Value::Null),
	)
}

async fn put_txn(app: &Router, txn_id: &str, events: Vec<Value>) {
	let uri = format!("{}?access_token={HS_TOKEN}", transaction_uri(txn_id));
	let request = json_request(Method::PUT, &uri, json!({ "events": events }));
	let (status, body) = send(app, request).await;
	assert_eq!((status, body), (StatusCode::OK, json!({})), "txn {txn_id}");
}

fn event(event_type: &str, event_id: &str, content: Value) -> Value {
	json!({
		"type": event_type,
		"event_id": event_id,
		"room_id": ROOM_ID,
		"sender": ALICE,
		"origin_server_ts": 1_757_750_400_000_u64,
		"content": content,
	})
}

fn message(event_id: &str, content: Value) -> Value {
	event("m.room.message", event_id, content)
}

fn text(event_id: &str, body: &str) -> Value {
	message(event_id, json!({ "msgtype": "m.text", "body": body }))
}

fn html(event_id: &str, body: &str, formatted_body: &str) -> Value {
	message(
		event_id,
		json!({
			"msgtype": "m.text",
			"body": body,
			"format": "org.matrix.custom.html",
			"formatted_body": formatted_body,
		}),
	)
}

fn last_message() -> Value {
	from(BOB, text("$last", LAST_MESSAGE))
}

fn from(sender: &str, mut event: Value) -> Value {
	event["sender"] = json!(sender);
	event
}

fn sent_after(delay: Duration, mut event: Value) -> Value {
	let origin_server_ts = event["origin_server_ts"].as_u64().unwrap();
	event["origin_server_ts"] = json!(origin_server_ts + delay.as_millis() as u64);
	event
}

fn mxc(media_id: &str) -> String {
	format!("mxc://{MEDIA_SERVER}/{media_id}")
}

fn image(event_id: &str, media_id: &str) -> Value {
	message(
		event_id,
		json!({
			"msgtype": "m.image",
			"body": format!("{media_id}.png"),
			"url": mxc(media_id),
			"info": { "mimetype": "image/png", "size": PNG.len(), "w": 1, "h": 1 },
		}),
	)
}

fn membership(event_id: &str, state_key: &str, membership: &str) -> Value {
	let mut member = event(
		"m.room.member",
		event_id,
		json!({ "membership": membership }),
	);
	member["state_key"] = json!(state_key);
	member
}

fn methods(calls: &[TelegramCall]) -> Vec<&str> {
	calls.iter().map(|call| call.method.as_str()).collect()
}

fn texts(calls: &[TelegramCall]) -> Vec<&str> {
	calls.iter().map(TelegramCall::text).collect()
}

fn bad_request(description: &str) -> Value {
	json!({ "ok": false, "error_code": 400, "description": description })
}
