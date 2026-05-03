import { Elysia, t } from "elysia";

function getEnv(name: string): string {
	const value = Bun.env[name];
	if (!value) {
		console.error(`${name} environment variable is required`);
		process.exit(1);
	}
	return value;
}

const HOMESERVER_URL = getEnv("HOMESERVER_URL");
const ROOM_ID = getEnv("ROOM_ID");

const PORT = Number(getEnv("PORT"));

const HOMESERVER_TOKEN = getEnv("HOMESERVER_TOKEN");
const APP_SERVICE_TOKEN = getEnv("APP_SERVICE_TOKEN");

const APP_SERVICE_USER = getEnv("APP_SERVICE_USER");

const TELEGRAM_CHAT_ID = getEnv("TELEGRAM_CHAT_ID");
const TELEGRAM_BOT_TOKEN = getEnv("TELEGRAM_BOT_TOKEN");

interface MatrixEvent {
	type: string;
	event_id: string;
	room_id: string;
	sender: string;
	origin_server_ts: number;
	content: MatrixEventContent;
	state_key?: string;
}

interface MatrixEventContent {
	msgtype?: string;
	body?: string;
	formatted_body?: string;
	format?: string;
	url?: string; // mxc:// URI (unencrypted media only)
	filename?: string;
	info?: {
		mimetype?: string;
		size?: number;
		w?: number;
		h?: number;
		duration?: number;
	};
	membership?: string;
	displayname?: string;
}

async function downloadMedia(
	mxc: string,
): Promise<{ blob: Blob; contentType: string } | null> {
	if (!mxc.startsWith("mxc://")) return null;
	const mediaUrl = mxc.substring("mxc://".length);
	const slash = mediaUrl.indexOf("/");
	if (slash === -1) return null;
	const serverName = mediaUrl.slice(0, slash);
	const mediaId = mediaUrl.slice(slash + 1);

	try {
		const res = await fetch(
			`${HOMESERVER_URL}/_matrix/client/v1/media/download/${encodeURIComponent(serverName)}/${encodeURIComponent(mediaId)}`,
			{ headers: { Authorization: `Bearer ${APP_SERVICE_TOKEN}` } },
		);
		if (!res.ok) {
			console.error(`Media download failed (${res.status}) for ${mxc}`);
			return null;
		}
		const blob = await res.blob();
		const contentType =
			res.headers.get("content-type") || "application/octet-stream";
		return { blob, contentType };
	} catch (err) {
		console.error(`Error downloading media ${mxc}:`, err);
	}
	return null;
}

const CAPTION_LIMIT = 1024;
const MESSAGE_LIMIT = 4096;

function truncate(text: string, limit: number): string {
	return text.length <= limit ? text : text.slice(0, limit - 1) + "…";
}

function escapeHtml(text: string): string {
	return text
		.replace(/&/g, "&amp;")
		.replace(/</g, "&lt;")
		.replace(/>/g, "&gt;");
}

async function tgCall(method: string, body: FormData | Record<string, string>) {
	const request = await fetch(
		`https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/${method}`,
		{
			method: "POST",
			headers: {
				...(!(body instanceof FormData) && {
					"Content-Type": "application/json",
				}),
			},
			body: body instanceof FormData ? body : JSON.stringify(body),
		},
	);
	if (!request.ok) throw new Error("Telegram API error");
}

async function sendText(html: string): Promise<void> {
	await tgCall("sendMessage", {
		chat_id: TELEGRAM_CHAT_ID!,
		text: truncate(html, MESSAGE_LIMIT),
		parse_mode: "HTML",
		disable_web_page_preview: "true",
	});
}

