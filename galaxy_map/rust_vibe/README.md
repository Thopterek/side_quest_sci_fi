# Parallax

A vault for star systems. Obsidian's shape, applied to the NASA Exoplanet Archive.

| Obsidian | Parallax |
| --- | --- |
| Vault | Your saved systems, persisted between runs |
| Note | A star system, with a dossier per planet |
| Graph view | The cube — real 3D positions, live orreries |
| `[[Wikilinks]]` | Type `[[TRAPPIST-1]]` in a note, an edge appears in the cube |
| `#tags` | Type `#habitable-zone`, it becomes a filter chip |

Data comes from the **Planetary Systems Composite Parameters** table (`pscomppars`)
via the archive's TAP service — the same parameters behind NASA's own catalog pages.

## Running

The vault lives in PostgreSQL.

```sh
createdb parallax
export PARALLAX_DATABASE_URL="host=localhost user=postgres dbname=parallax"
cargo run --release
```

The schema is created and migrated on first launch, and an empty database is
seeded with the shipped local neighbourhood. If the database cannot be reached
the app says so and runs from memory rather than refusing to start.

For the browser:

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
trunk serve --release        # then open the printed localhost URL
```

Requires **Rust 1.81 or newer** (egui 0.31's minimum). On Debian/Ubuntu the
build also needs X11, Wayland and GL headers:

```sh
sudo apt install pkg-config cmake libgl1-mesa-dev libx11-dev libxcursor-dev \
                 libxrandr-dev libxi-dev libxkbcommon-dev libwayland-dev
```

## Architecture

Three tiers. The split exists because the single-binary design was
single-operator-correct and multi-operator-broken.

```
  parallax (face)          parallax-server            PostgreSQL
  egui, no credentials  ->  axum, owns the pool  ->  the vault
       HTTP + SSE               field patches
```

Only the server holds credentials. The face is not even compiled with the `db`
feature, so it has no Postgres driver linked in — verify with
`cargo build --no-default-features --features gui,client`, which succeeds, and
which is exactly what `docker/Dockerfile.app` runs.

### Why it had to be split

With every client talking to PostgreSQL directly:

* **Dossier edits destroyed each other.** `save_record` wrote all four dossier
  columns from a stale in-memory copy, so two operators editing *different*
  fields of the same system lost one edit entirely. Not a rare race — 
  deterministic. `tests/concurrency.rs` keeps the failing scenario as
  `whole_row_writes_lose_a_concurrent_edit` so the regression stays documented.
* **`settings` was a global singleton**, so two operators shared one cursor and
  fought over the selection. It is now `user_settings`, keyed per operator.
* **Nothing propagated.** A client never learned of another's writes until
  restart.
* **One connection per client**, against a default `max_connections` of 100.

The server fixes each: writes are field-level patches, so disjoint edits merge;
`version` gives optimistic concurrency for same-field conflicts; a PostgreSQL
`LISTEN` is fanned out to clients over SSE at `/events`; and one pool serves
everyone.

Verified end to end against a live server — two simultaneous `PATCH`es from
different operators, one setting `imperial_name` and `arm`, the other
`population`, all three fields survive.

### API

```
GET    /health
GET    /vault?user=…
PUT    /systems/{id}                      refresh from the archive
DELETE /systems/{id}
PATCH  /systems/{id}/record               field-level, merges
PATCH  /systems/{id}/planets/{p}/record
PUT    /settings                          per operator
POST   /seed
GET    /events                            SSE change feed
```

## Docker

```sh
docker compose up -d db server            # backend only; run the face natively
docker compose --profile app up --build   # everything, including the GUI
docker compose --profile multi up         # two faces, to watch edits merge
docker compose --profile tools run --rm tests
docker compose --profile tools run --rm psql
```

Three images, one per tier:

| file | contents | carries |
| --- | --- | --- |
| `docker/Dockerfile.postgres` | schema baked into the init directory | the vault |
| `docker/Dockerfile.server` | `parallax-server`, no GL, no X11 | credentials |
| `docker/Dockerfile.app` | the face, built without `db` | nothing sensitive |

The face is behind a profile because it is a GUI, not a service, and needs a
display socket from the host. On Linux, `xhost +local:docker` first. macOS and
Windows have no X11 socket to share, so run the backend in Docker and the face
on the host:

```sh
PARALLAX_SERVER_URL=http://localhost:8080 \
  cargo run --release --no-default-features --features gui,client
