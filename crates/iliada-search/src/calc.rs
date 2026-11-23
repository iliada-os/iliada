//! Answers typed into the search: arithmetic ("2^10 / 3", "15% of 80"),
//! units ("5 km in miles", "100 f to c", "3 gb in mib") and dates ("days
//! until christmas", "10 days from now", "today").

use chrono::{Datelike, Duration, Local, NaiveDate};

/// The answer to `q`, when it is a sum, a conversion or a date question:
/// (what is shown, what Enter copies).
pub fn answer(q: &str) -> Option<(String, String)> {
    let q = q.trim();
    if q.is_empty() || q.len() > 200 {
        return None;
    }
    let lower = q.to_lowercase();
    if let Some(a) = convert(&lower) {
        return Some(a);
    }
    if let Some(a) = date(&lower) {
        return Some(a);
    }
    let v = arithmetic(&lower)?;
    let s = number(v);
    Some((format!("= {s}"), s))
}

/// A number as people write it: no needless decimals or exponent.
pub fn number(v: f64) -> String {
    if v.is_nan() {
        return "not a number".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "∞".into() } else { "-∞".into() };
    }
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{v:.0}");
    }
    if v.abs() >= 1e15 || v.abs() < 1e-6 {
        return format!("{v:e}");
    }
    let s = format!("{v:.10}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

// ---- arithmetic ----

/// The value of `q`, when it is a sum (not just a number: typing "2025"
/// searches for 2025).
pub fn arithmetic(q: &str) -> Option<f64> {
    // A date (2026-11-23) is searched for, not subtracted.
    let b = q.as_bytes();
    if b.len() == 10 && b[4] == b'-' && b[7] == b'-' && q.replace('-', "").bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let q = q.replace('×', "*").replace('÷', "/").replace(',', "");
    // "15% of 80"
    if let Some((p, of)) = q.split_once("% of ") {
        return Some(arithmetic_any(p)? / 100.0 * arithmetic_any(of)?);
    }
    let has_op = q.trim_start_matches('-').contains(['+', '-', '*', '/', '^', '(', '%'])
        || FUNCTIONS.iter().any(|f| q.contains(&format!("{f}(")))
        || q.split(|c: char| !c.is_alphabetic()).any(|w| w == "pi");
    if !has_op || !q.chars().any(|c| c.is_ascii_digit() || c == 'π') && !q.contains("pi") {
        return None;
    }
    arithmetic_any(&q)
}

fn arithmetic_any(q: &str) -> Option<f64> {
    let mut p = Parser { s: q.as_bytes(), i: 0 };
    let v = p.expr()?;
    p.ws();
    (p.i == p.s.len() && v.is_finite()).then_some(v)
}

const FUNCTIONS: [&str; 10] = ["sqrt", "sin", "cos", "tan", "ln", "log", "abs", "round", "floor", "ceil"];

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        loop {
            if self.eat(b'+') {
                v += self.term()?;
            } else if self.eat(b'-') {
                v -= self.term()?;
            } else {
                return Some(v);
            }
        }
    }
    fn term(&mut self) -> Option<f64> {
        let mut v = self.power()?;
        loop {
            if self.eat(b'*') || self.eat(b'x') {
                v *= self.power()?;
            } else if self.eat(b'/') {
                v /= self.power()?;
            } else {
                return Some(v);
            }
        }
    }
    fn power(&mut self) -> Option<f64> {
        let base = self.unary()?;
        if self.eat(b'^') {
            // Right to left: 2^3^2 = 2^9.
            let exp = self.power()?;
            return Some(base.powf(exp));
        }
        Some(base)
    }
    fn unary(&mut self) -> Option<f64> {
        if self.eat(b'-') {
            return Some(-self.unary()?);
        }
        if self.eat(b'+') {
            return self.unary();
        }
        let mut v = self.primary()?;
        // Postfix percent: "20%" is 0.2.
        while self.eat(b'%') {
            v /= 100.0;
        }
        Some(v)
    }
    fn primary(&mut self) -> Option<f64> {
        self.ws();
        if self.eat(b'(') {
            let v = self.expr()?;
            return self.eat(b')').then_some(v);
        }
        let start = self.i;
        if self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || *c == b'.') {
            while self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || *c == b'.') {
                self.i += 1;
            }
            // 1e3
            if self.s.get(self.i) == Some(&b'e') && self.s.get(self.i + 1).is_some_and(u8::is_ascii_digit) {
                self.i += 1;
                while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
                    self.i += 1;
                }
            }
            return std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok();
        }
        if self.s[self.i..].starts_with("π".as_bytes()) {
            self.i += "π".len();
            return Some(std::f64::consts::PI);
        }
        while self.s.get(self.i).is_some_and(u8::is_ascii_alphabetic) {
            self.i += 1;
        }
        let word = std::str::from_utf8(&self.s[start..self.i]).ok()?;
        match word {
            "pi" => Some(std::f64::consts::PI),
            "e" => Some(std::f64::consts::E),
            f if FUNCTIONS.contains(&f) => {
                if !self.eat(b'(') {
                    return None;
                }
                let x = self.expr()?;
                if !self.eat(b')') {
                    return None;
                }
                Some(match f {
                    "sqrt" => x.sqrt(),
                    "sin" => x.sin(),
                    "cos" => x.cos(),
                    "tan" => x.tan(),
                    "ln" => x.ln(),
                    "log" => x.log10(),
                    "abs" => x.abs(),
                    "round" => x.round(),
                    "floor" => x.floor(),
                    _ => x.ceil(),
                })
            }
            _ => None,
        }
    }
}

