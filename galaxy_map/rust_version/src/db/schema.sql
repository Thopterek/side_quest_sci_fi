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
  set search_path = parallax, pg_temp
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

-- ------------------------------------------------------------------ grants --

-- Capability links, in place of accounts.
--
-- The host mints a link; whoever holds it sees exactly one stage of the vault.
-- There is no registration, no password, no identity — the link *is* the
-- credential, and what it can see is a property of the grant rather than of a
-- person. A group shares one link and therefore one view.
--
-- The token itself is never stored. Only its SHA-256 lands here, so a database
-- dump does not hand out working links, and the plaintext exists exactly once:
-- in the response that created it.
create table if not exists grants (
  id            text primary key,
  -- Hex SHA-256 of the bearer token. Empty string for the anonymous grant,
  -- which is reached by presenting no token at all.
  token_hash    text not null,
  label         text not null default '',
  -- read  : may view its stage
  -- write : may also edit dossiers and add systems within its stage
  -- admin : may see everything and mint further grants
  capability    text not null default 'read',
  -- all     : the whole vault
  -- systems : an explicit list
  -- tag     : everything carrying a tag, so a stage can be curated from notes
  scope_kind    text not null default 'systems',
  scope_tag     text,
  scope_systems text[] not null default '{}',
  created_at    timestamptz not null default now(),
  expires_at    timestamptz,
  revoked_at    timestamptz,
  last_used_at  timestamptz,
  use_count     bigint not null default 0,

  constraint grants_capability_known check (capability in ('read','write','admin')),
  constraint grants_scope_kind_known check (scope_kind in ('all','systems','tag')),
  -- A tag-scoped grant without a tag would silently show nothing, which reads
  -- as a bug rather than as a deliberately empty stage.
  constraint grants_tag_scope_has_tag check (scope_kind <> 'tag' or btrim(coalesce(scope_tag,'')) <> ''),
  -- Only 'all' may be paired with admin; an admin restricted to a subset is a
  -- contradiction that would be easy to create by mistake.
  constraint grants_admin_sees_all check (capability <> 'admin' or scope_kind = 'all')
);

-- One row per live token. The anonymous grant is exempt: it has no token.
create unique index if not exists grants_token_hash_key
  on grants (token_hash) where token_hash <> '';

create index if not exists grants_live_idx
  on grants (revoked_at, expires_at);

-- What a first-time visitor sees: Sol, and nothing else.
--
-- Editable by the host — widen `scope_systems`, or point it at a tag, to change
-- what the public stage contains without touching any code.
insert into grants (id, token_hash, label, capability, scope_kind, scope_systems)
  values ('anonymous', '', 'Anyone with the address', 'read', 'systems', array['sol'])
  on conflict (id) do nothing;

-- Resolve a bearer token to a live grant, or nothing.
--
-- Expiry and revocation are checked here rather than in application code, so
-- every query that joins through this function gets them for free.
create or replace function live_grant(p_token_hash text)
  returns setof parallax.grants
  language sql stable
  -- Pinned, and every reference below is schema-qualified. A security-relevant
  -- function that resolves names through the caller's search_path can be made
  -- to read a table the caller controls instead of this one.
  set search_path = parallax, pg_temp
as $$
  select * from parallax.grants
   where token_hash = p_token_hash
     and token_hash <> ''
     and revoked_at is null
     and (expires_at is null or expires_at > now())
$$;

-- The set of systems a grant may see.
--
-- This is the single enforcement point. Every read path joins through it, so a
-- stage cannot leak by someone forgetting a WHERE clause in one query.
--
-- Sol is always included: it is the origin of the coordinate system, every
-- distance in the application is measured from it, and a cube without it has no
-- anchor. It is a landmark, not a secret.
create or replace function visible_systems(p_grant_id text)
  returns table (system_id text)
  language sql stable
  -- Same reasoning as live_grant, and more sharply: this function *is* the
  -- access boundary. It must not be redirectable by a connection setting.
  set search_path = parallax, pg_temp
as $$
  select s.id
    from parallax.systems s
    join parallax.grants g on g.id = p_grant_id
   where g.revoked_at is null
     and (g.expires_at is null or g.expires_at > now())
     and (
          g.scope_kind = 'all'
       or s.origin
       or (g.scope_kind = 'systems' and s.id = any (g.scope_systems))
       or (g.scope_kind = 'tag' and exists (
             select 1 from parallax.system_tags t
              where t.system_id = s.id and t.tag = lower(g.scope_tag)))
     )
$$;
