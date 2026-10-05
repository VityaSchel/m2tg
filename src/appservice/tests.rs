use axum::body::Body;
use axum::http::Request;
use serde_json::json;

use super::{RECENT_IDS_CAPACITY, RecentIds, message_job};
use crate::matrix::{Content, Msgtype};
use crate::outbox::{Job, Origin};

fn content(value: serde_json::Value) -> Content {
	serde_json::from_value(value).unwrap()
}

fn origin() -> Origin {
	Origin {
		sender: "@alice:test".into(),
		origin_server_ts: 0,
	}
}

fn text_of(job: Option<Job>) -> Option<(String, String)> {
	match job {
		Some(Job::Text { html, plain, .. }) => Some((html, plain)),
		Some(Job::Media { .. }) => panic!("expected a text job"),
		None => None,
	}
}

#[test]
fn authorized_accepts_query_and_bearer_rejects_others() {
	let ok_query = Request::builder()
		.uri("/x?access_token=secret")
		.body(Body::empty())
		.unwrap();
	assert!(super::authorized("secret", &ok_query));

	let ok_header = Request::builder()
		.uri("/x")
		.header("authorization", "Bearer secret")
		.body(Body::empty())
		.unwrap();
	assert!(super::authorized("secret", &ok_header));

	let bad_token = Request::builder()
		.uri("/x?access_token=nope")
		.body(Body::empty())
		.unwrap();
	assert!(!super::authorized("secret", &bad_token));

	let no_auth = Request::builder().uri("/x").body(Body::empty()).unwrap();
	assert!(!super::authorized("secret", &no_auth));

	let mismatched_pair = Request::builder()
		.uri("/x?access_token=nope")
		.header("authorization", "Bearer secret")
		.body(Body::empty())
		.unwrap();
	assert!(!super::authorized("secret", &mismatched_pair));

	let matching_pair = Request::builder()
		.uri("/x?access_token=secret")
		.header("authorization", "Bearer secret")
		.body(Body::empty())
		.unwrap();
	assert!(super::authorized("secret", &matching_pair));
}

#[test]
fn recent_ids_evict_oldest_beyond_capacity() {
	let mut ids = RecentIds::default();
	for i in 0..=RECENT_IDS_CAPACITY {
		ids.insert(format!("${i}"));
	}
	ids.insert("$1".into());
	assert!(!ids.contains("$0"));
	assert!(ids.contains("$1"));
	assert!(ids.contains(&format!("${RECENT_IDS_CAPACITY}")));
	assert_eq!(ids.order.len(), RECENT_IDS_CAPACITY);
	assert_eq!(ids.set.len(), RECENT_IDS_CAPACITY);
}

#[test]
fn reply_fallback_is_stripped_only_for_replies() {
	let reply = content(json!({
		"msgtype": "m.text",
		"body": "> <@alice:test> original\n\nthe reply",
		"m.relates_to": { "m.in_reply_to": { "event_id": "$orig" } }
	}));
	assert_eq!(
		text_of(message_job(&reply, origin())),
		Some(("the reply".into(), "the reply".into()))
	);

	let quote = content(json!({ "msgtype": "m.text", "body": "> <@alice:test> a famous quote" }));
	assert_eq!(
		text_of(message_job(&quote, origin())),
		Some((
			"&gt; &lt;@alice:test&gt; a famous quote".into(),
			"> <@alice:test> a famous quote".into()
		))
	);

	let reply_without_fallback = content(json!({
		"msgtype": "m.text",
		"body": "> a famous quote\n\nagreed",
		"m.relates_to": { "m.in_reply_to": { "event_id": "$orig" } }
	}));
	assert_eq!(
		text_of(message_job(&reply_without_fallback, origin())),
		Some((
			"&gt; a famous quote\n\nagreed".into(),
			"> a famous quote\n\nagreed".into()
		))
	);
}

#[test]
fn text_jobs_mark_replies() {
	let reply = content(json!({
		"msgtype": "m.text",
		"body": "answer",
		"m.relates_to": { "m.in_reply_to": { "event_id": "$orig" } }
	}));
	assert!(matches!(
		message_job(&reply, origin()),
		Some(Job::Text { is_reply: true, .. })
	));

	let caption = content(json!({ "msgtype": "m.text", "body": "caption" }));
	assert!(matches!(
		message_job(&caption, origin()),
		Some(Job::Text {
			is_reply: false,
			..
		})
	));
}

#[test]
fn invisible_text_is_ignored() {
	let blank = content(json!({ "msgtype": "m.text", "body": " \n " }));
	assert!(message_job(&blank, origin()).is_none());

	let quote_only_reply = content(json!({
		"msgtype": "m.text",
		"body": "> <@alice:test> quoted",
		"m.relates_to": { "m.in_reply_to": { "event_id": "$orig" } }
	}));
	assert!(message_job(&quote_only_reply, origin()).is_none());
}

#[test]
fn oversized_html_falls_back_to_escaped_plain() {
	let huge = content(json!({
		"msgtype": "m.text",
		"body": "a < b",
		"format": "org.matrix.custom.html",
		"formatted_body": "<b>x</b>".repeat(5000)
	}));
	assert_eq!(
		text_of(message_job(&huge, origin())),
		Some(("a &lt; b".into(), "a < b".into()))
	);
}

#[test]
fn media_job_sanitizes_filename_and_omits_duplicate_caption() {
	let image = content(json!({
		"msgtype": "m.image",
		"body": "cat\u{0}.png",
		"url": "mxc://test/abc",
		"info": { "mimetype": "image/png" }
	}));
	let Some(Job::Media {
		mxc,
		msgtype,
		mimetype,
		filename,
		caption_html,
		caption_plain,
		..
	}) = message_job(&image, origin())
	else {
		panic!("expected a media job");
	};
	assert_eq!(mxc, "mxc://test/abc");
	assert_eq!(msgtype, Msgtype::Image);
	assert_eq!(mimetype.as_deref(), Some("image/png"));
	assert_eq!(filename, "cat.png");
	assert!(caption_html.is_empty() && caption_plain.is_empty());
}

#[test]
fn media_job_keeps_distinct_caption() {
	let video = content(json!({
		"msgtype": "m.video",
		"body": "look at this",
		"filename": "clip.mp4",
		"url": "mxc://test/clip"
	}));
	let Some(Job::Media {
		filename,
		caption_html,
		caption_plain,
		..
	}) = message_job(&video, origin())
	else {
		panic!("expected a media job");
	};
	assert_eq!(filename, "clip.mp4");
	assert_eq!(caption_html, "look at this");
	assert_eq!(caption_plain, "look at this");
}

#[test]
fn media_without_url_is_forwarded_as_text() {
	let image = content(json!({ "msgtype": "m.image", "body": "broken" }));
	assert_eq!(
		text_of(message_job(&image, origin())),
		Some(("broken".into(), "broken".into()))
	);
}