// ---- units ----

/// (names, kind, how many base units one is).
const UNITS: &[(&[&str], &str, f64)] = &[
    (&["mm", "millimeter", "millimeters", "millimetre", "millimetres"], "length", 0.001),
    (&["cm", "centimeter", "centimeters", "centimetre", "centimetres"], "length", 0.01),
    (&["m", "meter", "meters", "metre", "metres"], "length", 1.0),
    (&["km", "kilometer", "kilometers", "kilometre", "kilometres"], "length", 1000.0),
    (&["in", "inch", "inches", "\""], "length", 0.0254),
    (&["ft", "foot", "feet", "'"], "length", 0.3048),
    (&["yd", "yard", "yards"], "length", 0.9144),
    (&["mi", "mile", "miles"], "length", 1609.344),
    (&["nmi", "nautical mile", "nautical miles"], "length", 1852.0),
    (&["mg", "milligram", "milligrams"], "mass", 1e-6),
    (&["g", "gram", "grams"], "mass", 0.001),
    (&["kg", "kilo", "kilos", "kilogram", "kilograms"], "mass", 1.0),
    (&["t", "tonne", "tonnes", "ton", "tons"], "mass", 1000.0),
    (&["oz", "ounce", "ounces"], "mass", 0.028_349_523_125),
    (&["lb", "lbs", "pound", "pounds"], "mass", 0.453_592_37),
    (&["st", "stone", "stones"], "mass", 6.350_293_18),
    (&["ml", "milliliter", "milliliters", "millilitre", "millilitres"], "volume", 0.001),
    (&["l", "liter", "liters", "litre", "litres"], "volume", 1.0),
    (&["gal", "gallon", "gallons"], "volume", 3.785_411_784),
    (&["qt", "quart", "quarts"], "volume", 0.946_352_946),
    (&["pt", "pint", "pints"], "volume", 0.473_176_473),
    (&["cup", "cups"], "volume", 0.236_588_236_5),
    (&["floz", "fl oz", "fluid ounce", "fluid ounces"], "volume", 0.029_573_529_562_5),
    (&["tbsp", "tablespoon", "tablespoons"], "volume", 0.014_786_764_781_25),
    (&["tsp", "teaspoon", "teaspoons"], "volume", 0.004_928_921_593_75),
    (&["ms", "millisecond", "milliseconds"], "time", 0.001),
    (&["s", "sec", "secs", "second", "seconds"], "time", 1.0),
    (&["min", "mins", "minute", "minutes"], "time", 60.0),
    (&["h", "hr", "hrs", "hour", "hours"], "time", 3600.0),
    (&["d", "day", "days"], "time", 86400.0),
    (&["wk", "week", "weeks"], "time", 604_800.0),
    (&["yr", "year", "years"], "time", 31_557_600.0),
    (&["b", "byte", "bytes"], "data", 1.0),
    (&["kb", "kilobyte", "kilobytes"], "data", 1e3),
    (&["mb", "megabyte", "megabytes"], "data", 1e6),
    (&["gb", "gigabyte", "gigabytes"], "data", 1e9),
    (&["tb", "terabyte", "terabytes"], "data", 1e12),
    (&["kib"], "data", 1024.0),
    (&["mib"], "data", 1_048_576.0),
    (&["gib"], "data", 1_073_741_824.0),
    (&["tib"], "data", 1_099_511_627_776.0),
    (&["m/s", "mps"], "speed", 1.0),
    (&["km/h", "kmh", "kph"], "speed", 1.0 / 3.6),
    (&["mph"], "speed", 0.447_04),
    (&["kn", "knot", "knots"], "speed", 0.514_444_444),
    (&["m2", "m²", "sq m", "square meter", "square meters"], "area", 1.0),
    (&["km2", "km²", "sq km", "square kilometer", "square kilometers"], "area", 1e6),
    (&["ft2", "ft²", "sq ft", "sqft", "square foot", "square feet"], "area", 0.092_903_04),
    (&["ha", "hectare", "hectares"], "area", 1e4),
    (&["acre", "acres"], "area", 4_046.856_422_4),
    (&["c", "°c", "celsius"], "temperature", 0.0),
    (&["f", "°f", "fahrenheit"], "temperature", 0.0),
    (&["k", "kelvin"], "temperature", 0.0),
];

