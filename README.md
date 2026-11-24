# SokoRelay API — Phase 1

Rust + Axum + PostgreSQL API scaffold for the Kenya-first SokoRelay marketplace.

## Included
- Axum HTTP server
- PostgreSQL connection pool using SQLx
- SQLx startup migrations
- Health endpoint
- Category listing endpoint
- Product listing, search, detail and create endpoints
- Input validation and centralized error responses
- Dockerfile and Railway configuration

## Requirements
- Rust toolchain
- PostgreSQL 14+
- SQLx CLI is optional; migrations run automatically at startup

## Local setup

1. Create a PostgreSQL database named `sokorelay`.
2. Copy `.env.example` to `.env` and adjust `DATABASE_URL`.
3. Run:

```bash
cargo run
```

The API listens on `http://localhost:8080`.

## Endpoints

- `GET /health`
- `GET /api/v1/categories`
- `GET /api/v1/products?limit=20&offset=0`
- `GET /api/v1/products?q=shirt`
- `GET /api/v1/products?category_id=<uuid>`
- `GET /api/v1/products/<uuid>`
- `POST /api/v1/products`

Example create request:

```json
{
  "title": "Blue denim jacket",
  "description": "Good condition, size M",
  "price_kes": 1800,
  "category_id": null,
  "image_url": "https://cdn.example.com/jacket.jpg",
  "condition": "used"
}
```

## Railway deployment

1. Push this directory to a Git repository.
2. Create a Railway project and add a PostgreSQL service.
3. Deploy the repository as a service using the included Dockerfile.
4. Add `DATABASE_URL` by referencing the Railway PostgreSQL service's connection URL.
5. Add `CORS_ORIGINS` with your actual frontend origin(s), comma-separated.
6. Railway provides `PORT`; the service binds to `0.0.0.0`.
7. Verify `/health`.

## Important MVP limitations

This is a development scaffold, not a production-ready marketplace:
- Product creation is intentionally unauthenticated for initial API wiring. Do not expose publicly until authentication and seller authorization are implemented.
- The initial migration has users but no public registration/login routes yet.
- No payment collection, escrow, refunds, courier integrations, or money movement is implemented.
- Do not hold customer funds in an ordinary account. Integrate a licensed payment provider / legally appropriate safeguarded settlement product before accepting real payments.
- Add rate limiting, request size limits, structured audit logs, secrets management, backups, security review, and automated tests before launch.
- Product images should be uploaded to object storage using signed upload URLs; do not store image bytes in PostgreSQL.
- Configure exact CORS origins in production; do not use wildcard origins with credentials.

## Phone OTP authentication

Authentication endpoints:

- `POST /api/v1/auth/otp/request` with `{ "phone": "+254712345678" }`
- `POST /api/v1/auth/otp/verify` with `{ "phone": "+254712345678", "code": "123456" }`
- `POST /api/v1/auth/refresh` with `{ "refresh_token": "..." }`
- `POST /api/v1/auth/logout` with `{ "refresh_token": "..." }`
- `GET /api/v1/auth/me` with `Authorization: Bearer <access_token>`

Phone input accepts Kenyan local format (`0712345678`), national digits (`712345678`), or international format (`+254712345678`) and stores it in E.164 format. OTPs expire after 5 minutes, are single-use, allow at most 5 attempts, and requests are limited to 3 per phone per hour. OTP values are stored as HMAC-SHA256 digests using `OTP_PEPPER`; refresh tokens are stored as SHA-256 hashes. Access tokens are HS256 JWTs (15 minutes); refresh tokens rotate on use and expire after 30 days.

For local development, `SMS_MODE=console` simulates delivery by writing the OTP to server logs. It is rejected when `APP_ENV=production`. The async `SmsSender` trait is the integration point for a real SMS gateway; no real SMS provider is configured in this scaffold. Before production, implement and configure a provider adapter, set `SMS_MODE` accordingly, and keep `JWT_SECRET` and `OTP_PEPPER` as independent Railway secrets (each at least 32 characters). Never expose development logs publicly.

Phone OTP is the sign-in method, so passwords are not required or accepted in this flow. The existing nullable `users.password_hash` column remains reserved for a future optional password feature; no plaintext password is stored. Password login/reset should only be added if product requirements call for it, using Argon2id and separate rate-limited flows.

### Example flow

```bash
curl -X POST http://localhost:8080/api/v1/auth/otp/request \
  -H 'content-type: application/json' \
  -d '{"phone":"0712345678"}'

# In development, read the simulated OTP from the server log.
curl -X POST http://localhost:8080/api/v1/auth/otp/verify \
  -H 'content-type: application/json' \
  -d '{"phone":"0712345678","code":"123456"}'
```

The verify response contains a short-lived access token and a refresh token. Store refresh tokens securely (prefer an HttpOnly, Secure, SameSite cookie when using a browser frontend); do not put them in localStorage. Logout revokes the supplied refresh token. Access JWTs remain valid until their short expiry, so clients should discard them immediately on logout.

**Important pre-launch work:** add per-IP throttling/WAF controls, abuse monitoring and SMS cost controls; integrate a real provider; add authenticated route middleware and require a verified seller identity before product creation. The current product `POST` endpoint remains a development endpoint and is not protected by this authentication module yet.