```

Details worth knowing. Both Rust images compile the dependency graph against
stub sources before copying `src/`, so editing code does not rebuild axum or
eframe. The database tuning is applied with an `include` appended during init
rather than `-c` flags, because the official entrypoint restarts the server
after init scripts run. The database healthcheck does not use `pg_isready` —
that reports success while init scripts are still executing, so it asks for the
`link_edges` view instead. And the server's healthcheck is the binary probing
its own `/health` over plain TCP, because that image ships no shell and no curl
on purpose.

## The database

The archive/dossier split is the central rule of the application: NASA owns some
columns, the operator owns others, and a refresh must never overwrite the second
kind. In the schema that stops being a convention and becomes a property of the
tables.

`upsert_system` — the refresh path — lists archive columns in its `ON CONFLICT
DO UPDATE` clause and nothing else. There is no code path, deliberate or
accidental, that can clobber a dossier.

`planet_records` is deliberately **not** a foreign key onto `planets`. A refresh
deletes and reinserts every planet row for a host; had the dossier cascaded from
`planets`, refreshing would have silently destroyed exactly what the split exists
to protect. It hangs off `systems` instead, so a note survives its planet being
rewritten — and survives the archive retracting that planet altogether, because
that is the operator's data to delete, not NASA's.

Tags and `[[wikilinks]]` are **views**, not stored columns, so they can never
drift out of sync with the notes that produced them. `slugify()` in SQL and
`slug()` in Rust are two implementations of one rule, and a test asserts they
agree on every seed hostname plus a set of awkward cases.

Search uses a generated `tsvector` column with a GIN index, using the built-in
`simple` configuration so no extension is required.

Writes go through a worker thread. Dossier fields are bound straight to text
boxes, so a naive implementation would issue one `UPDATE` per keystroke; the
worker coalesces instead, keyed by target, and flushes 400 ms after typing stops
with a 2 s ceiling so a fast typist is never starved. A measured test asserts
forty keystrokes collapse to about one write.

## Testing

The astronomy is deliberately separated from the rendering. Everything in
`src/core` — coordinate transforms, habitable zones, the camera, orrery layout,
vault semantics, the archive parser — has no egui dependency and is unit tested:

```sh
cargo test --no-default-features            # core only: no database, no window

PARALLAX_TEST_DATABASE_URL="host=localhost user=postgres dbname=parallax" \
  cargo test --features gui,db,client,server -- --test-threads=1   # 147 tests
```

The PostgreSQL tests skip themselves when that variable is unset, so a clean
checkout stays green.

Those tests are not decoration. They pin real results: that the position vector's
length reproduces the catalogued parallax, that ε Eridani and τ Ceti come out
5.4 ly apart, that GJ 1061 d lands inside its star's habitable zone and b does
not, that Earth lands inside the Sun's, that orbit rings never cross at any size
or scale, and that the camera pivot stays screen-centred through every rotation.

The database tests pin the invariants above: that a refresh updates `dist_pc` and
leaves an imperial name alone, that planet dossiers survive planet rows being
replaced, that the `system_tags` and `link_edges` views agree with the Rust
implementations, and that the schema rejects a declination of 999, an eighth
galactic arm, a second coordinate origin and a negative orbital period.

## Layout

```
src/core/            no egui, no I/O, fully tested
  astro.rs           coordinates, luminosity, habitable zones, colour, scaling
  camera.rs          pivot-following projection and easing
  model.rs           System / Planet / Record / Arm; the archive-vs-dossier rule
  orrery.rs          the reusable system view, as geometry rather than pixels
  vault.rs           the collection: filter, tags, wikilinks, extent, measurement
  nasa.rs            ADQL construction and row parsing
  seed.rs            the shipped local neighbourhood
src/ui/              egui rendering
  system_view.rs     paints an orrery into any rectangle or any plane
  cube.rs            the 3D map
  vault_panel.rs     left column, including archive search
  record_panel.rs    right column: Archive · NASA, then Dossier · yours
  theme.rs           plate / negative palettes
  http.rs            the only module that performs I/O
src/db/              PostgreSQL, native only
  schema.sql         tables, constraints, views; the invariant lives here
  pg.rs              VaultStore over the sync driver
  worker.rs          background thread with write coalescing
