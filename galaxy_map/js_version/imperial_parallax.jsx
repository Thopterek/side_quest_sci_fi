import { useState, useEffect, useRef, useMemo, useCallback } from "react";

/* ============================================================================
   PARALLAX — a vault for star systems

   Vault (left)   : saved systems + the NASA archive search
   Cube (centre)  : every saved system, live, in real 3D position
   Record (right) : Archive facts from NASA, and your own dossier on top

   Positions: real RA / Dec / distance -> equatorial Cartesian, in parsecs.
   Data: NASA Exoplanet Archive, pscomppars, via the TAP service.
   ========================================================================== */

/* ---------------------------------------------------------------- constants */
const PC_IN_AU = 206264.806;
const PC_IN_LY = 3.2615638;
const RSUN_IN_AU = 0.00465047;
const REARTH_IN_AU = 4.26352e-5;
const C_KMS = 299792.458;
const VOYAGER_KMS = 17.0;

const TAP = "https://exoplanetarchive.ipac.caltech.edu/TAP/sync";
const COLS = [
	"pl_name", "hostname", "sy_snum", "sy_pnum", "ra", "dec", "sy_dist",
	"pl_orbsmax", "pl_orbper", "pl_rade", "pl_bmasse", "pl_eqt", "pl_orbeccen",
	"st_teff", "st_rad", "st_mass", "st_spectype", "sy_vmag",
	"discoverymethod", "disc_year", "disc_facility",
].join(",");

const slug = (s) => String(s).toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");

/* ------------------------------------------------------------------- arms */
/* Everything inside ~1 kpc of Sol is genuinely in the Orion–Cygnus arm.
   The rest of this list is here so you can place systems of your own. */
const ARMS = [
	{ id: "local", name: "Orion–Cygnus", sub: "Local Arm", plate: "#2E7D6B", neg: "#63C7B0" },
	{ id: "perseus", name: "Perseus", sub: "outward", plate: "#8E3218", neg: "#E38B6C" },
	{ id: "sagittarius", name: "Sagittarius–Carina", sub: "inward", plate: "#6B4C9A", neg: "#AE93DD" },
	{ id: "scutum", name: "Scutum–Centaurus", sub: "inner", plate: "#9C6B12", neg: "#E3B85E" },
	{ id: "norma", name: "Norma", sub: "innermost", plate: "#3F6E1F", neg: "#95CF64" },
	{ id: "outer", name: "Outer", sub: "rim", plate: "#1C5C9E", neg: "#74B2EA" },
];
const armOf = (id) => ARMS.find((a) => a.id === id) || null;

/* ------------------------------------------------------------------ records */
const emptyRecord = () => ({ imperialName: "", arm: "", population: "", notes: "" });
const emptyPlanetRecord = () => ({ imperialName: "", population: "", continents: "", notes: "" });
const rec = (s) => ({ ...emptyRecord(), ...(s?.record || {}) });
const prec = (s, name) => ({ ...emptyPlanetRecord(), ...((s?.precords || {})[name] || {}) });

/* ------------------------------------------------------------- seed catalog */
const P = (pl_name, pl_orbsmax, pl_orbper, pl_rade, pl_bmasse, pl_orbeccen, disc_year, discoverymethod) =>
	({ pl_name, pl_orbsmax, pl_orbper, pl_rade, pl_bmasse, pl_orbeccen, disc_year, discoverymethod });

const SEED = [
	{
		hostname: "Sol", ra: 0, dec: 0, sy_dist: 0, st_teff: 5772, st_rad: 1, st_mass: 1,
		st_spectype: "G2 V", sy_vmag: -26.74, origin: true, source: "reference",
		record: {
			imperialName: "Sol Prime", arm: "local", population: "8.2 billion",
			notes: "Home. The only system here measured from the inside.\n\nEvery orbit you already recognise lives in this one, so it works as a ruler for everything else in the vault.\n\n#reference",
		},
		precords: {
			Earth: {
				imperialName: "Terra", population: "8.2 billion",
				continents: "Africa, Antarctica, Asia, Australia, Europe, North America, South America",
				notes: "The only confirmed biosphere in the catalog. Everything else in this vault is inference.",
			},
		},
		planets: [
			P("Mercury", 0.3871, 87.97, 0.383, 0.0553, 0.2056, null, "Direct Imaging"),
			P("Venus", 0.7233, 224.70, 0.949, 0.815, 0.0068, null, "Direct Imaging"),
			P("Earth", 1.0000, 365.26, 1.000, 1.000, 0.0167, null, "Direct Imaging"),
			P("Mars", 1.5237, 686.98, 0.532, 0.107, 0.0934, null, "Direct Imaging"),
			P("Jupiter", 5.2034, 4332.6, 11.209, 317.8, 0.0484, null, "Direct Imaging"),
			P("Saturn", 9.5371, 10759, 9.449, 95.16, 0.0539, null, "Direct Imaging"),
			P("Uranus", 19.191, 30685, 4.007, 14.54, 0.0473, null, "Direct Imaging"),
			P("Neptune", 30.069, 60190, 3.883, 17.15, 0.0086, null, "Direct Imaging"),
		],
	},
	{
		hostname: "Proxima Centauri", ra: 217.4289, dec: -62.6795, sy_dist: 1.301,
		st_teff: 3042, st_rad: 0.1542, st_mass: 0.1221, st_spectype: "M5.5 V", sy_vmag: 11.13,
		source: "seed", record: { arm: "local", notes: "Closest star to Sol — everything else here is at least half again as far.\n\nProxima b sits in the habitable zone, but the star flares violently. Compare with [[Ross 128]], which is quiet.\n\n#nearest #habitable-zone" },
		planets: [
			P("Proxima Cen d", 0.02885, 5.122, 0.81, 0.26, 0.04, 2022, "Radial Velocity"),
			P("Proxima Cen b", 0.04857, 11.186, 1.03, 1.07, 0.02, 2016, "Radial Velocity"),
			P("Proxima Cen c", 1.489, 1928, 12.3, 7.0, 0.04, 2019, "Radial Velocity"),
		],
	},
	{
		hostname: "GJ 699", ra: 269.4521, dec: 4.6933, sy_dist: 1.828,
		st_teff: 3195, st_rad: 0.187, st_mass: 0.162, st_spectype: "M4 V", sy_vmag: 9.51,
		source: "seed", record: { arm: "local", notes: "Barnard's Star. Fastest proper motion of any star — it visibly crosses the sky within a human lifetime.\n\nFour sub-Earth planets, all far too hot.\n\n#nearest" },
		planets: [
			P("GJ 699 d", 0.0188, 2.340, 0.63, 0.19, 0, 2025, "Radial Velocity"),
			P("GJ 699 b", 0.0229, 3.154, 0.73, 0.30, 0, 2024, "Radial Velocity"),
			P("GJ 699 c", 0.0274, 4.124, 0.76, 0.34, 0, 2025, "Radial Velocity"),
			P("GJ 699 e", 0.0381, 6.739, 0.70, 0.26, 0, 2025, "Radial Velocity"),
		],
	},
	{
		hostname: "GJ 411", ra: 165.8341, dec: 35.9699, sy_dist: 2.546,
		st_teff: 3601, st_rad: 0.392, st_mass: 0.389, st_spectype: "M2 V", sy_vmag: 7.52,
		source: "seed", record: { arm: "local", notes: "Lalande 21185. Brightest red dwarf in the northern sky.\n\nWide spread — 0.08 AU out to 2.9 AU. Good system for seeing what log compression is doing.\n\n#nearest" },
		planets: [
			P("GJ 411 b", 0.079, 12.95, 1.36, 2.69, 0.06, 2019, "Radial Velocity"),
			P("GJ 411 c", 2.94, 2946, 12.0, 13.6, 0.14, 2021, "Radial Velocity"),
		],
	},
	{
		hostname: "eps Eri", ra: 53.2327, dec: -9.4583, sy_dist: 3.216,
		st_teff: 5084, st_rad: 0.735, st_mass: 0.82, st_spectype: "K2 V", sy_vmag: 3.73,
		source: "seed", record: { arm: "local", notes: "Epsilon Eridani. Young, active, with a debris disc. Naked-eye visible.\n\nOne of the two original targets of Project Ozma in 1960.\n\n#naked-eye #debris-disc" },
		planets: [P("eps Eri b", 3.53, 2692, 12.4, 254.0, 0.07, 2000, "Radial Velocity")],
	},
	{
		hostname: "GJ 887", ra: 346.4667, dec: -35.8533, sy_dist: 3.290,
		st_teff: 3688, st_rad: 0.47, st_mass: 0.489, st_spectype: "M2 V", sy_vmag: 7.34,
		source: "seed", record: { arm: "local", notes: "Unusually quiet for an M dwarf — few starspots, little flaring. That makes it a good place to look for surviving atmospheres.\n\n#quiet-star" },
		planets: [
			P("GJ 887 b", 0.0681, 9.262, 1.9, 4.2, 0, 2020, "Radial Velocity"),
			P("GJ 887 c", 0.1194, 21.789, 2.2, 7.6, 0, 2020, "Radial Velocity"),
		],
	},
	{
		hostname: "GJ 367", ra: 145.2864, dec: -45.7757, sy_dist: 9.413,
		st_teff: 3522, st_rad: 0.454, st_mass: 0.455, st_spectype: "M1 V", sy_vmag: 10.15,
		source: "seed", record: { arm: "local", notes: "GJ 367 b is an ultra-short-period iron planet — a bare metallic core on an eight-hour year.\n\n#extreme" },
		planets: [
			P("GJ 367 b", 0.00709, 0.3219, 0.699, 0.633, 0, 2021, "Transit"),
			P("GJ 367 c", 0.0596, 11.53, 1.5, 4.13, 0, 2023, "Radial Velocity"),
			P("GJ 367 d", 0.0982, 34.37, 2.0, 6.03, 0, 2023, "Radial Velocity"),
		],
	},
	{
		hostname: "Ross 128", ra: 176.9375, dec: 0.8003, sy_dist: 3.375,
		st_teff: 3192, st_rad: 0.1967, st_mass: 0.168, st_spectype: "M4 V", sy_vmag: 11.13,
		source: "seed", record: { arm: "local", notes: "Quiet M dwarf, temperate planet. The calm alternative to [[Proxima Centauri]].\n\n#habitable-zone #quiet-star" },
		planets: [P("Ross 128 b", 0.0496, 9.866, 1.11, 1.40, 0.12, 2017, "Radial Velocity")],
	},
	{
		hostname: "GJ 1061", ra: 53.9955, dec: -44.5119, sy_dist: 3.670,
		st_teff: 2953, st_rad: 0.156, st_mass: 0.120, st_spectype: "M5.5 V", sy_vmag: 13.03,
		source: "seed",
		record: {
			arm: "local",
			notes: "Three super-Earths around a very small, very faint M dwarf.\n\nd sits inside the conservative habitable zone; c is right on the inner edge. The whole system is smaller than Mercury's orbit.\n\nCompare with [[TRAPPIST-1]] — same idea, tighter packing.\n\n#habitable-zone #compact",
		},
		planets: [
			P("GJ 1061 b", 0.021, 3.204, 1.04, 1.11, 0.05, 2020, "Radial Velocity"),
			P("GJ 1061 c", 0.035, 6.689, 1.18, 1.74, 0.03, 2020, "Radial Velocity"),
			P("GJ 1061 d", 0.054, 13.03, 1.16, 1.64, 0.05, 2020, "Radial Velocity"),
		],
	},
	{
		hostname: "YZ Cet", ra: 17.9938, dec: -16.9964, sy_dist: 3.712,
		st_teff: 3151, st_rad: 0.168, st_mass: 0.142, st_spectype: "M4.5 V", sy_vmag: 12.07,
		source: "seed", record: { arm: "local", notes: "Three planets inside 0.03 AU. First exoplanet with a possible detected magnetic field, via radio emission.\n\n#compact" },
		planets: [
			P("YZ Cet b", 0.01634, 2.021, 0.93, 0.70, 0.06, 2017, "Radial Velocity"),
			P("YZ Cet c", 0.02156, 3.060, 1.05, 1.14, 0.04, 2017, "Radial Velocity"),
			P("YZ Cet d", 0.02851, 4.656, 1.03, 1.09, 0.06, 2017, "Radial Velocity"),
		],
	},
	{
		hostname: "GJ 273", ra: 111.8523, dec: 5.2255, sy_dist: 3.786,
		st_teff: 3382, st_rad: 0.293, st_mass: 0.290, st_spectype: "M3.5 V", sy_vmag: 9.87,
		source: "seed", record: { arm: "local", notes: "Luyten's Star. A METI transmission was aimed here in 2017; it arrives in 2029.\n\n#habitable-zone" },
		planets: [
			P("GJ 273 c", 0.036, 4.723, 1.05, 1.18, 0.17, 2017, "Radial Velocity"),
			P("GJ 273 b", 0.09110, 18.650, 1.51, 2.89, 0.10, 2017, "Radial Velocity"),
		],
	},
	{
		hostname: "tau Cet", ra: 26.0170, dec: -15.9375, sy_dist: 3.603,
		st_teff: 5344, st_rad: 0.793, st_mass: 0.783, st_spectype: "G8 V", sy_vmag: 3.50,
		source: "seed", record: { arm: "local", notes: "The nearest single Sun-like star. Naked-eye visible. Thick debris disc, so probably a heavy bombardment environment.\n\nThe other original Project Ozma target, alongside [[eps Eri]].\n\n#naked-eye #sun-like #debris-disc" },
		planets: [
			P("tau Cet g", 0.133, 20.00, 1.2, 1.75, 0.06, 2017, "Radial Velocity"),
			P("tau Cet h", 0.243, 49.41, 1.2, 1.83, 0.23, 2017, "Radial Velocity"),
			P("tau Cet e", 0.538, 162.87, 1.8, 3.93, 0.18, 2017, "Radial Velocity"),
			P("tau Cet f", 1.334, 636.13, 1.8, 3.93, 0.16, 2017, "Radial Velocity"),
		],
	},
	{
		hostname: "TRAPPIST-1", ra: 346.6266, dec: -5.0414, sy_dist: 12.467,
		st_teff: 2566, st_rad: 0.1192, st_mass: 0.0898, st_spectype: "M8 V", sy_vmag: 18.80,
		source: "seed",
		record: { arm: "local", notes: "Seven Earth-sized transiting planets, all inside 0.07 AU. The whole system would fit inside Mercury's orbit several times over.\n\nThe far anchor of this vault — nearly four times further out than anything else here.\n\n#compact #habitable-zone #transit" },
		planets: [
			P("TRAPPIST-1 b", 0.01154, 1.5109, 1.116, 1.374, 0.006, 2016, "Transit"),
			P("TRAPPIST-1 c", 0.01580, 2.4218, 1.097, 1.308, 0.007, 2016, "Transit"),
			P("TRAPPIST-1 d", 0.02227, 4.0496, 0.788, 0.388, 0.008, 2016, "Transit"),
			P("TRAPPIST-1 e", 0.02925, 6.1010, 0.920, 0.692, 0.005, 2017, "Transit"),
			P("TRAPPIST-1 f", 0.03849, 9.2075, 1.045, 1.039, 0.010, 2017, "Transit"),
			P("TRAPPIST-1 g", 0.04683, 12.352, 1.129, 1.321, 0.002, 2017, "Transit"),
			P("TRAPPIST-1 h", 0.06189, 18.773, 0.755, 0.326, 0.006, 2017, "Transit"),
		],
	},
].map((s) => ({ ...s, id: slug(s.hostname), addedAt: 0, record: { ...emptyRecord(), ...(s.record || {}) }, precords: s.precords || {} }));

