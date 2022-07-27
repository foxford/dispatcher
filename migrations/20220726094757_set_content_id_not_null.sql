-- https://github.com/ankane/strong_migrations#setting-not-null-on-an-existing-column
ALTER TABLE class ADD CONSTRAINT class_content_id_null CHECK (class.content_id IS NOT NULL) NOT VALID;
ALTER TABLE class VALIDATE CONSTRAINT class_content_id_null;

ALTER TABLE class ALTER COLUMN content_id SET NOT NULL;

ALTER TABLE class DROP CONSTRAINT class_content_id_null;
