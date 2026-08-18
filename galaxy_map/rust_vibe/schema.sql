-- Parallax vault schema.
--
-- The central rule of the application is that NASA owns the archive columns and
-- the operator owns the dossier columns, and a refresh must never overwrite the
-- latter. Here that stops being a convention in application code and becomes a
-- property of the schema: `upsert_system` below lists archive columns in its
-- DO UPDATE clause and nothing else, so there is no code path that can clobber
-- a dossier even by mistake.

create schema if not exists parallax;
set search_path to parallax, public;

-- ---------------------------------------------------------------- helpers --

-- Must match core::model::slug exactly. There is a test asserting it does.
create or replace function slugify(txt text) returns text
  language sql immutable strict parallel safe
as $$
  select trim(both '-' from regexp_replace(lower(txt), '[^a-z0-9]+', '-', 'g'))
$$;

-- ---------------------------------------------------------------- systems --

create table if not exists systems (
  id            text primary key,
  -- archive: replaced wholesale on refresh
  hostname      text             not null,
  ra            double precision not null,
  dec           double precision not null,
  dist_pc       double precision,
  teff          double precision,
  radius_sun    double precision,
  mass_sun      double precision,
  spectype      text,
  vmag          double precision,
  source        text             not null default 'nasa',
  origin        boolean          not null default false,
  -- dossier: written only by the operator, never by a refresh
  imperial_name text             not null default '',
  arm           text,
  population    text             not null default '',
  notes         text             not null default '',

  added_at      timestamptz      not null default now(),
  updated_at    timestamptz      not null default now(),
  -- Optimistic concurrency. Bumped by trigger on every UPDATE, so a client that
  -- read version N can refuse to overwrite version N+1 rather than silently
  -- clobbering whoever wrote it.
  version       integer          not null default 1,

  constraint systems_id_matches_hostname check (id = slugify(hostname)),
  constraint systems_source_known        check (source in ('nasa', 'seed', 'reference')),
  constraint systems_arm_known           check (
    arm is null or arm in ('local','perseus','sagittarius','scutum','norma','outer')),
  constraint systems_dec_in_range        check (dec between -90 and 90),
  constraint systems_ra_in_range         check (ra  between 0 and 360),
  constraint systems_dist_non_negative   check (dist_pc is null or dist_pc >= 0)
);

-- Only one system may be the coordinate origin.
create unique index if not exists systems_single_origin
  on systems ((origin)) where origin;

create index if not exists systems_dist_idx on systems (dist_pc nulls last);

-- Full-text search over catalog name and dossier, using the built-in 'simple'
-- configuration so no extension is required.
alter table systems drop column if exists search;
alter table systems add column search tsvector
  generated always as (
    to_tsvector('simple',
      hostname || ' ' || imperial_name || ' ' || population || ' ' ||
      notes || ' ' || coalesce(spectype, ''))
  ) stored;

create index if not exists systems_search_idx on systems using gin (search);

-- `create table if not exists` above does nothing to a table that already
-- exists, so a vault created by an earlier version would never gain these
-- columns. Adding them explicitly is what makes this file a migration rather
-- than just a definition. Both forms are idempotent, so fresh installs are
-- unaffected.
alter table systems add column if not exists version integer not null default 1;

create or replace function bump_version() returns trigger
  language plpgsql
as $$
begin
  new.version := old.version + 1;
  new.updated_at := now();
  return new;
end
$$;

drop trigger if exists systems_bump_version on systems;
create trigger systems_bump_version
  before update on systems
  for each row execute function bump_version();

-- Broadcast a change so other sessions can refresh without polling. The payload
-- is deliberately just an id: NOTIFY has an 8000-byte limit and a row can
-- exceed it, so listeners re-read what they care about.
create or replace function notify_change() returns trigger
  language plpgsql
as $$
declare
  row_json jsonb;
  changed  text;
begin
  -- Routed through jsonb rather than reading new.system_id directly: PL/pgSQL
  -- resolves record fields at runtime and raises "record new has no field" on a
  -- table that lacks the column, and coalesce does not prevent that. This one
  -- function therefore serves both `systems` (keyed `id`) and `planet_records`
  -- (keyed `system_id`).
  row_json := coalesce(to_jsonb(new), to_jsonb(old));
  changed  := coalesce(row_json ->> 'system_id', row_json ->> 'id');
  if changed is not null then
    perform pg_notify('parallax_changed', changed);
  end if;
  return coalesce(new, old);
end
$$;

drop trigger if exists systems_notify on systems;
create trigger systems_notify
  after insert or update or delete on systems
  for each row execute function notify_change();

drop trigger if exists planet_records_notify on planet_records;