async function sendMediaGroup(items: BufferedMedia[]): Promise<void> {
	const form = new FormData();
	form.append("chat_id", TELEGRAM_CHAT_ID);

	const mediaArray = items.map((item, i) => {
		const key = `file${i}`;
		form.append(
			key,
			new Blob([item.blob], { type: item.contentType }),
			item.filename,
		);
		const isVideo =
			item.matrixMsgtype === "m.video" || item.contentType.startsWith("video/");
		const entry: Record<string, string> = {
			type: isVideo ? "video" : "photo",
			media: `attach://${key}`,
		};
		if (item.caption) {
			entry.caption = truncate(item.caption, CAPTION_LIMIT);
			entry.parse_mode = "HTML";
		}
		return entry;
	});

	form.append("media", JSON.stringify(mediaArray));
	await tgCall("sendMediaGroup", form);
}

async function sendMedia({
	blob,
	caption,
	contentType,
	filename,
	matrixMsgtype,
}: {
	blob: Blob;
	contentType: string;
	filename: string;
	caption: string;
	matrixMsgtype?: string;
}): Promise<void> {
	const form = new FormData();
	form.append("chat_id", TELEGRAM_CHAT_ID!);
	form.append("caption", truncate(caption, CAPTION_LIMIT));
	form.append("parse_mode", "HTML");

	let field: string;
	let method: string;
	if (matrixMsgtype === "m.image" || contentType.startsWith("image/")) {
		if (contentType === "image/gif") {
			method = "sendAnimation";
			field = "animation";
		} else {
			method = "sendPhoto";
			field = "photo";
		}
	} else if (matrixMsgtype === "m.video" || contentType.startsWith("video/")) {
		method = "sendVideo";
		field = "video";
	} else if (matrixMsgtype === "m.audio" || contentType.startsWith("audio/")) {
		method = "sendAudio";
		field = "audio";
	} else {
		method = "sendDocument";
		field = "document";
	}

	const file = new Blob([blob], { type: contentType });
	form.append(field, file, filename);

	await tgCall(method, form);
}

const processedEvents = new Set<string>();

interface BufferedMedia {
	blob: Blob;
	contentType: string;
	filename: string;
	caption: string;
	matrixMsgtype?: string;
}

const imageBuffer: BufferedMedia[] = [];
let flushTimer: ReturnType<typeof setTimeout> | null = null;
const MEDIA_GROUP_DELAY_MS = 5000;
const MEDIA_GROUP_MAX = 10;

async function doFlushImages(): Promise<void> {
	if (imageBuffer.length === 0) return;
	const batch = imageBuffer.splice(0);
	if (batch.length === 1) {
		await sendMedia(batch[0]!);
	} else {
		await sendMediaGroup(batch);
	}
}

async function flushImages(): Promise<void> {
	if (flushTimer !== null) {
		clearTimeout(flushTimer);
		flushTimer = null;
	}
	await doFlushImages();
}

function queueImage(item: BufferedMedia): void {
	imageBuffer.push(item);
	if (imageBuffer.length >= MEDIA_GROUP_MAX) {
		flushImages().catch(console.error);
		return;
	}
	if (flushTimer !== null) clearTimeout(flushTimer);
	flushTimer = setTimeout(() => {
		flushTimer = null;
		doFlushImages().catch(console.error);
	}, MEDIA_GROUP_DELAY_MS);
}

function extractBody(content: MatrixEventContent): string {
	let body = content.body ?? "";
	if (body.startsWith("> ")) {
		const lines = body.split("\n");
		let i = 0;
		while (i < lines.length && lines[i]!.startsWith("> ")) i++;
		if (i < lines.length && lines[i] === "") i++;
		body = lines.slice(i).join("\n");
	}
	return body;
}

function guessExt(mime: string): string {
	if (
		mime.startsWith("image/") ||
		mime.startsWith("video/") ||
		mime.startsWith("audio/")
	) {
		return "." + mime.slice(6).split(";")[0];
	}
	return "";
}

