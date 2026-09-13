use std::time::Duration;

use super::*;

#[tokio::test]
async fn health_ping_and_unknown_routes() {
	let TestBridge { app, .. } = TestBridge::start().await;

	let health = Request::get("/health").body(Body::empty()).unwrap();
	assert_eq!(
		send(&app, health).await,
		(StatusCode::OK, json!({ "ok": true }))
	);

	let ping = json_request(Method::POST, "/_matrix/app/v1/ping", json!({}));
	assert_eq!(
		send(&app, with_bearer(ping, HS_TOKEN)).await,
		(StatusCode::OK, json!({}))
	);

	let unauthorized_ping = json_request(Method::POST, "/_matrix/app/v1/ping", json!({}));
	assert_eq!(send(&app, unauthorized_ping).await.0, StatusCode::FORBIDDEN);

	let unknown = Request::get("/_matrix/app/v1/users/@x:test")
		.body(Body::empty())
		.unwrap();
	assert_eq!(
		send(&app, unknown).await,
		(
			StatusCode::NOT_FOUND,
			json!({ "errcode": "M_UNRECOGNIZED" })
		)
	);

	let legacy = json_request(
		Method::PUT,
		&format!("/transactions/t1?access_token={HS_TOKEN}"),
		json!({ "events": [] }),
	);
	assert_eq!(send(&app, legacy).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn hs_token_is_accepted_via_header_or_query_and_rejected_otherwise() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	let body = |event_id: &str, text_body: &str| json!({ "events": [text(event_id, text_body)] });
	let uri = transaction_uri("t1");

	let rejected = [
		json_request(Method::PUT, &uri, body("$none", "no token")),
		with_bearer(
			json_request(Method::PUT, &uri, body("$bad-header", "bad header")),
			"nope",
		),
		json_request(
			Method::PUT,
			&format!("{uri}?access_token=nope"),
			body("$bad-query", "bad query"),
		),
		with_bearer(
			json_request(
				Method::PUT,
				&format!("{uri}?access_token=nope"),
				body("$mixed", "mismatched pair"),
			),
			HS_TOKEN,
		),
	];
	for request in rejected {
		let (status, body) = send(&app, request).await;
		assert_eq!(status, StatusCode::FORBIDDEN);
		assert_eq!(body["errcode"], "M_FORBIDDEN");
	}

	let via_header = with_bearer(
		json_request(
			Method::PUT,
			&transaction_uri("t2"),
			body("$header", "via header"),
		),
		HS_TOKEN,
	);
	assert_eq!(send(&app, via_header).await, (StatusCode::OK, json!({})));

	let via_query = json_request(
		Method::PUT,
		&format!("{}?access_token={HS_TOKEN}", transaction_uri("t3")),
		body("$query", "via query"),
	);
	assert_eq!(send(&app, via_query).await, (StatusCode::OK, json!({})));

	let calls = mock.wait_for_calls(2).await;
	assert_eq!(texts(&calls), ["via header", "via query"]);
}

#[tokio::test]
async fn edits_own_messages_other_rooms_and_other_events_are_ignored() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	let mut own = text("$own", "echo from the bridge");
	own["sender"] = json!(BRIDGE_USER);
	let edit = message(
		"$edit",
		json!({
			"msgtype": "m.text",
			"body": "* fixed typo",
			"m.new_content": { "msgtype": "m.text", "body": "fixed typo" },
			"m.relates_to": { "rel_type": "m.replace", "event_id": "$1" },
		}),
	);
	let mut elsewhere = text("$elsewhere", "wrong room");
	elsewhere["room_id"] = json!("!other:test");
	let reaction = event(
		"m.reaction",
		"$reaction",
		json!({ "m.relates_to": { "rel_type": "m.annotation", "event_id": "$1", "key": "👍" } }),
	);
	let encrypted = event(
		"m.room.encrypted",
		"$encrypted",
		json!({ "algorithm": "m.megolm.v1.aes-sha2", "ciphertext": "AwgA" }),
	);
	let blank = text("$blank", " \n ");

	put_txn(
		&app,
		"t1",
		vec![
			own,
			edit,
			elsewhere,
			reaction,
			encrypted,
			blank,
			last_message(),
		],
	)
	.await;

	let calls = mock.wait_for_calls(1).await;
	assert_eq!(texts(&calls), [LAST_MESSAGE]);
}

#[tokio::test]
async fn malformed_event_does_not_drop_the_batch() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	let missing_fields = json!({ "type": "m.room.message" });
	let wrong_types = json!({
		"type": "m.room.message", "event_id": 7, "room_id": ROOM_ID, "sender": ALICE, "content": {}
	});

	put_txn(
		&app,
		"t1",
		vec![missing_fields, wrong_types, text("$ok", "survivor")],
	)
	.await;

	let calls = mock.wait_for_calls(1).await;
	assert_eq!(texts(&calls), ["survivor"]);
}

#[tokio::test]
async fn duplicate_event_id_across_transactions_is_sent_once() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;

	put_txn(&app, "t1", vec![text("$same", "only once")]).await;
	put_txn(&app, "t1", vec![text("$same", "only once")]).await;
	put_txn(&app, "t2", vec![text("$same", "only once"), last_message()]).await;

	let calls = mock.wait_for_calls(2).await;
	assert_eq!(texts(&calls), ["only once", LAST_MESSAGE]);
}

#[tokio::test]
async fn invite_for_bridge_user_triggers_join() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;

	put_txn(
		&app,
		"t1",
		vec![
			membership("$other", ALICE, "invite"),
			membership("$bridge", BRIDGE_USER, "invite"),
		],
	)
	.await;

	let joins = mock.wait_for_joins(1).await;
	assert_eq!(joins[0].room_id, ROOM_ID);
	assert_eq!(joins[0].user_id.as_deref(), Some(BRIDGE_USER));
	assert_eq!(joins[0].authorization, Some(format!("Bearer {AS_TOKEN}")));
	tokio::time::sleep(Duration::from_millis(200)).await;
	assert_eq!(mock.joins().len(), 1, "only the bridge user's invite joins");
	assert!(mock.calls().is_empty());
}

#[tokio::test]
async fn repeated_placeholder_invites_each_trigger_a_join() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	let invite = || membership("$placeholder", BRIDGE_USER, "invite");

	put_txn(&app, "placeholder", vec![invite()]).await;
	mock.wait_for_joins(1).await;
	put_txn(&app, "placeholder", vec![invite()]).await;

	let joins = mock.wait_for_joins(2).await;
	assert!(joins.iter().all(|join| join.room_id == ROOM_ID));
}
