use std::collections::{HashMap, VecDeque};
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{FromRequest, Multipart, Path, Query, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use serde_json::{Value, json};
use tokio::sync::watch;

pub const WAIT_TIMEOUT: Duration = Duration::from_secs(3);

type Log<T> = watch::Sender<Vec<T>>;

#[derive(Clone, Debug)]
pub struct TelegramCall {
	pub method: String,
	pub params: Value,
	pub files: Vec<FilePart>,
	pub received_at: Instant,
}

impl TelegramCall {
	pub fn param(&self, name: &str) -> Option<&str> {
		self.params.get(name).and_then(Value::as_str)
	}

	pub fn text(&self) -> &str {
		self.param("text").unwrap_or_default()
	}

	pub fn media_group(&self) -> Vec<Value> {
		serde_json::from_str(self.param("media").expect("media field")).expect("media JSON")
	}
}

#[derive(Clone, Debug)]
pub struct FilePart {
	pub field: String,
	pub filename: String,
	pub content_type: Option<String>,
	pub bytes: Bytes,
}

#[derive(Clone, Debug)]
pub struct Download {
	pub media_id: String,
	pub authorization: Option<String>,
	pub received_at: Instant,
}

#[derive(Clone, Debug)]
pub struct Join {
	pub room_id: String,
	pub user_id: Option<String>,
	pub authorization: Option<String>,
}

#[derive(Clone)]
struct MediaFile {
	content_type: &'static str,
	bytes: Bytes,
}

struct MockState {
	telegram_calls: Log<TelegramCall>,
	downloads: Log<Download>,
	joins: Log<Join>,
	queued_responses: Mutex<HashMap<String, VecDeque<(StatusCode, Value)>>>,
	media: Mutex<HashMap<String, MediaFile>>,
}

impl MockState {
	fn take_queued_response(&self, key: &str) -> Option<(StatusCode, Value)> {
		self.queued_responses
			.lock()
			.unwrap()
			.get_mut(key)
			.and_then(VecDeque::pop_front)
	}
}

pub struct Mock {
	pub base: String,
	state: Arc<MockState>,
}

impl Mock {
	pub async fn start() -> Self {
		let state = Arc::new(MockState {
			telegram_calls: Log::new(Vec::new()),
			downloads: Log::new(Vec::new()),
			joins: Log::new(Vec::new()),
			queued_responses: Mutex::default(),
			media: Mutex::default(),
		});
		let app = Router::new()
			.route("/bot{token}/{method}", post(telegram_method))
			.route(
				"/_matrix/client/v1/media/download/{server}/{media_id}",
				get(download_media),
			)
			.route("/_matrix/client/v3/join/{room_id}", post(join_room))
			.with_state(state.clone());
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let base = format!("http://{}", listener.local_addr().unwrap());
		tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
		Self { base, state }
	}

	pub fn respond_once(&self, method: &str, status: u16, body: Value) {
		let status = StatusCode::from_u16(status).unwrap();
		self.state
			.queued_responses
			.lock()
			.unwrap()
			.entry(method.to_string())
			.or_default()
			.push_back((status, body));
	}

	pub fn serve_media(&self, media_id: &str, content_type: &'static str, bytes: &'static [u8]) {
		let file = MediaFile {
			content_type,
			bytes: Bytes::from_static(bytes),
		};
		self.state
			.media
			.lock()
			.unwrap()
			.insert(media_id.to_string(), file);
	}

	pub async fn wait_for_calls(&self, count: usize) -> Vec<TelegramCall> {
		self.wait_for_calls_within(count, WAIT_TIMEOUT).await
	}

	pub async fn wait_for_calls_within(
		&self,
		count: usize,
		timeout: Duration,
	) -> Vec<TelegramCall> {
		wait_for_len(&self.state.telegram_calls, count, timeout).await
	}

	pub async fn wait_for_joins(&self, count: usize) -> Vec<Join> {
		wait_for_len(&self.state.joins, count, WAIT_TIMEOUT).await
	}

	pub fn calls(&self) -> Vec<TelegramCall> {
		self.state.telegram_calls.borrow().clone()
	}

	pub fn downloads(&self) -> Vec<Download> {
		self.state.downloads.borrow().clone()
	}

	pub fn joins(&self) -> Vec<Join> {
		self.state.joins.borrow().clone()
	}
}

async fn wait_for_len<T: Clone + Debug>(log: &Log<T>, count: usize, timeout: Duration) -> Vec<T> {
	let mut receiver = log.subscribe();
	let reached = receiver.wait_for(|items| items.len() >= count);
	match tokio::time::timeout(timeout, reached).await {
		Ok(Ok(items)) => items.clone(),
		_ => panic!(
			"expected {count} requests within {timeout:?}, got {:#?}",
			log.borrow()
		),
	}
}

fn header_string(
	headers: &HeaderMap,
	name: impl axum::http::header::AsHeaderName,
) -> Option<String> {
	headers
		.get(name)
		.and_then(|value| value.to_str().ok())
		.map(str::to_string)
}

async fn telegram_method(
	State(state): State<Arc<MockState>>,
	Path((_token, method)): Path<(String, String)>,
	request: Request<Body>,
) -> Response {
	let (params, files) = read_params(request).await;
	let call = TelegramCall {
		method: method.clone(),
		params,
		files,
		received_at: Instant::now(),
	};
	state.telegram_calls.send_modify(|calls| calls.push(call));
	let (status, body) = state
		.take_queued_response(&method)
		.unwrap_or((StatusCode::OK, json!({ "ok": true, "result": {} })));
	(status, Json(body)).into_response()
}

async fn read_params(request: Request<Body>) -> (Value, Vec<FilePart>) {
	let is_multipart = header_string(request.headers(), CONTENT_TYPE)
		.is_some_and(|content_type| content_type.starts_with("multipart/form-data"));
	if !is_multipart {
		let Json(params) = Json::<Value>::from_request(request, &())
			.await
			.expect("JSON body");
		return (params, Vec::new());
	}
	let mut multipart = Multipart::from_request(request, &())
		.await
		.expect("multipart body");
	let mut params = serde_json::Map::new();
	let mut files = Vec::new();
	while let Some(field) = multipart.next_field().await.expect("multipart field") {
		let name = field.name().unwrap_or_default().to_string();
		match field.file_name().map(str::to_string) {
			Some(filename) => files.push(FilePart {
				field: name,
				filename,
				content_type: field.content_type().map(str::to_string),
				bytes: field.bytes().await.expect("file part"),
			}),
			None => {
				params.insert(name, Value::String(field.text().await.expect("text part")));
			}
		}
	}
	(Value::Object(params), files)
}

async fn download_media(
	State(state): State<Arc<MockState>>,
	Path((_server, media_id)): Path<(String, String)>,
	headers: HeaderMap,
) -> Response {
	let download = Download {
		media_id: media_id.clone(),
		authorization: header_string(&headers, AUTHORIZATION),
		received_at: Instant::now(),
	};
	state
		.downloads
		.send_modify(|downloads| downloads.push(download));
	if let Some((status, body)) = state.take_queued_response("download") {
		return (status, Json(body)).into_response();
	}
	let file = state.media.lock().unwrap().get(&media_id).cloned();
	match file {
		Some(file) => ([(CONTENT_TYPE, file.content_type)], file.bytes).into_response(),
		None => (
			StatusCode::NOT_FOUND,
			Json(json!({ "errcode": "M_NOT_FOUND", "error": "Media not found" })),
		)
			.into_response(),
	}
}

async fn join_room(
	State(state): State<Arc<MockState>>,
	Path(room_id): Path<String>,
	Query(query): Query<HashMap<String, String>>,
	headers: HeaderMap,
) -> Json<Value> {
	let join = Join {
		room_id: room_id.clone(),
		user_id: query.get("user_id").cloned(),
		authorization: header_string(&headers, AUTHORIZATION),
	};
	state.joins.send_modify(|joins| joins.push(join));
	Json(json!({ "room_id": room_id }))
}