-- ---------------------------------------------------------------- planets --

-- Archive-owned. A refresh deletes and reinserts every row for a host.
create table if not exists planets (
  system_id     text not null references systems (id) on delete cascade,
  name          text not null,
  ordinal       int  not null,
  orbsmax       double precision,
  orbper        double precision,
  rade          double precision,
  bmasse        double precision,
  eqt           double precision,
  orbeccen      double precision,
  disc_year     bigint,
  disc_method   text,
  disc_facility text,
  primary key (system_id, name),
  constraint planets_orbsmax_positive check (orbsmax is null or orbsmax  > 0),
  constraint planets_orbper_positive  check (orbper  is null or orbper   > 0),
  constraint planets_eccen_in_range   check (orbeccen is null or orbeccen between 0 and 1)
);

create index if not exists planets_system_idx on planets (system_id, ordinal);

-- ------------------------------------------------------- planet dossiers --

-- Deliberately NOT a foreign key onto `planets`.
--
-- A refresh deletes every planet row for a host and reinserts it. If this table
-- cascaded from `planets`, refreshing would silently destroy the operator's
-- notes — the exact failure the whole archive/dossier split exists to prevent.
-- It therefore hangs off `systems`, and survives planet churn. A dossier for a
-- planet the archive later retracts is kept, not dropped: that is the operator's
-- data to delete.
create table if not exists planet_records (
  system_id     text not null references systems (id) on delete cascade,
  planet_name   text not null,
  imperial_name text not null default '',
  population    text not null default '',
  continents    text not null default '',
  notes         text not null default '',
  updated_at    timestamptz not null default now(),
  version       integer not null default 1,
  primary key (system_id, planet_name)
);

alter table planet_records add column if not exists version integer not null default 1;

drop trigger if exists planet_records_bump_version on planet_records;
create trigger planet_records_bump_version
  before update on planet_records
  for each row execute function bump_version();

create trigger planet_records_notify
  after insert or update or delete on planet_records
  for each row execute function notify_change();

-- --------------------------------------------------------------- settings --

-- Per user, not per database.
--
-- This was a singleton row, which is correct for one operator on one desktop and
-- actively wrong the moment there are two: whoever clicked last decided what
-- everybody else had selected. The vault is shared; the view onto it is not.
create table if not exists user_settings (
  user_id       text primary key,
  selected      text references systems (id) on delete set null,
  compare       text references systems (id) on delete set null,
  focus_planet  text,
  updated_at    timestamptz not null default now(),
  constraint user_settings_id_not_blank check (btrim(user_id) <> '')
);

-- Carry a single-user vault forward, then retire the old table.
do $$
begin
  if exists (select 1 from information_schema.tables
              where table_schema = 'parallax' and table_name = 'settings') then
    insert into user_settings (user_id, selected, compare, focus_planet)
      select 'local', selected, compare, focus_planet from settings
      on conflict (user_id) do nothing;
    drop table settings;
  end if;
end
$$;

insert into user_settings (user_id) values ('local') on conflict do nothing;

-- ------------------------------------------------------------------ views --

-- Tags, from system notes and planet notes alike. Derived rather than stored,
-- so a tag can never drift out of sync with the note that produced it.
create or replace view system_tags as
  select id as system_id, lower(m[1]) as tag
    from systems, regexp_matches(notes, '#([A-Za-z0-9_-]+)', 'g') as m
  union
  select system_id, lower(m[1]) as tag
    from planet_records, regexp_matches(notes, '#([A-Za-z0-9_-]+)', 'g') as m;

-- [[Wikilinks]], resolved to system ids. Unresolved and self links are dropped
-- here rather than in application code.
create or replace view system_links as
  with raw as (
    select id as source, slugify(m[1]) as target
      from systems, regexp_matches(notes, '\[\[([^\]]+)\]\]', 'g') as m
    union
    select system_id as source, slugify(m[1]) as target
      from planet_records, regexp_matches(notes, '\[\[([^\]]+)\]\]', 'g') as m
  )
  select distinct raw.source, raw.target
    from raw
    join systems t on t.id = raw.target
   where raw.source <> raw.target;

-- Undirected edge list for the cube, each pair exactly once.
create or replace view link_edges as
  select distinct least(source, target) as a, greatest(source, target) as b
    from system_links;

-- What the vault list needs, without pulling planet rows.
create or replace view system_summary as
  select s.id,
         s.hostname,
         coalesce(nullif(btrim(s.imperial_name), ''), s.hostname) as display_name,
         s.dist_pc,
         s.spectype,
         s.arm,
         s.origin,
         s.source,
         (select count(*) from planets p where p.system_id = s.id) as planet_count
    from systems s;
