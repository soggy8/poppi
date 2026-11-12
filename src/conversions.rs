//! Programmer base conversions, CSS color conversions (with RGBA swatches),
//! and timezone conversions.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversion {
    pub expression: String,
    pub result: String,
    /// `(r, g, b, a)` for color results, rendered as a swatch in the row.
    pub swatch: Option<(u8, u8, u8, u8)>,
}

pub fn evaluate(query: &str) -> Option<Conversion> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    evaluate_at(query, now)
}

pub fn evaluate_at(query: &str, now_unix: i64) -> Option<Conversion> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return None;
    }
    convert_color(trimmed)
        .or_else(|| convert_base(trimmed))
        .or_else(|| convert_timezone_at(trimmed, now_unix))
}

fn split_conversion(input: &str) -> Option<(&str, &str)> {
    for sep in [" in ", " to ", " as ", " -> ", " = "] {
        // Case-insensitive split on the last separator word.
        let lower = input.to_lowercase();
        if let Some(pos) = lower.rfind(sep) {
            let left = input[..pos].trim();
            let right = input[pos + sep.len()..].trim();
            if !left.is_empty() && !right.is_empty() {
                return Some((left, right));
            }
        }
    }
    None
}

// ---------- Programmer bases ----------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Base {
    Bin,
    Oct,
    Dec,
    Hex,
}

fn parse_target_base(name: &str) -> Option<Base> {
    match name.trim().to_lowercase().as_str() {
        "bin" | "binary" | "base2" => Some(Base::Bin),
        "oct" | "octal" | "base8" => Some(Base::Oct),
        "dec" | "decimal" | "base10" => Some(Base::Dec),
        "hex" | "hexadecimal" | "base16" => Some(Base::Hex),
        _ => None,
    }
}

fn parse_int_literal(text: &str) -> Option<(i128, Base)> {
    let text = text.trim();
    let (neg, rest) = match text.strip_prefix('-') {
        Some(r) => (true, r.trim_start()),
        None => (false, text.strip_prefix('+').unwrap_or(text).trim_start()),
    };
    let clean: String = rest.chars().filter(|&c| c != '_').collect();
    let (mag, base) = if let Some(hex) = clean
        .strip_prefix("0x")
        .or_else(|| clean.strip_prefix("0X"))
    {
        (i128::from_str_radix(hex, 16).ok()?, Base::Hex)
    } else if let Some(bin) = clean
        .strip_prefix("0b")
        .or_else(|| clean.strip_prefix("0B"))
    {
        (i128::from_str_radix(bin, 2).ok()?, Base::Bin)
    } else if let Some(oct) = clean
        .strip_prefix("0o")
        .or_else(|| clean.strip_prefix("0O"))
    {
        (i128::from_str_radix(oct, 8).ok()?, Base::Oct)
    } else {
        (clean.parse::<i128>().ok()?, Base::Dec)
    };
    Some((if neg { -mag } else { mag }, base))
}

fn format_in_base(value: i128, base: Base) -> String {
    let neg = value < 0;
    let mag = value.unsigned_abs();
    let sign = if neg { "-" } else { "" };
    match base {
        Base::Bin => format!("{}0b{:b}", sign, mag),
        Base::Oct => format!("{}0o{:o}", sign, mag),
        Base::Dec => format!("{}{}", sign, mag),
        Base::Hex => format!("{}0x{:x}", sign, mag),
    }
}

fn convert_base(query: &str) -> Option<Conversion> {
    if let Some((left, right)) = split_conversion(query) {
        let target = parse_target_base(right)?;
        let (value, _) = parse_int_literal(left)?;
        return Some(Conversion {
            expression: query.to_string(),
            result: format_in_base(value, target),
            swatch: None,
        });
    }

    // Bare prefixed integer (`0xff`, `0b101010`, `0o755`) shows decimal and siblings.
    if !query.contains(char::is_whitespace) {
        let lower = query.strip_prefix('-').unwrap_or(query);
        if lower.starts_with("0x")
            || lower.starts_with("0X")
            || lower.starts_with("0b")
            || lower.starts_with("0B")
            || lower.starts_with("0o")
            || lower.starts_with("0O")
        {
            let (value, source_base) = parse_int_literal(query)?;
            let result = format_in_base(value, Base::Dec);
            let mut others = vec![query.to_string()];
            for b in [Base::Hex, Base::Bin, Base::Oct] {
                if b != source_base {
                    others.push(format_in_base(value, b));
                }
            }
            return Some(Conversion {
                expression: others.join("  ·  "),
                result,
                swatch: None,
            });
        }
    }
    None
}