fn unit(name: &str) -> Option<(&'static str, &'static str, f64)> {
    let name = name.trim();
    UNITS.iter().find(|(names, ..)| names.contains(&name)).map(|(names, kind, f)| (names[0], *kind, *f))
}

fn to_kelvin(v: f64, u: &str) -> f64 {
    match u {
        "c" => v + 273.15,
        "f" => (v - 32.0) * 5.0 / 9.0 + 273.15,
        _ => v,
    }
}

fn from_kelvin(v: f64, u: &str) -> f64 {
    match u {
        "c" => v - 273.15,
        "f" => (v - 273.15) * 9.0 / 5.0 + 32.0,
        _ => v,
    }
}

/// "5 km in miles", "100f to c", "3.5 gb as mib".
fn convert(q: &str) -> Option<(String, String)> {
    let (left, to) = [" in ", " to ", " as ", " = "].iter().find_map(|sep| q.rsplit_once(sep))?;
    let left = left.trim();
    let split = left.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == ',' || c == ' '))?;
    let (num, from) = left.split_at(split);
    let v: f64 = num.replace([',', ' '], "").parse().ok()?;
    let (fu, fk, ff) = unit(from)?;
    let (tu, tk, tf) = unit(to)?;
    if fk != tk {
        return None;
    }
    let r = if fk == "temperature" { from_kelvin(to_kelvin(v, fu), tu) } else { v * ff / tf };
    // Rounded for reading; Enter copies the same.
    let r = (r * 1e6).round() / 1e6;
    let shown = format!("{} {}", number(r), display_unit(tu));
    Some((format!("= {shown}"), number(r)))
}

fn display_unit(u: &str) -> &str {
    match u {
        "c" => "°C",
        "f" => "°F",
        "k" => "K",
        "kb" => "kB",
        "mb" => "MB",
        "gb" => "GB",
        "tb" => "TB",
        "kib" => "KiB",
        "mib" => "MiB",
        "gib" => "GiB",
        "tib" => "TiB",
        "b" => "B",
        u => u,
    }
}

