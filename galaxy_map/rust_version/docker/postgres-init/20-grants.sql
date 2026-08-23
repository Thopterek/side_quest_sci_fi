-- Runs after 10-schema.sql, as the superuser created by initdb.
--
-- The application connects as a role that can read and write vault data but
-- cannot alter the schema. Migration is idempotent and already applied by the
-- image build, so the running app does not need DDL rights.

set search_path to parallax, public;

-- POSTGRES_USER is already a superuser; this is the least-privilege role the
-- app should actually use in anything other than a throwaway container.
do $$
begin
  if not exists (select 1 from pg_roles where rolname = 'parallax_app') then
    create role parallax_app login password 'parallax_app';
  end if;
end
$$;

grant usage on schema parallax to parallax_app;
grant select, insert, update, delete on all tables in schema parallax to parallax_app;
grant select on parallax.system_tags, parallax.system_links,
                parallax.link_edges, parallax.system_summary to parallax_app;
grant execute on function parallax.slugify(text) to parallax_app;

-- Tables created later by a migration inherit the same grants.
alter default privileges in schema parallax
  grant select, insert, update, delete on tables to parallax_app;
