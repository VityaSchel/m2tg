use super::{
	CAPTION_LIMIT, MediaKind, fits_caption, has_visible_text, send_message, truncate,
	truncate_plain, visible_len,
};
use crate::config::Config;
use crate::matrix::Msgtype;
use crate::request::RequestError;

const MB: usize = 1024 * 1024;

#[test]
fn visible_len_ignores_markup_and_counts_entities() {
	assert_eq!(visible_len("<b>hi</b>"), 2);
	assert_eq!(visible_len("a&amp;b"), 3);
	assert_eq!(visible_len("<a href=\"x\">go</a>"), 2);
	assert_eq!(visible_len("héllo 👋"), 7);
	assert_eq!(visible_len("a & b"), 5);
}

#[test]
fn has_visible_text_ignores_tags_and_whitespace() {
	assert!(!has_visible_text(""));
	assert!(!has_visible_text("<b> \n</b>\u{a0}"));
	assert!(has_visible_text("<b> x </b>"));
	assert!(has_visible_text("&lt;"));
}

#[test]
fn fits_caption_counts_only_visible_characters() {
	let full = "a".repeat(CAPTION_LIMIT);
	assert!(fits_caption(&format!(
		"<a href=\"https://example.org/{full}\">{full}</a>"
	)));
	assert!(fits_caption(&"&amp;".repeat(CAPTION_LIMIT)));
	assert!(!fits_caption(&format!("{full}b")));
}

#[test]
fn short_html_is_unchanged() {
	let html = "<b>hello</b>";
	assert_eq!(truncate(html, 4096), html);
}

#[test]
fn truncation_closes_open_tags_and_never_splits_a_tag() {
	let html = "<b>aaaaaaaaaa</b>";
	let out = truncate(html, 5);
	assert!(out.starts_with("<b>aaaa…"), "got {out}");
	assert!(out.ends_with("</b>"), "unclosed tag: {out}");
	assert_eq!(visible_len(&out), 5);
}

#[test]
fn truncation_does_not_cut_inside_an_entity() {
	let html = "aa&amp;aa&amp;aa";
	let out = truncate(html, 4);
	assert!(!out.contains("&am\u{2026}"), "split an entity: {out}");
	assert_eq!(visible_len(&out), 4);
}

#[test]
fn truncation_closes_nested_tags_in_reverse_order() {
	let out = truncate("<pre><code class=\"language-rs\">abcdef</code></pre>", 3);
	assert_eq!(out, "<pre><code class=\"language-rs\">ab…</code></pre>");
}

#[test]
fn plain_truncation_counts_scalar_values() {
	assert_eq!(truncate_plain("héllo", 5), "héllo");
	assert_eq!(truncate_plain("👋👋👋👋", 3), "👋👋…");
}

#[test]
fn classify_by_content_type() {
	use MediaKind::*;
	assert_eq!(MediaKind::classify(Msgtype::Image, "image/png", MB), Photo);
	assert_eq!(MediaKind::classify(Msgtype::Image, "IMAGE/PNG", MB), Photo);
	assert_eq!(
		MediaKind::classify(Msgtype::Image, "image/jpeg", 11 * MB),
		Document
	);
	assert_eq!(
		MediaKind::classify(Msgtype::Image, "image/gif", MB),
		Animation
	);
	assert_eq!(
		MediaKind::classify(Msgtype::Image, "image/svg+xml", MB),
		Document
	);
	assert_eq!(MediaKind::classify(Msgtype::Video, "video/mp4", MB), Video);
	assert_eq!(
		MediaKind::classify(Msgtype::Audio, "audio/ogg; codecs=opus", MB),
		Voice
	);
	assert_eq!(MediaKind::classify(Msgtype::Audio, "audio/mpeg", MB), Audio);
	assert_eq!(
		MediaKind::classify(Msgtype::File, "image/png", MB),
		Document
	);
}

#[test]
fn classify_falls_back_to_msgtype() {
	use MediaKind::*;
	let octet = "application/octet-stream";
	assert_eq!(MediaKind::classify(Msgtype::Image, octet, MB), Photo);
	assert_eq!(
		MediaKind::classify(Msgtype::Image, octet, 11 * MB),
		Document
	);
	assert_eq!(MediaKind::classify(Msgtype::Video, octet, MB), Video);
	assert_eq!(MediaKind::classify(Msgtype::Audio, octet, MB), Audio);
	assert_eq!(MediaKind::classify(Msgtype::File, octet, MB), Document);
}

#[test]
fn only_photos_and_videos_group() {
	assert!(MediaKind::Photo.groupable());
	assert!(MediaKind::Video.groupable());
	assert!(!MediaKind::Animation.groupable());
	assert!(!MediaKind::Document.groupable());
}

#[tokio::test]
async fn network_errors_are_transient_and_never_reveal_the_bot_token() {
	let _ = rustls::crypto::ring::default_provider().install_default();
	let config = Config {
		homeserver_url: "http://127.0.0.1:1".into(),
		room_id: "!room:test".into(),
		host: "127.0.0.1".into(),
		port: 0,
		homeserver_token: "hs".into(),
		app_service_token: "as".into(),
		app_service_user: "@m2tg:test".into(),
		telegram_chat_id: "42".into(),
		telegram_bot_token: "123:SECRET".into(),
		telegram_api_base: "http://127.0.0.1:1".into(),
	};
	let error = send_message(&reqwest::Client::new(), &config, "hi")
		.await
		.unwrap_err();
	assert!(matches!(error, RequestError::Transient { .. }), "{error}");
	assert!(!error.to_string().contains("SECRET"), "{error}");
}