// ---- dates ----

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

fn month(w: &str) -> Option<u32> {
    if w.len() < 3 {
        return None;
    }
    MONTHS.iter().position(|m| m.starts_with(w)).map(|i| i as u32 + 1)
}

/// A date as people type it: 2026-12-25, 25 december [2026], dec 25,
/// christmas, new year, tomorrow. Dates without a year are the next one.
fn parse_date(s: &str, today: NaiveDate) -> Option<NaiveDate> {
    let s = s.trim();
    match s {
        "today" => return Some(today),
        "tomorrow" => return today.succ_opt(),
        "yesterday" => return today.pred_opt(),
        _ => {}
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d);
    }
    let next = |m: u32, d: u32| {
        let this = NaiveDate::from_ymd_opt(today.year(), m, d)?;
        if this >= today { Some(this) } else { NaiveDate::from_ymd_opt(today.year() + 1, m, d) }
    };
    match s {
        "christmas" | "xmas" => return next(12, 25),
        "new year" | "new year's day" | "new years day" => return next(1, 1),
        "new year's eve" | "new years eve" => return next(12, 31),
        _ => {}
    }
    let words: Vec<&str> = s.split([' ', ',']).filter(|w| !w.is_empty()).collect();
    let (day, mon, year) = match words.as_slice() {
        [a, b] => match (a.parse::<u32>(), month(a)) {
            (Ok(d), _) => (d, month(b)?, None),
            (_, Some(m)) => (b.parse().ok()?, m, None),
            _ => return None,
        },
        [a, b, y] => match (a.parse::<u32>(), month(a)) {
            (Ok(d), _) => (d, month(b)?, Some(y.parse().ok()?)),
            (_, Some(m)) => (b.parse().ok()?, m, Some(y.parse().ok()?)),
            _ => return None,
        },
        _ => return None,
    };
    match year {
        Some(y) => NaiveDate::from_ymd_opt(y, mon, day),
        None => next(mon, day),
    }
}

fn long(d: NaiveDate) -> String {
    d.format("%A, %-d %B %Y").to_string()
}

fn date(q: &str) -> Option<(String, String)> {
    date_at(q, Local::now().naive_local())
}

