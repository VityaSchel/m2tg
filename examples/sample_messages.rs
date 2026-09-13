use std::env;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use m2tg::config::Config;
use m2tg::registration::registration_yaml;
use reqwest::{Client, RequestBuilder, header::CONTENT_TYPE};
use serde_json::{Value, json};
use urlencoding::encode;

const STEP_PAUSE: Duration = Duration::from_secs(4);
const USAGE: &str = "usage: ENV_FILE=<staging env file> cargo run --example sample_messages -- [registration|register|join|send]\nWithout a command it prints the staging steps. ENV_FILE is required so production ./.env is never used";
const FORMATTING_SAMPLE: &str = r#"<p>{tag} formatting</p>
<p><b>bold</b> <i>italic</i> <u>underline</u> <s>strike</s> <code>a &lt; b</code> <a href="https://example.org/?a=1&amp;b=2">link</a> <span data-mx-spoiler>spoiler</span></p>
<blockquote>
<p>quoted <b>bold</b></p>
</blockquote>
<ul>
<li>bullet one</li>
<li>bullet two</li>
</ul>
<ol start="3">
<li>three</li>
<li>four</li>
</ol>
<pre><code class="language-rust">fn main() {
	println!("&lt;hi&gt; &amp; bye");
}
</code></pre>
"#;

