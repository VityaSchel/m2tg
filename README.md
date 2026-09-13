# M2TG

Matrix appservice that mirrors messages from a public Matrix room to a Telegram channel.

- Supports formatting and media
- Photos and videos sent together are grouped
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

1. Add a [@BotFather](https://t.me/botfather) bot to your channel as an administrator with **Post Messages**.
2. Fill in the [configuration](#configuration). To generate tokens, run `openssl rand -hex 32`.
3. Run `m2tg registration` to print the appservice registration. On Continuwuity, send `!admin appservices register` to the admin room with the YAML in a code block in the same message. On Synapse, save it to a file listed in `app_service_config_files` and restart. `url` is `HOST` + `PORT`, edit it if the homeserver reaches m2tg at another address.
4. Start `m2tg` or [set up a systemd service](./contrib/m2tg@.service).
5. Invite `APP_SERVICE_USER` to the room. The room must be unencrypted. Continuwuity rejects the invite while it cannot reach `url`.

## Configuration

Set these in the environment or in `./.env` (see [.env.example](.env.example)):

| Variable                                 | Description                                    |
| ---------------------------------------- | ---------------------------------------------- |
| `HOMESERVER_URL`                         | e.g. `https://matrix.example.org`              |
| `APP_SERVICE_USER`                       | e.g. `@m2tg:example.org`                       |
| `ROOM_ID`                                | Internal room id, `!…`                         |
| `PORT`, `HOST`                           | Listen address; `HOST` defaults to `127.0.0.1` |
| `APP_SERVICE_TOKEN`, `HOMESERVER_TOKEN`  | `as_token` and `hs_token` of the registration  |
| `TELEGRAM_CHAT_ID`, `TELEGRAM_BOT_TOKEN` | Chat id (`-100…` or `@channel`) and bot token  |

Pass `ENV_FILE=path` to load env at a different file. Optionally, set `TELEGRAM_API_BASE` to replace the Bot API URL.

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](./LICENSE)

## Donate

[hloth.dev/donate](https://hloth.dev/donate)