src/app.rs           layout, clock, honest-scale strip
```

`core::store::VaultStore` is the seam. `PgStore` implements it for native
builds; `MemoryStore` implements it for wasm, for tests, and as the fallback
when a database is unreachable. Both honour the same dossier rule, and the same
assertions are run against each.

## Performance

Everything the render loop reads is recomputed sixty times a second, and several
of those reads were doing real work: four trig calls per system for its position,
a fresh lowercase haystack string per system for the filter, an O(n²) wikilink
resolution allocating per candidate edge, and a `BTreeSet` rebuild for tags.

`core::index::VaultIndex` caches all of it behind the vault's revision counter,
so a frame that changes nothing costs nothing. Measured with
`cargo run --release --example frame_bench`:

| systems | uncached/frame | cached/frame | speedup |
| ---: | ---: | ---: | ---: |
| 13 | 12.4 µs | 0.1 µs | 90× |
| 63 | 41.3 µs | 0.3 µs | 123× |
| 213 | 163.8 µs | 1.0 µs | 165× |
| 513 | 688.3 µs | 2.4 µs | 292× |

At 513 systems that is 4% of a 60 fps frame budget reclaimed from bookkeeping.
Two smaller wins alongside it: orbit ring points now build into a reusable
thread-local buffer rather than allocating a `Vec` per ring per frame (about
sixty-five allocations), and the cube's twelve edges are a compile-time constant
rather than an O(n²) vertex comparison rediscovering them each frame.

### The reusable system view

`core::orrery::layout` turns a system into rings and dots at any radius;
`ui::system_view::paint_into_plane` draws that into any rectangle *or any plane*.
Both are used by the 44 px vault thumbnails, the 92 px hover card, the 300 px
record-panel orrery, and every live orrery inside the cube. Detail tiers derive
from radius, so there is one implementation rather than four that drift apart.

Inside the cube the camera supplies a foreshortened basis, so orbits lie flat in
the equatorial plane and tilt with the view rather than always facing you.

## On scale

A single scale cannot hold a galaxy and a planet at once; the gap is about
seventeen orders of magnitude. Parallax uses four and labels which one you are
looking at everywhere it matters.

1. **Between systems — true.** Real RA/Dec/distance to Cartesian parsecs, Sun at
   the origin. Shift-click any two stars: the number is the real 3D separation.
   Only the glyph *size* is symbolic (apparent magnitude) and colour (temperature,
   or the arm you assigned).
2. **Distance compression — optional.** Add something at 200 pc and the local
   group truthfully collapses to a dot. `distance: log` replaces radius `r` with
   `ln(1 + r)`, keeping every bearing exact while pulling far systems into frame.
   Measurements stay true regardless; the strip flags the mode.
3. **Orbits — compressed.** Log radius by default so a 0.02 AU and a 30 AU orbit
   share one frame. Press **true** and watch the inner planets fall in — the size
   of that collapse is the size of the distortion. Compact systems barely move,
   which is itself the honest answer.
4. **Bodies — always symbolic.** Never to scale in any mode. The strip carries a
   live count of the exaggeration.

Planets move at true relative rates from one shared clock, so a TRAPPIST-1 planet
really does lap Neptune thousands of times.

## Arms

Every system carries a galactic arm you assign, and `colour: arm` recolours the
cube by it. Everything within about a kiloparsec of Sol genuinely is in the
Orion–Cygnus arm, so the seed catalog is set that way as fact; the other five
options exist for systems of your own invention.

## Fonts

The design is drawn for IBM Plex Sans Condensed and IBM Plex Mono; the binary
ships with egui's defaults so it has no asset dependency. To use Plex, drop the
`.ttf` files in `assets/` and register them in `Theme::apply` with
`egui::FontDefinitions` and `include_bytes!`.

## Verification status

Everything here has been compiled with `rustc 1.91`, run, and tested:

* 116 unit, 16 concurrency and 15 PostgreSQL integration tests pass — 147 in
  total, with no warnings.
* Each tier builds in isolation: the face without `db`, the server without
  `gui`. That is what makes the image split real rather than cosmetic.
* The concurrent-merge behaviour was confirmed against a running server, not
  only in tests.
* The release binary has been run headless under Xvfb against an empty database.
  It migrates the schema, seeds thirteen systems and forty-three planets, and
  renders. On restart it loads rather than reseeding, and a dossier written
  between runs survives.

The Docker images are the exception: Docker was not available where this was
built, so the layer ordering and compose wiring are reasoned rather than
executed. The SQL half — init scripts, grants, the tuning include, the schema —
was verified by replaying the container's exact init sequence against a real
cluster.

## Licence

MIT OR Apache-2.0. Astronomical data courtesy of the NASA Exoplanet Archive,
operated by Caltech under contract with NASA.
