//! Unit and currency conversion: `5 km in miles`, `100 f to c`, `20 usd in eur`.

use std::collections::HashMap;
use std::path::PathBuf;

/// A parsed `<amount> <from> in|to|as <to>` request. Unit names are lowercase.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub amount: f64,
    pub from: String,
    pub to: String,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Length,
    Mass,
    Volume,
    Data,
    Time,
    Speed,
    Area,
}

/// (names, kind, size in the kind's base unit, display symbol)
const UNITS: &[(&[&str], Kind, f64, &str)] = &[
    (&["mm", "millimeter", "millimeters", "millimetre", "millimetres"], Kind::Length, 0.001, "mm"),
    (&["cm", "centimeter", "centimeters", "centimetre", "centimetres"], Kind::Length, 0.01, "cm"),
    (&["m", "meter", "meters", "metre", "metres"], Kind::Length, 1.0, "m"),
    (&["km", "kilometer", "kilometers", "kilometre", "kilometres"], Kind::Length, 1000.0, "km"),
    (&["in", "inch", "inches", "\""], Kind::Length, 0.0254, "in"),
    (&["ft", "foot", "feet", "'"], Kind::Length, 0.3048, "ft"),
    (&["yd", "yard", "yards"], Kind::Length, 0.9144, "yd"),
    (&["mi", "mile", "miles"], Kind::Length, 1609.344, "mi"),
    (&["nmi", "nauticalmile", "nauticalmiles"], Kind::Length, 1852.0, "nmi"),
    (&["mg", "milligram", "milligrams"], Kind::Mass, 1e-6, "mg"),
    (&["g", "gram", "grams"], Kind::Mass, 0.001, "g"),
    (&["kg", "kilo", "kilos", "kilogram", "kilograms"], Kind::Mass, 1.0, "kg"),
    (&["t", "tonne", "tonnes"], Kind::Mass, 1000.0, "t"),
    (&["oz", "ounce", "ounces"], Kind::Mass, 0.028349523125, "oz"),
    (&["lb", "lbs", "pound", "pounds"], Kind::Mass, 0.45359237, "lb"),
    (&["st", "stone", "stones"], Kind::Mass, 6.35029318, "st"),
    (&["ml", "milliliter", "milliliters", "millilitre", "millilitres"], Kind::Volume, 0.001, "ml"),
    (&["cl", "centiliter", "centiliters"], Kind::Volume, 0.01, "cl"),
    (&["dl", "deciliter", "deciliters"], Kind::Volume, 0.1, "dl"),
    (&["l", "liter", "liters", "litre", "litres"], Kind::Volume, 1.0, "l"),
    (&["tsp", "teaspoon", "teaspoons"], Kind::Volume, 0.00492892159375, "tsp"),
    (&["tbsp", "tablespoon", "tablespoons"], Kind::Volume, 0.01478676478125, "tbsp"),
    (&["floz"], Kind::Volume, 0.0295735295625, "fl oz"),
    (&["cup", "cups"], Kind::Volume, 0.2365882365, "cups"),
    (&["pt", "pint", "pints"], Kind::Volume, 0.473176473, "pt"),
    (&["gal", "gallon", "gallons"], Kind::Volume, 3.785411784, "gal"),
    (&["bit", "bits"], Kind::Data, 0.125, "bit"),
    (&["b", "byte", "bytes"], Kind::Data, 1.0, "B"),
    (&["kb", "kilobyte", "kilobytes"], Kind::Data, 1e3, "kB"),
    (&["mb", "megabyte", "megabytes"], Kind::Data, 1e6, "MB"),
    (&["gb", "gigabyte", "gigabytes"], Kind::Data, 1e9, "GB"),
    (&["tb", "terabyte", "terabytes"], Kind::Data, 1e12, "TB"),
    (&["kib", "kibibyte", "kibibytes"], Kind::Data, 1024.0, "KiB"),
    (&["mib", "mebibyte", "mebibytes"], Kind::Data, 1_048_576.0, "MiB"),
    (&["gib", "gibibyte", "gibibytes"], Kind::Data, 1_073_741_824.0, "GiB"),
    (&["tib", "tebibyte", "tebibytes"], Kind::Data, 1_099_511_627_776.0, "TiB"),
    (&["ms", "millisecond", "milliseconds"], Kind::Time, 0.001, "ms"),
    (&["s", "sec", "secs", "second", "seconds"], Kind::Time, 1.0, "s"),
    (&["min", "mins", "minute", "minutes"], Kind::Time, 60.0, "min"),
    (&["h", "hr", "hrs", "hour", "hours"], Kind::Time, 3600.0, "h"),
    (&["d", "day", "days"], Kind::Time, 86_400.0, "days"),
    (&["wk", "week", "weeks"], Kind::Time, 604_800.0, "weeks"),
    (&["yr", "year", "years"], Kind::Time, 31_557_600.0, "years"),
    (&["m/s", "mps"], Kind::Speed, 1.0, "m/s"),
    (&["km/h", "kmh", "kph"], Kind::Speed, 1000.0 / 3600.0, "km/h"),
    (&["mph"], Kind::Speed, 0.44704, "mph"),
    (&["kn", "knot", "knots"], Kind::Speed, 1852.0 / 3600.0, "kn"),
    (&["m2", "sqm"], Kind::Area, 1.0, "m²"),
    (&["km2", "sqkm"], Kind::Area, 1e6, "km²"),
    (&["ft2", "sqft"], Kind::Area, 0.09290304, "ft²"),
    (&["ha", "hectare", "hectares"], Kind::Area, 1e4, "ha"),
    (&["acre", "acres"], Kind::Area, 4046.8564224, "acres"),
];

