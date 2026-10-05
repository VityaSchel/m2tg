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
const MEDIA_HOLD: Duration = Duration::from_secs(10);
const ALBUM_LIMIT: usize = 10;
const MAX_ATTEMPTS: u32 = 5;
const MEDIA_UNAVAILABLE: &str = "[media unavailable]";

pub struct Origin {
	pub sender: String,
	pub origin_server_ts: u64,
}

pub enum Job {
	Text {
		origin: Origin,
		html: String,
		plain: String,
		is_reply: bool,
	},
	Media {
		origin: Origin,
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
		pending: None,
	};
	(sender, tokio::spawn(worker.run(receiver)))
}

struct Outbox {
	config: Arc<Config>,
	http: Client,
	pending: Option<Pending>,
}

struct Pending {
	latest: Origin,
	media: Vec<OutMedia>,
	deadline: Instant,
	merged_caption: bool,
}

impl Pending {
	fn accepts_media(&self, origin: &Origin, media: &OutMedia) -> bool {
		self.latest.sender == origin.sender
			&& self.media.len() < ALBUM_LIMIT
			&& media.kind.groupable()
			&& self.media.iter().all(|item| item.kind.groupable())
	}

	fn accepts_caption(&self, origin: &Origin, html: &str) -> bool {
		let gap = origin
			.origin_server_ts
			.saturating_sub(self.latest.origin_server_ts);
		self.latest.sender == origin.sender
			&& Duration::from_millis(gap) <= MEDIA_HOLD
			&& self.media.iter().all(|item| item.caption_html.is_empty())
			&& telegram::fits_caption(html)
	}
}

impl Outbox {
	async fn run(mut self, mut receiver: mpsc::Receiver<Job>) {
		loop {
			let deadline = self.pending.as_ref().map(|pending| pending.deadline);
			tokio::select! {
				biased;
				() = wait_until(deadline) => self.flush_pending().await,
				job = receiver.recv() => match job {
					Some(job) => self.handle(job).await,
					None => break,
				},
			}
		}
		self.flush_pending().await;
	}

	async fn handle(&mut self, job: Job) {
		match job {
			Job::Text {
				origin,
				html,
				plain,
				is_reply,
			} => {
				let captioned = !is_reply && self.caption_pending(&origin, &html, &plain);
				self.flush_pending().await;
				if !captioned {
					self.send_text(&html, &plain).await;
				}
			}
			Job::Media {
				origin,
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
						self.flush_pending().await;
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
				let media = OutMedia {
					bytes: media.bytes,
					content_type,
					filename,
					caption_html,
					caption_plain,
					kind,
				};
				self.dispatch_media(origin, media).await;
			}
		}
	}

	fn caption_pending(&mut self, origin: &Origin, html: &str, plain: &str) -> bool {
		let Some(pending) = self
			.pending
			.as_mut()
			.filter(|pending| pending.accepts_caption(origin, html))
		else {
			return false;
		};
		let first = &mut pending.media[0];
		first.caption_html = html.to_string();
		first.caption_plain = plain.to_string();
		pending.merged_caption = true;
		true
	}

	async fn dispatch_media(&mut self, origin: Origin, media: OutMedia) {
		let joins_pending = self
			.pending
			.as_ref()
			.is_some_and(|pending| pending.accepts_media(&origin, &media));
		if !joins_pending {
			self.flush_pending().await;
		}
		if !media.kind.groupable() && !media.caption_html.is_empty() {
			self.send_single_media(&media).await;
			return;
		}
		let deadline = Instant::now() + MEDIA_HOLD;
		match &mut self.pending {
			Some(pending) => {
				pending.latest = origin;
				pending.media.push(media);
				pending.deadline = deadline;
			}
			None => {
				self.pending = Some(Pending {
					latest: origin,
					media: vec![media],
					deadline,
					merged_caption: false,
				});
			}
		}
	}

	async fn flush_pending(&mut self) {
		let Some(pending) = self.pending.take() else {
			return;
		};
		let first_delivered = match pending.media.as_slice() {
			[single] => self.send_single_media(single).await,
			items => self.send_album(items).await,
		};
		if pending.merged_caption && !first_delivered {
			let first = &pending.media[0];
			self.send_text(&first.caption_html, &first.caption_plain)
				.await;
		}
	}

	async fn send_album(&self, items: &[OutMedia]) -> bool {
		let result = retry("sendMediaGroup", || {
			telegram::send_media_group(&self.http, &self.config, items)
		})
		.await;
		match result {
			Err(RequestError::Permanent { message }) => {
				tracing::warn!("{message}; sending album items individually");
				let Some((first, rest)) = items.split_first() else {
					return false;
				};
				let first_delivered = self.send_single_media(first).await;
				for item in rest {
					self.send_single_media(item).await;
				}
				first_delivered
			}
			result => delivered("sendMediaGroup", result),
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
				delivered("sendMessage plain text", fallback);
			}
			result => {
				delivered("sendMessage", result);
			}
		}
	}

	async fn send_single_media(&self, media: &OutMedia) -> bool {
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
				delivered("sendDocument fallback", fallback)
			}
			result => delivered(method, result),
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

fn delivered(action: &str, result: Result<(), RequestError>) -> bool {
	result
		.inspect_err(|e| tracing::error!("{action} failed, dropping: {e}"))
		.is_ok()
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
