# M2TG

Lightweight binary Matrix appservice that reposts messages from a Matrix room to a Telegram channel using Telegram Bot API. Perfect for announcements read-only Matrix rooms.

Registration file:

```yaml
id: m2tg
url: http://localhost:PORT
as_token: APP_SERVICE_TOKEN
hs_token: HOMESERVER_TOKEN
sender_localpart: m2tg
namespaces:
  users:
    - exclusive: false
      regex: "@.*:.*"
  aliases: []
  rooms: []
rate_limited: false
```

Environment variables:

```env
# Full url
HOMESERVER_URL=""
# Not alias, internal room id
ROOM_ID=""
# Port for M2tg to work on
PORT=""

# Any random 64+ character strings
APP_SERVICE_TOKEN=""
HOMESERVER_TOKEN=""

# @botfather
TELEGRAM_CHAT_ID=""
TELEGRAM_BOT_TOKEN=""
```

## Usage

[Download a binary](https://git.hloth.dev/hloth/m2tg/releases) or [build yourself](#build).

## Build

[Bun](https://bun.sh) is required.

1. Clone repository
2. Install dependencies: `bun ci`
- To run a dev server: `bun src/index.ts`
- To build a binary for your platform: `bun run build`
- To build all binaries for all platforms: `bun run crossbuild`

## License

[MIT](./LICENSE)

## Donate

[hloth.dev/donate](https://hloth.dev/donate)