/* ------------------------------------------------------------------- colour */
const TEFF_STOPS = [
	[2300, [255, 122, 74]], [3000, [255, 150, 88]], [3900, [255, 194, 137]],
	[5200, [255, 226, 184]], [5900, [255, 246, 232]], [6600, [242, 241, 255]],
	[8000, [214, 226, 255]], [12000, [168, 195, 255]], [30000, [141, 175, 255]],
];
function teffRGB(t) {
	const T = Math.max(1800, Math.min(40000, t || 4000));
	for (let i = 0; i < TEFF_STOPS.length - 1; i++) {
		const [a, ca] = TEFF_STOPS[i], [b, cb] = TEFF_STOPS[i + 1];
		if (T >= a && T <= b) {
			const k = (T - a) / (b - a);
			return ca.map((v, j) => Math.round(v + (cb[j] - v) * k));
		}
	}
	return TEFF_STOPS[T < 2300 ? 0 : TEFF_STOPS.length - 1][1];
}
const rgbStr = (c) => `rgb(${c[0]},${c[1]},${c[2]})`;

/* On the light plate a star is an ink deposit, so temperature colours darken. */
function bodyColor(sys, plate, mode) {
	if (mode === "arm") {
		const a = armOf(rec(sys).arm);
		if (a) return plate ? a.plate : a.neg;
		return plate ? "#8A8F91" : "#767E86";
	}
	const c = teffRGB(sys.st_teff);
	if (!plate) return rgbStr(c);
	const ink = [22, 24, 26], k = 0.52;
	return rgbStr(c.map((v, i) => Math.round(v * (1 - k) + ink[i] * k)));
}

/* -------------------------------------------------------------------- maths */
const rad = (d) => (d * Math.PI) / 180;
function toXYZ(ra, dec, d) {
	const a = rad(ra || 0), b = rad(dec || 0), r = d || 0;
	return { x: r * Math.cos(b) * Math.cos(a), y: r * Math.cos(b) * Math.sin(a), z: r * Math.sin(b) };
}
const sep = (A, B) => Math.hypot(A.x - B.x, A.y - B.y, A.z - B.z);

/* Log-radial display: keeps direction exact, compresses distance from Sol.
   r' = s·ln(1 + r/s), so it is near-linear nearby and tames far additions. */
function displayPos(n, mode) {
	if (mode !== "log") return { x: n.x, y: n.y, z: n.z };
	const r = Math.hypot(n.x, n.y, n.z);
	if (r < 1e-9) return { x: 0, y: 0, z: 0 };
	const k = Math.log(1 + r) / r;
	return { x: n.x * k, y: n.y * k, z: n.z * k };
}

function habitableZone(st_rad, st_teff) {
	if (!st_rad || !st_teff) return null;
	const L = st_rad * st_rad * Math.pow(st_teff / 5772, 4);
	return { L, inner: Math.sqrt(L / 1.10), outer: Math.sqrt(L / 0.53) };
}
const axisFromPeriod = (days, mstar) =>
	(!days || !mstar) ? null : Math.cbrt(mstar * Math.pow(days / 365.25, 2));
const planetAxis = (p, sys) => p.pl_orbsmax || axisFromPeriod(p.pl_orbper, sys.st_mass) || null;

function planetClass(r) {
	if (!r) return "unclassified";
	if (r < 1.25) return "Terrestrial";
	if (r < 2.0) return "Super-Earth";
	if (r < 6.0) return "Neptune-like";
	return "Gas giant";
}
function fmt(v, d = 2) {
	if (v === null || v === undefined || Number.isNaN(v)) return "—";
	const a = Math.abs(v);
	if (a !== 0 && (a < 1e-3 || a >= 1e6)) return v.toExponential(1);
	return v.toFixed(d);
}

/* Three orbit-radius regimes, named honestly wherever they appear. */
function orbitNorm(a, aMin, aMax, mode) {
	if (!a || !aMax) return 0;
	if (mode === "true") return a / aMax;
	if (aMax === aMin) return 0.62;
	if (mode === "sqrt") {
		const lo = aMin * 0.55;
		return Math.sqrt(Math.max(0, a - lo)) / Math.sqrt(aMax - lo) * 0.86 + 0.1;
	}
	const lo = Math.log(aMin * 0.55), hi = Math.log(aMax * 1.25);
	return ((Math.log(a) - lo) / (hi - lo)) * 0.9 + 0.08;
}
/* stable per-planet starting angle so systems don't all line up */
const phaseOf = (i) => (i * 2.399963229) % (Math.PI * 2);

/* ============================================================== SYSTEM VIEW */
/* One SVG component at every size — vault rows, search results, the record
   panel orrery. Detail tiers derive from size, so it really is one component. */
function SystemView({ sys, size = 300, plate = true, clock = 0, scaleMode = "log", trueMix = 0, showHZ = true, colorMode = "teff", onPick, picked }) {
	const detail = size >= 320 ? "full" : size >= 110 ? "card" : "thumb";
	const pad = detail === "full" ? 46 : detail === "card" ? 12 : 5;
	const cx = size / 2, cy = size / 2, R = size / 2 - pad;

	const planets = (sys.planets || []).map((p) => ({ ...p, a: planetAxis(p, sys) })).filter((p) => p.a);
	const axes = planets.map((p) => p.a);
	const aMin = axes.length ? Math.min(...axes) : 1;
	const aMax = axes.length ? Math.max(...axes) : 1;

	const norm = (a) => {
		const base = orbitNorm(a, aMin, aMax, scaleMode);
		return trueMix ? base * (1 - trueMix) + (a / aMax) * trueMix : base;
	};

	const hz = showHZ && detail !== "thumb" ? habitableZone(sys.st_rad, sys.st_teff) : null;
	const sc = bodyColor(sys, plate, colorMode);
	const rule = plate ? "#B3B4A6" : "#2B3037";
	const ink = plate ? "#16181A" : "#E7E8E3";
	const soft = plate ? "#6D7276" : "#8B9299";
	const accent = plate ? "#1F3F9E" : "#7FA0FF";

	const starTrueAU = (sys.st_rad || 0.5) * RSUN_IN_AU;
	const starR = trueMix > 0.5
		? Math.max(1.1, (starTrueAU / aMax) * R)
		: Math.max(2.4, Math.min(R * 0.1, 9 + Math.log10((sys.st_rad || 0.3) + 0.05) * 5));

	return (
		<svg viewBox={`0 0 ${size} ${size}`} width={size} height={size} style={{ display: "block", overflow: "visible" }}>
			{hz && (() => {
				const ri = norm(hz.inner) * R, ro = norm(hz.outer) * R;
				if (!(ro > ri) || ro < 1) return null;
				return (
					<g>
						<circle cx={cx} cy={cy} r={(ri + ro) / 2} fill="none"
							stroke={plate ? "rgba(31,63,158,0.13)" : "rgba(120,160,255,0.16)"} strokeWidth={ro - ri} />
						{detail === "full" && [ri, ro].map((r, i) => (
							<circle key={i} cx={cx} cy={cy} r={r} fill="none"
								stroke={plate ? "rgba(31,63,158,0.4)" : "rgba(130,170,255,0.45)"} strokeWidth="0.6" strokeDasharray="2 3" />
						))}
					</g>
				);
			})()}

			{planets.map((p, i) => {
				const r = norm(p.a) * R;
				if (r < 0.35) return null;
				return <circle key={i} cx={cx} cy={cy} r={r} fill="none" stroke={rule}
					strokeWidth={detail === "thumb" ? 0.5 : 0.75} opacity={0.85} />;
			})}

			<circle cx={cx} cy={cy} r={starR * 2.6} fill={sc} opacity={plate ? 0.1 : 0.16} />
			<circle cx={cx} cy={cy} r={starR} fill={sc} />

			{planets.map((p, i) => {
				const r = norm(p.a) * R;
				const ang = p.pl_orbper ? phaseOf(i) + (clock / p.pl_orbper) * Math.PI * 2 : phaseOf(i);
				const px = cx + Math.cos(ang) * r, py = cy + Math.sin(ang) * r;
				const trueRad = ((p.pl_rade || 1) * REARTH_IN_AU / aMax) * R;
				const symRad = detail === "thumb" ? 1.6
					: Math.max(2, Math.min(9, 2.1 + Math.log10((p.pl_rade || 1) + 0.4) * 5.2));
				const pr = Math.max(0.35, symRad * (1 - trueMix) + trueRad * trueMix);
				const inHZ = hz && p.a >= hz.inner && p.a <= hz.outer;
				const isPick = picked === p.pl_name;
				return (
					<g key={i} style={{ cursor: onPick ? "pointer" : "default" }}
						onClick={onPick ? (e) => { e.stopPropagation(); onPick(p.pl_name); } : undefined}>
						{onPick && <circle cx={px} cy={py} r={Math.max(pr + 6, 9)} fill="transparent" />}
						{(inHZ || isPick) && r > 1 && (
							<circle cx={px} cy={py} r={pr + 3.5} fill="none" strokeWidth={isPick ? 1.4 : 1}
								stroke={isPick ? accent : (plate ? "rgba(31,63,158,0.55)" : "rgba(130,170,255,0.6)")} />
						)}
						<circle cx={px} cy={py} r={pr} fill={ink} />
						{detail === "full" && (
							<text x={px} y={py - pr - 7} textAnchor="middle" fill={isPick ? accent : soft}
								style={{ font: "500 9.5px 'IBM Plex Mono', monospace", letterSpacing: ".02em" }}>
								{p.pl_name.replace(sys.hostname, "").trim() || p.pl_name}
							</text>
						)}
					</g>
				);
			})}

			{detail === "full" && (
				<g>
					<line x1={cx} y1={cy + R + 16} x2={cx + R} y2={cy + R + 16} stroke={rule} strokeWidth="0.75" />
					<line x1={cx + R} y1={cy + R + 12} x2={cx + R} y2={cy + R + 20} stroke={rule} strokeWidth="0.75" />
					<line x1={cx} y1={cy + R + 12} x2={cx} y2={cy + R + 20} stroke={rule} strokeWidth="0.75" />
					<text x={cx + R} y={cy + R + 32} textAnchor="end" fill={soft}
						style={{ font: "500 9.5px 'IBM Plex Mono', monospace" }}>{fmt(aMax, aMax < 0.1 ? 3 : 2)} AU</text>
					<text x={cx} y={cy + R + 32} fill={soft} style={{ font: "500 9.5px 'IBM Plex Mono', monospace" }}>0</text>
				</g>
			)}
		</svg>
	);
}