// ---------- Colors ----------

#[derive(Clone, Copy, PartialEq, Eq)]
enum ColorFormat {
    Hex,
    Rgb,
    Hsl,
}

fn parse_color_format(name: &str) -> Option<ColorFormat> {
    match name.trim().to_lowercase().as_str() {
        "hex" => Some(ColorFormat::Hex),
        "rgb" | "rgba" => Some(ColorFormat::Rgb),
        "hsl" | "hsla" => Some(ColorFormat::Hsl),
        _ => None,
    }
}

fn parse_hex_color(s: &str) -> Option<(u8, u8, u8, u8)> {
    let hex = s.strip_prefix('#')?;
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |slice: &str| u8::from_str_radix(slice, 16).ok();
    let nibble = |slice: &str| byte(slice).map(|v| v * 17);
    match hex.len() {
        3 => Some((
            nibble(&hex[0..1])?,
            nibble(&hex[1..2])?,
            nibble(&hex[2..3])?,
            255,
        )),
        4 => Some((
            nibble(&hex[0..1])?,
            nibble(&hex[1..2])?,
            nibble(&hex[2..3])?,
            nibble(&hex[3..4])?,
        )),
        6 => Some((byte(&hex[0..2])?, byte(&hex[2..4])?, byte(&hex[4..6])?, 255)),
        8 => Some((
            byte(&hex[0..2])?,
            byte(&hex[2..4])?,
            byte(&hex[4..6])?,
            byte(&hex[6..8])?,
        )),
        _ => None,
    }
}

fn parse_alpha(s: &str) -> Option<u8> {
    let s = s.trim();
    if let Some(pct) = s.strip_suffix('%') {
        let v: f64 = pct.trim().parse().ok()?;
        return (0.0..=100.0)
            .contains(&v)
            .then(|| (v / 100.0 * 255.0).round() as u8);
    }
    let v: f64 = s.parse().ok()?;
    (0.0..=1.0).contains(&v).then(|| (v * 255.0).round() as u8)
}

