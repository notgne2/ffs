CREATE TABLE points (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "name" VARCHAR NOT NULL,
  "path" VARCHAR,
  "hash" VARCHAR UNIQUE NOT NULL
);