/* =============================================================== THE CUBE */
function CubeMap({
	systems, selected, compareId, onSelect, onCompare, plate, showLinks,
	onScale, clock, sysScale, colorMode, distMode, fitToken, focusToken,
}) {
	const wrapRef = useRef(null);
	const canvasRef = useRef(null);
	/* tx/ty/tz is the pivot the camera orbits and zooms toward; wx/wy/wz is where
	   it is heading. Selecting a system re-centres the pivot on it. */
	const camRef = useRef({
		yaw: 0.6, pitch: -0.62, dist: 22, fl: 620, want: 22,
		tx: 0, ty: 0, tz: 0, wx: 0, wy: 0, wz: 0,
	});
	const dragRef = useRef(null);
	const [hover, setHover] = useState(null);
	const [, force] = useState(0);
	const repaint = useCallback(() => force((t) => t + 1), []);

	/* true position for measurement, display position for drawing */
	const nodes = useMemo(() => systems.map((s) => {
		const t = toXYZ(s.ra, s.dec, s.sy_dist || 0);
		const d = displayPos(t, distMode);
		const pl = (s.planets || []).map((p) => ({ ...p, a: planetAxis(p, s) })).filter((p) => p.a);
		const ax = pl.map((p) => p.a);
		return {
			id: s.id, sys: s, t, ...d, planets: pl,
			aMin: ax.length ? Math.min(...ax) : 1, aMax: ax.length ? Math.max(...ax) : 1,
			hz: habitableZone(s.st_rad, s.st_teff),
		};
	}), [systems, distMode]);

	const extent = useMemo(() => {
		let m = 0.4;
		nodes.forEach((n) => { m = Math.max(m, Math.abs(n.x), Math.abs(n.y), Math.abs(n.z)); });
		const ladder = [0.05, 0.1, 0.25, 0.5, 1, 2, 2.5, 4, 5, 10, 20, 25, 50, 100, 250, 500, 1000, 2500, 5000];
		const step = ladder.find((s) => s * 4 >= m * 1.06) || Math.ceil(m / 4);
		return step * 4;
	}, [nodes]);

	/* the pivot follows whatever is selected, so rotation and zoom act on it */
	useEffect(() => {
		const c = camRef.current;
		const n = nodes.find((x) => x.id === selected);
		c.wx = n ? n.x : 0; c.wy = n ? n.y : 0; c.wz = n ? n.z : 0;
	}, [selected, nodes]);

	/* the cube re-frames itself whenever the vault's reach changes */
	useEffect(() => { camRef.current.want = extent * 3.3; }, [extent, fitToken]);
	const firstFocus = useRef(true);
	useEffect(() => {
		if (firstFocus.current) { firstFocus.current = false; return; }
		camRef.current.want = Math.max(0.8, extent * 0.1);
	}, [focusToken, extent]);

	useEffect(() => {
		let raf;
		const ease = () => {
			const c = camRef.current;
			const gd = c.want - c.dist;
			const gx = c.wx - c.tx, gy = c.wy - c.ty, gz = c.wz - c.tz;
			if (Math.abs(gd) > extent * 0.003 || Math.hypot(gx, gy, gz) > extent * 0.002) {
				c.dist += gd * 0.14; c.tx += gx * 0.16; c.ty += gy * 0.16; c.tz += gz * 0.16;
				repaint(); raf = requestAnimationFrame(ease);
			} else {
				c.dist = c.want; c.tx = c.wx; c.ty = c.wy; c.tz = c.wz; repaint();
			}
		};
		raf = requestAnimationFrame(ease);
		return () => cancelAnimationFrame(raf);
	}, [extent, fitToken, focusToken, selected, repaint]);

	/* edges come only from [[links]] you wrote — distance is measured, not drawn */
	const edges = useMemo(() => {
		if (!showLinks) return [];
		const out = [];
		const byId = Object.fromEntries(nodes.map((n) => [n.id, n]));
		nodes.forEach((n) => (n.sys.links || []).forEach((lid) => {
			const m = byId[lid];
			if (m && n.id < m.id) out.push({ a: n, b: m });
		}));
		return out;
	}, [nodes, showLinks]);

	const project = useCallback((p, W, H) => {
		const c = camRef.current;
		const px = p.x - c.tx, py = p.y - c.ty, pz = p.z - c.tz;
		const cy = Math.cos(c.yaw), sy = Math.sin(c.yaw);
		const x1 = px * cy - py * sy, y1 = px * sy + py * cy, z1 = pz;
		const cp = Math.cos(c.pitch), sp = Math.sin(c.pitch);
		const y2 = y1 * cp - z1 * sp, z2 = y1 * sp + z1 * cp;
		const depth = z2 + c.dist;
		if (depth <= 0.2) return null;
		const k = c.fl / depth;
		return { sx: W / 2 + x1 * k, sy: H / 2 - y2 * k, depth, k };
	}, []);

	const projRef = useRef([]);

	const draw = useCallback(() => {
		const cv = canvasRef.current, wrap = wrapRef.current;
		if (!cv || !wrap) return;
		const dpr = Math.min(window.devicePixelRatio || 1, 2);
		const W = wrap.clientWidth, H = wrap.clientHeight;
		if (cv.width !== W * dpr || cv.height !== H * dpr) { cv.width = W * dpr; cv.height = H * dpr; }
		const g = cv.getContext("2d");
		g.setTransform(dpr, 0, 0, dpr, 0, 0);

		const bg = plate ? "#E4E4DA" : "#0B0D10";
		const rule = plate ? "rgba(120,122,108,0.42)" : "rgba(150,162,176,0.24)";
		const faint = plate ? "rgba(120,122,108,0.15)" : "rgba(150,162,176,0.085)";
		const ink = plate ? "#16181A" : "#E7E8E3";
		const soft = plate ? "#71767A" : "#8B9299";
		const acc = plate ? "#1F3F9E" : "#7FA0FF";
		g.fillStyle = bg; g.fillRect(0, 0, W, H);

		const E = extent;
		const seg = (a, b, col, w = 1, dash = null) => {
			const A = project(a, W, H), B = project(b, W, H);
			if (!A || !B) return;
			g.strokeStyle = col; g.lineWidth = w;
			g.setLineDash(dash || []);
			g.beginPath(); g.moveTo(A.sx, A.sy); g.lineTo(B.sx, B.sy); g.stroke(); g.setLineDash([]);
		};

		const step = E / 4;
		for (let i = -4; i <= 4; i++) {
			const t = i * step, mid = i === 0, edge = Math.abs(i) === 4;
			const col = mid || edge ? rule : faint;
			seg({ x: t, y: -E, z: 0 }, { x: t, y: E, z: 0 }, col, mid ? 1 : 0.6);
			seg({ x: -E, y: t, z: 0 }, { x: E, y: t, z: 0 }, col, mid ? 1 : 0.6);
		}
		const V = [];
		for (const a of [-1, 1]) for (const b of [-1, 1]) for (const c of [-1, 1]) V.push({ x: a * E, y: b * E, z: c * E });
		for (let i = 0; i < 8; i++) for (let j = i + 1; j < 8; j++) {
			let diff = 0;
			if (V[i].x !== V[j].x) diff++; if (V[i].y !== V[j].y) diff++; if (V[i].z !== V[j].z) diff++;
			if (diff === 1) seg(V[i], V[j], rule, 0.7, [3, 4]);
		}
		for (let i = -4; i <= 4; i++) {
			if (!i) continue;
			const p = project({ x: -E, y: -E, z: i * step }, W, H);
			if (!p) continue;
			g.fillStyle = soft; g.font = "500 9px 'IBM Plex Mono', monospace"; g.textAlign = "right";
			g.fillText(fmt(i * step, E < 4 ? 1 : 0), p.sx - 5, p.sy + 3);
		}

		edges.forEach((e) => seg(e.a, e.b, plate ? "rgba(31,63,158,0.5)" : "rgba(127,160,255,0.45)", 1.2));

		if (selected && compareId) {
			const A = nodes.find((n) => n.id === selected), B = nodes.find((n) => n.id === compareId);
			if (A && B) {
				seg(A, B, acc, 1.6);
				const pa = project(A, W, H), pb = project(B, W, H);
				if (pa && pb) {
					const d = sep(A.t, B.t);
					g.fillStyle = acc; g.font = "600 10.5px 'IBM Plex Mono', monospace"; g.textAlign = "center";
					g.fillText(`${d.toFixed(2)} pc · ${(d * PC_IN_LY).toFixed(2)} ly`, (pa.sx + pb.sx) / 2, (pa.sy + pb.sy) / 2 - 8);
				}
			}
		}

		/* --- systems, far to near ------------------------------------------- */
		const items = [];
		nodes.forEach((n) => {
			const p = project(n, W, H);
			if (p) items.push({ n, p, foot: project({ x: n.x, y: n.y, z: 0 }, W, H) });
		});
		items.sort((a, b) => b.p.depth - a.p.depth);
		projRef.current = items;

		const eps = E * 0.015;
		items.forEach(({ n, p, foot }) => {
			const isSel = n.id === selected, isCmp = n.id === compareId, isHov = hover === n.id;
			const col = bodyColor(n.sys, plate, colorMode);

			if (foot) {
				g.strokeStyle = plate ? "rgba(120,122,108,0.34)" : "rgba(150,162,176,0.2)";
				g.lineWidth = 0.7; g.setLineDash([2, 2]);
				g.beginPath(); g.moveTo(p.sx, p.sy); g.lineTo(foot.sx, foot.sy); g.stroke(); g.setLineDash([]);
				g.fillStyle = plate ? "rgba(120,122,108,0.5)" : "rgba(150,162,176,0.3)";
				g.beginPath(); g.arc(foot.sx, foot.sy, 1.3, 0, 7); g.fill();
			}

			/* screen basis for the equatorial plane at this node, so each orrery
			   lies flat in the cube instead of facing the camera */
			const pu = project({ x: n.x + eps, y: n.y, z: n.z }, W, H);
			const pv = project({ x: n.x, y: n.y + eps, z: n.z }, W, H);
			let ux = 1, uy = 0, vx = 0, vy = 1;
			if (pu && pv) {
				ux = pu.sx - p.sx; uy = pu.sy - p.sy;
				vx = pv.sx - p.sx; vy = pv.sy - p.sy;
				const L = Math.hypot(ux, uy) || 1;
				ux /= L; uy /= L; vx /= L; vy /= L;
			}
			const at = (r, ang) => [p.sx + r * (Math.cos(ang) * ux + Math.sin(ang) * vx),
			p.sy + r * (Math.cos(ang) * uy + Math.sin(ang) * vy)];

			const zoomK = Math.min(2.4, Math.max(0.4, p.k / 190));
			const R = sysScale * 17 * zoomK;
			const drawOrrery = sysScale > 0 && R >= 7 && n.planets.length > 0;

			if (drawOrrery) {
				if (n.hz) {
					const ri = orbitNorm(n.hz.inner, n.aMin, n.aMax, "log") * R;
					const ro = orbitNorm(n.hz.outer, n.aMin, n.aMax, "log") * R;
					if (ro > ri && ri > 0) {
						g.strokeStyle = plate ? "rgba(31,63,158,0.16)" : "rgba(120,160,255,0.2)";
						g.lineWidth = ro - ri;
						g.beginPath();
						for (let a = 0; a <= 64; a++) { const [X, Y] = at((ri + ro) / 2, (a / 64) * Math.PI * 2); a ? g.lineTo(X, Y) : g.moveTo(X, Y); }
						g.stroke();
					}
				}
				g.strokeStyle = plate ? "rgba(120,122,108,0.55)" : "rgba(150,162,176,0.4)";
				g.lineWidth = 0.6;
				n.planets.forEach((pl) => {
					const r = orbitNorm(pl.a, n.aMin, n.aMax, "log") * R;
					if (r < 1.5) return;
					g.beginPath();
					for (let a = 0; a <= 48; a++) { const [X, Y] = at(r, (a / 48) * Math.PI * 2); a ? g.lineTo(X, Y) : g.moveTo(X, Y); }
					g.closePath(); g.stroke();
				});
			}

			const mag = n.sys.sy_vmag;
			const base = n.sys.origin ? 6.5 : Math.max(2.6, 7.2 - (mag == null ? 4 : mag) * 0.22);
			const starR = drawOrrery ? Math.max(2, Math.min(R * 0.16, base * zoomK * 0.8)) : Math.max(2, base * zoomK);

			g.globalAlpha = 0.16; g.fillStyle = col;
			g.beginPath(); g.arc(p.sx, p.sy, starR * 3.1, 0, 7); g.fill();
			g.globalAlpha = 1; g.fillStyle = col;
			g.beginPath(); g.arc(p.sx, p.sy, starR, 0, 7); g.fill();
			g.strokeStyle = plate ? "rgba(22,24,26,0.55)" : "rgba(231,232,227,0.4)";
			g.lineWidth = 0.7; g.beginPath(); g.arc(p.sx, p.sy, starR, 0, 7); g.stroke();

			if (drawOrrery) {
				g.fillStyle = ink;
				n.planets.forEach((pl, i) => {
					const r = orbitNorm(pl.a, n.aMin, n.aMax, "log") * R;
					if (r < 1.5) return;
					const ang = pl.pl_orbper ? phaseOf(i) + (clock / pl.pl_orbper) * Math.PI * 2 : phaseOf(i);
					const [X, Y] = at(r, ang);
					const pr = Math.max(1, Math.min(3.6, 1 + Math.log10((pl.pl_rade || 1) + 0.4) * 2.4) * Math.min(1.6, zoomK));
					g.beginPath(); g.arc(X, Y, pr, 0, 7); g.fill();
				});
			}

			if (n.sys.origin) {
				g.strokeStyle = ink; g.lineWidth = 0.9;
				[[-1, 0], [1, 0], [0, -1], [0, 1]].forEach(([dx, dy]) => {
					const o = drawOrrery ? R + 4 : starR + 3, o2 = o + 5;
					g.beginPath(); g.moveTo(p.sx + dx * o, p.sy + dy * o); g.lineTo(p.sx + dx * o2, p.sy + dy * o2); g.stroke();
				});
			}
			if (isSel || isCmp || isHov) {
				const ring = drawOrrery ? R + 7 : starR + 6;
				g.strokeStyle = acc; g.lineWidth = isSel ? 1.5 : 1;
				g.beginPath(); g.arc(p.sx, p.sy, ring, 0, 7); g.stroke();
				if (isCmp) { g.setLineDash([2, 3]); g.beginPath(); g.arc(p.sx, p.sy, ring + 4, 0, 7); g.stroke(); g.setLineDash([]); }
			}
			if (isSel || isCmp || isHov || n.sys.origin || p.k > 120) {
				const off = (drawOrrery ? R : starR) + 8;
				const label = rec(n.sys).imperialName || n.sys.hostname;
				g.fillStyle = isSel || isCmp ? acc : ink;
				g.font = `${isSel ? 600 : 500} 10.5px 'IBM Plex Sans Condensed','IBM Plex Sans',sans-serif`;
				g.textAlign = "left";
				g.fillText(label, p.sx + off, p.sy + 3.5);
			}
		});

		const o = project({ x: 0, y: 0, z: 0 }, W, H), u = project({ x: 1, y: 0, z: 0 }, W, H);
		if (o && u) onScale?.(Math.hypot(u.sx - o.sx, u.sy - o.sy));
	}, [nodes, edges, extent, plate, selected, compareId, hover, project, onScale, clock, sysScale, colorMode]);

	useEffect(() => { draw(); });
	useEffect(() => {
		const ro = new ResizeObserver(() => draw());
		if (wrapRef.current) ro.observe(wrapRef.current);
		return () => ro.disconnect();
	}, [draw]);
	useEffect(() => { document.fonts?.ready?.then(repaint); }, [repaint]);

	const pickAt = (mx, my) => {
		let best = null, bd = 22;
		projRef.current.forEach(({ n, p }) => {
			const d = Math.hypot(p.sx - mx, p.sy - my);
			if (d < bd) { bd = d; best = n.id; }
		});
		return best;
	};
	const onDown = (e) => {
		const r = canvasRef.current.getBoundingClientRect();
		dragRef.current = { x: e.clientX, y: e.clientY, moved: 0, sx: e.clientX - r.left, sy: e.clientY - r.top, shift: e.shiftKey };
		e.currentTarget.setPointerCapture?.(e.pointerId);
	};
	const onMove = (e) => {
		const r = canvasRef.current.getBoundingClientRect();
		const d = dragRef.current;
		if (d) {
			const dx = e.clientX - d.x, dy = e.clientY - d.y;
			d.moved += Math.abs(dx) + Math.abs(dy);
			camRef.current.yaw += dx * 0.007;
			camRef.current.pitch = Math.max(-1.53, Math.min(1.53, camRef.current.pitch - dy * 0.007));
			d.x = e.clientX; d.y = e.clientY;
			repaint();
		} else {
			const h = pickAt(e.clientX - r.left, e.clientY - r.top);
			if (h !== hover) setHover(h);
		}
	};
	const onUp = () => {
		const d = dragRef.current; dragRef.current = null;
		if (!d || d.moved > 6) return;
		const id = pickAt(d.sx, d.sy);
		if (!id) return;
		if (d.shift) onCompare(id === compareId ? null : id); else onSelect(id);
	};
	const onWheel = (e) => {
		e.preventDefault();
		const c = camRef.current;
		/* zoom travels toward the pivot, which is the system you last clicked */
		c.want = c.dist = Math.max(extent * 0.02, Math.min(extent * 22, c.dist * (1 + Math.sign(e.deltaY) * 0.13)));
		repaint();
	};

	const hoverSys = hover ? systems.find((s) => s.id === hover) : null;
	const centreSys = systems.find((s) => s.id === selected);
	const centreName = centreSys ? (rec(centreSys).imperialName || centreSys.hostname) : "Sol";

	return (
		<div ref={wrapRef} className="cube-wrap">
			<canvas ref={canvasRef} className="cube-canvas"
				onPointerDown={onDown} onPointerMove={onMove} onPointerUp={onUp}
				onPointerLeave={() => { dragRef.current = null; setHover(null); }}
				onWheel={onWheel} style={{ cursor: hover ? "pointer" : "grab" }} />
			<div className="cube-corner">
				<div className="eyebrow">the cube · equatorial cartesian</div>
				<div className="mono tiny">half-width {fmt(extent, extent < 4 ? 1 : 0)} pc ({fmt(extent * PC_IN_LY, 0)} ly)
					{distMode === "log" && <span className="badge comp inline-badge">log-radial</span>}</div>
				<div className="mono tiny dim">drag rotate · wheel zoom · click re-centre · shift-click measure</div>
				<div className="mono tiny centred">
					centred on {centreName}
				</div>
			</div>
			{colorMode === "arm" && (
				<div className="legend">
					<span className="eyebrow">arm</span>
					{ARMS.map((a) => (
						<span key={a.id} className="lg"><i style={{ background: plate ? a.plate : a.neg }} />{a.name}</span>
					))}
					<span className="lg dim"><i style={{ background: plate ? "#8A8F91" : "#767E86" }} />unassigned</span>
				</div>
			)}
			{hoverSys && (
				<div className="cube-peek">
					<SystemView sys={hoverSys} size={92} plate={plate} clock={clock} colorMode={colorMode} />
					<div>
						<div className="peek-name">{rec(hoverSys).imperialName || hoverSys.hostname}</div>
						{rec(hoverSys).imperialName && <div className="mono tiny dim">{hoverSys.hostname}</div>}
						<div className="mono tiny dim">{hoverSys.st_spectype || "—"} · {fmt(hoverSys.sy_dist, 2)} pc</div>
						<div className="mono tiny dim">{(hoverSys.planets || []).length} planets</div>
					</div>
				</div>
			)}
		</div>
	);
}

