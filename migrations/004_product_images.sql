-- A product can have up to eight seller-uploaded images. The first image is
-- the primary/cover image; sort_order controls the buyer-facing gallery order.
CREATE TABLE product_images (
    product_id UUID NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    uploaded_image_id UUID NOT NULL REFERENCES uploaded_images(id) ON DELETE RESTRICT,
    sort_order SMALLINT NOT NULL CHECK (sort_order >= 0),
    is_primary BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (product_id, uploaded_image_id),
    UNIQUE (product_id, sort_order)
);

CREATE UNIQUE INDEX product_images_one_primary_per_product_idx
    ON product_images(product_id) WHERE is_primary;
CREATE INDEX product_images_uploaded_image_idx
    ON product_images(uploaded_image_id);

-- Backfill existing single-image listings when their image_url points to an
-- image uploaded by that same seller. Works with both /media/... and absolute
-- local URLs ending in /media/<generated filename>.
INSERT INTO product_images (product_id, uploaded_image_id, sort_order, is_primary)
SELECT p.id, ui.id, 0, TRUE
FROM products p
JOIN uploaded_images ui
  ON ui.owner_id = p.seller_id
 AND (
      p.image_url = ui.public_url
      OR RIGHT(p.image_url, LENGTH(ui.public_url)) = ui.public_url
      OR p.image_url LIKE '%' || ui.file_name
 )
WHERE p.image_url IS NOT NULL
ON CONFLICT (product_id, uploaded_image_id) DO NOTHING;
