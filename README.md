# the-boys-api

Rust/Axum backend for TheBoys — a trip planning and expense splitting app for friend groups.

## Stack

- **Rust** (edition 2021) + **Axum 0.7**
- **PostgreSQL 16** via SQLx 0.8 (runtime queries)
- **Argon2id** password hashing + **JWT** (HS256, 7-day)
- **Docker** for local Postgres

## Local Setup

**Prerequisites:** Rust, Docker Desktop

```powershell
# 1. Start Postgres (from monorepo root)
docker-compose up -d

# 2. Run (migrations apply automatically on startup)
cargo run
```

Server starts on `http://localhost:3000`.

## Environment Variables

| Variable | Description |
|---|---|
| `DATABASE_URL` | Postgres connection string |
| `JWT_SECRET` | HS256 signing secret (change in production) |
| `BIND_ADDR` | Listen address (default `0.0.0.0:3000`) |
| `RUST_LOG` | Log filter (e.g. `the_boys_api=debug,tower_http=debug`) |

## Database

Connect with any Postgres client using:
```
postgresql://theboys:theboys_dev@localhost:5432/theboys
```

Migrations in `migrations/` run automatically on startup via `sqlx::migrate!`.

| Migration | Description |
|---|---|
| `0001_create_users.sql` | `users` table |
| `0002_create_groups.sql` | `groups` + `group_members` tables |
| `0003_add_name_fields_to_users.sql` | `first_name`, `last_name` columns on `users` |

## API

All routes prefixed `/api/v1`. Protected routes require `Authorization: Bearer <token>`.

### Auth

| Method | Path | Auth | Description |
|---|---|---|---|
| POST | `/auth/register` | — | Register — body: `email`, `password`, `first_name`, `last_name`, `display_name` |
| POST | `/auth/login` | — | Login — body: `email`, `password`. Returns JWT |
| GET  | `/auth/me` | ✓ | Current user profile |
| POST | `/auth/logout` | ✓ | Validates token, returns 204 (client clears token) |

### Groups

| Method | Path | Auth | Description |
|---|---|---|---|
| POST | `/groups` | ✓ | Create group (caller becomes leader) |
| GET | `/groups` | ✓ | List groups you belong to |
| GET | `/groups/:id` | ✓ member | Get group details |
| PUT | `/groups/:id` | ✓ leader | Update name/description |
| DELETE | `/groups/:id` | ✓ leader | Delete group and all data |
| POST | `/groups/join` | ✓ | Join group — body: `invite_code` |
| POST | `/groups/:id/invite/regenerate` | ✓ leader | Rotate invite code |
| GET | `/groups/:id/members` | ✓ member | List members |
| DELETE | `/groups/:id/members/:uid` | ✓ leader | Remove a member |

## Development

```powershell
cargo check          # fast type check
cargo clippy         # lint
cargo test           # run tests
cargo build --release
```
