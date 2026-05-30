# EC2 Backend + Vercel Frontend Demo Deploy

This is the simple public-demo setup:

- EC2 runs Docker Compose backend services plus both demo simulators.
- Nginx on EC2 exposes only `https://api.sarvaex.in`.
- Vercel hosts the Vite frontend.
- Vercel builds the frontend with `VITE_API_BASE_URL=https://api.sarvaex.in`.

This is not the full production target from `docs/10_production_phase.md`; it is a public demo deployment.

## 1. EC2 Instance

Use Ubuntu 24.04 LTS or 22.04 LTS.

Suggested demo size:

- Minimum: `t3.large` or `t3a.large`
- Better for a smooth demo: `t3.xlarge`
- Disk: 40-80 GB gp3

Security group inbound rules:

- SSH `22` from your IP only
- HTTP `80` from `0.0.0.0/0`
- HTTPS `443` from `0.0.0.0/0`

Do not open Postgres, Redis, NATS, gRPC, or the raw gateway ports publicly.

## 2. Install Server Packages

```bash
sudo apt update
sudo apt install -y ca-certificates curl git nginx certbot python3-certbot-nginx jq
```

Install Docker Engine and the Docker Compose plugin using Docker's Ubuntu instructions, then verify:

```bash
docker --version
docker compose version
```

## 3. Clone And Configure Backend

```bash
git clone <YOUR_REPO_URL> sarvex
cd sarvex
cp .env.ec2.example .env
nano .env
```

Change at least:

- `POSTGRES_PASSWORD`
- `ALLOWED_ORIGINS`

Use the final Vercel domain in `ALLOWED_ORIGINS`. During first setup you can include both:

```env
ALLOWED_ORIGINS=https://sarvaex.in,https://www.sarvaex.in,https://YOUR_PROJECT.vercel.app
```

## 4. Start Backend And Simulators

```bash
ENV_FILE=.env ./scripts/start-demo-backend.sh
```

Expected final line:

```text
Health OK: 17/17 running, 0 down
```

Check locally on EC2:

```bash
curl http://127.0.0.1:18080/v1/health/overview | jq '.summary'
```

## 5. Nginx For `api.sarvaex.in`

Copy the example config:

```bash
sudo cp deploy/nginx/sarvex-api.conf.example /etc/nginx/sites-available/sarvex-api
sudo ln -s /etc/nginx/sites-available/sarvex-api /etc/nginx/sites-enabled/sarvex-api
sudo nginx -t
sudo systemctl reload nginx
```

Point DNS:

- `api.sarvaex.in` A record -> EC2 public IP

Then enable HTTPS:

```bash
sudo certbot --nginx -d api.sarvaex.in
```

Verify:

```bash
curl https://api.sarvaex.in/readyz
curl https://api.sarvaex.in/v1/health/overview | jq '.summary'
```

## 6. Vercel Frontend

In Vercel:

- Framework preset: Vite
- Root directory: `frontend`
- Build command: `npm run build`
- Output directory: `dist`

Set environment variable:

```env
VITE_API_BASE_URL=https://api.sarvaex.in
```

Deploy. After deployment, add the Vercel production URL to EC2 `.env` `ALLOWED_ORIGINS`, then restart backend:

```bash
ENV_FILE=.env ./scripts/start-demo-backend.sh
```

## 7. Demo Restart Commands

Start:

```bash
cd ~/sarvex
ENV_FILE=.env ./scripts/start-demo-backend.sh
```

Stop:

```bash
cd ~/sarvex
docker compose --env-file .env down
pkill -f 'sarvex-demo-sim|cmd/demo-sim' || true
```

Health:

```bash
curl https://api.sarvaex.in/v1/health/overview | jq '.summary'
```

## 8. Important Demo Notes

- The frontend must never use `localhost` in Vercel.
- Only `VITE_` variables are exposed to Vite client code.
- Keep raw backend ports closed in the EC2 security group.
- The current demo auth is simple demo-token auth, so do not present this as production security.
- The simulators intentionally create market activity. Keep them running during the demo.