fn parse_rgb_fn(s: &str) -> Option<(u8, u8, u8, u8)> {
    let lower = s.trim().to_lowercase();
    let inner = lower
        .strip_prefix("rgba(")
        .or_else(|| lower.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let parts: Vec<&str> = inner
        .split([',', '/'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let chan = |p: &str| -> Option<u8> {
        let v: u16 = p.parse().ok()?;
        (v <= 255).then_some(v as u8)
    };
    let a = if parts.len() == 4 {
        parse_alpha(parts[3])?
    } else {
        255
    };
    Some((chan(parts[0])?, chan(parts[1])?, chan(parts[2])?, a))
}

fn parse_hsl_fn(s: &str) -> Option<(u8, u8, u8, u8)> {
    let lower = s.trim().to_lowercase();
    let inner = lower
        .strip_prefix("hsla(")
        .or_else(|| lower.strip_prefix("hsl("))?
        .strip_suffix(')')?;
    let parts: Vec<&str> = inner
        .split([',', '/'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let h: f64 = parts[0].trim_end_matches("deg").trim().parse().ok()?;
    let sat: f64 = parts[1].strip_suffix('%')?.trim().parse().ok()?;
    let light: f64 = parts[2].strip_suffix('%')?.trim().parse().ok()?;
    if !(0.0..=100.0).contains(&sat) || !(0.0..=100.0).contains(&light) {
        return None;
    }
    let a = if parts.len() == 4 {
        parse_alpha(parts[3])?
    } else {
        255
    };
    let (r, g, b) = hsl_to_rgb(h.rem_euclid(360.0), sat / 100.0, light / 100.0);
    Some((r, g, b, a))
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r1, g1, b1) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |v: f64| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    (byte(r1), byte(g1), byte(b1))
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (u16, u8, u8) {
    let rf = f64::from(r) / 255.0;
    let gf = f64::from(g) / 255.0;
    let bf = f64::from(b) / 255.0;
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d.abs() < 1e-9 {
        return (0, 0, (l * 100.0).round() as u8);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if (max - rf).abs() < 1e-9 {
        60.0 * (((gf - bf) / d).rem_euclid(6.0))
    } else if (max - gf).abs() < 1e-9 {
        60.0 * (((bf - rf) / d) + 2.0)
    } else {
        60.0 * (((rf - gf) / d) + 4.0)
    };
    (
        (h.round() as u16) % 360,
        (s * 100.0).round() as u8,
        (l * 100.0).round() as u8,
    )
}

fn parse_color(s: &str) -> Option<((u8, u8, u8, u8), ColorFormat)> {
    if let Some(rgba) = parse_hex_color(s) {
        return Some((rgba, ColorFormat::Hex));
    }
    if let Some(rgba) = parse_rgb_fn(s) {
        return Some((rgba, ColorFormat::Rgb));
    }
    if let Some(rgba) = parse_hsl_fn(s) {
        return Some((rgba, ColorFormat::Hsl));
    }
    None
}

fn format_color((r, g, b, a): (u8, u8, u8, u8), format: ColorFormat) -> String {
    match format {
        ColorFormat::Hex if a == 255 => format!("#{:02x}{:02x}{:02x}", r, g, b),
        ColorFormat::Hex => format!("#{:02x}{:02x}{:02x}{:02x}", r, g, b, a),
        ColorFormat::Rgb if a == 255 => format!("rgb({}, {}, {})", r, g, b),
        ColorFormat::Rgb => {
            let alpha = (f64::from(a) / 255.0 * 100.0).round() / 100.0;
            format!("rgba({}, {}, {}, {})", r, g, b, alpha)
        }
        ColorFormat::Hsl => {
            let (h, s, l) = rgb_to_hsl(r, g, b);
            if a == 255 {
                format!("hsl({}, {}%, {}%)", h, s, l)
            } else {
                let alpha = (f64::from(a) / 255.0 * 100.0).round() / 100.0;
                format!("hsla({}, {}%, {}%, {})", h, s, l, alpha)
            }
        }
    }
}

fn convert_color(query: &str) -> Option<Conversion> {
    if let Some((left, right)) = split_conversion(query) {
        let target = parse_color_format(right)?;
        let (rgba, _) = parse_color(left)?;
        return Some(Conversion {
            expression: query.to_string(),
            result: format_color(rgba, target),
            swatch: Some(rgba),
        });
    }

    let (rgba, source) = parse_color(query)?;
    let hex = format_color(rgba, ColorFormat::Hex);
    let rgb = format_color(rgba, ColorFormat::Rgb);
    let hsl = format_color(rgba, ColorFormat::Hsl);
    let (result, subtitle) = match source {
        ColorFormat::Hex => (rgb, format!("{}  ·  {}", hex, hsl)),
        ColorFormat::Rgb => (hex, format!("{}  ·  {}", rgb, hsl)),
        ColorFormat::Hsl => (hex, format!("{}  ·  {}", hsl, rgb)),
    };
    Some(Conversion {
        expression: subtitle,
        result,
        swatch: Some(rgba),
    })
}

// ---------- Timezones (RFC 8536 TZif + standard abbreviations) ----------

const FIXED_ZONES: &[(&[&str], i32, &str)] = &[
    (&["utc", "gmt", "z"], 0, "UTC"),
    (&["bst"], 3600, "BST"),
    (&["cet", "wat"], 3600, "CET"),
    (&["cest", "cat"], 2 * 3600, "CEST"),
    (&["eet"], 2 * 3600, "EET"),
    (&["eest", "msk", "eat"], 3 * 3600, "EEST"),
    (&["gst"], 4 * 3600, "GST"),
    (&["pkt"], 5 * 3600, "PKT"),
    (&["ist"], 5 * 3600 + 1800, "IST"),
    (&["ict", "wib"], 7 * 3600, "ICT"),
    (&["sgt", "hkt", "awst"], 8 * 3600, "SGT"),
    (&["jst"], 9 * 3600, "JST"),
    (&["kst"], 9 * 3600, "KST"),
    (&["acst"], 9 * 3600 + 1800, "ACST"),
    (&["aest"], 10 * 3600, "AEST"),
    (&["aedt"], 11 * 3600, "AEDT"),
    (&["nzst"], 12 * 3600, "NZST"),
    (&["nzdt"], 13 * 3600, "NZDT"),
    (&["brt", "art"], -3 * 3600, "BRT"),
    (&["edt", "ast"], -4 * 3600, "EDT"),
    (&["est", "cdt"], -5 * 3600, "EST"),
    (&["cst", "mdt"], -6 * 3600, "CST"),
    (&["mst", "pdt"], -7 * 3600, "MST"),
    (&["pst", "akdt"], -8 * 3600, "PST"),
    (&["akst"], -9 * 3600, "AKST"),
    (&["hst"], -10 * 3600, "HST"),
];

const CITY_ZONES: &[(&[&str], &str)] = &[
    (&["london", "uk", "dublin", "lisbon"], "Europe/London"),
    (
        &[
            "paris",
            "berlin",
            "rome",
            "madrid",
            "amsterdam",
            "brussels",
            "vienna",
            "warsaw",
            "prague",
            "budapest",
            "stockholm",
            "oslo",
            "copenhagen",
            "zurich",
            "ljubljana",
            "zagreb",
            "belgrade",
        ],
        "Europe/Berlin",
    ),
    (
        &[
            "athens",
            "helsinki",
            "bucharest",
            "kyiv",
            "sofia",
            "riga",
            "tallinn",
            "vilnius",
        ],
        "Europe/Athens",
    ),
    (&["istanbul"], "Europe/Istanbul"),
    (&["moscow"], "Europe/Moscow"),
    (&["cairo"], "Africa/Cairo"),
    (&["johannesburg"], "Africa/Johannesburg"),
    (&["dubai"], "Asia/Dubai"),
    (
        &[
            "mumbai",
            "delhi",
            "newdelhi",
            "kolkata",
            "bangalore",
            "india",
        ],
        "Asia/Kolkata",
    ),
    (&["bangkok", "jakarta"], "Asia/Bangkok"),
    (&["singapore", "kualalumpur"], "Asia/Singapore"),
    (
        &["hongkong", "shanghai", "beijing", "taipei", "china"],
        "Asia/Shanghai",
    ),
    (&["tokyo", "osaka", "japan"], "Asia/Tokyo"),
    (&["seoul", "korea"], "Asia/Seoul"),
    (&["perth"], "Australia/Perth"),
    (&["sydney", "melbourne", "brisbane"], "Australia/Sydney"),
    (&["auckland", "nz", "newzealand"], "Pacific/Auckland"),
    (&["honolulu", "hawaii"], "Pacific/Honolulu"),
    (&["anchorage", "alaska"], "America/Anchorage"),
    (
        &[
            "losangeles",
            "la",
            "sanfrancisco",
            "sf",
            "seattle",
            "vancouver",
            "pacific",
        ],
        "America/Los_Angeles",
    ),
    (&["denver", "mountain"], "America/Denver"),
    (&["phoenix"], "America/Phoenix"),
    (
        &[
            "chicago",
            "dallas",
            "houston",
            "austin",
            "central",
            "mexicocity",
        ],
        "America/Chicago",
    ),
    (
        &[
            "newyork",
            "nyc",
            "boston",
            "miami",
            "washington",
            "toronto",
            "montreal",
            "eastern",
        ],
        "America/New_York",
    ),
    (&["saopaulo", "rio", "brazil"], "America/Sao_Paulo"),
    (
        &["buenosaires", "argentina"],
        "America/Argentina/Buenos_Aires",
    ),
];

fn be_u32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn be_i32(data: &[u8], offset: usize) -> Option<i32> {
    Some(i32::from_be_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn be_i64(data: &[u8], offset: usize) -> Option<i64> {
    Some(i64::from_be_bytes(
        data.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

/// Read `(utc_offset_seconds, abbreviation)` at `unix_secs` from a TZif binary.
fn parse_tzif_offset(data: &[u8], unix_secs: i64) -> Option<(i32, String)> {
    if data.get(0..4)? != b"TZif" {
        return None;
    }
    let version = *data.get(4)?;
    let header_counts = |hdr: usize| -> Option<(usize, usize, usize, usize, usize, usize)> {
        Some((
            be_u32(data, hdr + 20)? as usize,
            be_u32(data, hdr + 24)? as usize,
            be_u32(data, hdr + 28)? as usize,
            be_u32(data, hdr + 32)? as usize,
            be_u32(data, hdr + 36)? as usize,
            be_u32(data, hdr + 40)? as usize,
        ))
    };

    let (isut, isstd, leap, time1, type1, char1) = header_counts(0)?;
    let v1_len = time1 * 5 + type1 * 6 + char1 + leap * 8 + isstd + isut;
    let h2 = 44 + v1_len;

    let (pos, timecnt, typecnt, charcnt, wide) =
        if version >= b'2' && data.get(h2..h2 + 4) == Some(b"TZif") {
            let (_, _, _, time2, type2, char2) = header_counts(h2)?;
            (h2 + 44, time2, type2, char2, true)
        } else {
            (44, time1, type1, char1, false)
        };
    if typecnt == 0 {
        return None;
    }

    let step = if wide { 8 } else { 4 };
    let times_end = pos.checked_add(timecnt.checked_mul(step)?)?;
    let types_end = times_end.checked_add(timecnt)?;
    let ttinfo_end = types_end.checked_add(typecnt.checked_mul(6)?)?;
    let chars_end = ttinfo_end.checked_add(charcnt)?;
    if chars_end > data.len() {
        return None;
    }

    let read_time = |idx: usize| -> Option<i64> {
        let off = pos + idx * step;
        if wide {
            be_i64(data, off)
        } else {
            be_i32(data, off).map(i64::from)
        }
    };

    // Binary search for the last transition <= unix_secs.
    let mut lo = 0usize;
    let mut hi = timecnt;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if read_time(mid)? <= unix_secs {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }

    let type_idx = if lo > 0 {
        *data.get(types_end - timecnt + (lo - 1))? as usize
    } else {
        // Before the first recorded transition: pick the first standard-time entry.
        (0..typecnt)
            .find(|&i| data.get(types_end + i * 6 + 4) == Some(&0))
            .unwrap_or(0)
    };
    if type_idx >= typecnt {
        return None;
    }

    let tt = types_end + type_idx * 6;
    let utoff = be_i32(data, tt)?;
    let abbr_start = ttinfo_end + (*data.get(tt + 5)? as usize);
    let abbr_bytes = data.get(abbr_start..chars_end)?;
    let nul = abbr_bytes
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(abbr_bytes.len());
    let abbr = String::from_utf8_lossy(&abbr_bytes[..nul]).into_owned();
    Some((utoff, abbr))
}

fn read_zonefile(path: &Path, unix_secs: i64) -> Option<(i32, String)> {
    let data = std::fs::read(path).ok()?;
    parse_tzif_offset(&data, unix_secs)
}

fn parse_utc_offset(compact: &str) -> Option<(i32, String)> {
    let rest = compact
        .strip_prefix("utc")
        .or_else(|| compact.strip_prefix("gmt"))?;
    if rest.is_empty() {
        return Some((0, "UTC".to_string()));
    }
    let (sign, digits) = match rest.chars().next()? {
        '+' => (1, &rest[1..]),
        '-' => (-1, &rest[1..]),
        _ => return None,
    };
    let (h, m): (i32, i32) = match digits.split_once(':') {
        Some((hs, ms)) => (hs.parse().ok()?, ms.parse().ok()?),
        None => (digits.parse().ok()?, 0),
    };
    if h > 14 || m >= 60 {
        return None;
    }
    let secs = sign * (h * 3600 + m * 60);
    let label = if m == 0 {
        format!("UTC{}{}", if sign >= 0 { "+" } else { "-" }, h)
    } else {
        format!("UTC{}{}:{:02}", if sign >= 0 { "+" } else { "-" }, h, m)
    };
    Some((secs, label))
}

fn resolve_zone(raw: &str, unix_secs: i64) -> Option<(i32, String)> {
    let compact: String = raw
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect();
    if compact.is_empty() {
        return None;
    }
    if matches!(compact.as_str(), "local" | "here") {
        return read_zonefile(Path::new("/etc/localtime"), unix_secs).or(Some((0, "UTC".into())));
    }
    if let Some(off) = parse_utc_offset(&compact) {
        return Some(off);
    }
    if let Some((_, secs, abbr)) = FIXED_ZONES
        .iter()
        .find(|(names, ..)| names.contains(&compact.as_str()))
    {
        return Some((*secs, (*abbr).to_string()));
    }
    if let Some((_, iana)) = CITY_ZONES
        .iter()
        .find(|(names, ..)| names.contains(&compact.as_str()))
    {
        let path = Path::new("/usr/share/zoneinfo").join(iana);
        if let Some((secs, abbr)) = read_zonefile(&path, unix_secs) {
            let label = if abbr.starts_with('+') || abbr.starts_with('-') || abbr.is_empty() {
                iana.rsplit('/').next().unwrap_or(iana).replace('_', " ")
            } else {
                abbr
            };
            return Some((secs, label));
        }
    }
    // Direct IANA name such as `America/New_York`.
    let trimmed = raw.trim();
    if trimmed.contains('/') && !trimmed.contains("..") && !trimmed.starts_with('/') {
        let path = Path::new("/usr/share/zoneinfo").join(trimmed);
        if let Some(res) = read_zonefile(&path, unix_secs) {
            return Some(res);
        }
    }
    None
}

/// Parse `<hour>[:<minute>][am|pm]`. Requires either `:` or `am`/`pm` so bare
/// integers like `5` stay unit conversions (`5 km in miles`).
fn parse_clock_time(s: &str) -> Option<(u32, u32)> {
    let lower: String = s
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    let (meridiem, core) = if let Some(rest) = lower.strip_suffix("am") {
        (Some(false), rest)
    } else if let Some(rest) = lower.strip_suffix("pm") {
        (Some(true), rest)
    } else if lower.contains(':') {
        (None, lower.as_str())
    } else {
        return None;
    };

    let (h, m): (u32, u32) = match core.split_once(':') {
        Some((hs, ms)) => (hs.parse().ok()?, ms.parse().ok()?),
        None => (core.parse().ok()?, 0),
    };
    if m >= 60 {
        return None;
    }
    let hour24 = match meridiem {
        Some(false) => {
            if !(1..=12).contains(&h) {
                return None;
            }
            if h == 12 {
                0
            } else {
                h
            }
        }
        Some(true) => {
            if !(1..=12).contains(&h) {
                return None;
            }
            if h == 12 {
                12
            } else {
                h + 12
            }
        }
        None => {
            if h >= 24 {
                return None;
            }
            h
        }
    };
    Some((hour24, m))
}

fn split_time_and_zone(left: &str) -> Option<(&str, &str)> {
    let left = left.trim();
    // Handle `3:30 pm est` (space before am/pm) as well as `3pm est` and `14:30 utc`.
    let tokens: Vec<&str> = left.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }
    if tokens.len() >= 3 && matches!(tokens[1].to_lowercase().as_str(), "am" | "pm") {
        let after_first = left.find(tokens[1])? + tokens[1].len();
        return Some((left[..after_first].trim(), left[after_first..].trim()));
    }
    let (time_part, zone_part) = left.split_once(char::is_whitespace)?;
    Some((time_part.trim(), zone_part.trim()))
}

fn format_wall_clock(local_secs: i64, abbr: &str, day_diff: i64) -> String {
    let secs_in_day = local_secs.rem_euclid(86_400);
    let h = secs_in_day / 3600;
    let m = (secs_in_day % 3600) / 60;
    let suffix = match day_diff {
        0 => String::new(),
        1 => " (+1 day)".to_string(),
        -1 => " (-1 day)".to_string(),
        n if n > 0 => format!(" (+{} days)", n),
        n => format!(" ({} days)", n),
    };
    format!("{:02}:{:02} {}{}", h, m, abbr, suffix)
}

pub fn convert_timezone_at(query: &str, now_unix: i64) -> Option<Conversion> {
    let (left, right) = split_conversion(query)?;
    let left_lower = left.to_lowercase();

    if matches!(left_lower.as_str(), "now" | "time") {
        let (from_off, _) = resolve_zone("local", now_unix)?;
        let (to_off, to_abbr) = resolve_zone(right, now_unix)?;
        let from_local = now_unix + i64::from(from_off);
        let to_local = now_unix + i64::from(to_off);
        let day_diff = to_local.div_euclid(86_400) - from_local.div_euclid(86_400);
        return Some(Conversion {
            expression: query.to_string(),
            result: format_wall_clock(to_local, &to_abbr, day_diff),
            swatch: None,
        });
    }

    let (time_str, from_zone) = split_time_and_zone(left)?;
    let (hour, minute) = parse_clock_time(time_str)?;
    let (initial_from_off, _) = resolve_zone(from_zone, now_unix)?;
    let from_now_local = now_unix + i64::from(initial_from_off);
    let from_midnight = from_now_local.div_euclid(86_400) * 86_400;
    let target_local = from_midnight + i64::from(hour) * 3600 + i64::from(minute) * 60;
    let rough_unix = target_local - i64::from(initial_from_off);
    let (from_off, _) = resolve_zone(from_zone, rough_unix)?;
    let exact_unix = target_local - i64::from(from_off);

    let (to_off, to_abbr) = resolve_zone(right, exact_unix)?;
    let to_local = exact_unix + i64::from(to_off);
    let day_diff = to_local.div_euclid(86_400) - target_local.div_euclid(86_400);

    Some(Conversion {
        expression: query.to_string(),
        result: format_wall_clock(to_local, &to_abbr, day_diff),
        swatch: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_conversions_and_prefixed_literals() {
        assert_eq!(evaluate("255 in hex").unwrap().result, "0xff");
        assert_eq!(evaluate("255 to binary").unwrap().result, "0b11111111");
        assert_eq!(evaluate("0xff in dec").unwrap().result, "255");
        assert_eq!(evaluate("0b101010 to dec").unwrap().result, "42");
        assert_eq!(evaluate("0o755 in hex").unwrap().result, "0x1ed");
        assert_eq!(evaluate("-16 in hex").unwrap().result, "-0x10");

        let bare = evaluate("0xff").unwrap();
        assert_eq!(bare.result, "255");
        assert!(bare.expression.contains("0b11111111") && bare.expression.contains("0o377"));
        assert!(evaluate("2048").is_none());
    }

    #[test]
    fn color_conversions_and_swatches() {
        let hex = evaluate("#4a9eff").unwrap();
        assert_eq!(hex.result, "rgb(74, 158, 255)");
        assert_eq!(hex.swatch, Some((74, 158, 255, 255)));
        assert!(hex.expression.contains("hsl("));

        let to_hsl = evaluate("#4a9eff to hsl").unwrap();
        assert_eq!(to_hsl.result, "hsl(212, 100%, 65%)");
        assert_eq!(to_hsl.swatch, Some((74, 158, 255, 255)));

        let to_hex = evaluate("rgb(74, 158, 255) in hex").unwrap();
        assert_eq!(to_hex.result, "#4a9eff");

        let short = evaluate("#f08 to rgb").unwrap();
        assert_eq!(short.result, "rgb(255, 0, 136)");

        let hsl_back = evaluate("hsl(0, 100%, 50%) to hex").unwrap();
        assert_eq!(hsl_back.result, "#ff0000");

        assert!(evaluate("#hashtag").is_none());
    }

    #[test]
    fn timezone_conversions() {
        // 2026-11-12 12:00:00 UTC (winter: EST=-5, CET=+1, JST=+9)
        let winter = 1_794_484_800;
        assert_eq!(
            evaluate_at("3pm est in cet", winter).unwrap().result,
            "21:00 CET"
        );
        assert_eq!(
            evaluate_at("3:30 pm est to jst", winter).unwrap().result,
            "05:30 JST (+1 day)"
        );
        assert_eq!(
            evaluate_at("01:00 cet in est", winter).unwrap().result,
            "19:00 EST (-1 day)"
        );
        assert_eq!(
            evaluate_at("14:30 utc to utc+2", winter).unwrap().result,
            "16:30 UTC+2"
        );
        if Path::new("/usr/share/zoneinfo/Asia/Tokyo").exists() {
            assert_eq!(
                evaluate_at("12:00 utc in tokyo", winter).unwrap().result,
                "21:00 JST"
            );
            assert!(evaluate_at("now in tokyo", winter).is_some());
        }
        // Plain unit requests are not hijacked.
        assert!(evaluate_at("5 km in miles", winter).is_none());
        assert!(evaluate_at("100 usd in eur", winter).is_none());
    }
}
