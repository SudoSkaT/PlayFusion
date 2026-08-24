-- 0004: letras sincronizadas (formato LRC) en la caché de letras.
--
-- Se añade un flag `synced` a `lyrics_cache` para distinguir las letras planas
-- (YouTube Music) de las LRC con timestamps (LRCLIB) usadas para el karaoke.

ALTER TABLE lyrics_cache ADD COLUMN synced INTEGER NOT NULL DEFAULT 0;