fn lookup(name: &str) -> Option<(Kind, f64, &'static str)> {
    UNITS
        .iter()
        .find(|(names, ..)| names.contains(&name))
        .map(|(_, kind, factor, symbol)| (*kind, *factor, *symbol))
}

fn temperature(name: &str) -> Option<char> {
    match name {
        "c" | "°c" | "celsius" => Some('c'),
        "f" | "°f" | "fahrenheit" => Some('f'),
        "k" | "kelvin" => Some('k'),
        _ => None,
    }
}

/// Recognise the shape of a conversion without knowing the units yet.
pub fn parse(query: &str) -> Option<Request> {
    let lower = query.trim().to_lowercase();
    // Split "5km" into "5" and "km".
    let number_end = lower
        .char_indices()
        .find(|(i, c)| !(c.is_ascii_digit() || *c == '.' || *c == ',' || (*i == 0 && *c == '-')))
        .map(|(i, _)| i)
        .unwrap_or(lower.len());
    let (number, rest) = lower.split_at(number_end);
    let amount: f64 = number.replace(',', ".").parse().ok()?;

    let words: Vec<&str> = rest.split_whitespace().collect();
    let separator = words
        .iter()
        .skip(1)
        .position(|w| matches!(*w, "in" | "to" | "as" | "=" | "->"))?
        + 1;
    // "square meters" style names are written without the space.
    let from = words[..separator].concat();
    let to = words[separator + 1..].concat();
    if from.is_empty() || to.is_empty() {
        return None;
    }
    Some(Request { amount, from, to })
}

/// Up to six significant digits, without trailing zeros.
pub fn format_number(value: f64) -> String {
    if value == 0.0 || !value.is_finite() {
        return "0".to_string();
    }
    let magnitude = value.abs().log10().floor() as i32;
    if !(-5..15).contains(&magnitude) {
        return format!("{:.5e}", value);
    }
    let decimals = (5 - magnitude).clamp(0, 10) as usize;
    let text = format!("{:.*}", decimals, value);
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

/// Convert between two physical units. `None` when either is unknown or
/// they measure different things.
pub fn convert(request: &Request) -> Option<String> {
    if let (Some(from), Some(to)) = (temperature(&request.from), temperature(&request.to)) {
        let kelvin = match from {
            'c' => request.amount + 273.15,
            'f' => (request.amount - 32.0) * 5.0 / 9.0 + 273.15,
            _ => request.amount,
        };
        let (value, symbol) = match to {
            'c' => (kelvin - 273.15, "°C"),
            'f' => ((kelvin - 273.15) * 9.0 / 5.0 + 32.0, "°F"),
            _ => (kelvin, "K"),
        };
        return Some(format!("{} {}", format_number(value), symbol));
    }

    let (from_kind, from_factor, _) = lookup(&request.from)?;
    let (to_kind, to_factor, symbol) = lookup(&request.to)?;
    if from_kind != to_kind {
        return None;
    }
    Some(format!(
        "{} {}",
        format_number(request.amount * from_factor / to_factor),
        symbol
    ))
}

/// Exchange rates against the euro, as published by the European Central
/// Bank (via frankfurter.dev). Fetched at most twice a day and cached.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Rates {
    pub date: String,
    /// Units of each currency per euro; includes EUR itself.
    per_euro: HashMap<String, f64>,
}

const RATES_URL: &str = "https://api.frankfurter.dev/v1/latest?base=EUR";
const RATES_MAX_AGE_SECS: u64 = 12 * 3600;

impl Rates {
    pub fn parse(json: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(json).ok()?;
        let mut per_euro: HashMap<String, f64> = value
            .get("rates")?
            .as_object()?
            .iter()
            .filter_map(|(code, rate)| Some((code.to_lowercase(), rate.as_f64()?)))
            .collect();
        if per_euro.is_empty() {
            return None;
        }
        per_euro.insert("eur".to_string(), 1.0);
        Some(Self {
            date: value
                .get("date")
                .and_then(|d| d.as_str())
                .unwrap_or_default()
                .to_string(),
            per_euro,
        })
    }

    pub fn convert(&self, request: &Request) -> Option<String> {
        let from = self.per_euro.get(&request.from)?;
        let to = self.per_euro.get(&request.to)?;
        let value = request.amount / from * to;
        Some(format!("{:.2} {}", value, request.to.to_uppercase()))
    }

