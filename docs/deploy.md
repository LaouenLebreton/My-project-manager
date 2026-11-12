# Putting a team online

By default a team lives inside the Super PM app of whoever created it (see
`decisions.md`, "Teams without a server to set up"). To reach it from anywhere, run the team
server online. It is one small program (`super-pm-server`) with one database file.

## What you need

1. **A hosting account**: anything that runs a Docker container with a disk that survives
   restarts. Recommended: **Fly.io** (small machine plus a 1 GB volume, a few dollars a month;
   HTTPS included). Alternatives: Render, Railway, or any Linux server with Docker and Caddy.
2. **An address** (optional but nicer), e.g. `pm.yourcompany.com`. Without one, the provider's
   address works (`super-pm-yourteam.fly.dev`).
3. **Backups**: the provider's volume snapshots, or copy `/data/server.sqlite` regularly.

## Settings

| Setting | Meaning |
|---|---|
| `SUPER_PM_PUBLIC_URL` | The address people use, e.g. `https://pm.yourcompany.com`. Goes into invites. |
| `SUPER_PM_OPEN_SIGNUP=1` | Anyone can create an account in the app (like an online service). Leave unset to admit people by invitation only. |
| `SUPER_PM_DATA` | Where data is kept (`/data` in the container). |
| `SUPER_PM_LISTEN` | Address and port (`0.0.0.0:8080` in the container). |
| `SUPER_PM_SMTP_URL` | Optional: a mail account to email invites with (see below). |
| `SUPER_PM_MAIL_FROM` | The sender of those emails, e.g. `Super PM <pm@yourcompany.com>`. |

### Invites by email

With a mail account, the server emails each invite itself (and tells people who already have an
account that they were added to a team). Without one, the app shows the invites to copy and send
by hand, as before. Either way the invite can still be copied in the app, and if an email cannot
be sent the app says so next to that person.

Set the account as one secret, never on the command line (it holds the password):

- TLS from the start (port 465): `smtps://USER:PASSWORD@smtp.example.com`
- STARTTLS (port 587): `smtp://USER:PASSWORD@smtp.example.com:587?tls=required`

Special characters in the user or password must be URL-encoded (`@` is `%40`). Most mail
providers (Postmark, Mailgun, Amazon SES, Fastmail, Google Workspace with an app password) give
these settings. Email also needs `SUPER_PM_PUBLIC_URL`, since the address goes in the invite. On
Fly.io:

```sh
fly secrets set SUPER_PM_SMTP_URL='smtps://USER:PASSWORD@smtp.example.com' SUPER_PM_MAIL_FROM='Super PM <pm@yourcompany.com>'
```

At start the server checks that the mail server answers and prints the result in its log.

## Fly.io, step by step

```bash
fly launch --no-deploy --name super-pm-yourteam --copy-config=false
fly volumes create super_pm_data --size 1
fly secrets set SUPER_PM_PUBLIC_URL=https://super-pm-yourteam.fly.dev
fly deploy
```

With this `fly.toml` next to the Dockerfile:

```toml
app = "super-pm-yourteam"
primary_region = "cdg"

[build]

[env]
  SUPER_PM_DATA = "/data"
  SUPER_PM_LISTEN = "0.0.0.0:8080"

[mounts]
  source = "super_pm_data"
  destination = "/data"

[http_service]
  internal_port = 8080
  force_https = true
  auto_stop_machines = "off"
  min_machines_running = 1
```

Keep exactly one machine: the server keeps each open project's latest state in memory and
orders changes there. (Several machines need the Postgres storage planned in
`iteration-3-plan.md`, step 7.)

## Moving teams you already have

If your teams live on your Mac, choose **Team > Team Server > Connect Teams to a Server…**,
type the server's address and create your account there (or tick "new server" the very first
time). Everything moves: people, projects with their history, permissions and chat. Send the
new invites it shows you; until teammates use them, their app tells them where the team went.
Keep Super PM open on your Mac for a few days so it can tell them.

## First use

1. In Super PM: **Team > Team Server > Sign In to a Team Server…**, type the address, tick
   **"This is a new server"**, and enter your name, email and a password. You become the
   server's administrator.
2. **Team > Create Team Project…** and **Invite People…** work as on your Mac; the invites
   now carry the online address, so people can join from anywhere.

## Any Linux server

```bash
docker build -t super-pm-server .
docker run -d --restart unless-stopped --name super-pm \
  -p 127.0.0.1:8080:8080 -v super-pm-data:/data \
  -e SUPER_PM_PUBLIC_URL=https://pm.yourcompany.com super-pm-server
```

Then put Caddy in front for HTTPS (`/etc/caddy/Caddyfile`):

```
pm.yourcompany.com {
    reverse_proxy 127.0.0.1:8080
}
```
