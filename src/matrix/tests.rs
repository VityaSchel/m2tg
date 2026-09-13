use super::{Content, Event, Msgtype};

#[test]
fn strip_reply_fallback_passes_through_non_fallbacks() {
	assert_eq!(super::strip_reply_fallback("hello world"), "hello world");
	assert_eq!(
		super::strip_reply_fallback("> a quote\n\nmy take"),
		"> a quote\n\nmy take"
	);
}

#[test]
fn strip_reply_fallback_strips_quote_and_blank_line() {
	assert_eq!(
		super::strip_reply_fallback("> <@a:b> quoted\n\nactual reply"),
		"actual reply"
	);
	assert_eq!(
		super::strip_reply_fallback("> * <@a:b> waves\n\nwave back"),
		"wave back"
	);
}

#[test]
fn strip_reply_fallback_strips_quote_without_blank_line() {
	assert_eq!(super::strip_reply_fallback("> <@a:b> a\n> b\nreal"), "real");
}

#[test]
fn strip_reply_fallback_all_quote_becomes_empty() {
	assert_eq!(super::strip_reply_fallback("> <@a:b> a\n> b"), "");
}

#[test]
fn msgtype_known_and_unknown() {
	assert_eq!(
		serde_json::from_str::<Msgtype>("\"m.image\"").unwrap(),
		Msgtype::Image
	);
	assert_eq!(
		serde_json::from_str::<Msgtype>("\"m.sticker\"").unwrap(),
		Msgtype::Other
	);
}

#[test]
fn event_with_missing_content_defaults() {
	let json = r#"{"type":"m.room.message","event_id":"$1","room_id":"!r","sender":"@a:b"}"#;
	let event: Event = serde_json::from_str(json).unwrap();
	assert!(event.content.msgtype.is_none());
	assert!(event.state_key.is_none());
}

#[test]
fn content_parses_relations() {
	let edit: Content = serde_json::from_str(
		r#"{"msgtype":"m.text","body":"hi","m.relates_to":{"rel_type":"m.replace"}}"#,
	)
	.unwrap();
	assert_eq!(edit.msgtype, Some(Msgtype::Text));
	assert!(edit.is_edit());
	assert!(!edit.is_reply());

	let reply: Content = serde_json::from_str(
		r#"{"body":"> q\n\nhi","m.relates_to":{"m.in_reply_to":{"event_id":"$x"}}}"#,
	)
	.unwrap();
	assert!(reply.is_reply());
	assert!(!reply.is_edit());
}

#[test]
fn state_events_parse_state_key_and_replacement_room() {
	let json = r#"{"type":"m.room.tombstone","event_id":"$t","room_id":"!r","sender":"@a:b","state_key":"","content":{"replacement_room":"!new:b"}}"#;
	let event: Event = serde_json::from_str(json).unwrap();
	assert_eq!(event.state_key.as_deref(), Some(""));
	assert_eq!(event.content.replacement_room.as_deref(), Some("!new:b"));
}