/* ================================================================ NASA fetch */
async function tapQuery(adql) {
	const url = `${TAP}?query=${encodeURIComponent(adql)}&format=json`;
	const routes = [url,
		`https://corsproxy.io/?url=${encodeURIComponent(url)}`,
		`https://api.allorigins.win/raw?url=${encodeURIComponent(url)}`];
	let last = null;
	for (const u of routes) {
		try {
			const r = await fetch(u);
			if (!r.ok) { last = new Error(`${r.status} from the archive`); continue; }
			const json = JSON.parse(await r.text());
			if (Array.isArray(json)) return json;
			last = new Error("unexpected response shape");
		} catch (err) { last = err; }
	}
	throw last || new Error("no route to the archive");
}

function rowsToSystems(rows) {
	const by = {};
	rows.forEach((r) => {
		const h = r.hostname;
		if (!by[h]) by[h] = {
			id: slug(h), hostname: h, ra: r.ra, dec: r.dec, sy_dist: r.sy_dist,
			st_teff: r.st_teff, st_rad: r.st_rad, st_mass: r.st_mass,
			st_spectype: r.st_spectype, sy_vmag: r.sy_vmag, sy_snum: r.sy_snum, sy_pnum: r.sy_pnum,
			planets: [], record: emptyRecord(), precords: {}, links: [],
			source: "nasa", fetchedAt: Date.now(),
		};
		by[h].planets.push({
			pl_name: r.pl_name, pl_orbsmax: r.pl_orbsmax, pl_orbper: r.pl_orbper,
			pl_rade: r.pl_rade, pl_bmasse: r.pl_bmasse, pl_eqt: r.pl_eqt,
			pl_orbeccen: r.pl_orbeccen, disc_year: r.disc_year,
			discoverymethod: r.discoverymethod, disc_facility: r.disc_facility,
		});
	});
	return Object.values(by).map((s) => {
		s.planets.sort((a, b) => (a.pl_orbsmax || a.pl_orbper || 0) - (b.pl_orbsmax || b.pl_orbper || 0));
		return s;
	});
}

