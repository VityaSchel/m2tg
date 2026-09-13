use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::config::Config;
use crate::format;
use crate::matrix::{self, Msgtype};
use crate::request::RequestError;
use crate::telegram::{self, MediaKind, OutMedia};

const CHANNEL_CAPACITY: usize = 256;
const ALBUM_DELAY: Duration = Duration::from_secs(5);
const ALBUM_LIMIT: usize = 10;
const MAX_ATTEMPTS: u32 = 5;
const MEDIA_UNAVAILABLE: &str = "[media unavailable]";

pub enum Job {
	Text {
		html: String,
		plain: String,
	},
	Media {
		mxc: String,
		msgtype: Msgtype,
		mimetype: Option<String>,
		filename: String,
		caption_html: String,
		caption_plain: String,
	},
}

pub type Sender = mpsc::Sender<Job>;

pub fn spawn(config: Arc<Config>, http: Client) -> (Sender, JoinHandle<()>) {
	let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);
	let worker = Outbox {
		config,
		http,
		album: Vec::new(),
		album_deadline: None,
	};
	(sender, tokio::spawn(worker.run(receiver)))
}

struct Outbox {
	config: Arc<Config>,
	http: Client,
	album: Vec<OutMedia>,
	album_deadline: Option<Instant>,
}

impl Outbox {
	async fn run(mut self, mut receiver: mpsc::Receiver<Job>) {
		loop {
			tokio::select! {
				biased;
				() = wait_until(self.album_deadline) => self.flush_album().await,
				job = receiver.recv() => match job {
					Some(job) => self.handle(job).await,
					None => break,
				},
			}
		}
		self.flush_album().await;
	}

	async fn handle(&mut self, job: Job) {
		match job {
			Job::Text { html, plain } => {
				self.flush_album().await;
				self.send_text(&html, &plain).await;
			}
			Job::Media {
				mxc,
				msgtype,
				mimetype,
				filename,
				caption_html,
				caption_plain,
			} => {
				let download = retry("media download", || {
					matrix::download_media(&self.http, &self.config, &mxc)
				})
				.await;
				let media = match download {
					Ok(media) => media,
					Err(e) => {
						tracing::error!("media {mxc} unavailable: {e}");
						self.flush_album().await;
						let (html, plain) = if caption_plain.is_empty() {
							(format::escape_html(&filename), filename)
						} else {
							(caption_html, caption_plain)
						};
						let html = format!("{MEDIA_UNAVAILABLE} {html}");
						let plain = format!("{MEDIA_UNAVAILABLE} {plain}");
						self.send_text(html.trim_end(), plain.trim_end()).await;
						return;
					}
				};
				let content_type = mimetype
					.filter(|mime| !mime.is_empty())
					.unwrap_or(media.content_type);
				let filename = if filename.is_empty() {
					format!("file{}", extension_for(&content_type))
				} else {
					filename
				};
				let kind = MediaKind::classify(msgtype, &content_type, media.bytes.len());
				self.dispatch_media(OutMedia {
					bytes: media.bytes,
					content_type,
					filename,
					caption_html,
					caption_plain,
					kind,
				})
				.await;
			}
		}
	}

	async fn dispatch_media(&mut self, media: OutMedia) {
		if media.kind.groupable() {
			self.album.push(media);
			self.album_deadline = Some(Instant::now() + ALBUM_DELAY);
			if self.album.len() >= ALBUM_LIMIT {
				self.flush_album().await;
			}
		} else {
			self.flush_album().await;
			self.send_single_media(&media).await;
		}
	}

	async fn flush_album(&mut self) {
		self.album_deadline = None;
		let album = std::mem::take(&mut self.album);
		match album.as_slice() {
			[] => {}
			[single] => self.send_single_media(single).await,
			items => {
				let result = retry("sendMediaGroup", || {
					telegram::send_media_group(&self.http, &self.config, items)
				})
				.await;
				match result {
					Err(RequestError::Permanent { message }) => {
						tracing::warn!("{message}; sending album items individually");
						for item in items {
							self.send_single_media(item).await;
						}
					}
					result => log_dropped("sendMediaGroup", result),
				}
			}
		}
	}

	async fn send_text(&self, html: &str, plain: &str) {
		let result = retry("sendMessage", || {
			telegram::send_message(&self.http, &self.config, html)
		})
		.await;
		match result {
			Err(RequestError::Permanent { message }) => {
				tracing::warn!("{message}; resending as plain text");
				let fallback = retry("sendMessage plain text", || {
					telegram::send_plain_message(&self.http, &self.config, plain)
				})
				.await;
				log_dropped("sendMessage plain text", fallback);
			}
			result => log_dropped("sendMessage", result),
		}
	}

	async fn send_single_media(&self, media: &OutMedia) {
		let method = media.kind.method();
		let result = retry(method, || {
			telegram::send_media(&self.http, &self.config, media)
		})
		.await;
		match result {
			Err(RequestError::Permanent { message }) if media.kind != MediaKind::Document => {
				tracing::warn!("{message}; resending as document");
				let fallback = retry("sendDocument fallback", || {
					telegram::send_media_as_document(&self.http, &self.config, media)
				})
				.await;
				log_dropped("sendDocument fallback", fallback);
			}
			result => log_dropped(method, result),
		}
	}
}

async fn wait_until(deadline: Option<Instant>) {
	match deadline {
		Some(deadline) => tokio::time::sleep_until(deadline).await,
		None => std::future::pending().await,
	}
}

async fn retry<T, F, Fut>(action: &str, mut attempt: F) -> Result<T, RequestError>
where
	F: FnMut() -> Fut,
	Fut: Future<Output = Result<T, RequestError>>,
{
	let mut attempts = 1;
	loop {
		match attempt().await {
			Err(RequestError::Transient {
				retry_after,
				message,
			}) if attempts < MAX_ATTEMPTS => {
				let delay = retry_after.unwrap_or(Duration::from_secs(1 << attempts));
				tracing::warn!(
					"{action} attempt {attempts} failed, retrying in {delay:?}: {message}"
				);
				tokio::time::sleep(delay).await;
				attempts += 1;
			}
			result => return result,
		}
	}
}

fn log_dropped(action: &str, result: Result<(), RequestError>) {
	if let Err(e) = result {
		tracing::error!("{action} failed, dropping: {e}");
	}
}

fn extension_for(content_type: &str) -> String {
	let essence = telegram::mime_essence(content_type);
	match essence.split_once('/') {
		Some(("image" | "video" | "audio", subtype)) if !subtype.is_empty() => {
			format!(".{subtype}")
		}
		_ => String::new(),
	}
}

#[cfg(test)]
mod tests;