type Fixture = (&'static str, &'static str, &'static [u8]);

macro_rules! fixture {
	($file:literal, $mime:literal) => {
		($file, $mime, include_bytes!(concat!("fixtures/", $file)))
	};
}

enum Message {
	Text(String),
	Html(&'static str, &'static str),
	Reply(&'static str),
	Edit,
	Media(&'static str, Fixture, Option<&'static str>),
}

struct Step(&'static str, Vec<Message>);

#[rustfmt::skip]
fn steps() -> Vec<Step> {
	use Message::*;
	vec![
		Step("text with '<not a tag> & \"quotes\"' shown literally", vec![Text("{tag} plain <not a tag> & \"quotes\"".into())]),
		Step("bold, italic, underline, strike, code 'a < b', link, spoiler, blockquote, 2 bullets, items 3. and 4., rust code block; no 'plain fallback'", vec![Html("{tag} formatting plain fallback", FORMATTING_SAMPLE)]),
		Step("reply text only, no quoted lines; 'EDIT MUST NOT APPEAR' appears nowhere", vec![Reply("{tag} reply, quote must be gone"), Edit]),
		Step("red photo whose caption shows '<b>literal</b>' as text", vec![Media("m.image", fixture!("red.png", "image/png"), Some("{tag} captioned photo <b>literal</b>"))]),
		Step("header text, then one album of red, green, blue", vec![
			Text("{tag} album of 3 follows".into()),
			Media("m.image", fixture!("red.png", "image/png"), None),
			Media("m.image", fixture!("green.png", "image/png"), None),
			Media("m.image", fixture!("blue.png", "image/png"), None),
		]),
		Step("GIF animation with caption", vec![Media("m.image", fixture!("blink.gif", "image/gif"), Some("{tag} gif"))]),
		Step("audio player tone.mp3 with caption", vec![Media("m.audio", fixture!("tone.mp3", "audio/mpeg"), Some("{tag} audio"))]),
		Step("document notes.txt with caption", vec![Media("m.file", fixture!("notes.txt", "text/plain"), Some("{tag} text document"))]),
		Step("long text cut to 4096 characters, ending in …", vec![Text(format!("{{tag}} long\n{}", "a".repeat(5000)))]),
		Step("':blob:' alone and untagged; 'custom emoji plain fallback' means the emoji was lost", vec![Html("{tag} custom emoji plain fallback", r#"<img data-mx-emoticon src="mxc://example.org/blob" alt=":blob:" title=":blob:" height="32" />"#)]),
		Step("done; if missing, check the staging m2tg log", vec![Text("{tag} done".into())]),
	]
}

struct SampleSender {
	http: Client,
	homeserver: String,
	token: String,
	room: String,
	bridge: String,
	localpart: String,
	user: String,
}

impl SampleSender {
	fn new(config: &Config) -> Result<Self> {
		let localpart = format!("{}-sender", config.app_service_localpart());
		Ok(Self {
			http: Client::builder().timeout(Duration::from_secs(60)).build()?,
			homeserver: config.homeserver_url.clone(),
			token: config.app_service_token.clone(),
			room: config.room_id.clone(),
			bridge: config.app_service_user.clone(),
			user: format!("@{localpart}:{}", config.server_name()),
			localpart,
		})
	}

	fn url(&self, path: &str) -> String {
		let user = encode(&self.user);
		format!("{}/_matrix/{path}?user_id={user}", self.homeserver)
	}

	fn room_url(&self, path: &str) -> String {
		self.url(&format!("client/v3/rooms/{}/{path}", encode(&self.room)))
	}

	async fn call(&self, request: RequestBuilder) -> Result<Value> {
		let response = request.bearer_auth(&self.token).send().await?;
		let status = response.status();
		let body: Value = response.json().await.unwrap_or_default();
		ensure!(status.is_success(), "{status}: {body}");
		Ok(body)
	}

	async fn register(&self) -> Result<()> {
		let body = json!({ "type": "m.login.application_service", "username": self.localpart, "inhibit_login": true });
		let register = format!("{}/_matrix/client/v3/register", self.homeserver);
		match self.call(self.http.post(register).json(&body)).await {
			Ok(_) => println!("registered {}", self.user),
			Err(e) => ensure!(e.to_string().contains("M_USER_IN_USE"), e),
		}
		Ok(())
	}

	async fn join(&self) -> Result<()> {
		self.register().await?;
		let url = self.url(&format!("client/v3/join/{}", encode(&self.room)));
		self.call(self.http.post(url).json(&json!({}))).await?;
		println!("{} joined {}", self.user, self.room);
		Ok(())
	}

	async fn require_joined_room(&self) -> Result<()> {
		let (user, room, bridge) = (&self.user, &self.room, &self.bridge);
		let url = self.url("client/v3/joined_rooms");
		let mut rooms = self.call(self.http.get(url)).await?;
		let rooms: Vec<String> = serde_json::from_value(rooms["joined_rooms"].take())?;
		ensure!(rooms.contains(room), "{user} is not in {room}; run `join`");
		let url = self.room_url("joined_members");
		let members = self.call(self.http.get(url)).await?;
		let bridge_joined = members["joined"][bridge].is_object();
		ensure!(bridge_joined, "{bridge} is not in {room}; start m2tg");
		Ok(())
	}

	async fn upload(&self, mimetype: &str, bytes: &'static [u8]) -> Result<Value> {
		let request = self.http.post(self.url("media/v3/upload"));
		let request = request.header(CONTENT_TYPE, mimetype).body(bytes);
		Ok(self.call(request).await?["content_uri"].take())
	}

	async fn content(&self, message: &Message, tag: &str, original: &Value) -> Result<Value> {
		let tagged = |template: &str| template.replace("{tag}", tag);
		Ok(match message {
			Message::Text(body) => json!({ "msgtype": "m.text", "body": tagged(body) }),
			Message::Html(body, html) => json!({
				"msgtype": "m.text", "body": tagged(body),
				"format": "org.matrix.custom.html", "formatted_body": tagged(html),
			}),
			Message::Reply(body) => json!({
				"msgtype": "m.text", "format": "org.matrix.custom.html",
				"body": format!("> <{}> quoted original\n> second line\n\n{}", self.user, tagged(body)),
				"formatted_body": format!("<mx-reply><blockquote>quoted original</blockquote></mx-reply>{}", tagged(body)),
				"m.relates_to": { "m.in_reply_to": { "event_id": original } },
			}),
			Message::Edit => json!({
				"msgtype": "m.text", "body": "* EDIT MUST NOT APPEAR",
				"m.new_content": { "msgtype": "m.text", "body": "EDIT MUST NOT APPEAR" },
				"m.relates_to": { "rel_type": "m.replace", "event_id": original },
			}),
			Message::Media(msgtype, (name, mimetype, bytes), caption) => json!({
				"msgtype": msgtype, "filename": name,
				"body": caption.map_or_else(|| name.to_string(), tagged),
				"url": self.upload(mimetype, bytes).await?,
				"info": { "mimetype": mimetype, "size": bytes.len() },
			}),
		})
	}

	async fn send(&self) -> Result<()> {
		self.require_joined_room().await?;
		let run_id = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
		let steps = steps();
		let mut original = Value::Null;
		for (index, Step(_, messages)) in steps.iter().enumerate() {
			tokio::time::sleep(STEP_PAUSE).await;
			let tag = format!("[sample {run_id}] {:02}", index + 1);
			println!("sending {tag}");
			for (part, message) in messages.iter().enumerate() {
				let content = self.content(message, &tag, &original).await?;
				let url = self.room_url(&format!("send/m.room.message/{run_id}-{index}-{part}"));
				let mut sent = self.call(self.http.put(url).json(&content)).await?;
				if original.is_null() {
					original = sent["event_id"].take();
				}
			}
		}
		println!(
			"\nExpect in the staging Telegram chat, in order; text and captions are tagged [sample {run_id}] NN:"
		);
		for (index, Step(expect, _)) in steps.iter().enumerate() {
			println!("{:02} {expect}", index + 1);
		}
		println!(
			"\nThe bridge log must show no WARN or ERROR after `joined` except `attempt N failed, retrying`."
		);
		Ok(())
	}
}

fn print_preparation_steps(env_file: &str) {
	println!(
		r#"Staging steps for ENV_FILE={env_file}

1. Fill {env_file} from .env.example with an APP_SERVICE_USER, PORT, bot, channel and room separate from production, e.g. APP_SERVICE_USER=@m2tg-staging:example.org. Its localpart becomes the registration id, so reusing production's replaces the production registration. The room must be invite-only and unencrypted. Generate both tokens with `openssl rand -hex 32`.
2. Add the bot to the channel as an administrator with Post Messages, post in the channel, and set TELEGRAM_CHAT_ID to the id this prints:
   curl -s "https://api.telegram.org/bot<TELEGRAM_BOT_TOKEN>/getUpdates" | jq '.result[] | (.my_chat_member // .channel_post) | .chat | {{id, title}}'"#
	);
}

fn print_steps(env_file: &str, config: &Config, sender: &SampleSender) {
	let example = format!("ENV_FILE={env_file} cargo run --example sample_messages --");
	let id = config.app_service_localpart();
	let server_name = config.server_name();
	let SampleSender {
		bridge, user, room, ..
	} = sender;
	println!(
		r#"3. Register the output of the command below. On continuwuity, send `!admin appservices register` to the admin room with the YAML in a code block in the same message. On Synapse, save it to a file listed in app_service_config_files and restart. url comes from HOST and PORT; edit it if the homeserver reaches the bridge at another address.
   {example} registration
4. Start the bridge on a host where the homeserver reaches url:
   ENV_FILE={env_file} cargo run
5. Invite {bridge} into {room} and wait for `joined` in the bridge log. continuwuity rejects the invite while it cannot reach url.
6. Register {user}:
   {example} register
7. Invite {user}, then accept the invite:
   {example} join
8. Send the samples and compare the Telegram chat with the list printed at the end:
   {example} send
   Then post an image from an account on a homeserver other than {server_name} and check that it arrives.
9. Stop the bridge and send `!admin appservices unregister {id}` to the admin room."#
	);
}

#[tokio::main]
async fn main() -> Result<()> {
	let env_file = env::var("ENV_FILE").unwrap_or_default();
	ensure!(!env_file.is_empty(), USAGE);
	let command = env::args().nth(1);
	if command.is_none() {
		print_preparation_steps(&env_file);
	}
	let _ = rustls::crypto::ring::default_provider().install_default();
	let config = if command.is_none() {
		Config::from_env()
			.with_context(|| format!("fill {env_file}, then run this again for steps 3 to 9"))?
	} else {
		Config::from_env()?
	};
	let sender = SampleSender::new(&config)?;
	match command.as_deref() {
		None => print_steps(&env_file, &config, &sender),
		Some("registration") => print!("{}", registration_yaml(&config, &[&sender.localpart])),
		Some("register") => sender.register().await?,
		Some("join") => sender.join().await?,
		Some("send") => sender.send().await?,
		Some(other) => bail!("unknown command {other:?}\n{USAGE}"),
	}
	Ok(())
}
