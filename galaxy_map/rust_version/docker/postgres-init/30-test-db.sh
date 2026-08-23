#!/usr/bin/env bash
# A separate database for the integration suite, so `docker compose run tests`
# truncates a throwaway schema rather than the one holding a real vault.
set -euo pipefail

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname postgres <<-SQL
	create database parallax_test owner $POSTGRES_USER;
SQL

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname parallax_test \
     -f /docker-entrypoint-initdb.d/10-schema.sql
