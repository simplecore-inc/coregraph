# cross-lang-matched fixture

Exercises cross-language edge generation across a realistic three-tier
mini-project:

- `server/UserController.java` — Spring Boot REST endpoints
  `/api/v1/users` (GET + POST)
- `server/OrderStatus.java` — Java enum with three variants
- `server/application.yml` — Spring config keys
- `client/userApi.ts` — TypeScript fetch calls against the same paths
- `client/orderStatus.ts` — TS constants mirroring the Java enum values
- `worker/job_runner.py` — Python env reads mirroring the YAML keys

Expected joins (see `tests/cross_language_fixtures.rs`):

| Kind | Endpoints |
|---|---|
| `ApiPathMatch` | `UserController.listUsers` ↔ `userApi.listUsers` via `/api/v1/users` |
| `ApiPathMatch` | `UserController.createUser` ↔ `userApi.createUser` via `/api/v1/users` |
| `EnumValueMatch` | `OrderStatus.ACTIVE` ↔ `orderStatus.ACTIVE` |
| `EnumValueMatch` | `OrderStatus.PENDING` ↔ `orderStatus.PENDING` |
| `EnumValueMatch` | `OrderStatus.CLOSED` ↔ `orderStatus.CLOSED` |
| `Configures` | `database.pool.size` (YAML) ↔ `DATABASE_POOL_SIZE` (Python env) |

The fixture does NOT include build configuration files (`pom.xml`,
`package.json`, `pyproject.toml`) so it is inert under normal `cargo
build` / `cargo test` and is only consumed by the integration test.
