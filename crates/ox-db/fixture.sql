-- Esquema usado pelos testes do PostgreSQL (`BRASA_TEST_PG*`). Rode num banco vazio, por exemplo:
--   docker run -d --name pg-teste -e POSTGRES_PASSWORD=teste123 -p 127.0.0.1:55432:5432 postgres:16-alpine
--   psql -h 127.0.0.1 -p 55432 -U postgres -c 'CREATE DATABASE fixture'
--   psql -h 127.0.0.1 -p 55432 -U postgres -d fixture -f crates/ox-db/fixture.sql
--   BRASA_TEST_PG=127.0.0.1:55432 BRASA_TEST_PG_PASSWORD=teste123 BRASA_TEST_PG_DB=fixture cargo test -p ox-db
CREATE TABLE pessoas (id serial PRIMARY KEY, nome text NOT NULL, idade int, nota numeric(5,2), ativo boolean DEFAULT true);
INSERT INTO pessoas (nome, idade, nota, ativo) SELECT 'Pessoa ' || i, 18 + i % 50, round((i * 1.37)::numeric, 2), i % 2 = 0 FROM generate_series(1, 250) i;
INSERT INTO pessoas (nome, idade, nota, ativo) VALUES ('Sem dados', NULL, NULL, NULL);
CREATE TABLE tags (a text, b text, rotulo text, PRIMARY KEY (a, b));
INSERT INTO tags VALUES ('x', 'y', 'um'), ('x', 'z', 'dois');
CREATE TABLE tags_pessoas (id serial PRIMARY KEY, pessoa_id int REFERENCES pessoas(id), rotulo text);
CREATE TABLE solta (x text, y int);
INSERT INTO solta VALUES ('a', 1), ('b', 2);
CREATE VIEW adultos AS SELECT nome FROM pessoas WHERE idade >= 30;
CREATE SCHEMA extra;
CREATE TABLE extra.log (id bigserial PRIMARY KEY, msg text);
CREATE INDEX idx_nome ON pessoas(nome);
CREATE FUNCTION carimbo() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN NEW.rotulo := coalesce(NEW.rotulo, 'sem rótulo'); RETURN NEW; END $$;
CREATE TRIGGER trg_carimbo BEFORE INSERT ON tags_pessoas FOR EACH ROW EXECUTE FUNCTION carimbo();