    fn cache_path() -> Option<PathBuf> {
        dirs::cache_dir().map(|dir| dir.join("poppi_launcher").join("rates.json"))
    }

    /// Rates from the cache, and whether they are due for a refresh.
    pub fn load_cached() -> (Option<Self>, bool) {
        let Some(path) = Self::cache_path() else {
            return (None, true);
        };
        let rates = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| Self::parse(&text));
        let fresh = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age.as_secs() < RATES_MAX_AGE_SECS);
        let stale = rates.is_none() || !fresh;
        (rates, stale)
    }

    /// Download current rates with curl and cache them. Blocking.
    pub fn fetch() -> Result<Self, String> {
        let output = std::process::Command::new("curl")
            .args(["--silent", "--fail", "--max-time", "8", RATES_URL])
            .output()
            .map_err(|e| format!("Could not run curl: {}", e))?;
        if !output.status.success() {
            return Err("Could not download exchange rates".to_string());
        }
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let rates = Self::parse(&text).ok_or("Unexpected exchange rate data")?;
        if let Some(path) = Self::cache_path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, text);
        }
        Ok(rates)
    }
}

/// True when both names look like currency codes rather than units.
pub fn looks_like_currency(request: &Request) -> bool {
    let is_code = |name: &str| name.len() == 3 && name.chars().all(|c| c.is_ascii_lowercase());
    is_code(&request.from)
        && is_code(&request.to)
        && lookup(&request.from).is_none()
        && lookup(&request.to).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(query: &str) -> Option<String> {
        convert(&parse(query)?)
    }

    #[test]
    fn parses_the_request_shape() {
        assert_eq!(
            parse("5 km in miles"),
            Some(Request { amount: 5.0, from: "km".into(), to: "miles".into() })
        );
        assert_eq!(parse("5km to mi").unwrap().from, "km");
        assert_eq!(parse("2,5 kg as lb").unwrap().amount, 2.5);
        assert_eq!(parse("-40 f to c").unwrap().amount, -40.0);
        assert_eq!(parse("10 cm in in").unwrap().to, "in");
        assert_eq!(parse("3 sq ft to m2").unwrap().from, "sqft");
        assert_eq!(parse("firefox"), None);
        assert_eq!(parse("5 km"), None);
        assert_eq!(parse("5 in"), None);
        assert_eq!(parse("km in miles"), None);
    }

    #[test]
    fn converts_units() {
        assert_eq!(conv("5 km in miles").unwrap(), "3.10686 mi");
        assert_eq!(conv("1 mile in km").unwrap(), "1.60934 km");
        assert_eq!(conv("10 cm in in").unwrap(), "3.93701 in");
        assert_eq!(conv("2 gb in mb").unwrap(), "2000 MB");
        assert_eq!(conv("1 gib in mib").unwrap(), "1024 MiB");
        assert_eq!(conv("90 min in h").unwrap(), "1.5 h");
        assert_eq!(conv("100 km/h in mph").unwrap(), "62.1371 mph");
        assert_eq!(conv("1 kg in lb").unwrap(), "2.20462 lb");
        assert_eq!(conv("1 gal in l").unwrap(), "3.78541 l");
    }

    #[test]
    fn converts_temperatures() {
        assert_eq!(conv("100 c to f").unwrap(), "212 °F");
        assert_eq!(conv("-40 f to c").unwrap(), "-40 °C");
        assert_eq!(conv("0 c to k").unwrap(), "273.15 K");
    }

    #[test]
    fn mismatched_or_unknown_units_are_rejected() {
        assert_eq!(conv("5 km in kg"), None);
        assert_eq!(conv("5 blorps in km"), None);
        assert_eq!(conv("5 c in km"), None);
    }

    #[test]
    fn number_formatting() {
        assert_eq!(format_number(1234567.0), "1234567");
        assert_eq!(format_number(0.000123456789), "0.000123457");
        assert_eq!(format_number(2.50), "2.5");
        assert_eq!(format_number(0.0), "0");
    }

    #[test]
    fn currencies() {
        let rates = Rates::parse(
            r#"{"amount":1.0,"base":"EUR","date":"2026-11-10","rates":{"USD":1.25,"GBP":0.8}}"#,
        )
        .unwrap();
        assert_eq!(rates.date, "2026-11-10");

        let usd_eur = parse("100 usd in eur").unwrap();
        assert!(looks_like_currency(&usd_eur));
        assert_eq!(rates.convert(&usd_eur).unwrap(), "80.00 EUR");
        assert_eq!(rates.convert(&parse("16 gbp to usd").unwrap()).unwrap(), "25.00 USD");
        assert_eq!(rates.convert(&parse("10 xyz to usd").unwrap()), None);

        // Three-letter unit names are units, not currencies.
        assert!(!looks_like_currency(&parse("5 cup in tsp").unwrap()));
        assert!(Rates::parse("{}").is_none());
    }
}
