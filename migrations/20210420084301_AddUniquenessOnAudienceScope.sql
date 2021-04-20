-- Add migration script here
CREATE UNIQUE INDEX uniq_audience_scope ON class (audience, scope);
