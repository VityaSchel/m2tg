# M2TG

Lightweight binary Matrix appservice that reposts messages from a Matrix room to a Telegram channel using Telegram Bot API. Perfect for announcements read-only Matrix rooms.

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

## License

[MIT](./LICENSE)

## Donate

[hloth.dev/donate](https://hloth.dev/donate)

