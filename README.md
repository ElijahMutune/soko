# SokoRelay API — Phase 1

Rust + Axum + PostgreSQL API scaffold for the Kenya-first SokoRelay marketplace.

## Included
- Axum HTTP server
- PostgreSQL connection pool using SQLx
- SQLx startup migrations
- Health endpoint
- Category listing endpoint
- Product listing, search, detail and authenticated create endpoints
- Authenticated single and batch image uploads (JPEG, PNG, WEBP; up to 8 images per listing) for local development
- Ordered product photo galleries with primary-image selection, add/remove/reorder operations
- Owner-scoped multi-image AI listing suggestions using the Groq multimodal Chat Completions API
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
- `POST /api/v1/products` (requires authentication; accepts optional ordered `image_ids`, maximum 8)
- `GET /api/v1/products/<uuid>` and `GET /api/v1/products` include an `images` gallery array
- `POST /api/v1/uploads/images` (one multipart `file`; requires authentication)
- `POST /api/v1/uploads/images/batch` (1-8 repeated multipart fields named `file`; 8 MB per image, 32 MB total; requires authentication)
- `POST /api/v1/products/<uuid>/images` with `{ "image_ids": ["<uuid>", "<uuid>"] }` to attach additional images (max 8 total)
- `PUT /api/v1/products/<uuid>/images/reorder` with the complete ordered `image_ids` array; first item becomes primary
- `DELETE /api/v1/products/<uuid>/images/<image-uuid>` to remove an image; first remaining image becomes primary
- `GET /media/<server-generated-filename>` (serves local development uploads)
- `POST /api/v1/ai/listing-suggestions` with `{ "image_ids": ["<uuid>", "<uuid>"] }` for multi-view AI analysis; legacy `{ "image_id": "<uuid>" }` remains supported

Example create request (include the access token in the Authorization header):

```http
POST /api/v1/products HTTP/1.1
Authorization: Bearer <access_token>
Content-Type: application/json
```

```json
{
  "title": "Blue denim jacket",
  "description": "Good condition, size M",
  "price_kes": 1800,
  "category_id": null,
  "image_ids": ["<uploaded-image-uuid-1>", "<uploaded-image-uuid-2>"],
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
- Product creation requires a valid bearer access token and an active user account. The server assigns `seller_id` from the token; clients cannot choose another seller.
- No payment collection, escrow, refunds, courier integrations, or money movement is implemented.
- Do not hold customer funds in an ordinary account. Integrate a licensed payment provider / legally appropriate safeguarded settlement product before accepting real payments.
- Add rate limiting, request size limits, structured audit logs, secrets management, backups, security review, and automated tests before launch.
- Local product image uploads are for development only. Configure durable object storage (for example, Cloudinary or S3-compatible storage) before deploying image uploads to Railway; its container filesystem is ephemeral. Image bytes are stored on disk locally, never in PostgreSQL.
- Add per-IP throttling and a persistent storage quota before public launch. The AI listing endpoint currently limits each seller to 10 attempts per hour.
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

**Important pre-launch work:** add per-IP throttling/WAF controls, abuse monitoring and SMS cost controls; integrate a real provider; apply the same authenticated-user check to every future seller write route; add seller verification and moderation controls before public launch. Product creation is now protected, but the rest of the marketplace write workflows have not been implemented yet.


## Product images and AI listing drafts

### Local development configuration

Add these variables to `.env`:

```env
IMAGE_STORAGE_MODE=local
GROQ_API_KEY=your-groq-api-key
GROQ_VISION_MODEL=qwen/qwen3.8-27b
```

The Groq API key is optional for server startup and is only required when requesting AI suggestions. Keep it server-side in `.env` and in Railway secrets; never put it in browser JavaScript. The default vision model is `qwen/qwen3.8-27b`, which accepts text and image input and supports JSON mode. Groq's free plan has rate and daily request limits; check the current limits in your Groq Console because limits can change.

### Groq API setup

1. Create a key in [GroqCloud Console](https://console.groq.com/keys).
2. Add `GROQ_API_KEY` to your local `.env` file. Do not commit `.env`.
3. The server sends image bytes as a base64 data URL to `https://api.groq.com/openai/v1/chat/completions`; the key stays on the Rust backend.
4. The model can be changed with `GROQ_VISION_MODEL` without changing the request body.

**Free-tier note:** Groq documents a free plan with per-model request and token limits. The Qwen vision model's limits and availability are account- and plan-dependent; verify them in your Groq Console before relying on it. A 429 response means you have hit one of the applicable limits; respect `Retry-After` and the rate-limit headers.

**Image privacy:** Only upload product-only photographs during development. Avoid faces, identity documents, home addresses, receipts, or other sensitive/private material. Review Groq's current data-use and retention terms before submitting customer images.


## Multiple product photographs

A listing can have up to 8 photographs. The first uploaded/attached image is the primary cover image. Buyers receive a stable, ordered `images` array with each product response. The top-level `image_url` field remains for backwards compatibility and is synchronized to the primary image's relative `/media/...` URL when using the gallery API.

### Recommended Postman flow

1. Upload 2-8 images in one request:
   - `POST /api/v1/uploads/images/batch`
   - Authorization: Bearer access token
   - Body: form-data, add one row per image; every row's key must be `file`, and each row's type must be `File`.
   - Response: `{ "images": [{ "id": "...", "image_url": "..." }] }`.
2. Create a product with `POST /api/v1/products` and the uploaded IDs in the order to display them:

```json
{
  "title": "Yellow block heel sandals",
  "description": "Two-strap yellow block heel sandals.",
  "price_kes": 2500,
  "category_id": "<shoes-category-uuid>",
  "image_ids": ["<first-upload-id>", "<second-upload-id>"],
  "condition": "used"
}
```

3. The response includes `images`, ordered by `sort_order`, and `is_primary: true` for the cover.
4. Send the full ordered array to `PUT /api/v1/products/<product-id>/images/reorder` to change the gallery order and cover image.
5. Use `POST /api/v1/products/<product-id>/images` to add more images, or `DELETE /api/v1/products/<product-id>/images/<image-id>` to remove one.
6. Multi-image AI suggestions use `POST /api/v1/ai/listing-suggestions` with `{ "image_ids": ["...", "..."] }`. AI analysis accepts 1-8 images in one call and limits combined bytes to 24 MB. Existing `image_id` requests continue to work.

Every uploaded image must belong to the authenticated seller. A seller cannot attach another user's uploads to a product. The first image in the ordered IDs is the primary image. Product image URLs in the gallery are relative paths; the frontend resolves them against the API base URL.

**Development limitations:** the batch endpoint validates all incoming parts first, writes them to local disk, and records their metadata in one database transaction. Local disk storage is not durable enough for Railway production. Move to object storage and add image resizing/EXIF metadata removal before production. Use product-only photos with no private or location metadata when calling third-party AI APIs.