fn date_at(q: &str, now: chrono::NaiveDateTime) -> Option<(String, String)> {
    let today = now.date();
    match q {
        "today" | "date" | "what day is it" => {
            let s = long(today);
            return Some((s.clone(), s));
        }
        "now" | "time" | "what time is it" => {
            let s = now.format("%H:%M").to_string();
            return Some((format!("{s}, {}", long(today)), s));
        }
        _ => {}
    }
    // days until christmas / how many days until 2027-01-01
    if let Some(rest) = q
        .strip_prefix("how many ")
        .unwrap_or(q)
        .strip_prefix("days until ")
        .or_else(|| q.strip_prefix("how many ").unwrap_or(q).strip_prefix("days to "))
    {
        let d = parse_date(rest, today)?;
        let n = (d - today).num_days();
        let s = format!("{n} day{}", if n.abs() == 1 { "" } else { "s" });
        return Some((format!("{s} until {}", long(d)), n.to_string()));
    }
    // in 10 days / 3 weeks from now / 2 months ago / 10 days from tomorrow
    let (n, unit, base, sign) = {
        let (body, base, sign) = if let Some(b) = q.strip_prefix("in ") {
            (b, today, 1)
        } else if let Some(b) = q.strip_suffix(" ago") {
            (b, today, -1)
        } else if let Some((b, from)) = q.split_once(" from ").or_else(|| q.split_once(" after ")) {
            let base = if from == "now" { today } else { parse_date(from, today)? };
            (b, base, 1)
        } else if let Some((b, from)) = q.split_once(" before ") {
            (b, parse_date(from, today)?, -1)
        } else {
            return None;
        };
        let (n, unit) = body.trim().split_once(' ')?;
        (n.parse::<i64>().ok()?, unit.trim(), base, sign)
    };
    let n = n * sign;
    let d = match unit {
        "day" | "days" => base + Duration::days(n),
        "week" | "weeks" => base + Duration::weeks(n),
        "month" | "months" => {
            if n >= 0 {
                base.checked_add_months(chrono::Months::new(n as u32))?
            } else {
                base.checked_sub_months(chrono::Months::new((-n) as u32))?
            }
        }
        "year" | "years" => {
            if n >= 0 {
                base.checked_add_months(chrono::Months::new(12 * n as u32))?
            } else {
                base.checked_sub_months(chrono::Months::new(12 * (-n) as u32))?
            }
        }
        _ => return None,
    };
    let s = long(d);
    Some((s.clone(), d.format("%Y-%m-%d").to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(q: &str) -> Option<String> {
        answer(q).map(|(shown, _)| shown)
    }

    #[test]
    fn arithmetic() {
        assert_eq!(a("2+2").as_deref(), Some("= 4"));
        assert_eq!(a("2 ^ 10 / 4").as_deref(), Some("= 256"));
        assert_eq!(a("(1+2)*3 - -1").as_deref(), Some("= 10"));
        assert_eq!(a("15% of 80").as_deref(), Some("= 12"));
        assert_eq!(a("sqrt(16) + 1").as_deref(), Some("= 5"));
        assert_eq!(a("1/3").as_deref(), Some("= 0.3333333333"));
        assert_eq!(a("1,000 * 3").as_deref(), Some("= 3000"));
        assert_eq!(a("2 × 3 ÷ 4").as_deref(), Some("= 1.5"));
        assert_eq!(a("2*pi").as_deref(), Some("= 6.2831853072"));
        // Not sums: searched for instead.
        assert_eq!(a("2025"), None);
        assert_eq!(a("report-2025"), None);
        assert_eq!(a("firefox"), None);
        assert_eq!(a("1/0"), None);
        assert_eq!(a("2026-11-23"), None);
    }

    #[test]
    fn units() {
        assert_eq!(a("5 km in miles").as_deref(), Some("= 3.106856 mi"));
        assert_eq!(a("100f to c").as_deref(), Some("= 37.777778 °C"));
        assert_eq!(a("0 c in f").as_deref(), Some("= 32 °F"));
        assert_eq!(a("1 gib in mb").as_deref(), Some("= 1073.741824 MB"));
        assert_eq!(a("2 hours in minutes").as_deref(), Some("= 120 min"));
        assert_eq!(a("5 kg in km"), None);
        assert_eq!(answer("1 ft in cm").map(|(_, c)| c).as_deref(), Some("30.48"));
    }

    #[test]
    fn dates() {
        let now = NaiveDate::from_ymd_opt(2026, 11, 23).unwrap().and_hms_opt(14, 5, 0).unwrap();
        let d = |q: &str| date_at(q, now).map(|(s, _)| s);
        assert_eq!(d("days until christmas").as_deref(), Some("32 days until Friday, 25 December 2026"));
        assert_eq!(d("days until 2027-01-01").as_deref(), Some("39 days until Friday, 1 January 2027"));
        assert_eq!(d("days until 24 nov").as_deref(), Some("1 day until Tuesday, 24 November 2026"));
        assert_eq!(d("in 10 days").as_deref(), Some("Thursday, 3 December 2026"));
        assert_eq!(d("3 weeks from now").as_deref(), Some("Monday, 14 December 2026"));
        assert_eq!(d("2 months ago").as_deref(), Some("Wednesday, 23 September 2026"));
        assert_eq!(d("1 year from christmas").as_deref(), Some("Saturday, 25 December 2027"));
        assert_eq!(d("today").as_deref(), Some("Monday, 23 November 2026"));
        assert_eq!(d("now").as_deref(), Some("14:05, Monday, 23 November 2026"));
        assert_eq!(d("in the box"), None);
    }
}
