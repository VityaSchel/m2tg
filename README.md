# M2TG

Matrix appservice that mirrors messages from a public Matrix room to a Telegram channel.

- Supports formatting and media
- Photos and videos sent within 10 s of each other are grouped
- Media without a caption waits up to 10 s for one: if the next message is a text from the same sender, it becomes the caption unless it is a reply or too long, as Element Web sends captions separately
- Edits, redactions and reactions are not forwarded
- Failed sends are retried, then dropped with an error log

## Install

- Download a binary (`x86_64` or `aarch64`) from [Releases](https://git.hloth.dev/hloth/m2tg/releases) (3 MB):

   ```sh
   wget https://git.hloth.dev/hloth/m2tg/releases/download/v2.0.0/m2tg-linux-x86_64
   install -Dm755 m2tg-linux-x86_64 /usr/local/bin/m2tg
   ```

   Builds are [reproducible](./CONTRIBUTING.md#release-builds).

- Or build it with `cargo build --release --locked`. The host needs `ca-certificates`.

## Setup

1. Add your Telegram bot to a channel as an administrator with **Post Messages**.
2. Install the systemd unit, the [configuration](#configuration) and the tokens for an instance named `example`, then fill `/etc/m2tg/example.env` except the tokens. In a source checkout, skip `wget` and use `contrib/m2tg@.service`.

   ```sh
   RAW=https://git.hloth.dev/hloth/m2tg/raw/branch/main
   wget $RAW/contrib/m2tg@.service $RAW/.env.example
   install -Dm644 m2tg@.service /etc/systemd/system/m2tg@.service
   install -Dm600 .env.example /etc/m2tg/example.env
   install -d -m700 /etc/m2tg/credentials/example
   cd /etc/m2tg/credentials/example && umask 077
   printf %s "$(openssl rand -hex 32)" > app-service-token
   printf %s "$(openssl rand -hex 32)" > homeserver-token
   printf %s '<BotFather token>' > telegram-bot-token
   ```

3. Run `(set -a; . /etc/m2tg/example.env && CREDENTIALS_DIRECTORY=/etc/m2tg/credentials/example exec m2tg registration)` to print the appservice registration.
   - On Continuwuity, send `!admin appservices register` to the admin room with the YAML in a code block in the same message.
   - On Synapse, save it to a file listed in `app_service_config_files` and restart.
   - `url` is `HOST` + `PORT`, edit it if the homeserver reaches m2tg at another address.
4. Run `systemctl daemon-reload && systemctl enable --now m2tg@example`.
5. Invite `APP_SERVICE_USER` to the room. The room must be unencrypted. 
   - Continuwuity rejects the invite while it cannot reach `url`.

## Configuration

m2tg reads environment variables, set by systemd `EnvironmentFile=`, `docker run --env-file` or `set -a; . ./m2tg.env; set +a` in a shell (see [.env.example](.env.example)):

| Variable                                 | Description                                    |
| ---------------------------------------- | ---------------------------------------------- |
| `HOMESERVER_URL`                         | e.g. `https://matrix.example.org`              |
| `APP_SERVICE_USER`                       | e.g. `@m2tg:example.org`                       |
| `ROOM_ID`                                | Internal room id, `!…`                         |
| `PORT`, `HOST`                           | Listen address; `HOST` defaults to `127.0.0.1` |
| `APP_SERVICE_TOKEN`, `HOMESERVER_TOKEN`  | `as_token` and `hs_token` of the registration  |
| `TELEGRAM_CHAT_ID`, `TELEGRAM_BOT_TOKEN` | Chat id (`-100…` or `@channel`) and bot token  |

Optionally, set `TELEGRAM_API_BASE` to replace the Bot API URL.

When `$CREDENTIALS_DIRECTORY` is set, as by `LoadCredential=` in [m2tg@.service](./contrib/m2tg@.service), the three tokens are read from files named after their variables: `app-service-token`, `homeserver-token` and `telegram-bot-token`. Otherwise they are read from the environment.

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md). To test a staging bridge against a real homeserver and Telegram chat, run `ENV_FILE=.env.staging cargo run --example sample_messages`.

## License

[MIT](./LICENSE)

## Donate

[hloth.dev/donate](https://hloth.dev/donate)
