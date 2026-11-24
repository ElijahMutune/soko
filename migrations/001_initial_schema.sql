CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE categories (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(80) NOT NULL UNIQUE,
    slug VARCHAR(100) NOT NULL UNIQUE,
    sort_order INTEGER NOT NULL DEFAULT 0,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email TEXT UNIQUE,
    phone_e164 TEXT UNIQUE,
    password_hash TEXT,
    public_username VARCHAR(40) NOT NULL UNIQUE,
    role VARCHAR(20) NOT NULL DEFAULT 'buyer_seller'
        CHECK (role IN ('buyer_seller', 'admin', 'support')),
    account_status VARCHAR(20) NOT NULL DEFAULT 'pending'
        CHECK (account_status IN ('pending', 'active', 'suspended', 'closed')),
    email_verified_at TIMESTAMPTZ,
    phone_verified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (email IS NOT NULL OR phone_e164 IS NOT NULL)
);

CREATE TABLE products (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    seller_id UUID REFERENCES users(id) ON DELETE SET NULL,
    title VARCHAR(120) NOT NULL,
    description TEXT,
    price_kes BIGINT NOT NULL CHECK (price_kes > 0),
    category_id UUID REFERENCES categories(id),
    image_url TEXT,
    condition VARCHAR(30) NOT NULL DEFAULT 'used',
    status VARCHAR(20) NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft', 'active', 'reserved', 'sold', 'removed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX products_active_created_idx ON products(status, created_at DESC);
CREATE INDEX products_category_idx ON products(category_id);
CREATE INDEX products_seller_idx ON products(seller_id);

INSERT INTO categories (name, slug, sort_order) VALUES
('Clothing', 'clothing', 1),
('Shoes', 'shoes', 2),
('Electronics', 'electronics', 3),
('Home & Living', 'home-living', 4),
('Beauty & Personal Care', 'beauty-personal-care', 5),
('Bags & Accessories', 'bags-accessories', 6),
('Sports & Outdoors', 'sports-outdoors', 7),
('Books & Stationery', 'books-stationery', 8),
('Other', 'other', 99)
ON CONFLICT (slug) DO NOTHING;
