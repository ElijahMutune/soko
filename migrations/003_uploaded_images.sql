CREATE TABLE uploaded_images (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    storage_provider VARCHAR(20) NOT NULL DEFAULT 'local'
        CHECK (storage_provider IN ('local')),
    file_name VARCHAR(120) NOT NULL UNIQUE,
    public_url TEXT NOT NULL,
    mime_type VARCHAR(40) NOT NULL
        CHECK (mime_type IN ('image/jpeg', 'image/png', 'image/webp')),
    byte_size BIGINT NOT NULL CHECK (byte_size > 0 AND byte_size <= 8388608),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX uploaded_images_owner_created_idx
    ON uploaded_images(owner_id, created_at DESC);


-- Audit and cost-control ledger: at most 10 AI listing attempts per seller per hour.
CREATE TABLE ai_listing_requests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    uploaded_image_id UUID REFERENCES uploaded_images(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX ai_listing_requests_owner_created_idx
    ON ai_listing_requests(owner_id, created_at DESC);