/* =============================================================== ARCHIVE UI */
function Facts({ rows }) {
	return (
		<dl className="facts mono">
			{rows.map(([k, v], i) => <div key={i}><dt>{k}</dt><dd>{v}</dd></div>)}
		</dl>
	);
}
function Field({ label, value, onChange, placeholder, area, hint }) {
	return (
		<label className="fld">
			<span className="eyebrow">{label}</span>
			{area
				? <textarea className="field notes" value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} />
				: <input className="field" value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} />}
			{hint && <span className="hint mono tiny dim">{hint}</span>}
		</label>
	);
}

/* ===================================================================== APP */
const VAULT_KEY = "parallax:vault:v2";

export default function Parallax() {
	const [plate, setPlate] = useState(true);
	const [systems, setSystems] = useState(SEED);
	const [selected, setSelected] = useState("gj-1061");
	const [focusPlanet, setFocusPlanet] = useState(null);
	const [compareId, setCompareId] = useState(null);
	const [loaded, setLoaded] = useState(false);
	const [status, setStatus] = useState(null);
	const [pane, setPane] = useState("map");

	const [addOpen, setAddOpen] = useState(false);
	const [query, setQuery] = useState("");
	const [results, setResults] = useState(null);
	const [busy, setBusy] = useState(false);
	const [addErr, setAddErr] = useState(null);
	const [pasteVal, setPasteVal] = useState("");

	const [filter, setFilter] = useState("");
	const [showLinks, setShowLinks] = useState(true);
	const [colorMode, setColorMode] = useState("teff");
	const [distMode, setDistMode] = useState("linear");
	const [sysScale, setSysScale] = useState(1);
	const [fitToken, setFitToken] = useState(0);
	const [focusToken, setFocusToken] = useState(0);

	const [scaleMode, setScaleMode] = useState("log");
	const [trueMix, setTrueMix] = useState(0);
	const [speed, setSpeed] = useState(4);
	const [clock, setClock] = useState(0);
	const [pxPerPc, setPxPerPc] = useState(60);
	const [scaleOpen, setScaleOpen] = useState(false);

	/* ---- vault persistence ------------------------------------------------ */
	useEffect(() => {
		(async () => {
			try {
				const r = await window.storage.get(VAULT_KEY);
				const v = r ? JSON.parse(r.value) : null;
				if (v?.systems?.length) {
					/* migrate v1 vaults: bare notes become the record's notes field */
					const migrated = v.systems.map((s) => ({
						...s,
						record: { ...emptyRecord(), ...(s.record || {}), notes: s.record?.notes ?? s.notes ?? "" },
						precords: s.precords || {},
					}));
					setSystems(migrated);
					setSelected(v.selected || migrated[0].id);
				}
			} catch { /* first run, or no storage — the seed catalog stands */ }
			setLoaded(true);
		})();
	}, []);
	const persist = useCallback(async (next, sel) => {
		try { await window.storage.set(VAULT_KEY, JSON.stringify({ systems: next, selected: sel, v: 2 })); }
		catch { setStatus("Saved for this session only — storage is unavailable here."); }
	}, []);
	useEffect(() => { if (loaded) persist(systems, selected); }, [systems, selected, loaded, persist]);

	/* ---- shared orbital clock, throttled to ~33 fps ----------------------- */
	useEffect(() => {
		if (!speed) return;
		let raf, last = performance.now(), acc = 0;
		const step = (t) => {
			const dt = (t - last) / 1000; last = t; acc += dt;
			if (acc > 0.031) { setClock((c) => c + acc * speed * 8); acc = 0; }
			raf = requestAnimationFrame(step);
		};
		raf = requestAnimationFrame(step);
		return () => cancelAnimationFrame(raf);
	}, [speed]);

	/* ---- true-scale collapse tween ---------------------------------------- */
	const wantTrue = scaleMode === "true";
	useEffect(() => {
		let raf, start = null;
		const from = trueMix, to = wantTrue ? 1 : 0;
		if (from === to) return;
		const run = (t) => {
			if (start === null) start = t;
			const k = Math.min(1, (t - start) / 1300);
			const e = k < 0.5 ? 2 * k * k : 1 - Math.pow(-2 * k + 2, 2) / 2;
			setTrueMix(from + (to - from) * e);
			if (k < 1) raf = requestAnimationFrame(run);
		};
		raf = requestAnimationFrame(run);
		return () => cancelAnimationFrame(raf);
	}, [wantTrue]); // eslint-disable-line react-hooks/exhaustive-deps

	/* ---- derived ---------------------------------------------------------- */
	const sys = systems.find((s) => s.id === selected) || systems[0];
	const cmp = compareId ? systems.find((s) => s.id === compareId) : null;
	const R = rec(sys);
	const planet = focusPlanet ? (sys?.planets || []).find((p) => p.pl_name === focusPlanet) : null;
	const PR = planet ? prec(sys, planet.pl_name) : null;

	const allTags = useMemo(() => {
		const t = new Set();
		systems.forEach((s) => {
			const blob = [rec(s).notes, ...Object.values(s.precords || {}).map((r) => r.notes || "")].join(" ");
			blob.match(/#[\w-]+/g)?.forEach((x) => t.add(x.slice(1)));
		});
		return [...t].sort();
	}, [systems]);

	const shown = useMemo(() => {
		const q = filter.trim().toLowerCase();
		if (!q) return systems;
		return systems.filter((s) => {
			const r = rec(s);
			const blob = [s.hostname, r.imperialName, r.notes, r.population, s.st_spectype,
			...(s.planets || []).map((p) => p.pl_name),
			...Object.values(s.precords || {}).flatMap((x) => [x.imperialName, x.notes, x.continents])]
				.join(" ").toLowerCase();
			return blob.includes(q);
		});
	}, [systems, filter]);

	const withLinks = useMemo(() => systems.map((s) => {
		const blob = [rec(s).notes, ...Object.values(s.precords || {}).map((r) => r.notes || "")].join(" ");
		const names = [...blob.matchAll(/\[\[([^\]]+)\]\]/g)].map((m) => slug(m[1]));
		return { ...s, links: [...new Set(names.filter((n) => n !== s.id && systems.some((x) => x.id === n)))] };
	}), [systems]);

	const stats = useMemo(() => ({
		n: systems.length,
		p: systems.reduce((a, s) => a + (s.planets || []).length, 0),
		far: systems.length ? Math.max(...systems.map((s) => s.sy_dist || 0)) : 0,
	}), [systems]);

	const hz = habitableZone(sys?.st_rad, sys?.st_teff);
	const axes = (sys?.planets || []).map((p) => planetAxis(p, sys)).filter(Boolean);
	const aMax = axes.length ? Math.max(...axes) : 1;
	const aMin = axes.length ? Math.min(...axes) : 1;
	const trueWidthPx = ((aMax * 2) / PC_IN_AU) * pxPerPc;
	const dist = sys && cmp ? sep(toXYZ(sys.ra, sys.dec, sys.sy_dist), toXYZ(cmp.ra, cmp.dec, cmp.sy_dist)) : null;

	/* ---- mutations -------------------------------------------------------- */
	const patch = (id, fn) => setSystems((prev) => prev.map((s) => (s.id === id ? fn(s) : s)));
	const setRecord = (id, k, v) => patch(id, (s) => ({ ...s, record: { ...rec(s), [k]: v } }));
	const setPRecord = (id, name, k, v) => patch(id, (s) => ({
		...s, precords: { ...(s.precords || {}), [name]: { ...prec(s, name), [k]: v } },
	}));

	const addSystem = (s, jump = true) => {
		setSystems((prev) => {
			const i = prev.findIndex((x) => x.id === s.id);
			if (i < 0) return [...prev, { ...s, addedAt: Date.now() }];
			/* archive fields refresh; your dossier is never overwritten */
			return prev.map((x, j) => j === i
				? { ...x, ...s, record: rec(x), precords: x.precords || {}, addedAt: x.addedAt } : x);
		});
		setSelected(s.id); setFocusPlanet(null);
		setFitToken((t) => t + 1);
		if (jump) { setStatus(`${s.hostname} saved. The cube re-framed to fit it.`); setPane("map"); setAddOpen(false); }
	};

	const runSearch = async (term) => {
		const t = (term ?? query).trim();
		if (!t) return;
		setBusy(true); setAddErr(null); setResults(null);
		const esc = t.replace(/'/g, "''").toUpperCase();
		try {
			const rows = await tapQuery(`select ${COLS} from pscomppars where upper(hostname) like '%${esc}%' or upper(pl_name) like '%${esc}%'`);
			const found = rowsToSystems(rows);
			setResults(found);
			if (!found.length) setAddErr(`Nothing in pscomppars matches "${t}". Try a catalog name — GJ, HD, Kepler, TOI, TRAPPIST.`);
		} catch (e) {
			setAddErr(`Couldn't reach the archive from this page (${e.message}). Open the query yourself and paste the response below.`);
		}
		setBusy(false);
	};
	const refreshFromNasa = async (s) => {
		setBusy(true);
		try {
			const rows = await tapQuery(`select ${COLS} from pscomppars where upper(hostname) = '${s.hostname.replace(/'/g, "''").toUpperCase()}'`);
			const found = rowsToSystems(rows)[0];
			if (found) { addSystem(found, false); setStatus(`${s.hostname} refreshed. Your dossier was kept.`); }
			else setStatus(`${s.hostname} isn't in pscomppars under that name — seed values stay.`);
		} catch (e) { setStatus(`Refresh failed: ${e.message}`); }
		setBusy(false);
	};
	const importPaste = () => {
		try {
			const rows = JSON.parse(pasteVal);
			const found = rowsToSystems(Array.isArray(rows) ? rows : [rows]);
			if (!found.length) throw new Error("empty");
			found.forEach((f) => addSystem(f, false));
			setPasteVal(""); setAddErr(null); setAddOpen(false); setPane("map");
			setStatus(`Imported ${found.length} system${found.length > 1 ? "s" : ""}.`);
		} catch { setAddErr("That isn't the archive's JSON. Paste the whole response, starting with ["); }
	};
	const removeSystem = (id) => {
		setSystems((prev) => {
			const next = prev.filter((s) => s.id !== id);
			if (selected === id) { setSelected(next[0]?.id); setFocusPlanet(null); }
			return next;
		});
		if (compareId === id) setCompareId(null);
		setFitToken((t) => t + 1);
	};

	const queryUrl = `${TAP}?query=${encodeURIComponent(
		`select ${COLS} from pscomppars where upper(hostname) like '%${(query || "GJ 1061").toUpperCase()}%'`)}&format=json`;

	/* ------------------------------------------------------------------ view */
	return (
		<div className={`px ${plate ? "plate" : "neg"}`}>
			<style>{CSS}</style>

			<header className="bar">
				<div className="brand">
					<span className="mark" aria-hidden="true" />
					<span className="wordmark">Parallax</span>
					<span className="tagline">a vault for star systems</span>
				</div>
				<div className="bar-stats mono tiny">{stats.n} systems · {stats.p} planets · furthest {fmt(stats.far, 1)} pc</div>
				<div className="bar-actions">
					<button className="btn ghost" onClick={() => setScaleOpen(true)}>Scale</button>
					<button className="btn ghost" onClick={() => setPlate(!plate)}>{plate ? "Negative" : "Plate"}</button>
				</div>
			</header>

			<div className="mobile-tabs">
				{[["vault", "vault"], ["map", "cube"], ["note", "record"]].map(([k, l]) => (
					<button key={k} className={`mtab ${pane === k ? "on" : ""}`} onClick={() => setPane(k)}>{l}</button>
				))}
			</div>

			<main className="grid">
				{/* ======================================================== VAULT === */}
				<aside className={`col vault ${pane === "vault" ? "show" : ""}`}>
					<div className="col-head">
						<span className="eyebrow">Vault</span>
						<button className={`btn tiny-btn ${addOpen ? "solid" : ""}`} onClick={() => setAddOpen(!addOpen)}>
							{addOpen ? "Close" : "+ NASA"}
						</button>
					</div>

					{addOpen && (
						<div className="addbox">
							<p className="lede tiny">
								Search the <b>pscomppars</b> table — the same composite parameters behind NASA's catalog pages.
							</p>
							<div className="searchrow">
								<input className="field grow" placeholder="GJ 1061, TRAPPIST-1, Kepler-186…" value={query}
									onChange={(e) => setQuery(e.target.value)} onKeyDown={(e) => e.key === "Enter" && runSearch()} />
								<button className="btn solid tiny-btn" onClick={() => runSearch()} disabled={busy}>{busy ? "…" : "Go"}</button>
							</div>
							<div className="quickrow">
								{["GJ 1061", "Kepler-186", "TOI-700", "K2-18", "HD 219134"].map((q) => (
									<button key={q} className="chip" onClick={() => { setQuery(q); runSearch(q); }}>{q}</button>
								))}
							</div>
							{addErr && <div className="notice warn tiny">{addErr}</div>}
							{results && (
								<div className="results">
									{results.map((r) => (
										<div key={r.id} className="rrow">
											<SystemView sys={r} size={46} plate={plate} clock={clock} colorMode={colorMode} />
											<div className="grow">
												<div className="vrow-name">{r.hostname}</div>
												<div className="mono tiny dim">{fmt(r.sy_dist, 2)} pc · {r.planets.length}p · {r.st_spectype || "—"}</div>
											</div>
											<button className="btn solid tiny-btn" onClick={() => addSystem(r)}>
												{systems.some((s) => s.id === r.id) ? "Update" : "Save"}
											</button>
										</div>
									))}
								</div>
							)}
							<details className="fallback">
								<summary>Archive unreachable?</summary>
								<p className="lede tiny">Some browsers block cross-site requests. Open the query, copy the response, paste it here — identical result.</p>
								<div className="searchrow">
									<a className="btn ghost tiny-btn" href={queryUrl} target="_blank" rel="noreferrer">Open query</a>
									<button className="btn ghost tiny-btn" onClick={() => navigator.clipboard?.writeText(queryUrl)}>Copy URL</button>
								</div>
								<textarea className="field ta" placeholder='Paste JSON — starts with [{"pl_name":…'
									value={pasteVal} onChange={(e) => setPasteVal(e.target.value)} />
								<button className="btn solid tiny-btn" onClick={importPaste} disabled={!pasteVal.trim()}>Import</button>
							</details>
						</div>
					)}

					<input className="field" placeholder="Filter name, dossier, tag" value={filter} onChange={(e) => setFilter(e.target.value)} />
					{allTags.length > 0 && (
						<div className="tagrow">
							{allTags.map((t) => (
								<button key={t} className={`chip ${filter === "#" + t ? "on" : ""}`}
									onClick={() => setFilter(filter === "#" + t ? "" : "#" + t)}>#{t}</button>
							))}
						</div>
					)}

					<div className="vault-list">
						{shown.map((s) => {
							const r = rec(s), arm = armOf(r.arm);
							return (
								<button key={s.id} className={`vrow ${s.id === selected ? "on" : ""} ${s.id === compareId ? "cmp" : ""}`}
									onClick={() => { setSelected(s.id); setFocusPlanet(null); setPane("map"); }}>
									<SystemView sys={s} size={44} plate={plate} clock={clock} colorMode={colorMode} />
									<div className="vrow-txt">
										<div className="vrow-name">
											{r.imperialName || s.hostname}
											{s.origin && <span className="here">you are here</span>}
										</div>
										<div className="mono tiny dim">
											{r.imperialName ? `${s.hostname} · ` : ""}{fmt(s.sy_dist, 2)} pc · {(s.planets || []).length}p
										</div>
									</div>
									{arm && <span className="armdot" style={{ background: plate ? arm.plate : arm.neg }} title={arm.name} />}
								</button>
							);
						})}
						{!shown.length && <p className="empty">No match. Clear the filter, or pull a system from NASA.</p>}
					</div>
				</aside>

				{/* ========================================================= CUBE === */}
				<section className={`col centre ${pane === "map" ? "show" : ""}`}>
					<CubeMap systems={withLinks} selected={selected} compareId={compareId}
						onSelect={(id) => { setSelected(id); setFocusPlanet(null); }} onCompare={setCompareId}
						plate={plate} showLinks={showLinks} onScale={setPxPerPc}
						clock={clock} sysScale={sysScale} colorMode={colorMode} distMode={distMode}
						fitToken={fitToken} focusToken={focusToken} />

					<div className="ctrls">
						<div className="ctrl">
							<span className="eyebrow">systems</span>
							<div className="seg small">
								{[[0, "dots"], [1, "×1"], [2, "×2"], [3.5, "×4"]].map(([k, l]) => (
									<button key={l} className={`segbtn ${sysScale === k ? "on" : ""}`} onClick={() => setSysScale(k)}>{l}</button>
								))}
							</div>
						</div>
						<div className="ctrl">
							<span className="eyebrow">motion</span>
							<div className="seg small">
								{[[0, "❚❚"], [1, "1×"], [4, "4×"], [30, "30×"]].map(([k, l]) => (
									<button key={l} className={`segbtn ${speed === k ? "on" : ""}`} onClick={() => setSpeed(k)}>{l}</button>
								))}
							</div>
						</div>
						<div className="ctrl">
							<span className="eyebrow">colour</span>
							<div className="seg small">
								{[["teff", "temp"], ["arm", "arm"]].map(([k, l]) => (
									<button key={k} className={`segbtn ${colorMode === k ? "on" : ""}`} onClick={() => setColorMode(k)}>{l}</button>
								))}
							</div>
						</div>
						<div className="ctrl">
							<span className="eyebrow">distance</span>
							<div className="seg small">
								{[["linear", "true"], ["log", "log"]].map(([k, l]) => (
									<button key={k} className={`segbtn ${distMode === k ? "on" : ""}`} onClick={() => setDistMode(k)}>{l}</button>
								))}
							</div>
						</div>
						<div className="ctrl">
							<span className="eyebrow">framing</span>
							<div className="rangerow">
								<button className="btn ghost tiny-btn" onClick={() => setFocusToken((t) => t + 1)}>Focus</button>
								<button className="btn ghost tiny-btn" onClick={() => setFitToken((t) => t + 1)}>Fit all</button>
							</div>
						</div>
						<div className="ctrl">
							<span className="eyebrow">&nbsp;</span>
							<div className="rangerow">
								<label className="chkline">
									<input type="checkbox" checked={showLinks} onChange={(e) => setShowLinks(e.target.checked)} />
									<span className="mono tiny">[[links]]</span>
								</label>
								{cmp && <button className="btn ghost tiny-btn" onClick={() => setCompareId(null)}>clear measure</button>}
							</div>
						</div>
					</div>
				</section>

				{/* ======================================================= RECORD === */}
				<aside className={`col note ${pane === "note" ? "show" : ""}`}>
					{sys && (
						<>
							<div className="col-head">
								<span className="eyebrow">Record</span>
								<div className="row-gap">
									<button className="btn tiny-btn ghost" onClick={() => refreshFromNasa(sys)} disabled={busy}>{busy ? "…" : "Refresh"}</button>
									{!sys.origin && <button className="btn tiny-btn ghost danger" onClick={() => removeSystem(sys.id)}>Remove</button>}
								</div>
							</div>

							<h2 className="sysname">{R.imperialName || sys.hostname}</h2>
							<div className="srcline mono tiny">
								{R.imperialName && <span className="dim">{sys.hostname} · </span>}
								<span className={`src ${sys.source}`}>
									{sys.source === "nasa" ? "NASA pscomppars" : sys.source === "reference" ? "reference" : "seed values"}
								</span>
							</div>

							{/* entity selector: the system, then every planet */}
							<div className="entities">
								<button className={`ent ${!focusPlanet ? "on" : ""}`} onClick={() => setFocusPlanet(null)}>★ system</button>
								{(sys.planets || []).map((p) => {
									const short = p.pl_name.replace(sys.hostname, "").trim() || p.pl_name;
									const nm = prec(sys, p.pl_name).imperialName;
									return (
										<button key={p.pl_name} className={`ent ${focusPlanet === p.pl_name ? "on" : ""}`}
											onClick={() => setFocusPlanet(p.pl_name)}>{nm || short}</button>
									);
								})}
							</div>

							{/* -------------------------------------------- SYSTEM VIEW -- */}
							{!focusPlanet && (
								<>
									<div className="preview">
										<SystemView sys={sys} size={250} plate={plate} clock={clock} scaleMode={scaleMode}
											trueMix={trueMix} colorMode={colorMode} onPick={setFocusPlanet} picked={focusPlanet} />
									</div>
									<div className="orrctl">
										<button className="btn ghost tiny-btn" onClick={() => { setPane("map"); setFocusToken((t) => t + 1); }}>Focus in cube</button>
										<span className="eyebrow">orbit scale</span>
										<div className="seg small">
											{[["log", "log"], ["sqrt", "√"], ["true", "true"]].map(([k, l]) => (
												<button key={k} className={`segbtn ${scaleMode === k ? "on" : ""}`} onClick={() => setScaleMode(k)}>{l}</button>
											))}
										</div>
									</div>
									<p className="verdict tiny">
										{trueMix > 0.55
											? <>True relative orbits. Innermost is <b>{fmt((aMin / aMax) * 100, 1)}%</b> of the outermost.</>
											: <>Compressed so all orbits stay legible — they really differ by <b>{fmt(aMax / aMin, 1)}×</b>.</>}
									</p>

									<span className="secthead">Archive · NASA</span>
									<Facts rows={[
										["distance", `${fmt(sys.sy_dist, 3)} pc / ${fmt((sys.sy_dist || 0) * PC_IN_LY, 2)} ly`],
										["RA / Dec", `${fmt(sys.ra, 4)}° ${fmt(sys.dec, 4)}°`],
										["spectral type", `${sys.st_spectype || "—"} · ${fmt(sys.st_teff, 0)} K`],
										["radius / mass", `${fmt(sys.st_rad, 3)} R☉ · ${fmt(sys.st_mass, 3)} M☉`],
										["luminosity", hz ? `${hz.L < 0.01 ? hz.L.toExponential(2) : fmt(hz.L, 3)} L☉` : "—"],
										["habitable zone", hz ? `${fmt(hz.inner, 3)} – ${fmt(hz.outer, 3)} AU` : "—"],
										["V magnitude", fmt(sys.sy_vmag, 2)],
										["planets", String((sys.planets || []).length)],
									]} />

									<span className="secthead">Dossier · yours</span>
									<Field label="Imperial name" value={R.imperialName} placeholder="unnamed"
										onChange={(v) => setRecord(sys.id, "imperialName", v)} />
									<label className="fld">
										<span className="eyebrow">Galactic arm</span>
										<select className="field" value={R.arm} onChange={(e) => setRecord(sys.id, "arm", e.target.value)}>
											<option value="">unassigned</option>
											{ARMS.map((a) => <option key={a.id} value={a.id}>{a.name}{a.sub ? ` — ${a.sub}` : ""}</option>)}
										</select>
										<span className="hint mono tiny dim">
											Colours the cube when colour is set to <b>arm</b>. Everything inside ~1 kpc of Sol is really
											in Orion–Cygnus; the rest of the list is for systems of your own.
										</span>
									</label>
									<Field label="Population" value={R.population} placeholder="e.g. 4.1 billion, 12 stations, uninhabited"
										onChange={(v) => setRecord(sys.id, "population", v)} />
									<Field label="Notes" area value={R.notes}
										placeholder="#tags become filters. [[System name]] draws a link in the cube."
										onChange={(v) => setRecord(sys.id, "notes", v)} />

									{(withLinks.find((s) => s.id === sys.id)?.links || []).length > 0 && (
										<div className="linkrow">
											<span className="eyebrow">Links</span>
											{withLinks.find((s) => s.id === sys.id).links.map((l) => (
												<button key={l} className="chip" onClick={() => { setSelected(l); setFocusPlanet(null); }}>
													{rec(systems.find((s) => s.id === l)).imperialName || systems.find((s) => s.id === l)?.hostname}
												</button>
											))}
										</div>
									)}

									{dist !== null && cmp && (
										<div className="measure">
											<span className="eyebrow">Measured separation</span>
											<div className="mbig mono">{fmt(dist, 3)} pc</div>
											<div className="mono tiny">{sys.hostname} ↔ {cmp.hostname}</div>
											<ul className="mono tiny dim mlist">
												<li>{fmt(dist * PC_IN_LY, 3)} light years</li>
												<li>{(dist * PC_IN_AU).toExponential(2)} AU</li>
												<li>{fmt((dist * PC_IN_LY) / (VOYAGER_KMS / C_KMS), 0)} years at Voyager 1's speed</li>
											</ul>
										</div>
									)}
								</>
							)}

							{/* -------------------------------------------- PLANET VIEW -- */}
							{planet && (() => {
								const a = planetAxis(planet, sys);
								const derivedA = !planet.pl_orbsmax && a;
								const S = hz && a ? hz.L / (a * a) : null;
								const teq = planet.pl_eqt || (hz && a ? 255 * Math.pow(hz.L, 0.25) / Math.sqrt(a) : null);
								const inHZ = hz && a && a >= hz.inner && a <= hz.outer;
								return (
									<>
										<h3 className="plname">
											{PR.imperialName || planet.pl_name}
											{inHZ && <span className="hzflag">habitable zone</span>}
										</h3>
										{PR.imperialName && <div className="mono tiny dim">{planet.pl_name}</div>}

										<span className="secthead">Archive · NASA</span>
										<Facts rows={[
											["class", planetClass(planet.pl_rade)],
											["semi-major axis", `${fmt(a, a && a < 0.1 ? 4 : 3)} AU${derivedA ? " *" : ""}`],
											["orbital period", `${fmt(planet.pl_orbper, planet.pl_orbper > 100 ? 1 : 3)} days`],
											["radius", `${fmt(planet.pl_rade, 3)} R⊕`],
											["mass", `${fmt(planet.pl_bmasse, 3)} M⊕`],
											["eccentricity", fmt(planet.pl_orbeccen, 3)],
											["insolation", S ? `${fmt(S, 2)} S⊕` : "—"],
											["equilibrium temp", teq ? `${fmt(teq, 0)} K${planet.pl_eqt ? "" : " *"}` : "—"],
											["discovery", `${planet.discoverymethod || "—"}${planet.disc_year ? `, ${planet.disc_year}` : ""}`],
											["facility", planet.disc_facility || "—"],
										]} />
										<p className="footnote mono tiny">
											* derived, not measured — axis from Kepler's third law, temperature from luminosity at albedo 0.3.
										</p>

										<span className="secthead">Dossier · yours</span>
										<Field label="Imperial name" value={PR.imperialName} placeholder="unnamed"
											onChange={(v) => setPRecord(sys.id, planet.pl_name, "imperialName", v)} />
										<Field label="Population" value={PR.population} placeholder="e.g. 900 million, orbital only, none"
											onChange={(v) => setPRecord(sys.id, planet.pl_name, "population", v)} />
										<Field label="Continents" value={PR.continents} placeholder="comma separated"
											onChange={(v) => setPRecord(sys.id, planet.pl_name, "continents", v)}
											hint={PR.continents ? `${PR.continents.split(",").filter((x) => x.trim()).length} listed` : null} />
										<Field label="Notes" area value={PR.notes}
											placeholder="#tags and [[links]] work here too."
											onChange={(v) => setPRecord(sys.id, planet.pl_name, "notes", v)} />
									</>
								);
							})()}
						</>
					)}
				</aside>
			</main>

			{/* ---------------------------------------------------- SCALE STRIP -- */}
			<footer className="strip" onClick={() => setScaleOpen(true)} role="button" tabIndex={0}
				onKeyDown={(e) => e.key === "Enter" && setScaleOpen(true)}>
				<div className="strip-cell">
					<span className="eyebrow">cube</span>
					<span className={`badge ${distMode === "linear" ? "true" : "comp"}`}>{distMode === "linear" ? "true" : "log-radial"}</span>
					<span className="mono tiny">1 pc = {fmt(pxPerPc, 1)} px</span>
				</div>
				<div className="strip-cell">
					<span className="eyebrow">orbits</span>
					<span className={`badge ${scaleMode === "true" ? "true" : "comp"}`}>{scaleMode === "true" ? "true" : "compressed"}</span>
					<span className="mono tiny">outermost {fmt(aMax, aMax < 0.1 ? 3 : 2)} AU</span>
				</div>
				<div className="strip-cell grow">
					<span className="eyebrow">honest number</span>
					<span className="mono tiny">
						drawn truthfully in the cube, {sys?.hostname} would be <b>{trueWidthPx.toExponential(1)} px</b> wide —
						the orreries exaggerate it <b>{(34 / Math.max(1e-12, trueWidthPx)).toExponential(1)}×</b>
					</span>
				</div>
				<span className="strip-more mono tiny">why ↗</span>
			</footer>

			{status && <div className="toast" onClick={() => setStatus(null)}>{status}</div>}

			{/* ---------------------------------------------------- SCALE SHEET -- */}
			{scaleOpen && (
				<div className="sheet-bg" onClick={() => setScaleOpen(false)}>
					<div className="sheet" onClick={(e) => e.stopPropagation()}>
						<div className="col-head">
							<span className="eyebrow">How scale works here</span>
							<button className="btn ghost tiny-btn" onClick={() => setScaleOpen(false)}>Close</button>
						</div>
						<p className="lede">
							One scale cannot hold a galaxy and a planet at once — the gap is about seventeen orders of magnitude.
							Parallax uses four, and labels which one you are looking at wherever it matters.
						</p>

						<div className="rung">
							<span className="badge true">true</span>
							<h3>Where systems sit — the cube</h3>
							<p>
								Real right ascension, declination and distance, converted to Cartesian parsecs with the Sun at the
								origin. Shift-click any two stars and the number you get is the real three-dimensional separation,
								never a projection. Currently {fmt(pxPerPc, 1)} px per parsec.
							</p>
							<p className="dim">
								The camera pivots on whatever you last clicked, so rotating and zooming both act on that system
								rather than on the Sun. <b>Focus</b> pulls in until its orbits are readable; <b>Fit all</b> pushes
								back out to the whole cube. Saving a system re-frames automatically, so nothing you add lands off-screen.
							</p>
						</div>

						<div className="rung">
							<span className="badge comp">optional</span>
							<h3>Distance compression — log-radial</h3>
							<p>
								Add something at 200 pc and the local neighbourhood collapses to a dot, because it truthfully is one.
								Switching distance to <b>log</b> replaces each radius r with ln(1 + r), keeping every direction exact
								while pulling far systems inward. Near stars barely move; distant ones come into frame. The strip
								below flags it whenever it is on, and measurements stay true regardless.
							</p>
						</div>

						<div className="rung">
							<span className="badge comp">compressed</span>
							<h3>Orbits — inside every system</h3>
							<p>
								The orreries in the cube and in the record panel both place orbits on a log radius, so a 0.02 AU
								orbit and a 30 AU orbit share one frame. Planets move at their real relative rates from a shared
								clock, so a TRAPPIST-1 planet really does lap a Neptune thousands of times.
								Set orbit scale to <b>true</b> in the record panel and watch the inner planets fall into the star —
								the size of that collapse is the size of the distortion.
							</p>
						</div>

						<div className="rung">
							<span className="badge symb">symbolic</span>
							<h3>Bodies — always exaggerated</h3>
							<p>
								Star and planet discs are never to scale, in any mode. For {sys?.hostname} the star's radius is about{" "}
								{sys?.st_rad ? ((sys.st_rad * RSUN_IN_AU / aMax) * 100).toFixed(3) : "—"}% of the outermost orbit,
								and an Earth-sized planet about {((REARTH_IN_AU / aMax) * 100).toExponential(1)}%. Drawn faithfully
								they would be well under a pixel. Disc area encodes radius on a log scale, and colour encodes either
								surface temperature or the arm you assigned.
							</p>
						</div>

						<div className="rung">
							<h3>Sources and method</h3>
							<p className="mono tiny">
								Data: NASA Exoplanet Archive, Planetary Systems Composite Parameters (pscomppars), via TAP.
								<br />Position: x = d·cos δ·cos α, y = d·cos δ·sin α, z = d·sin δ.
								<br />Luminosity: L = R²·(T/5772)⁴. Habitable zone: conservative limits at S = 1.10 and 0.53 S⊕.
								<br />Missing axes from Kepler's third law; missing T_eq from luminosity at albedo 0.3.
								<br />Arm assignment is yours, not the archive's. 1 pc = 3.26156 ly = 206,265 AU.
							</p>
						</div>
					</div>
				</div>
			)}
		</div>
	);
}

/* ==================================================================== CSS */
const CSS = `
@import url('https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@400;500;600&family=IBM+Plex+Sans+Condensed:wght@400;500;600;700&display=swap');

.px{
  --plate:#E4E4DA; --deep:#DBDBD0; --sunk:#D0D0C4;
  --ink:#16181A; --soft:#6D7276; --dim:#8A8F91;
  --rule:#B3B4A6; --hair:#C6C7BA;
  --acc:#1F3F9E; --acc-soft:rgba(31,63,158,0.1);
  --warn:#8E3218;
  --ui:'IBM Plex Sans Condensed','IBM Plex Sans',system-ui,sans-serif;
  --mono:'IBM Plex Mono',ui-monospace,monospace;
  position:fixed;inset:0;display:flex;flex-direction:column;
  background:var(--plate);color:var(--ink);font-family:var(--ui);
  font-size:14px;line-height:1.45;overflow:hidden;
}
.px.neg{
  --plate:#0B0D10; --deep:#101317; --sunk:#171B20;
  --ink:#E7E8E3; --soft:#9AA1A8; --dim:#767E86;
  --rule:#2B3037; --hair:#22262C;
  --acc:#7FA0FF; --acc-soft:rgba(127,160,255,0.13);
  --warn:#E08363;
}
.px *{box-sizing:border-box}
.px button,.px input,.px textarea,.px select{font-family:inherit;color:inherit}
.px :focus-visible{outline:2px solid var(--acc);outline-offset:2px}

.eyebrow{font:600 9.5px/1 var(--ui);letter-spacing:.16em;text-transform:uppercase;color:var(--soft);display:block}
.secthead{font:600 9.5px/1 var(--ui);letter-spacing:.16em;text-transform:uppercase;color:var(--ink);
  display:block;margin:16px 0 6px;padding-bottom:5px;border-bottom:1px solid var(--rule)}
.mono{font-family:var(--mono);font-variant-numeric:tabular-nums}
.tiny{font-size:10.5px;line-height:1.5}
.dim{color:var(--dim)}
.grow{flex:1;min-width:0}
.row-gap{display:flex;gap:6px}

.bar{display:flex;align-items:center;gap:18px;padding:0 16px;height:50px;
  border-bottom:1px solid var(--rule);background:var(--deep);flex:none}
.brand{display:flex;align-items:baseline;gap:9px}
.mark{width:9px;height:9px;border-radius:50%;background:var(--ink);
  box-shadow:0 0 0 3px var(--plate),0 0 0 4px var(--rule);align-self:center;flex:none}
.wordmark{font:700 17px/1 var(--ui);letter-spacing:.13em;text-transform:uppercase}
.tagline{font:400 11px/1 var(--ui);color:var(--soft)}
.bar-stats{margin-left:auto;color:var(--soft)}
.bar-actions{display:flex;gap:6px}

.btn{border:1px solid var(--rule);background:transparent;padding:5px 11px;
  font:600 10.5px/1.4 var(--ui);letter-spacing:.09em;text-transform:uppercase;
  cursor:pointer;border-radius:2px;transition:background .12s,border-color .12s;
  text-decoration:none;display:inline-block;white-space:nowrap}
.btn:hover{background:var(--sunk);border-color:var(--soft)}
.btn.solid{background:var(--ink);color:var(--plate);border-color:var(--ink)}
.btn.solid:hover{opacity:.85;background:var(--ink)}
.btn.solid:disabled{opacity:.35;cursor:default}
.btn.danger:hover{border-color:var(--warn);color:var(--warn)}
.tiny-btn{padding:3px 8px;font-size:9.5px}

.field{width:100%;background:var(--plate);border:1px solid var(--rule);border-radius:2px;
  padding:7px 9px;font-size:12.5px;font-family:var(--mono)}
.field:focus{border-color:var(--acc);outline:none}
.field::placeholder{color:var(--dim)}
.ta{min-height:80px;resize:vertical;font-size:11px;margin:8px 0}
.fld{display:block;margin-top:11px}
.fld .field{margin-top:4px}
.hint{display:block;margin-top:4px;line-height:1.5}
.hint b{color:var(--soft)}

.mobile-tabs{display:none}
.grid{flex:1;display:grid;grid-template-columns:262px minmax(0,1fr) 336px;min-height:0}
.col{min-height:0;display:flex;flex-direction:column;overflow-y:auto;padding:12px;gap:8px}
.vault{border-right:1px solid var(--rule);background:var(--deep)}
.note{border-left:1px solid var(--rule);background:var(--deep);gap:0}
.centre{padding:12px 14px;gap:10px}
.col-head{display:flex;align-items:center;justify-content:space-between;gap:8px;flex:none}

.addbox{border:1px solid var(--rule);border-radius:2px;padding:10px;display:flex;flex-direction:column;gap:8px;background:var(--plate)}
.lede{font-size:12.5px;color:var(--soft);margin:0}
.searchrow{display:flex;gap:6px;align-items:center;flex-wrap:wrap}
.tagrow,.quickrow,.linkrow{display:flex;flex-wrap:wrap;gap:4px;align-items:center}
.chip{border:1px solid var(--hair);background:transparent;border-radius:20px;padding:2px 8px;
  font:500 10px/1.5 var(--mono);color:var(--soft);cursor:pointer}
.chip:hover{border-color:var(--acc);color:var(--acc)}
.chip.on{background:var(--acc);border-color:var(--acc);color:var(--plate)}
.results{display:flex;flex-direction:column;gap:1px}
.rrow{display:flex;align-items:center;gap:9px;padding:7px;border:1px solid var(--hair);border-radius:2px}
.notice{border:1px solid var(--rule);border-left-width:2px;border-left-color:var(--warn);
  padding:8px 10px;border-radius:2px;color:var(--soft)}
.fallback summary{cursor:pointer;font:600 9.5px/1.4 var(--ui);letter-spacing:.13em;text-transform:uppercase;color:var(--soft)}

.vault-list{display:flex;flex-direction:column;gap:1px}
.vrow{display:flex;align-items:center;gap:9px;padding:7px 8px;border:1px solid transparent;
  background:transparent;cursor:pointer;text-align:left;border-radius:2px}
.vrow:hover{background:var(--sunk)}
.vrow.on{background:var(--acc-soft);border-color:var(--acc)}
.vrow.cmp{border-style:dashed;border-color:var(--acc)}
.vrow-txt{min-width:0;flex:1}
.vrow-name{font:600 12.5px/1.3 var(--ui);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.here{font:500 8.5px/1 var(--mono);letter-spacing:.1em;text-transform:uppercase;color:var(--soft);margin-left:6px}
.armdot{width:7px;height:7px;border-radius:50%;flex:none}
.empty{color:var(--dim);font-size:12px;padding:14px 4px}

.seg{display:flex;border:1px solid var(--rule);border-radius:2px;overflow:hidden;width:fit-content;flex:none}
.segbtn{background:transparent;border:0;border-right:1px solid var(--rule);padding:5px 12px;
  font:600 10px/1.4 var(--ui);letter-spacing:.11em;text-transform:uppercase;cursor:pointer;color:var(--soft)}
.segbtn:last-child{border-right:0}
.segbtn:hover{background:var(--sunk);color:var(--ink)}
.segbtn.on{background:var(--ink);color:var(--plate)}
.seg.small .segbtn{padding:4px 9px;font-size:9.5px}

.cube-wrap{position:relative;flex:1;min-height:320px;border:1px solid var(--rule);
  border-radius:2px;overflow:hidden;background:var(--plate)}
.cube-canvas{width:100%;height:100%;display:block;touch-action:none}
.cube-corner{position:absolute;left:11px;top:10px;pointer-events:none;display:flex;flex-direction:column;gap:2px}
.cube-corner .tiny{color:var(--soft)}
.centred{color:var(--acc);margin-top:3px}
.inline-badge{margin-left:6px}
.legend{position:absolute;left:11px;bottom:11px;display:flex;flex-direction:column;gap:2px;
  background:var(--deep);border:1px solid var(--rule);border-radius:2px;padding:7px 9px;pointer-events:none}
.lg{display:flex;align-items:center;gap:6px;font:500 10px/1.5 var(--mono);color:var(--soft)}
.lg i{width:7px;height:7px;border-radius:50%;flex:none}
.cube-peek{position:absolute;right:11px;bottom:11px;display:flex;gap:10px;align-items:center;
  background:var(--deep);border:1px solid var(--rule);border-radius:2px;padding:8px 11px;pointer-events:none}
.peek-name{font:600 13px/1.3 var(--ui)}

.ctrls{display:flex;align-items:flex-end;gap:16px;flex-wrap:wrap;flex:none}
.ctrl{display:flex;flex-direction:column;gap:4px}
.rangerow{display:flex;align-items:center;gap:8px}
.rangerow input[type=range]{width:120px;accent-color:var(--acc)}
.chkline{display:flex;align-items:center;gap:5px;cursor:pointer}
.chkline input{accent-color:var(--acc)}

.sysname{font:700 21px/1.15 var(--ui);margin:6px 0 2px}
.plname{font:700 17px/1.2 var(--ui);margin:12px 0 2px;display:flex;align-items:center;gap:8px;flex-wrap:wrap}
.srcline{margin-bottom:8px}
.src{border:1px solid var(--hair);padding:1px 6px;border-radius:2px;letter-spacing:.06em}
.src.nasa{border-color:var(--acc);color:var(--acc)}

.entities{display:flex;flex-wrap:wrap;gap:3px;padding:8px 0;border-top:1px solid var(--hair);border-bottom:1px solid var(--hair)}
.ent{border:1px solid var(--hair);background:transparent;border-radius:2px;padding:3px 9px;
  font:500 10.5px/1.5 var(--mono);color:var(--soft);cursor:pointer}
.ent:hover{border-color:var(--acc);color:var(--acc)}
.ent.on{background:var(--ink);border-color:var(--ink);color:var(--plate)}

.preview{display:flex;justify-content:center;padding:12px 0 4px}
.orrctl{display:flex;align-items:center;gap:9px;flex-wrap:wrap}
.verdict{color:var(--soft);margin:6px 0 0;border-left:2px solid var(--rule);padding:5px 10px}
.verdict b{font-family:var(--mono);color:var(--ink)}

.facts{display:flex;flex-direction:column;font-size:11px;margin:0}
.facts>div{display:flex;justify-content:space-between;gap:10px;padding:4px 0;border-bottom:1px solid var(--hair)}
.facts dt{color:var(--soft);white-space:nowrap}
.facts dd{margin:0;text-align:right}
.footnote{color:var(--dim);margin:6px 0 0}
.hzflag{font:600 8.5px/1.7 var(--mono);letter-spacing:.1em;text-transform:uppercase;color:var(--acc);
  border:1px solid var(--acc);padding:0 5px;border-radius:2px}

.measure{border:1px solid var(--acc);border-radius:2px;padding:10px 12px;margin-top:16px;background:var(--acc-soft)}
.mbig{font-size:21px;font-weight:600;margin:2px 0}
.mlist{list-style:none;padding:0;margin:7px 0 0}
.notes{min-height:120px;line-height:1.65}
.linkrow{margin-top:11px}

.strip{display:flex;align-items:center;gap:22px;padding:0 16px;height:46px;flex:none;
  border-top:1px solid var(--rule);background:var(--deep);cursor:pointer;overflow-x:auto}
.strip:hover{background:var(--sunk)}
.strip-cell{display:flex;align-items:center;gap:7px;white-space:nowrap}
.strip-cell .eyebrow{display:inline}
.strip-more{color:var(--acc);margin-left:auto;font-weight:600;letter-spacing:.1em;text-transform:uppercase}
.badge{font:600 8.5px/1.6 var(--mono);letter-spacing:.13em;text-transform:uppercase;
  padding:1px 6px;border-radius:2px;border:1px solid currentColor}
.badge.true{color:var(--acc)}
.badge.comp{color:var(--warn)}
.badge.symb{color:var(--soft)}

.toast{position:absolute;left:50%;bottom:60px;transform:translateX(-50%);z-index:40;
  background:var(--ink);color:var(--plate);padding:9px 15px;border-radius:2px;font-size:12px;cursor:pointer;max-width:80vw}
.sheet-bg{position:absolute;inset:0;background:rgba(10,11,9,.42);display:flex;
  justify-content:center;align-items:flex-start;padding:34px 16px;z-index:50;overflow-y:auto}
.px.neg .sheet-bg{background:rgba(0,0,0,.66)}
.sheet{background:var(--plate);border:1px solid var(--rule);border-radius:3px;
  max-width:660px;width:100%;padding:20px 24px 26px;display:flex;flex-direction:column;gap:14px}
.rung{border-top:1px solid var(--hair);padding-top:12px;display:flex;flex-direction:column;gap:5px}
.rung h3{font:600 14px/1.3 var(--ui);margin:2px 0 0}
.rung p{margin:0;font-size:12.5px;color:var(--soft);max-width:64ch}
.rung .badge{align-self:flex-start}
.rung b{color:var(--ink);font-weight:600}

@media (max-width:960px){
  .grid{grid-template-columns:1fr}
  .col{display:none;padding:11px}
  .col.show{display:flex}
  .vault,.note{border:0}
  .mobile-tabs{display:flex;border-bottom:1px solid var(--rule);background:var(--deep);flex:none}
  .mtab{flex:1;background:transparent;border:0;border-right:1px solid var(--rule);padding:9px;
    font:600 10px/1.4 var(--ui);letter-spacing:.14em;text-transform:uppercase;color:var(--soft);cursor:pointer}
  .mtab.on{background:var(--ink);color:var(--plate)}
  .bar-stats,.tagline{display:none}
  .strip{gap:14px}
  .strip-cell.grow{min-width:260px}
  .sheet{padding:16px}
}
@media (prefers-reduced-motion:reduce){.px *{transition:none!important}}
`;
