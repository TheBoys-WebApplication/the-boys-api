# the-boys-api

Rust/Axum backend for TheBoys — a trip planning and expense splitting app for friend groups.

## Stack

- **Rust** (edition 2021) + **Axum 0.7**
- **PostgreSQL 16** via SQLx 0.8 (runtime queries)
- **Argon2id** password hashing + **JWT** (HS256, 7-day)
- **Docker** for local Postgres

## Local Setup

**Prerequisites:** Rust, Docker Desktop

```bash
# 1. Start Postgres
docker-compose up -d          # run from repo root

# 2. Copy env and configure
cp .env.example .env          # edit JWT_SECRET if desired

# 3. Run (migrations apply automatically on startup)
cargo run
```

Server starts on `http://localhost:3000`.

## Environment Variables

| Variable | Description |
|---|---|
| `DATABASE_URL` | Postgres connection string |
| `JWT_SECRET` | HS256 signing secret (change in production) |
| `BIND_ADDR` | Listen address (default `0.0.0.0:3000`) |
| `RUST_LOG` | Log filter (e.g. `the_boys_api=debug`) |

## API

All routes are prefixed `/api/v1`. Protected routes require `Authorization: Bearer <token>`.

### Auth
| Method | Path | Auth | Description |
|---|---|---|---|
| POST | `/auth/register` | — | Register with email, password, display_name |
| POST | `/auth/login` | — | Login, returns JWT |

### Groups
| Method | Path | Auth | Description |
|---|---|---|---|
| POST | `/groups` | ✓ | Create group (caller becomes leader) |
| GET | `/groups` | ✓ | List groups you belong to |
| GET | `/groups/:id` | ✓ member | Get group details |
| PUT | `/groups/:id` | ✓ leader | Update name/description |
| DELETE | `/groups/:id` | ✓ leader | Delete group |
| POST | `/groups/join` | ✓ | Join group via `invite_code` |
| POST | `/groups/:id/invite/regenerate` | ✓ leader | Rotate invite code |
| GET | `/groups/:id/members` | ✓ member | List members |
| DELETE | `/groups/:id/members/:uid` | ✓ leader | Remove a member |

## Development

```bash
cargo check          # fast type check
cargo clippy         # lint
cargo test           # run tests
cargo build --release
```
