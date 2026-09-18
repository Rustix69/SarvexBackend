# Sarvex Rust Services

This directory is the new backend workspace. The previous Go/C++ implementation is preserved under `../backend-old/`.

## Phase 01 local stack

```bash
docker compose -f services/docker-compose.yml up --build
```

The refdata API is available at `http://localhost:18080/v1/markets?state=OPEN&limit=50`.

The frontend remains outside this workspace and is intentionally unchanged in Phase 01.