async function handleMessage({ content }: MatrixEvent): Promise<void> {
	const msgtype = content.msgtype;
	const text = extractBody(content);

	if (
		typeof content.url === "string" &&
		msgtype &&
		["m.image", "m.video", "m.audio", "m.file"].includes(msgtype)
	) {
		const media = await downloadMedia(content.url);
		if (!media) {
			await flushImages();
			await sendText(`[media unavailable] ${escapeHtml(text)}`);
			return;
		}
		const filename =
			content.filename || content.body || `file${guessExt(media.contentType)}`;
		const caption = text && text !== filename ? escapeHtml(text) : "";
		const contentType = content.info?.mimetype || media.contentType;

		const isGroupable =
			(msgtype === "m.image" ||
				msgtype === "m.video" ||
				contentType.startsWith("image/") ||
				contentType.startsWith("video/")) &&
			contentType !== "image/gif";

		if (isGroupable) {
			queueImage({
				blob: media.blob,
				contentType,
				filename,
				caption,
				matrixMsgtype: msgtype,
			});
			return;
		}

		await flushImages();
		await sendMedia({
			blob: media.blob,
			contentType,
			filename,
			caption,
			matrixMsgtype: msgtype,
		});
		return;
	}

	if (!text) return;
	await flushImages();
	await sendText(escapeHtml(text));
}

async function joinRoom(roomId: string): Promise<void> {
	const res = await fetch(
		`${HOMESERVER_URL}/_matrix/client/v3/join/${encodeURIComponent(roomId)}?user_id=${encodeURIComponent(APP_SERVICE_USER)}`,
		{
			method: "POST",
			headers: {
				Authorization: `Bearer ${APP_SERVICE_TOKEN}`,
				"Content-Type": "application/json",
			},
			body: "{}",
		},
	);
	if (!res.ok) {
		console.error(
			`Failed to join room ${roomId}: ${res.status} ${await res.text()}`,
		);
	}
}

async function handleEvent(event: MatrixEvent): Promise<void> {
	if (
		event.type === "m.room.member" &&
		event.content.membership === "invite" &&
		event.room_id === ROOM_ID
	) {
		await new Promise((resolve) => setTimeout(resolve, 200));
		await joinRoom(event.room_id);
		return;
	}

	if (event.room_id !== ROOM_ID) return;
	if (event.sender.startsWith(`@m2tg:`)) return;
	if (processedEvents.has(event.event_id)) return;
	processedEvents.add(event.event_id);
	if (event.type !== "m.room.message") return;

	try {
		await handleMessage(event);
	} catch (err) {
		console.error(`Error handling event ${event.event_id}:`, err);
	}
}

const processedTxns = new Set<string>();

new Elysia()
	.onError(({ error, code, set }) => {
		switch (code) {
			case "PARSE":
			case "VALIDATION":
				set.status = 400;
				return { errcode: "M_BAD_REQUEST", error: error.message };
			case "NOT_FOUND":
				set.status = 404;
				return { errcode: "M_NOT_FOUND" };
			default:
				console.error(error);
				return { errcode: "M_UNKNOWN", error: "Internal server error" };
		}
	})
	.macro("auth", () => ({
		beforeHandle: ({ query, headers, set }) => {
			if (
				query.access_token !== HOMESERVER_TOKEN &&
				headers.authorization !== `Bearer ${HOMESERVER_TOKEN}`
			) {
				set.status = 403;
				return { errcode: "M_FORBIDDEN", error: "Bad hs_token" };
			}
		},
	}))
	.guard({ auth: true }, (app) =>
		app.put(
			"/_matrix/app/v1/transactions/:txnId",
			async ({ params, body }) => {
				if (!processedTxns.has(params.txnId)) {
					processedTxns.add(params.txnId);
					for (const event of body.events) {
						await handleEvent(event);
					}
				}
				return {};
			},
			{
				body: t.Object(
					{ events: t.Array(t.Any()) },
					{ additionalProperties: true },
				),
			},
		),
	)
	.get("/health", () => ({ ok: true }))
	.listen(PORT, ({ protocol, hostname, port }) => {
		console.log(`M2tg listening on ${protocol}://${hostname}:${port}`);
	});

await joinRoom(ROOM_ID);
